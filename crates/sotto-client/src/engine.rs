//! The messaging engine.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sotto_core::envelope::{CallSignal, Envelope, FileRef, Payload, Profile as WireProfile};
use sotto_core::invite::Invite;
use sotto_core::keys::{Identity, sign, signing_keypair};
use sotto_core::wire::{Cmd, Resp, auth_message};
use sotto_core::{Id, QueueAddr, hex, new_id, pqxdh, time};
use sotto_vault::Store;
use tokio::sync::{Mutex, broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::calls::{Call, CallView, Calls, Direction, EndReason, JoinInfo, Phase, RoomGrant};
use crate::conn::{Push, RelayConn, Status, Transport};
use crate::model::*;
use crate::{Error, Result, files};

const INVITE_TTL: u64 = 7 * 86_400;
const QUEUE_TTL_DAYS: u8 = 14;

#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Used only when no profile exists yet.
    pub default_relay: String,
    pub transport: Transport,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Contacts,
    Contact { id: String },
    Message { contact_id: String, id: String },
    MessageRemoved { contact_id: String, id: String },
    Relay { url: String, status: Status },
    /// A call changed state. Never carries join tokens or keys.
    Call { call: CallView },
    #[serde(skip)]
    Extension { contact_id: Id, payload: Payload },
}

struct Inner {
    store: Arc<Store>,
    profile: Mutex<Profile>,
    contacts: Mutex<HashMap<Id, Contact>>,
    relays: Mutex<HashMap<String, RelayConn>>,
    push_tx: mpsc::UnboundedSender<Push>,
    group_tx: Mutex<Option<mpsc::UnboundedSender<Push>>>,
    events: broadcast::Sender<Event>,
    cover: AtomicBool,
    /// Under Tor, remote relays must be onion services (no clearnet hop, no local probing).
    tor_only: AtomicBool,
    /// expires_at -> (store name, blob id)
    expiry: Mutex<BTreeMap<(u64, String), Option<String>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    calls: Mutex<Calls>,
}

#[derive(Clone)]
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    pub async fn start(store: Arc<Store>, config: EngineConfig) -> Result<Engine> {
        let profile = match store.get::<Profile>("profile")? {
            Some(p) => p,
            None => {
                crate::conn::validate_relay_url(&config.default_relay)?;
                let p = Profile {
                    identity: Identity::generate(),
                    display_name: String::new(),
                    default_relay: config.default_relay.clone(),
                    transport: config.transport.clone(),
                    created_at: time::now(),
                };
                store.put("profile", &p)?;
                p
            }
        };
        let mut contacts = HashMap::new();
        for name in store.names("contact/") {
            if let Some(c) = store.get::<Contact>(&name)? {
                contacts.insert(c.id, c);
            }
        }
        let mut expiry = BTreeMap::new();
        for name in store.names("msg/") {
            if let Some(m) = store.get::<Message>(&name)? {
                if let Some(e) = m.expires_at {
                    expiry.insert((e, name), blob_of(&m));
                }
            }
        }
        let (push_tx, push_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(512);
        let engine = Engine {
            inner: Arc::new(Inner {
                store,
                profile: Mutex::new(profile),
                contacts: Mutex::new(contacts),
                relays: Mutex::new(HashMap::new()),
                push_tx,
                group_tx: Mutex::new(None),
                events,
                cover: AtomicBool::new(false),
                tor_only: AtomicBool::new(false),
                expiry: Mutex::new(expiry),
                tasks: Mutex::new(Vec::new()),
                calls: Mutex::new(Calls::default()),
            }),
        };
        let tor = matches!(engine.inner.profile.lock().await.transport, Transport::Tor { .. });
        engine.inner.tor_only.store(tor, Ordering::Relaxed);
        let subs: Vec<MyQueue> = engine.inner.contacts.lock().await.values().map(|c| c.my_queue.clone()).collect();
        for q in subs {
            engine.relay(&q.relay).await.subscribe_queue(q.rcv_id, q.seed).await;
        }
        let looper = engine.clone();
        let handle = tokio::spawn(async move { looper.push_loop(push_rx).await });
        engine.inner.tasks.lock().await.push(handle);
        Ok(engine)
    }

    /// Stop background work. Relay connections close once their handles are dropped.
    pub async fn shutdown(&self) {
        for t in self.inner.tasks.lock().await.drain(..) {
            t.abort();
        }
        self.inner.relays.lock().await.clear();
    }

    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    fn emit(&self, e: Event) {
        let _ = self.inner.events.send(e);
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.inner.store
    }

    /// Route group (case) pushes to another component.
    pub async fn set_group_sink(&self, tx: mpsc::UnboundedSender<Push>) {
        *self.inner.group_tx.lock().await = Some(tx);
    }

    pub async fn relay(&self, url: &str) -> RelayConn {
        let mut relays = self.inner.relays.lock().await;
        if let Some(r) = relays.get(url) {
            return r.clone();
        }
        let transport = self.inner.profile.lock().await.transport.clone();
        let conn = RelayConn::spawn(url.to_string(), transport, self.inner.push_tx.clone());
        conn.set_cover(self.inner.cover.load(Ordering::Relaxed));
        let mut watch = conn.status_watch();
        let me = self.clone();
        let u = url.to_string();
        let h = tokio::spawn(async move {
            while watch.changed().await.is_ok() {
                let s = *watch.borrow();
                me.emit(Event::Relay { url: u.clone(), status: s });
            }
        });
        self.inner.tasks.lock().await.push(h);
        relays.insert(url.to_string(), conn.clone());
        conn
    }

    /// Check a relay address that came from someone else (invite, reply queue, case invite).
    pub fn check_remote_relay(&self, url: &str) -> Result<()> {
        crate::conn::validate_relay_url(url)?;
        if self.inner.tor_only.load(Ordering::Relaxed) {
            let host = url.trim_start_matches("ws://").split(['/', ':']).next().unwrap_or("");
            if !host.ends_with(".onion") {
                return Err(Error::Invalid("That relay isn't an onion service, so Sotto won't connect to it."));
            }
        }
        Ok(())
    }

    pub async fn relay_statuses(&self) -> Vec<(String, Status)> {
        self.inner.relays.lock().await.iter().map(|(u, c)| (u.clone(), c.status())).collect()
    }

    pub async fn set_cover_traffic(&self, on: bool) {
        self.inner.cover.store(on, Ordering::Relaxed);
        for r in self.inner.relays.lock().await.values() {
            r.set_cover(on);
        }
    }

    // ---------- Profile ----------

    pub async fn profile(&self) -> Profile {
        self.inner.profile.lock().await.clone()
    }

    pub async fn update_profile(&self, display_name: Option<String>, default_relay: Option<String>, transport: Option<Transport>) -> Result<()> {
        let mut p = self.inner.profile.lock().await;
        if let Some(n) = display_name {
            p.display_name = n.chars().take(80).collect();
        }
        if let Some(r) = default_relay {
            crate::conn::validate_relay_url(&r)?;
            p.default_relay = r;
        }
        if let Some(t) = transport {
            p.transport = t;
        }
        self.inner.store.put("profile", &*p)?;
        Ok(())
    }

    // ---------- Contacts ----------

    async fn new_queue(&self, relay: &str) -> Result<MyQueue> {
        let conn = self.relay(relay).await;
        let (seed, public) = signing_keypair();
        match conn.request(move |_| Cmd::New { rcv_auth: public, ttl_days: QUEUE_TTL_DAYS }).await? {
            Resp::Ids { rcv_id, snd_id } => Ok(MyQueue { relay: relay.to_string(), rcv_id, snd_id, seed }),
            _ => Err(Error::BadResponse),
        }
    }

    fn persist(&self, c: &Contact) -> Result<()> {
        self.inner.store.put(&contact_name(&c.id), c)?;
        Ok(())
    }

    /// Create a one-time invite. Returns the new contact id and the invite link.
    /// `listed` uses the profile identity (verifiable through an outlet directory);
    /// otherwise a fresh identity is made for this contact alone.
    pub async fn create_invite(&self, nickname: &str, label: &str, listed: bool) -> Result<(Id, String)> {
        let profile = self.profile().await;
        let queue = self.new_queue(&profile.default_relay).await?;
        let identity = if listed { profile.identity.clone() } else { Identity::generate() };
        let addr = QueueAddr { relay: queue.relay.clone(), snd_id: queue.snd_id };
        let (invite, secret) = Invite::create(&identity, addr, label, INVITE_TTL)?;
        let now = time::now();
        let contact = Contact {
            id: new_id(),
            nickname: nickname.chars().take(80).collect(),
            their_name: String::new(),
            my_identity: identity,
            listed_identity: listed,
            their_identity: None,
            ratchet: None,
            my_queue: queue.clone(),
            their_queue: None,
            state: ContactState::Invited,
            verified: false,
            ttl_secs: Some(7 * 86_400),
            created_at: now,
            last_activity: now,
            unread: 0,
            invite_id: Some(invite.id()),
            invite_expires: invite.expires_at,
            pinned: false,
            archived: false,
            read_receipts: false,
            preview: "Invite waiting".into(),
            preview_at: now,
            preview_outgoing: true,
        };
        self.inner.store.put(&invite_name(&invite.id()), &PendingInvite { secret, contact_id: contact.id })?;
        self.persist(&contact)?;
        let id = contact.id;
        self.inner.contacts.lock().await.insert(id, contact);
        self.relay(&queue.relay).await.subscribe_queue(queue.rcv_id, queue.seed).await;
        self.emit(Event::Contacts);
        Ok((id, invite.to_link()?))
    }

    /// Use someone else's invite. `share_name` is what they will see (may be empty).
    pub async fn accept_invite(&self, link: &str, nickname: &str, share_name: &str) -> Result<Id> {
        let invite = Invite::from_link(link)?;
        invite.verify(time::now())?;
        if self.inner.store.get::<PendingInvite>(&invite_name(&invite.id()))?.is_some() {
            return Err(Error::OwnInvite);
        }
        self.check_remote_relay(&invite.queue.relay)?;
        let profile = self.profile().await;
        let queue = self.new_queue(&profile.default_relay).await?;
        let identity = Identity::generate();
        let (header, mut ratchet) = pqxdh::join(&identity, &invite)?;
        let hello = Payload::Hello {
            reply_queue: QueueAddr { relay: queue.relay.clone(), snd_id: queue.snd_id },
            profile: WireProfile { name: share_name.chars().take(80).collect() },
        };
        let msg = ratchet.encrypt(&hello.to_bytes()?)?;
        let blob = Envelope::Join { header, msg }.to_blob()?;
        let target = self.relay(&invite.queue.relay).await;
        let snd_id = invite.queue.snd_id;
        target.request(move |_| Cmd::Send { snd_id, blob }).await?;
        let now = time::now();
        let contact = Contact {
            id: new_id(),
            nickname: if nickname.is_empty() { invite.label.clone() } else { nickname.chars().take(80).collect() },
            their_name: invite.label.clone(),
            my_identity: identity,
            listed_identity: false,
            their_identity: Some(invite.identity),
            ratchet: Some(ratchet),
            my_queue: queue.clone(),
            their_queue: Some(invite.queue.clone()),
            state: ContactState::Ready,
            verified: false,
            ttl_secs: Some(7 * 86_400),
            created_at: now,
            last_activity: now,
            unread: 0,
            invite_id: None,
            invite_expires: 0,
            pinned: false,
            archived: false,
            read_receipts: false,
            preview: String::new(),
            preview_at: now,
            preview_outgoing: false,
        };
        self.persist(&contact)?;
        let id = contact.id;
        self.inner.contacts.lock().await.insert(id, contact);
        self.relay(&queue.relay).await.subscribe_queue(queue.rcv_id, queue.seed).await;
        self.emit(Event::Contacts);
        Ok(id)
    }

    pub async fn contacts(&self) -> Vec<ContactView> {
        let mut v: Vec<ContactView> = self.inner.contacts.lock().await.values().map(|c| c.view()).collect();
        v.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
        v
    }

    pub async fn contact(&self, id: &Id) -> Option<ContactView> {
        self.inner.contacts.lock().await.get(id).map(|c| c.view())
    }

    async fn update_contact(&self, id: &Id, f: impl FnOnce(&mut Contact)) -> Result<()> {
        let mut contacts = self.inner.contacts.lock().await;
        let c = contacts.get_mut(id).ok_or(Error::NotFound)?;
        f(c);
        self.persist(c)?;
        drop(contacts);
        self.emit(Event::Contact { id: hex(id) });
        Ok(())
    }

    pub async fn rename_contact(&self, id: &Id, nickname: &str) -> Result<()> {
        let n: String = nickname.chars().take(80).collect();
        self.update_contact(id, |c| c.nickname = n).await
    }

    pub async fn set_verified(&self, id: &Id, verified: bool) -> Result<()> {
        self.update_contact(id, |c| c.verified = verified).await
    }

    /// Mark everything read; sends read receipts only if they are on for this contact.
    pub async fn mark_read(&self, id: &Id) -> Result<()> {
        let receipts = self.inner.contacts.lock().await.get(id).map(|c| c.read_receipts).ok_or(Error::NotFound)?;
        let mut ids = Vec::new();
        for name in self.inner.store.names(&format!("msg/{}/", hex(id))) {
            if let Some(mut m) = self.inner.store.get::<Message>(&name)? {
                if !m.outgoing && !m.read && !matches!(m.body, Body::System(_)) {
                    m.read = true;
                    self.save_message(&m)?;
                    ids.push(m.id);
                }
            }
        }
        if receipts && !ids.is_empty() {
            let _ = self.send_payload(id, &Payload::Read { ids }).await;
        }
        self.update_contact(id, |c| c.unread = 0).await
    }

    pub async fn set_pinned(&self, id: &Id, on: bool) -> Result<()> {
        self.update_contact(id, |c| c.pinned = on).await
    }

    pub async fn set_archived(&self, id: &Id, on: bool) -> Result<()> {
        self.update_contact(id, |c| c.archived = on).await
    }

    pub async fn set_read_receipts(&self, id: &Id, on: bool) -> Result<()> {
        self.update_contact(id, |c| c.read_receipts = on).await
    }

    async fn set_preview(&self, id: &Id, m: &Message) {
        let (preview, at, out) = (m.preview(), m.received_at / 1000, m.outgoing);
        let _ = self
            .update_contact(id, |c| {
                c.preview = preview;
                c.preview_at = at;
                c.preview_outgoing = out;
            })
            .await;
    }

    /// React to a message (yours or theirs). One reaction per emoji per side.
    pub async fn react(&self, contact: &Id, message: &Id, emoji: &str, on: bool) -> Result<()> {
        let emoji: String = emoji.chars().take(8).collect();
        if emoji.is_empty() {
            return Err(Error::Invalid("Choose a reaction."));
        }
        let mut m = self.find_message(contact, message).ok_or(Error::NotFound)?;
        m.reactions.retain(|r| !(r.mine && r.emoji == emoji));
        if on {
            m.reactions.push(Reaction { emoji: emoji.clone(), mine: true });
        }
        self.save_message(&m)?;
        self.emit(Event::Message { contact_id: hex(contact), id: hex(message) });
        self.send_payload(contact, &Payload::Reaction { target: *message, emoji, on }).await
    }

    /// Edit one of your own text messages, on both sides.
    pub async fn edit(&self, contact: &Id, message: &Id, body: &str) -> Result<()> {
        let body = body.trim_end().to_string();
        if body.is_empty() {
            return Err(Error::Invalid("A message can't be empty. Delete it instead."));
        }
        let mut m = self.find_message(contact, message).ok_or(Error::NotFound)?;
        if !m.outgoing || !matches!(m.body, Body::Text(_)) {
            return Err(Error::Invalid("You can only edit your own text messages."));
        }
        m.body = Body::Text(body.clone());
        m.edited = true;
        self.save_message(&m)?;
        self.emit(Event::Message { contact_id: hex(contact), id: hex(message) });
        self.send_payload(contact, &Payload::Edit { target: *message, body }).await
    }

    /// Change the disappearing-message timer for both sides.
    pub async fn set_timer(&self, id: &Id, ttl_secs: Option<u32>) -> Result<()> {
        let ready = self.inner.contacts.lock().await.get(id).map(|c| c.state == ContactState::Ready).ok_or(Error::NotFound)?;
        if ready {
            self.send_payload(id, &Payload::Timer { ttl_secs }).await?;
        }
        self.update_contact(id, |c| c.ttl_secs = ttl_secs).await?;
        self.add_system(id, timer_text(ttl_secs, true)).await
    }

    /// Delete a contact, its queue on the relay and every message and file.
    pub async fn delete_contact(&self, id: &Id) -> Result<()> {
        let c = self.inner.contacts.lock().await.remove(id).ok_or(Error::NotFound)?;
        let conn = self.relay(&c.my_queue.relay).await;
        conn.unsubscribe_queue(&c.my_queue.rcv_id).await;
        let (rcv_id, seed) = (c.my_queue.rcv_id, c.my_queue.seed);
        let _ = conn.request(move |n| Cmd::Del { rcv_id, sig: sign(&seed, &auth_message(n, "del", &[&rcv_id])) }).await;
        for name in self.inner.store.names(&format!("msg/{}/", hex(id))) {
            if let Some(m) = self.inner.store.get::<Message>(&name)? {
                self.remove_message_files(&m)?;
            }
            self.inner.store.delete(&name)?;
        }
        if let Some(inv) = c.invite_id {
            self.inner.store.delete(&invite_name(&inv))?;
        }
        self.inner.store.delete(&contact_name(id))?;
        self.emit(Event::Contacts);
        Ok(())
    }

    // ---------- Sending ----------

    /// Encrypt and send any payload to a ready contact.
    pub async fn send_payload(&self, id: &Id, payload: &Payload) -> Result<()> {
        let bytes = payload.to_bytes()?;
        let (relay, snd_id, blob) = {
            let mut contacts = self.inner.contacts.lock().await;
            let c = contacts.get_mut(id).ok_or(Error::NotFound)?;
            let q = c.their_queue.clone().ok_or(Error::NotReady)?;
            let r = c.ratchet.as_mut().ok_or(Error::NotReady)?;
            if !r.can_send() {
                return Err(Error::NotReady);
            }
            let m = r.encrypt(&bytes)?;
            let blob = Envelope::Msg(m).to_blob()?;
            c.last_activity = time::now();
            self.persist(c)?;
            (q.relay, q.snd_id, blob)
        };
        self.relay(&relay).await.request(move |_| Cmd::Send { snd_id, blob }).await?;
        Ok(())
    }

    fn save_message(&self, m: &Message) -> Result<()> {
        self.inner.store.put(&m.store_name(), m)?;
        Ok(())
    }

    async fn track_expiry(&self, m: &Message) {
        if let Some(e) = m.expires_at {
            self.inner.expiry.lock().await.insert((e, m.store_name()), blob_of(m));
        }
    }

    async fn ttl_of(&self, id: &Id) -> Result<Option<u32>> {
        self.inner.contacts.lock().await.get(id).map(|c| c.ttl_secs).ok_or(Error::NotFound)
    }

    pub async fn send_text(&self, id: &Id, body: &str, reply_to: Option<Id>) -> Result<MessageView> {
        let body = body.trim_end().to_string();
        if body.is_empty() {
            return Err(Error::Invalid("Write a message first."));
        }
        if body.len() > 12_000 {
            return Err(Error::Invalid("That message is too long. Send it as a file instead."));
        }
        let ttl = self.ttl_of(id).await?;
        let now_ms = time::now_ms();
        let mut m = Message {
            id: new_id(),
            contact_id: *id,
            outgoing: true,
            body: Body::Text(body.clone()),
            sent_at: now_ms,
            received_at: now_ms,
            expires_at: ttl.map(|t| time::now() + t as u64),
            delivery: Delivery::Sending,
            remote_file: None,
            reply_to,
            reactions: Vec::new(),
            edited: false,
            read: false,
        };
        self.save_message(&m)?;
        self.track_expiry(&m).await;
        self.set_preview(id, &m).await;
        let result = self.send_payload(id, &Payload::Text { id: m.id, body, sent_at: now_ms, ttl_secs: ttl, reply_to }).await;
        m.delivery = if result.is_ok() { Delivery::Sent } else { Delivery::Failed };
        self.save_message(&m)?;
        self.emit(Event::Message { contact_id: hex(id), id: hex(&m.id) });
        result.map(|_| m.view())
    }

    /// Resend a failed text message.
    pub async fn retry(&self, contact: &Id, message: &Id) -> Result<()> {
        let mut m = self.find_message(contact, message).ok_or(Error::NotFound)?;
        let Body::Text(body) = m.body.clone() else { return Err(Error::Invalid("Only text messages can be resent.")) };
        let ttl = self.ttl_of(contact).await?;
        let result = self.send_payload(contact, &Payload::Text { id: m.id, body, sent_at: m.sent_at, ttl_secs: ttl, reply_to: m.reply_to }).await;
        m.delivery = if result.is_ok() { Delivery::Sent } else { Delivery::Failed };
        self.save_message(&m)?;
        self.emit(Event::Message { contact_id: hex(contact), id: hex(&m.id) });
        result
    }

    /// Send a file read from `input`. A local copy is kept in the vault.
    pub async fn send_file(&self, id: &Id, name: &str, mime: &str, caption: &str, input: &mut (impl Read + Send)) -> Result<MessageView> {
        {
            let contacts = self.inner.contacts.lock().await;
            let c = contacts.get(id).ok_or(Error::NotFound)?;
            if c.state != ContactState::Ready {
                return Err(Error::NotReady);
            }
        }
        let ttl = self.ttl_of(id).await?;
        let relay_url = self.profile().await.default_relay;
        let conn = self.relay(&relay_url).await;
        let mid = new_id();
        let blob_id = format!("file/{}", hex(&mid));
        let file_key: [u8; 32] = sotto_core::rand::bytes();
        let mut writer = self.inner.store.blob_writer(&blob_id)?;
        let up = files::upload(&conn, &file_key, input, Some(&mut writer), 1).await?;
        writer.finish()?;
        let chunks = files::chunk_list(&conn, up.chunks, 1).await?;
        let name: String = sanitize_name(name);
        let file = FileRef {
            id: mid,
            name: name.clone(),
            mime: mime.chars().take(100).collect(),
            size: up.size,
            sha256: up.sha256,
            key: file_key,
            relay: relay_url,
            chunks,
        };
        let now_ms = time::now_ms();
        let mut m = Message {
            id: mid,
            contact_id: *id,
            outgoing: true,
            body: Body::File {
                meta: FileMeta {
                    name,
                    mime: file.mime.clone(),
                    size: up.size,
                    sha256: up.sha256,
                    blob_id: Some(blob_id),
                    status: FileStatus::Sent,
                    review: Review::Own,
                },
                caption: caption.chars().take(2000).collect(),
            },
            sent_at: now_ms,
            received_at: now_ms,
            expires_at: ttl.map(|t| time::now() + t as u64),
            delivery: Delivery::Sending,
            remote_file: None,
            reply_to: None,
            reactions: Vec::new(),
            edited: false,
            read: false,
        };
        let payload = Payload::File { id: mid, file, caption: caption.chars().take(2000).collect(), sent_at: now_ms, ttl_secs: ttl };
        let result = self.send_payload(id, &payload).await;
        m.delivery = if result.is_ok() { Delivery::Sent } else { Delivery::Failed };
        self.save_message(&m)?;
        self.track_expiry(&m).await;
        self.set_preview(id, &m).await;
        self.emit(Event::Message { contact_id: hex(id), id: hex(&mid) });
        result.map(|_| m.view())
    }

    // ---------- Reading ----------

    pub fn messages(&self, contact: &Id, limit: usize) -> Result<Vec<MessageView>> {
        let names = self.inner.store.names(&format!("msg/{}/", hex(contact)));
        let start = names.len().saturating_sub(limit);
        let mut out = Vec::new();
        for n in &names[start..] {
            if let Some(m) = self.inner.store.get::<Message>(n)? {
                out.push(m.view());
            }
        }
        Ok(out)
    }

    pub fn find_message(&self, contact: &Id, id: &Id) -> Option<Message> {
        let suffix = hex(id);
        let name = self.inner.store.names(&format!("msg/{}/", hex(contact))).into_iter().find(|n| n.ends_with(&suffix))?;
        self.inner.store.get::<Message>(&name).ok().flatten()
    }

    /// Update a file message's review state (used by safe review).
    pub fn set_review(&self, contact: &Id, id: &Id, review: Review) -> Result<()> {
        let mut m = self.find_message(contact, id).ok_or(Error::NotFound)?;
        if let Body::File { meta, .. } = &mut m.body {
            meta.review = review;
        }
        self.save_message(&m)?;
        self.emit(Event::Message { contact_id: hex(contact), id: hex(id) });
        Ok(())
    }

    pub async fn add_system(&self, contact: &Id, text: String) -> Result<()> {
        let now_ms = time::now_ms();
        let m = Message {
            id: new_id(),
            contact_id: *contact,
            outgoing: false,
            body: Body::System(text),
            sent_at: now_ms,
            received_at: now_ms,
            expires_at: None,
            delivery: Delivery::Received,
            remote_file: None,
            reply_to: None,
            reactions: Vec::new(),
            edited: false,
            read: false,
        };
        self.save_message(&m)?;
        self.emit(Event::Message { contact_id: hex(contact), id: hex(&m.id) });
        Ok(())
    }

    fn remove_message_files(&self, m: &Message) -> Result<()> {
        if let Body::File { meta, .. } = &m.body {
            if let Some(b) = &meta.blob_id {
                self.inner.store.delete_blob(b)?;
            }
            if let Review::Clean { clean_blob, .. } = &meta.review {
                self.inner.store.delete_blob(clean_blob)?;
            }
        }
        Ok(())
    }

    /// Delete one message locally, and ask the other side to delete it too.
    pub async fn delete_message(&self, contact: &Id, id: &Id, for_both: bool) -> Result<()> {
        let m = self.find_message(contact, id).ok_or(Error::NotFound)?;
        self.remove_message_files(&m)?;
        self.inner.store.delete(&m.store_name())?;
        if for_both {
            let _ = self.send_payload(contact, &Payload::Delete { ids: vec![*id] }).await;
        }
        self.emit(Event::MessageRemoved { contact_id: hex(contact), id: hex(id) });
        Ok(())
    }

    /// Remove messages whose disappearing timer has run out.
    pub async fn sweep_expired(&self) -> Result<usize> {
        let now = time::now();
        let due: Vec<((u64, String), Option<String>)> = {
            let mut exp = self.inner.expiry.lock().await;
            let keep = exp.split_off(&(now + 1, String::new()));
            std::mem::replace(&mut *exp, keep).into_iter().collect()
        };
        for ((_, name), _) in &due {
            if let Some(m) = self.inner.store.get::<Message>(name)? {
                self.remove_message_files(&m)?;
                self.inner.store.delete(name)?;
                self.emit(Event::MessageRemoved { contact_id: hex(&m.contact_id), id: hex(&m.id) });
            }
        }
        Ok(due.len())
    }

    // ---------- Receiving ----------

    async fn push_loop(&self, mut rx: mpsc::UnboundedReceiver<Push>) {
        while let Some(push) = rx.recv().await {
            match push {
                Push::Msg { relay, rcv_id, msg_id, blob } => self.on_queue_message(relay, rcv_id, msg_id, blob).await,
                g @ Push::GMsg { .. } => {
                    if let Some(tx) = self.inner.group_tx.lock().await.as_ref() {
                        let _ = tx.send(g);
                    }
                }
            }
        }
    }

    async fn on_queue_message(&self, relay: String, rcv_id: Id, msg_id: Id, blob: Vec<u8>) {
        let (contact_id, seed, payload) = {
            let mut contacts = self.inner.contacts.lock().await;
            let Some(c) = contacts.values_mut().find(|c| c.my_queue.relay == relay && c.my_queue.rcv_id == rcv_id) else { return };
            let seed = c.my_queue.seed;
            let id = c.id;
            let payload = self.open_envelope(c, &blob);
            if payload.is_ok() {
                c.last_activity = time::now();
                let _ = self.persist(c);
            }
            (id, seed, payload)
        };
        // State is saved before the acknowledgement, so a crash cannot lose a ratchet step.
        let _ = self.relay(&relay).await.ack(rcv_id, msg_id, seed).await;
        match payload {
            Ok(Some(p)) => {
                if let Err(_e) = self.on_payload(&contact_id, p).await {
                    // Payload handling errors are local (storage) and are surfaced through state, not logs.
                }
            }
            Ok(None) => {
                self.emit(Event::Contact { id: hex(&contact_id) });
                self.emit(Event::Contacts);
            }
            Err(_) => {}
        }
    }

    /// Decrypt an envelope for contact `c`, completing a handshake if this is a join.
    fn open_envelope(&self, c: &mut Contact, blob: &[u8]) -> Result<Option<Payload>> {
        match Envelope::from_blob(blob)? {
            Envelope::Join { header, msg } => {
                if c.state != ContactState::Invited || c.invite_id != Some(header.invite_id) {
                    return Err(Error::Invalid("unexpected join"));
                }
                if time::now() > c.invite_expires {
                    return Err(Error::Invalid("expired invite"));
                }
                let pending: PendingInvite = self.inner.store.get(&invite_name(&header.invite_id))?.ok_or(Error::NotFound)?;
                if pending.contact_id != c.id {
                    return Err(Error::Invalid("join for another contact"));
                }
                let mut ratchet = pqxdh::accept(&c.my_identity, &pending.secret, &header)?;
                let p = Payload::from_bytes(&ratchet.decrypt(&msg)?)?;
                let Payload::Hello { reply_queue, profile } = p else { return Err(Error::Invalid("join without hello")) };
                self.check_remote_relay(&reply_queue.relay)?;
                c.their_queue = Some(reply_queue);
                c.their_name = profile.name.chars().take(80).collect();
                c.their_identity = Some(header.identity);
                c.ratchet = Some(ratchet);
                c.state = ContactState::Ready;
                c.invite_id = None;
                self.inner.store.delete(&invite_name(&header.invite_id))?;
                Ok(None)
            }
            Envelope::Msg(m) => {
                let r = c.ratchet.as_mut().ok_or(Error::NotReady)?;
                Ok(Some(Payload::from_bytes(&r.decrypt(&m)?)?))
            }
        }
    }

    async fn on_payload(&self, cid: &Id, p: Payload) -> Result<()> {
        let now = time::now();
        let now_ms = time::now_ms();
        match p {
            Payload::Text { id, body, sent_at, ttl_secs, reply_to } => {
                let m = Message {
                    id,
                    contact_id: *cid,
                    outgoing: false,
                    body: Body::Text(body.chars().take(12_000).collect()),
                    sent_at,
                    received_at: now_ms,
                    expires_at: ttl_secs.map(|t| now + t as u64),
                    delivery: Delivery::Received,
                    remote_file: None,
                    reply_to,
                    reactions: Vec::new(),
                    edited: false,
                    read: false,
                };
                self.accept_incoming(cid, m).await?;
                let _ = self.send_payload(cid, &Payload::Receipt { ids: vec![id] }).await;
            }
            Payload::File { id, file, caption, sent_at, ttl_secs } => {
                let meta = FileMeta {
                    name: sanitize_name(&file.name),
                    mime: file.mime.chars().take(100).collect(),
                    size: file.size,
                    sha256: file.sha256,
                    blob_id: None,
                    status: FileStatus::Downloading,
                    review: Review::Quarantined,
                };
                let m = Message {
                    id,
                    contact_id: *cid,
                    outgoing: false,
                    body: Body::File { meta, caption: caption.chars().take(2000).collect() },
                    sent_at,
                    received_at: now_ms,
                    expires_at: ttl_secs.map(|t| now + t as u64),
                    delivery: Delivery::Received,
                    remote_file: Some(file),
                    reply_to: None,
                    reactions: Vec::new(),
                    edited: false,
                    read: false,
                };
                self.accept_incoming(cid, m).await?;
                let _ = self.send_payload(cid, &Payload::Receipt { ids: vec![id] }).await;
                let me = self.clone();
                let cid = *cid;
                tokio::spawn(async move {
                    let _ = me.download(&cid, &id).await;
                });
            }
            Payload::Reaction { target, emoji, on } => {
                if let Some(mut m) = self.find_message(cid, &target) {
                    let emoji: String = emoji.chars().take(8).collect();
                    m.reactions.retain(|r| r.mine || r.emoji != emoji);
                    if on && m.reactions.iter().filter(|r| !r.mine).count() < 20 {
                        m.reactions.push(Reaction { emoji, mine: false });
                    }
                    self.save_message(&m)?;
                    self.emit(Event::Message { contact_id: hex(cid), id: hex(&target) });
                }
            }
            Payload::Edit { target, body } => {
                if let Some(mut m) = self.find_message(cid, &target) {
                    if !m.outgoing && matches!(m.body, Body::Text(_)) {
                        m.body = Body::Text(body.chars().take(12_000).collect());
                        m.edited = true;
                        self.save_message(&m)?;
                        self.emit(Event::Message { contact_id: hex(cid), id: hex(&target) });
                    }
                }
            }
            Payload::Read { ids } => {
                for id in ids.into_iter().take(500) {
                    if let Some(mut m) = self.find_message(cid, &id) {
                        if m.outgoing && !m.read {
                            m.read = true;
                            self.save_message(&m)?;
                            self.emit(Event::Message { contact_id: hex(cid), id: hex(&id) });
                        }
                    }
                }
            }
            Payload::Receipt { ids } => {
                for id in ids {
                    if let Some(mut m) = self.find_message(cid, &id) {
                        if m.outgoing && m.delivery != Delivery::Delivered {
                            m.delivery = Delivery::Delivered;
                            self.save_message(&m)?;
                            self.emit(Event::Message { contact_id: hex(cid), id: hex(&id) });
                        }
                    }
                }
            }
            Payload::Delete { ids } => {
                for id in ids.into_iter().take(500) {
                    if let Some(m) = self.find_message(cid, &id) {
                        self.remove_message_files(&m)?;
                        self.inner.store.delete(&m.store_name())?;
                        self.emit(Event::MessageRemoved { contact_id: hex(cid), id: hex(&id) });
                    }
                }
            }
            Payload::Timer { ttl_secs } => {
                self.update_contact(cid, |c| c.ttl_secs = ttl_secs).await?;
                self.add_system(cid, timer_text(ttl_secs, false)).await?;
            }
            Payload::Profile(p) => {
                let n: String = p.name.chars().take(80).collect();
                self.update_contact(cid, |c| c.their_name = n).await?;
            }
            Payload::QueueRotate { queue } => {
                self.check_remote_relay(&queue.relay)?;
                self.update_contact(cid, |c| c.their_queue = Some(queue)).await?;
            }
            Payload::Hello { .. } => {}
            Payload::Call(signal) => self.on_call(cid, signal).await,
            other => self.emit(Event::Extension { contact_id: *cid, payload: other }),
        }
        Ok(())
    }

    async fn accept_incoming(&self, cid: &Id, m: Message) -> Result<()> {
        // A message id we already have is a duplicate delivery; keep the first.
        if self.find_message(cid, &m.id).is_some() {
            return Ok(());
        }
        self.save_message(&m)?;
        self.track_expiry(&m).await;
        let (preview, at) = (m.preview(), m.received_at / 1000);
        self.update_contact(cid, |c| {
            c.unread = c.unread.saturating_add(1);
            c.preview = preview;
            c.preview_at = at;
            c.preview_outgoing = false;
        })
        .await?;
        self.emit(Event::Message { contact_id: hex(cid), id: hex(&m.id) });
        Ok(())
    }

    // ---------- Calls ----------

    /// Shorten how long calls ring (tests).
    pub async fn set_ring_time(&self, d: std::time::Duration) {
        self.inner.calls.lock().await.ring = Some(d);
    }

    /// Calls still going on (at most one).
    pub async fn calls(&self) -> Vec<CallView> {
        self.inner.calls.lock().await.map.values().filter(|c| c.phase != Phase::Ended).map(|c| c.view(None)).collect()
    }

    async fn call_view(&self, call_id: &Id) -> Option<CallView> {
        self.inner.calls.lock().await.map.get(call_id).map(|c| c.view(None))
    }

    async fn call_phase(&self, call_id: &Id) -> Option<Phase> {
        self.inner.calls.lock().await.map.get(call_id).map(|c| c.phase)
    }

    async fn emit_call(&self, call_id: &Id) {
        if let Some(v) = self.call_view(call_id).await {
            self.emit(Event::Call { call: v });
        }
    }

    fn spawn_task(&self, fut: impl std::future::Future<Output = ()> + Send + 'static) {
        let h = tokio::spawn(fut);
        if let Ok(mut t) = self.inner.tasks.try_lock() {
            t.retain(|t| !t.is_finished());
            t.push(h);
        }
    }

    /// Start a call to a contact in a room the call broker created. The offer travels as an
    /// ordinary end-to-end encrypted message carrying the other side's join token and a fresh
    /// media key. The caller joins the room at once and waits there.
    pub async fn start_call(&self, contact: &Id, grant: RoomGrant) -> Result<CallView> {
        {
            let contacts = self.inner.contacts.lock().await;
            let c = contacts.get(contact).ok_or(Error::NotFound)?;
            if c.state != ContactState::Ready || c.ratchet.is_none() {
                return Err(Error::NotReady);
            }
        }
        let call_id = new_id();
        let key: [u8; 32] = sotto_core::rand::bytes();
        let ring = {
            let mut calls = self.inner.calls.lock().await;
            if calls.busy() {
                return Err(Error::Invalid("You're already on a call."));
            }
            let ring = calls.ring();
            calls.map.insert(
                call_id,
                Call {
                    id: call_id,
                    contact: *contact,
                    direction: Direction::Outgoing,
                    phase: Phase::Calling,
                    url: grant.url.clone(),
                    room: grant.room,
                    token: grant.my_token,
                    key,
                    started_ms: time::now_ms(),
                    answered_ms: None,
                    group: false,
                    invited: Vec::new(),
                },
            );
            ring
        };
        let offer = CallSignal::Offer {
            call_id,
            url: grant.url,
            room: self.call_room(&call_id).await.map(|r| r.1).unwrap_or_default(),
            token: grant.their_token,
            key,
            video: false,
            expires_at: time::now_ms() + ring.as_millis() as u64,
            group: false,
        };
        if let Err(e) = self.send_payload(contact, &Payload::Call(offer)).await {
            self.inner.calls.lock().await.map.remove(&call_id);
            return Err(e);
        }
        let view = self.call_view(&call_id).await.ok_or(Error::NotFound)?;
        self.emit(Event::Call { call: view.clone() });
        let (me, contact) = (self.clone(), *contact);
        self.spawn_task(async move {
            tokio::time::sleep(ring).await;
            if me.call_phase(&call_id).await == Some(Phase::Calling) {
                let _ = me.send_payload(&contact, &Payload::Call(CallSignal::Cancel { call_id })).await;
                me.finish_call(&call_id, EndReason::NoAnswer).await;
            }
        });
        Ok(view)
    }

    /// Answer a ringing call. The call window then joins with [`Engine::join_info`].
    pub async fn answer_call(&self, call_id: &Id) -> Result<CallView> {
        let contact = {
            let mut calls = self.inner.calls.lock().await;
            let c = calls.map.get_mut(call_id).ok_or(Error::NotFound)?;
            if c.phase != Phase::Ringing {
                return Err(Error::Invalid("This call isn't ringing any more."));
            }
            c.phase = Phase::Active;
            c.answered_ms = Some(time::now_ms());
            c.contact
        };
        // If this message is lost, the caller still sees us arrive in the room.
        let _ = self.send_payload(&contact, &Payload::Call(CallSignal::Answer { call_id: *call_id })).await;
        let v = self.call_view(call_id).await.ok_or(Error::NotFound)?;
        self.emit(Event::Call { call: v.clone() });
        Ok(v)
    }

    /// Decline a ringing call. Silent unless `say` is set ("can't talk now").
    pub async fn decline_call(&self, call_id: &Id, say: bool) -> Result<()> {
        let contact = {
            let calls = self.inner.calls.lock().await;
            let c = calls.map.get(call_id).ok_or(Error::NotFound)?;
            if c.phase != Phase::Ringing {
                return Err(Error::Invalid("This call isn't ringing any more."));
            }
            c.contact
        };
        if say {
            let _ = self.send_payload(&contact, &Payload::Call(CallSignal::Decline { call_id: *call_id })).await;
        }
        self.finish_call(call_id, EndReason::Declined).await;
        Ok(())
    }

    /// The call window saw more than one other person: from now on one person leaving doesn't end it.
    pub async fn mark_group_call(&self, call_id: &Id) -> Result<()> {
        let changed = {
            let mut calls = self.inner.calls.lock().await;
            let c = calls.map.get_mut(call_id).ok_or(Error::NotFound)?;
            let was = c.group;
            c.group = true;
            !was
        };
        if changed {
            self.emit_call(call_id).await;
        }
        Ok(())
    }

    /// The broker room behind a call this desk started or joined by adding people.
    pub async fn call_room(&self, call_id: &Id) -> Option<(String, String)> {
        self.inner.calls.lock().await.map.get(call_id).filter(|c| !c.room.is_empty()).map(|c| (c.url.clone(), c.room.clone()))
    }

    /// Add someone to a call that's going: they get an ordinary invitation with the same media key
    /// and a join token for the same room.
    pub async fn invite_to_call(&self, call_id: &Id, contact: &Id, token: String) -> Result<CallView> {
        {
            let contacts = self.inner.contacts.lock().await;
            let c = contacts.get(contact).ok_or(Error::NotFound)?;
            if c.state != ContactState::Ready || c.ratchet.is_none() {
                return Err(Error::NotReady);
            }
        }
        let (url, room, key, ring) = {
            let mut calls = self.inner.calls.lock().await;
            let ring = calls.ring();
            let c = calls.map.get_mut(call_id).ok_or(Error::NotFound)?;
            if !matches!(c.phase, Phase::Calling | Phase::Active) {
                return Err(Error::Invalid("This call has ended."));
            }
            if c.contact == *contact || c.invited.contains(contact) {
                return Err(Error::Invalid("They're already in this call."));
            }
            c.group = true;
            c.invited.push(*contact);
            (c.url.clone(), c.room.clone(), c.key, ring)
        };
        let offer = CallSignal::Offer { call_id: *call_id, url, room, token, key, video: false, expires_at: time::now_ms() + ring.as_millis() as u64, group: true };
        if let Err(e) = self.send_payload(contact, &Payload::Call(offer)).await {
            if let Some(c) = self.inner.calls.lock().await.map.get_mut(call_id) {
                c.invited.retain(|x| x != contact);
            }
            return Err(e);
        }
        self.emit_call(call_id).await;
        self.call_view(call_id).await.ok_or(Error::NotFound)
    }

    /// Refuse a ringing call without telling the caller, because calls are off here.
    pub async fn refuse_call(&self, call_id: &Id) {
        if self.call_phase(call_id).await == Some(Phase::Ringing) {
            self.finish_call(call_id, EndReason::Off).await;
        }
    }

    /// Hang up, cancel an outgoing call, or (while ringing) decline silently.
    pub async fn end_call(&self, call_id: &Id) -> Result<()> {
        let (contact, phase) = {
            let calls = self.inner.calls.lock().await;
            let c = calls.map.get(call_id).ok_or(Error::NotFound)?;
            (c.contact, c.phase)
        };
        match phase {
            Phase::Calling => {
                let _ = self.send_payload(&contact, &Payload::Call(CallSignal::Cancel { call_id: *call_id })).await;
                self.finish_call(call_id, EndReason::Cancelled).await;
            }
            Phase::Ringing => self.finish_call(call_id, EndReason::Declined).await,
            Phase::Active => {
                let _ = self.send_payload(&contact, &Payload::Call(CallSignal::End { call_id: *call_id })).await;
                self.finish_call(call_id, EndReason::Finished).await;
            }
            Phase::Ended => {}
        }
        Ok(())
    }

    /// The call window saw the other person arrive in the room.
    pub async fn call_peer_joined(&self, call_id: &Id) -> Result<()> {
        {
            let mut calls = self.inner.calls.lock().await;
            let c = calls.map.get_mut(call_id).ok_or(Error::NotFound)?;
            if c.phase != Phase::Calling {
                return Ok(());
            }
            c.phase = Phase::Active;
            c.answered_ms = Some(time::now_ms());
        }
        self.emit_call(call_id).await;
        Ok(())
    }

    /// What the call window needs to join. Only once the call is answered (or, for the caller,
    /// right away, so it can wait in the room).
    pub async fn join_info(&self, call_id: &Id) -> Result<JoinInfo> {
        let calls = self.inner.calls.lock().await;
        let c = calls.map.get(call_id).ok_or(Error::NotFound)?;
        match c.phase {
            Phase::Calling | Phase::Active => Ok(JoinInfo {
                call_id: c.id,
                contact_id: c.contact,
                url: c.url.clone(),
                token: c.token.clone(),
                key: c.key,
                outgoing: c.direction == Direction::Outgoing,
                group: c.group,
            }),
            Phase::Ringing => Err(Error::Invalid("Answer the call first.")),
            Phase::Ended => Err(Error::Invalid("This call has ended.")),
        }
    }

    /// End a call locally: drop its keys, leave a line in the conversation, tell the UI.
    async fn finish_call(&self, call_id: &Id, reason: EndReason) {
        let Some(mut call) = self.inner.calls.lock().await.map.remove(call_id) else { return };
        call.phase = Phase::Ended;
        let talked = call.answered_ms.map(|a| time::now_ms().saturating_sub(a) / 1000);
        let (text, unread) = crate::calls::record(call.direction, reason, talked, call.group);
        self.add_call_record(&call.contact, text, unread).await;
        self.emit(Event::Call { call: call.view(Some(reason)) });
    }

    async fn add_call_record(&self, contact: &Id, text: String, unread: bool) {
        let now_ms = time::now_ms();
        let ttl = self.ttl_of(contact).await.ok().flatten();
        let m = Message {
            id: new_id(),
            contact_id: *contact,
            outgoing: false,
            body: Body::System(text),
            sent_at: now_ms,
            received_at: now_ms,
            expires_at: ttl.map(|t| time::now() + t as u64),
            delivery: Delivery::Received,
            remote_file: None,
            reply_to: None,
            reactions: Vec::new(),
            edited: false,
            read: !unread,
        };
        if self.save_message(&m).is_err() {
            return;
        }
        self.track_expiry(&m).await;
        let (preview, at) = (m.preview(), m.received_at / 1000);
        let _ = self
            .update_contact(contact, |c| {
                if unread {
                    c.unread = c.unread.saturating_add(1);
                }
                c.preview = preview;
                c.preview_at = at;
                c.preview_outgoing = false;
            })
            .await;
        self.emit(Event::Message { contact_id: hex(contact), id: hex(&m.id) });
    }

    async fn on_call(&self, cid: &Id, signal: CallSignal) {
        let call_id = signal.call_id();
        match signal {
            CallSignal::Offer { url, room, token, key, expires_at, group, .. } => {
                let now_ms = time::now_ms();
                let busy = {
                    let mut calls = self.inner.calls.lock().await;
                    if calls.map.contains_key(&call_id) {
                        return;
                    }
                    let busy = calls.busy();
                    if !busy && expires_at > now_ms {
                        calls.map.insert(
                            call_id,
                            Call {
                                id: call_id,
                                contact: *cid,
                                direction: Direction::Incoming,
                                phase: Phase::Ringing,
                                url,
                                room,
                                token,
                                key,
                                started_ms: now_ms,
                                answered_ms: None,
                                group,
                                invited: Vec::new(),
                            },
                        );
                    }
                    busy
                };
                if busy {
                    let (text, unread) = crate::calls::record(Direction::Incoming, EndReason::MissedBusy, None, false);
                    self.add_call_record(cid, text, unread).await;
                    return;
                }
                if expires_at <= now_ms {
                    // Arrived after it stopped ringing (for example while this desk was locked).
                    let (text, unread) = crate::calls::record(Direction::Incoming, EndReason::Missed, None, false);
                    self.add_call_record(cid, text, unread).await;
                    return;
                }
                self.emit_call(&call_id).await;
                let me = self.clone();
                let wait = std::time::Duration::from_millis(expires_at - now_ms);
                self.spawn_task(async move {
                    tokio::time::sleep(wait).await;
                    if me.call_phase(&call_id).await == Some(Phase::Ringing) {
                        me.finish_call(&call_id, EndReason::Missed).await;
                    }
                });
            }
            other => {
                // Every other signal must come from the contact the call is with.
                let (direction, phase, answered, group) = {
                    let calls = self.inner.calls.lock().await;
                    match calls.map.get(&call_id) {
                        Some(c) if c.contact == *cid || c.invited.contains(cid) => (c.direction, c.phase, c.answered_ms.is_some(), c.group),
                        _ => return,
                    }
                };
                match other {
                    CallSignal::Answer { .. } if phase == Phase::Calling => {
                        let _ = self.call_peer_joined(&call_id).await;
                    }
                    CallSignal::Decline { .. } if phase == Phase::Calling => self.finish_call(&call_id, EndReason::TheyDeclined).await,
                    CallSignal::Cancel { .. } if phase == Phase::Ringing => self.finish_call(&call_id, EndReason::Missed).await,
                    // In a group call one person leaving doesn't end it: the call window ends it
                    // when the room is empty.
                    CallSignal::End { .. } if group && answered => {
                        // They left; they can be added back later.
                        if let Some(c) = self.inner.calls.lock().await.map.get_mut(&call_id) {
                            c.invited.retain(|x| x != cid);
                        }
                        self.emit_call(&call_id).await;
                    }
                    CallSignal::End { .. } => {
                        let reason = match (answered, direction) {
                            (true, _) => EndReason::Finished,
                            (false, Direction::Outgoing) => EndReason::NoAnswer,
                            (false, Direction::Incoming) => EndReason::Missed,
                        };
                        self.finish_call(&call_id, reason).await;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Fetch a received file into the vault. Called automatically; can be retried.
    pub async fn download(&self, cid: &Id, mid: &Id) -> Result<()> {
        let mut m = self.find_message(cid, mid).ok_or(Error::NotFound)?;
        let file = m.remote_file.clone().ok_or(Error::NotFound)?;
        let blob_id = format!("file/{}", hex(mid));
        let conn = self.relay(&file.relay).await;
        let result = async {
            let mut w = self.inner.store.blob_writer(&blob_id)?;
            files::download(&conn, &file.key, &file.chunks, file.size, &file.sha256, &mut w).await?;
            w.finish()?;
            Ok::<_, Error>(())
        }
        .await;
        if let Body::File { meta, .. } = &mut m.body {
            match &result {
                Ok(()) => {
                    meta.status = FileStatus::Ready;
                    meta.blob_id = Some(blob_id);
                }
                Err(_) => meta.status = FileStatus::Failed,
            }
        }
        // The file key is no longer needed once the file is stored.
        if result.is_ok() {
            m.remote_file = None;
        }
        self.save_message(&m)?;
        self.track_expiry(&m).await;
        self.emit(Event::Message { contact_id: hex(cid), id: hex(mid) });
        result
    }

    /// Decrypt a stored file blob into `out`.
    pub fn read_blob(&self, blob_id: &str, out: &mut impl std::io::Write) -> Result<u64> {
        Ok(self.inner.store.read_blob(blob_id, out)?)
    }
}

fn blob_of(m: &Message) -> Option<String> {
    match &m.body {
        Body::File { meta, .. } => meta.blob_id.clone(),
        _ => None,
    }
}

fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.trim().is_empty() { "file".into() } else { clean }
}

fn timer_text(ttl: Option<u32>, by_me: bool) -> String {
    let who = if by_me { "You set" } else { "They set" };
    match ttl {
        None => format!("{who} messages to stay until deleted."),
        Some(s) => format!("{who} messages to disappear after {}.", human_duration(s)),
    }
}

pub fn human_duration(s: u32) -> String {
    match s {
        s if s % 86_400 == 0 => {
            let d = s / 86_400;
            format!("{d} day{}", if d == 1 { "" } else { "s" })
        }
        s if s % 3600 == 0 => {
            let h = s / 3600;
            format!("{h} hour{}", if h == 1 { "" } else { "s" })
        }
        s => {
            let m = (s / 60).max(1);
            format!("{m} minute{}", if m == 1 { "" } else { "s" })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_durations() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name("C:\\Users\\x\\leak.pdf"), "leak.pdf");
        assert_eq!(sanitize_name("\u{7}\u{8}"), "file");
        assert_eq!(human_duration(86_400), "1 day");
        assert_eq!(human_duration(7 * 86_400), "7 days");
        assert_eq!(human_duration(3600 * 5), "5 hours");
        assert_eq!(human_duration(300), "5 minutes");
    }
}
