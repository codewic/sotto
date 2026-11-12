//! Groups: end-to-end encrypted group chats. The same machinery as Cases (an MLS group over an
//! ordered relay group queue, with only the creator committing), carrying chat messages: text,
//! replies, reactions, edits and deletes. See docs/groups.md.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, header};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_cases::Processed;
use sotto_core::envelope::{FileRef, Payload};
use sotto_core::wire::{BLOB_SIZE, Cmd, Resp};
use sotto_core::{Id, hex, new_id, pad, time, unhex};

use crate::ApiError;
use crate::cases::{Runtime, ensure_mls, my_name, persist_mls};
use crate::state::{Agent, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

/// Largest group to start with (docs/groups.md).
const MAX_MEMBERS: usize = 100;

#[derive(Clone, Serialize, Deserialize)]
pub struct Member {
    pub name: String,
    pub contact_id: Option<Id>,
    pub pending: bool,
    pub owner: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GroupRecord {
    pub id: Id,
    pub title: String,
    group_id: Vec<u8>,
    relay: String,
    read_id: Id,
    write_id: Id,
    next_seq: u64,
    own_seqs: Vec<u64>,
    pub owner: bool,
    /// The creator's MLS signature key. Control messages and commits from anyone else are ignored.
    owner_key: Vec<u8>,
    pub members: Vec<Member>,
    created_at: u64,
    pub removed: bool,
    pub ttl_secs: Option<u32>,
    pub unread: u32,
    pub last_activity: u64,
    pub preview: String,
    pub preview_at: u64,
    pub pinned: bool,
    pub archived: bool,
}

/// Application messages inside the MLS group.
#[derive(Serialize, Deserialize)]
enum GroupMsg {
    /// `name`: the sender's chosen name, shown on their messages.
    Text { id: Id, name: String, body: String, sent_at: u64, reply_to: Option<Id> },
    React { target: Id, emoji: String, on: bool },
    Edit { target: Id, body: String },
    Delete { target: Id },
    /// From the creator: who is in the group (name, is creator).
    Members(Vec<(String, bool)>),
    Title(String),
    Timer(Option<u32>),
    /// A member is leaving; the creator removes them.
    Leave,
    /// The creator deleted the group.
    Closed,
    /// Pin or unpin a message (the creator or an admin).
    Pin { target: Id, on: bool },
    /// From the creator: who else may manage the group, as (name, MLS signature key).
    Admins(Vec<(String, Vec<u8>)>),
    /// A file; `name` is the sender's chosen name, as on text.
    File { id: Id, name: String, file: Box<FileRef>, caption: String, sent_at: u64 },
    /// From an admin to the creator: please make this change. Only the creator commits, so
    /// admins ask, and the creator's desk carries it out.
    Propose(Proposal),
    /// From the creator: the Welcome for someone an admin proposed. That admin delivers it over
    /// their own conversation with the new member.
    WelcomeFor { identity: Vec<u8>, welcome: Vec<u8> },
}

#[derive(Serialize, Deserialize)]
enum Proposal {
    Add { key_package: Vec<u8> },
    Remove { name: String },
}

/// Group state kept beside the record (so older records still load): admins, pinned messages,
/// mute, and, on an admin's desk, the people they proposed who are waiting for a Welcome.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Extra {
    admins: Vec<(String, Vec<u8>)>,
    pins: Vec<Id>,
    muted: bool,
    proposed: Vec<(Vec<u8>, Id)>,
}

fn extra(s: &Session, gid: &Id) -> Extra {
    s.cases.get::<Extra>(&format!("gx/{}", hex(gid))).ok().flatten().unwrap_or_default()
}

fn save_extra(s: &Session, gid: &Id, x: &Extra) -> Result<(), ApiError> {
    s.cases.put(&format!("gx/{}", hex(gid)), x)?;
    Ok(())
}

/// Whether `body` mentions `name` as `@name` (case-insensitive, not inside a longer word).
fn mentions(body: &str, name: &str) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    let (b, n) = (body.to_lowercase(), format!("@{}", name.to_lowercase()));
    b.match_indices(&n).any(|(i, _)| {
        let before = b[..i].chars().last();
        let after = b[i + n.len()..].chars().next();
        let word = |c: char| c.is_alphanumeric() || c == '_';
        before.is_none_or(|c| !word(c)) && after.is_none_or(|c| !word(c))
    })
}

/// Unread mentions of you in a group, kept beside the record (cleared when you read it).
fn mention_count(s: &Session, gid: &Id) -> u32 {
    s.cases.get::<u32>(&format!("gmention/{}", hex(gid))).ok().flatten().unwrap_or(0)
}

fn add_mention(s: &Session, gid: &Id) -> Result<(), ApiError> {
    s.cases.put(&format!("gmention/{}", hex(gid)), &(mention_count(s, gid) + 1))?;
    Ok(())
}

/// Where a group message's file lives in `shared`.
fn file_at(gid: &Id, mid: &Id) -> String {
    format!("g/{}/{}", hex(gid), hex(mid))
}

#[derive(Clone, Serialize, Deserialize)]
struct Stored {
    id: Id,
    author: String,
    author_key: Vec<u8>,
    mine: bool,
    system: bool,
    body: String,
    sent_at: u64,
    received_at: u64,
    expires_at: Option<u64>,
    reply_to: Option<Id>,
    /// (emoji, who reacted by signature key)
    reactions: Vec<(String, Vec<u8>)>,
    edited: bool,
}

fn key(id: &Id) -> String {
    format!("group/{}", hex(id))
}

/// A group's name, for lists elsewhere (Safe review).
pub fn title_of(s: &Session, id: &Id) -> Option<String> {
    record(s, id).ok().map(|g| g.title)
}

fn records(s: &Session) -> Vec<GroupRecord> {
    s.cases.names("group/").iter().filter_map(|n| s.cases.get::<GroupRecord>(n).ok().flatten()).collect()
}

fn record(s: &Session, id: &Id) -> Result<GroupRecord, ApiError> {
    s.cases.get(&key(id))?.ok_or(ApiError::NotFound)
}

fn save(s: &Session, g: &GroupRecord) -> Result<(), ApiError> {
    s.cases.put(&key(&g.id), g)?;
    Ok(())
}

fn msg_prefix(gid: &Id) -> String {
    format!("gmsg/{}/", hex(gid))
}

fn put_msg(s: &Session, gid: &Id, m: &Stored) -> Result<(), ApiError> {
    s.cases.put(&format!("{}{:020}-{}", msg_prefix(gid), m.received_at, hex(&m.id)), m)?;
    Ok(())
}

fn find_msg(s: &Session, gid: &Id, mid: &Id) -> Option<(String, Stored)> {
    let suffix = format!("-{}", hex(mid));
    let name = s.cases.names(&msg_prefix(gid)).into_iter().find(|n| n.ends_with(&suffix))?;
    let m = s.cases.get::<Stored>(&name).ok().flatten()?;
    Some((name, m))
}

/// Messages in order, dropping any whose disappearing timer has run out.
fn messages(s: &Session, gid: &Id) -> Vec<Stored> {
    let now = time::now();
    let mut out = Vec::new();
    let mut names = s.cases.names(&msg_prefix(gid));
    names.sort();
    for n in names {
        if let Ok(Some(m)) = s.cases.get::<Stored>(&n) {
            if m.expires_at.is_some_and(|e| e <= now) {
                let _ = s.cases.delete(&n);
                crate::shared::remove(s, &file_at(gid, &m.id));
            } else {
                out.push(m);
            }
        }
    }
    out
}

fn system(s: &Session, g: &mut GroupRecord, text: String) -> Result<(), ApiError> {
    let now_ms = time::now_ms();
    let m = Stored {
        id: new_id(),
        author: String::new(),
        author_key: vec![],
        mine: false,
        system: true,
        body: text.clone(),
        sent_at: now_ms,
        received_at: now_ms,
        expires_at: g.ttl_secs.map(|t| time::now() + t as u64),
        reply_to: None,
        reactions: vec![],
        edited: false,
    };
    put_msg(s, &g.id, &m)?;
    g.preview = text;
    g.preview_at = time::now();
    g.last_activity = time::now();
    Ok(())
}

async fn gsend(s: &Session, g: &mut GroupRecord, bytes: Vec<u8>) -> Result<(), ApiError> {
    let blob = pad::pad(&bytes, BLOB_SIZE).map_err(|_| ApiError::Invalid("That message is too long. Send it in parts.".into()))?;
    let write_id = g.write_id;
    match s.engine.relay(&g.relay).await.request(move |_| Cmd::GSend { write_id, blob }).await? {
        Resp::GSeq { seq } => {
            g.own_seqs.push(seq);
            if g.own_seqs.len() > 500 {
                g.own_seqs.drain(..100);
            }
            Ok(())
        }
        _ => Err(ApiError::Internal),
    }
}

async fn broadcast(s: &Session, rt: &Runtime, g: &mut GroupRecord, msg: &GroupMsg) -> Result<(), ApiError> {
    let mls = rt.mls.as_ref().ok_or(ApiError::Internal)?;
    let data = mls.encrypt(&g.group_id, &postcard::to_stdvec(msg).map_err(|_| ApiError::Internal)?).map_err(|_| ApiError::Internal)?;
    gsend(s, g, data).await
}

async fn subscribe(s: &Session, rt: &mut Runtime, g: &GroupRecord) {
    let next = rt.seqs.entry(g.read_id).or_insert_with(|| Arc::new(AtomicU64::new(g.next_seq))).clone();
    s.engine.relay(&g.relay).await.subscribe_group(g.read_id, next).await;
}

fn members_list(g: &GroupRecord) -> Vec<(String, bool)> {
    g.members.iter().filter(|m| !m.pending).map(|m| (m.name.clone(), m.owner)).collect()
}

/// Called after unlock (from cases::start): subscribe every group; sweep disappearing messages.
pub async fn start(_agent: &Arc<Agent>, s: &Arc<Session>) {
    {
        let mut rt = s.cases_rt.lock().await;
        for g in records(s) {
            if !g.removed {
                subscribe(s, &mut rt, &g).await;
            }
        }
    }
    let sess = s.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            for g in records(&sess) {
                if g.ttl_secs.is_some() {
                    let _ = messages(&sess, &g.id);
                }
            }
        }
    });
}

fn emit(agent: &Agent, g: &GroupRecord) {
    agent.emit(json!({ "type": "group", "id": hex(&g.id) }));
}

pub async fn on_group(agent: &Arc<Agent>, s: &Arc<Session>, read_id: Id, seq: u64, blob: Vec<u8>) -> Result<(), ApiError> {
    let mut rt = s.cases_rt.lock().await;
    let Some(mut g) = records(s).into_iter().find(|g| g.read_id == read_id) else { return Ok(()) };
    if seq < g.next_seq {
        return Ok(());
    }
    g.next_seq = seq + 1;
    if let Some(n) = rt.seqs.get(&read_id) {
        n.store(g.next_seq, Ordering::SeqCst);
    }
    if g.own_seqs.contains(&seq) {
        save(s, &g)?;
        return Ok(());
    }
    ensure_mls(s, &mut rt).await?;
    let committer = (!g.owner_key.is_empty()).then_some(g.owner_key.as_slice());
    let processed = pad::unpad(&blob).ok().and_then(|b| rt.mls.as_ref()?.process(&g.group_id, b, committer).ok());
    match processed {
        Some(Processed::Application { sender, sender_key, data }) => {
            let credential = String::from_utf8_lossy(&sender).to_string();
            let from_owner = !g.owner_key.is_empty() && sender_key == g.owner_key;
            let mut x = extra(s, &g.id);
            let from_admin = x.admins.iter().any(|(_, k)| k == &sender_key);
            let manager = from_owner || from_admin;
            match postcard::from_bytes::<GroupMsg>(&data) {
                Ok(GroupMsg::Text { id, name, body, sent_at, reply_to }) => {
                    if find_msg(s, &g.id, &id).is_none() {
                        let author: String = if name.trim().is_empty() { credential } else { name }.chars().take(60).collect();
                        let body: String = body.chars().take(12_000).collect();
                        let m = Stored {
                            id,
                            author: author.clone(),
                            author_key: sender_key,
                            mine: false,
                            system: false,
                            body: body.clone(),
                            sent_at,
                            received_at: time::now_ms(),
                            expires_at: g.ttl_secs.map(|t| time::now() + t as u64),
                            reply_to,
                            reactions: vec![],
                            edited: false,
                        };
                        put_msg(s, &g.id, &m)?;
                        g.unread = g.unread.saturating_add(1);
                        // "@Ada" or "@Ada Mensah" (the name you sign with, or your display name).
                        let me = rt.mls.as_ref().map(|m| name_in(m, &g.group_id)).unwrap_or_default();
                        let shown = s.engine.profile().await.display_name;
                        if mentions(&body, &me) || mentions(&body, &shown) {
                            add_mention(s, &g.id)?;
                        }
                        g.preview = format!("{author}: {}", body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect::<String>());
                        g.preview_at = time::now();
                        g.last_activity = time::now();
                    }
                }
                Ok(GroupMsg::React { target, emoji, on }) => {
                    if let Some((n, mut m)) = find_msg(s, &g.id, &target) {
                        m.reactions.retain(|(e, k)| !(e == &emoji && k == &sender_key));
                        if on && emoji.chars().count() <= 8 {
                            m.reactions.push((emoji, sender_key));
                        }
                        s.cases.put(&n, &m)?;
                    }
                }
                Ok(GroupMsg::Edit { target, body }) => {
                    if let Some((n, mut m)) = find_msg(s, &g.id, &target)
                        && m.author_key == sender_key {
                            m.body = body.chars().take(12_000).collect();
                            m.edited = true;
                            s.cases.put(&n, &m)?;
                        }
                }
                Ok(GroupMsg::Delete { target }) => {
                    if let Some((n, m)) = find_msg(s, &g.id, &target)
                        && m.author_key == sender_key {
                            s.cases.delete(&n)?;
                            crate::shared::remove(s, &file_at(&g.id, &target));
                        }
                }
                Ok(GroupMsg::File { id, name, file, caption, sent_at }) => {
                    if find_msg(s, &g.id, &id).is_none() {
                        let author: String = if name.trim().is_empty() { credential } else { name }.chars().take(60).collect();
                        let caption: String = caption.chars().take(2000).collect();
                        let file_name = file.name.clone();
                        crate::shared::receive(agent, s, &file_at(&g.id, &id), *file, json!({ "type": "group", "id": hex(&g.id) }))?;
                        let m = Stored {
                            id,
                            author: author.clone(),
                            author_key: sender_key,
                            mine: false,
                            system: false,
                            body: caption,
                            sent_at,
                            received_at: time::now_ms(),
                            expires_at: g.ttl_secs.map(|t| time::now() + t as u64),
                            reply_to: None,
                            reactions: vec![],
                            edited: false,
                        };
                        put_msg(s, &g.id, &m)?;
                        g.unread = g.unread.saturating_add(1);
                        g.preview = format!("{author}: {}", file_name.chars().take(120).collect::<String>());
                        g.preview_at = time::now();
                        g.last_activity = time::now();
                    }
                }
                Ok(GroupMsg::Pin { target, on }) if manager => {
                    x.pins.retain(|p| p != &target);
                    if on {
                        x.pins.push(target);
                        system(s, &mut g, format!("{credential} pinned a message."))?;
                    }
                    save_extra(s, &g.id, &x)?;
                }
                Ok(GroupMsg::Admins(list)) if !g.owner && from_owner => {
                    x.admins = list.into_iter().take(MAX_MEMBERS).collect();
                    save_extra(s, &g.id, &x)?;
                }
                Ok(GroupMsg::Propose(p)) if g.owner && from_admin => match p {
                    Proposal::Add { key_package } => {
                        if let Ok((name, welcome)) = admit(s, &rt, &mut g, &key_package).await {
                            g.members.push(Member { name: name.clone(), contact_id: None, pending: false, owner: false });
                            share_members(s, &rt, &mut g).await?;
                            broadcast(s, &rt, &mut g, &GroupMsg::WelcomeFor { identity: name.clone().into_bytes(), welcome }).await?;
                            system(s, &mut g, format!("{credential} added {name}."))?;
                        }
                    }
                    Proposal::Remove { name } => {
                        if expel(s, &rt, &mut g, &mut x, &name).await? {
                            system(s, &mut g, format!("{credential} removed {name} from the group."))?;
                        }
                    }
                },
                Ok(GroupMsg::WelcomeFor { identity, welcome }) if !g.owner && from_owner => {
                    // Ours to deliver only if we proposed this person.
                    if let Some(i) = x.proposed.iter().position(|(id, _)| id == &identity) {
                        let (_, contact) = x.proposed.remove(i);
                        save_extra(s, &g.id, &x)?;
                        let invite = Payload::GroupInvite { group_id: g.id, title: g.title.clone(), welcome, relay: g.relay.clone(), read_id: g.read_id, write_id: g.write_id, owner_key: g.owner_key.clone() };
                        let _ = s.engine.send_payload(&contact, &invite).await;
                    }
                }
                Ok(GroupMsg::Members(list)) if !g.owner && from_owner => {
                    let owner_contact = g.members.iter().find(|m| m.owner).and_then(|m| m.contact_id);
                    // People we proposed stay listed as joining until they're in.
                    let waiting: Vec<Member> = g.members.iter().filter(|m| m.pending && m.contact_id.is_some() && !m.owner).cloned().collect();
                    g.members = list
                        .into_iter()
                        .take(MAX_MEMBERS)
                        .map(|(name, owner)| Member { name, contact_id: if owner { owner_contact } else { None }, pending: false, owner })
                        .collect();
                    for w in waiting {
                        if !g.members.iter().any(|m| m.name == w.name) && x.proposed.iter().any(|(_, c)| Some(*c) == w.contact_id) {
                            g.members.push(w);
                        }
                    }
                }
                Ok(GroupMsg::Title(t)) if manager => {
                    g.title = t.chars().take(80).collect();
                    let text = format!("{credential} renamed the group {}.", g.title);
                    system(s, &mut g, text)?;
                }
                Ok(GroupMsg::Timer(ttl)) if manager => {
                    g.ttl_secs = ttl;
                    system(s, &mut g, timer_text(ttl))?;
                }
                Ok(GroupMsg::Leave) if g.owner => {
                    // Remove whoever is leaving (their MLS credential), then tell everyone.
                    let commit = rt.mls.as_ref().ok_or(ApiError::Internal)?.remove_members(&g.group_id, std::slice::from_ref(&sender)).map_err(|_| ApiError::Internal)?;
                    persist_mls(s, &rt)?;
                    gsend(s, &mut g, commit).await?;
                    g.members.retain(|m| m.name.as_bytes() != sender.as_slice());
                    if x.admins.iter().any(|(_, k)| k == &sender_key) {
                        x.admins.retain(|(_, k)| k != &sender_key);
                        save_extra(s, &g.id, &x)?;
                        broadcast(s, &rt, &mut g, &GroupMsg::Admins(x.admins.clone())).await?;
                    }
                    let list = members_list(&g);
                    broadcast(s, &rt, &mut g, &GroupMsg::Members(list)).await?;
                    system(s, &mut g, format!("{credential} left the group."))?;
                }
                Ok(GroupMsg::Closed) if !g.owner && from_owner => {
                    g.removed = true;
                    system(s, &mut g, "The group was deleted by the person who made it. You can still read what you had.".into())?;
                }
                _ => {}
            }
        }
        Some(Processed::Commit { added, removed }) => {
            for a in added {
                system(s, &mut g, format!("{} joined the group.", String::from_utf8_lossy(&a)))?;
            }
            for r in removed {
                system(s, &mut g, format!("{} left the group.", String::from_utf8_lossy(&r)))?;
            }
            if !rt.mls.as_ref().map(|m| m.is_active(&g.group_id)).unwrap_or(false) && !g.removed {
                g.removed = true;
                system(s, &mut g, "You're no longer in this group. You can still read what you had.".into())?;
            }
        }
        _ => {}
    }
    persist_mls(s, &rt)?;
    save(s, &g)?;
    emit(agent, &g);
    Ok(())
}

/// The name everyone in the group sees for a member: only the name they chose themselves.
/// Your own nickname for a contact is private and never goes into the member list.
fn shared_name(their_name: String) -> String {
    if their_name.trim().is_empty() { "Member".into() } else { their_name }
}

fn timer_text(ttl: Option<u32>) -> String {
    match ttl {
        None => "Disappearing messages are off.".into(),
        Some(t) => format!("Messages now disappear after {}.", sotto_client::engine::human_duration(t)),
    }
}

/// Add whoever owns this key package (the creator only). Returns their chosen name and the
/// Welcome to deliver to them.
async fn admit(s: &Session, rt: &Runtime, g: &mut GroupRecord, key_package: &[u8]) -> Result<(String, Vec<u8>), ApiError> {
    if g.members.len() >= MAX_MEMBERS {
        return Err(ApiError::Invalid(format!("A group can have up to {MAX_MEMBERS} people.")));
    }
    let mls = rt.mls.as_ref().ok_or(ApiError::Internal)?;
    // Their name in the group is the one signed into their own key package: what they chose,
    // and exactly what MLS removal matches on later.
    let identity = mls.key_package_identity(key_package).map_err(|_| ApiError::Invalid("Their key package was invalid.".into()))?;
    let (commit, welcome) = mls.add_member(&g.group_id, key_package).map_err(|_| ApiError::Invalid("Their key package was invalid.".into()))?;
    persist_mls(s, rt)?;
    gsend(s, g, commit).await?;
    Ok((shared_name(String::from_utf8_lossy(&identity).chars().take(80).collect()), welcome))
}

/// Send the member list (and the admins and timer, for anyone new) to everyone.
async fn share_members(s: &Session, rt: &Runtime, g: &mut GroupRecord) -> Result<(), ApiError> {
    let list = members_list(g);
    broadcast(s, rt, g, &GroupMsg::Members(list)).await?;
    let admins = extra(s, &g.id).admins;
    if !admins.is_empty() {
        broadcast(s, rt, g, &GroupMsg::Admins(admins)).await?;
    }
    if let Some(t) = g.ttl_secs {
        broadcast(s, rt, g, &GroupMsg::Timer(Some(t))).await?;
    }
    persist_mls(s, rt)
}

/// Remove a member by name (the creator only). Returns whether anyone was removed.
async fn expel(s: &Session, rt: &Runtime, g: &mut GroupRecord, x: &mut Extra, name: &str) -> Result<bool, ApiError> {
    let Some(i) = g.members.iter().position(|m| m.name == name && !m.owner) else { return Ok(false) };
    let m = g.members.remove(i);
    if !m.pending {
        let commit = rt.mls.as_ref().ok_or(ApiError::Internal)?.remove_members(&g.group_id, &[m.name.clone().into_bytes()]).map_err(|_| ApiError::Internal)?;
        persist_mls(s, rt)?;
        gsend(s, g, commit).await?;
        if x.admins.iter().any(|(n, _)| n == &m.name) {
            x.admins.retain(|(n, _)| n != &m.name);
            save_extra(s, &g.id, x)?;
            broadcast(s, rt, g, &GroupMsg::Admins(x.admins.clone())).await?;
        }
        let list = members_list(g);
        broadcast(s, rt, g, &GroupMsg::Members(list)).await?;
        persist_mls(s, rt)?;
    }
    Ok(true)
}

/// A key package from someone we invited. The creator adds them and sends the Welcome privately;
/// an admin passes it to the creator, then delivers the Welcome when it comes back.
pub async fn on_key_package(agent: &Arc<Agent>, s: &Arc<Session>, contact: Id, group: Id, data: Vec<u8>) -> Result<(), ApiError> {
    let mut rt = s.cases_rt.lock().await;
    let Ok(mut g) = record(s, &group) else { return Ok(()) };
    let Some(i) = g.members.iter().position(|m| m.pending && m.contact_id == Some(contact)) else { return Ok(()) };
    if g.removed {
        return Ok(());
    }
    ensure_mls(s, &mut rt).await?;
    if g.owner {
        let (name, welcome) = admit(s, &rt, &mut g, &data).await?;
        g.members[i].pending = false;
        g.members[i].name = name.clone();
        let owner_key = rt.mls.as_ref().ok_or(ApiError::Internal)?.signature_key();
        let invite = Payload::GroupInvite { group_id: g.id, title: g.title.clone(), welcome, relay: g.relay.clone(), read_id: g.read_id, write_id: g.write_id, owner_key };
        s.engine.send_payload(&contact, &invite).await?;
        share_members(s, &rt, &mut g).await?;
        system(s, &mut g, format!("{name} joined the group."))?;
    } else {
        let mls = rt.mls.as_ref().ok_or(ApiError::Internal)?;
        let mut x = extra(s, &g.id);
        if !x.admins.iter().any(|(_, k)| k == &mls.signature_key()) {
            return Ok(());
        }
        let identity = mls.key_package_identity(&data).map_err(|_| ApiError::Invalid("Their key package was invalid.".into()))?;
        x.proposed.retain(|(_, c)| c != &contact);
        x.proposed.push((identity, contact));
        save_extra(s, &g.id, &x)?;
        broadcast(s, &rt, &mut g, &GroupMsg::Propose(Proposal::Add { key_package: data })).await?;
        persist_mls(s, &rt)?;
    }
    save(s, &g)?;
    emit(agent, &g);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn on_invite(
    agent: &Arc<Agent>,
    s: &Arc<Session>,
    contact: Id,
    id: Id,
    title: String,
    welcome: Vec<u8>,
    relay: String,
    read_id: Id,
    write_id: Id,
    owner_key: Vec<u8>,
) -> Result<(), ApiError> {
    s.engine.check_remote_relay(&relay)?;
    if owner_key.is_empty() || record(s, &id).is_ok() {
        return Ok(());
    }
    let mut rt = s.cases_rt.lock().await;
    ensure_mls(s, &mut rt).await?;
    let group_id = rt.mls.as_ref().ok_or(ApiError::Internal)?.join(&welcome).map_err(|_| ApiError::Invalid("The group invitation couldn't be opened.".into()))?;
    persist_mls(s, &rt)?;
    let mut g = GroupRecord {
        id,
        title: title.chars().take(80).collect(),
        group_id,
        relay,
        read_id,
        write_id,
        next_seq: 0,
        own_seqs: vec![],
        owner: false,
        owner_key,
        // The person who invited us is the owner, and we know them from our own conversation:
        // remember that link so the group can call them. The owner's member list fills in the rest.
        members: vec![Member { name: String::new(), contact_id: Some(contact), pending: false, owner: true }],
        created_at: time::now(),
        removed: false,
        ttl_secs: None,
        unread: 1,
        last_activity: time::now(),
        preview: String::new(),
        preview_at: time::now(),
        pinned: false,
        archived: false,
    };
    system(s, &mut g, "You were added to this group. Messages here are end-to-end encrypted.".into())?;
    save(s, &g)?;
    subscribe(s, &mut rt, &g).await;
    emit(agent, &g);
    Ok(())
}

// ---------- JSON ----------

fn group_json(s: &Session, g: &GroupRecord, me: &str) -> Value {
    let x = extra(s, &g.id);
    let admin = |name: &str| x.admins.iter().any(|(n, _)| n == name);
    // Which entry is this desk: the owner's own record says so; elsewhere it's the member under our chosen name.
    let you = |m: &Member| if g.owner { m.owner } else { !m.owner && m.name == me };
    json!({
        "id": hex(&g.id),
        "title": g.title,
        "owner": g.owner,
        "removed": g.removed,
        "members": g.members.iter().map(|m| json!({ "name": m.name, "owner": m.owner, "admin": admin(&m.name), "pending": m.pending, "you": you(m), "contact_id": m.contact_id.map(|c| hex(&c)) })).collect::<Vec<_>>(),
        "member_count": g.members.iter().filter(|m| !m.pending).count(),
        "am_admin": !g.owner && g.members.iter().any(|m| you(m) && admin(&m.name)),
        "pins": x.pins.iter().map(|p| hex(p)).collect::<Vec<_>>(),
        "muted": x.muted,
        "mentions": mention_count(s, &g.id),
        "ttl_secs": g.ttl_secs,
        "unread": g.unread,
        "last_activity": g.last_activity,
        "preview": g.preview,
        "preview_at": g.preview_at,
        "pinned": g.pinned,
        "archived": g.archived,
        "created_at": g.created_at,
    })
}

/// The same shape as a 1:1 message, plus `author`, so the conversation components render it.
fn msg_json(s: &Session, gid: &Id, m: &Stored, me: &[u8]) -> Value {
    let file = (!m.system).then(|| crate::shared::get(s, &file_at(gid, &m.id))).flatten();
    let mut reactions: Vec<(String, u32, bool)> = vec![];
    for (e, k) in &m.reactions {
        match reactions.iter_mut().find(|r| &r.0 == e) {
            Some(r) => {
                r.1 += 1;
                r.2 |= k == me;
            }
            None => reactions.push((e.clone(), 1, k == me)),
        }
    }
    json!({
        "id": hex(&m.id),
        "contact_id": hex(gid),
        "outgoing": m.mine,
        "kind": if m.system { "system" } else if file.is_some() { "file" } else { "text" },
        "text": if file.is_some() { None } else { Some(&m.body) },
        "file": file.map(|f| crate::shared::view(&f, &m.body)),
        "sent_at": m.sent_at,
        "received_at": m.received_at,
        "expires_at": m.expires_at,
        "delivery": if m.mine { "sent" } else { "received" },
        "reply_to": m.reply_to.map(|r| hex(&r)),
        "reactions": reactions.into_iter().map(|(emoji, count, mine)| json!({ "emoji": emoji, "count": count, "mine": mine })).collect::<Vec<_>>(),
        "edited": m.edited,
        "read": true,
        "author": m.author,
    })
}

// ---------- Routes ----------

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/groups", get(list).post(create))
        .route("/api/groups/{id}", get(show).patch(update).delete(leave))
        .route("/api/groups/{id}/read", post(read))
        .route("/api/groups/{id}/messages", get(list_messages).post(send))
        .route("/api/groups/{id}/messages/{mid}", delete(delete_message).patch(edit))
        .route("/api/groups/{id}/messages/{mid}/react", post(react))
        .route("/api/groups/{id}/members", post(add_member))
        .route("/api/groups/{id}/members/{who}", delete(remove_member))
        .route("/api/groups/{id}/files", post(send_file))
        .route("/api/groups/{id}/messages/{mid}/pin", post(pin))
        .route("/api/groups/{id}/messages/{mid}/review", post(review_file))
        .route("/api/groups/{id}/admins", post(set_admin))
        .route("/api/groups/{id}/safety", get(safety))
}

fn gid(s: &str) -> Result<Id, ApiError> {
    unhex::<16>(s).map_err(|_| ApiError::NotFound)
}

async fn list(State(agent): A) -> R {
    let s = agent.session().await?;
    let mut v = records(&s);
    v.sort_by_key(|x| std::cmp::Reverse(x.last_activity));
    let mut out = Vec::with_capacity(v.len());
    for g in &v {
        out.push(group_json(&s, g, &my_name_in(&s, g).await));
    }
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
struct CreateIn {
    title: String,
    #[serde(default)]
    members: Vec<String>,
}

async fn create(State(agent): A, Json(b): Json<CreateIn>) -> R {
    let s = agent.session().await?;
    let title: String = b.title.trim().chars().take(80).collect();
    if title.is_empty() {
        return Err(ApiError::Invalid("Give the group a name.".into()));
    }
    if b.members.len() + 1 > MAX_MEMBERS {
        return Err(ApiError::Invalid(format!("A group can have up to {MAX_MEMBERS} people.")));
    }
    let mut contacts = Vec::new();
    for c in &b.members {
        let id = gid(c)?;
        let v = s.engine.contact(&id).await.ok_or(ApiError::NotFound)?;
        if v.state != "ready" {
            return Err(ApiError::Invalid(format!("{} hasn't accepted your conversation invite yet.", if v.nickname.is_empty() { "Someone" } else { &v.nickname })));
        }
        contacts.push((id, shared_name(v.their_name)));
    }
    let mut rt = s.cases_rt.lock().await;
    ensure_mls(&s, &mut rt).await?;
    let group_id = rt.mls.as_ref().ok_or(ApiError::Internal)?.create_group().map_err(|_| ApiError::Internal)?;
    persist_mls(&s, &rt)?;
    let relay = s.engine.profile().await.default_relay;
    let (read_id, write_id) = match s.engine.relay(&relay).await.request(|_| Cmd::GNew { ttl_days: 30 }).await? {
        Resp::GIds { read_id, write_id } => (read_id, write_id),
        _ => return Err(ApiError::Internal),
    };
    let me = my_name(&s).await;
    let mut g = GroupRecord {
        id: new_id(),
        title,
        group_id,
        relay,
        read_id,
        write_id,
        next_seq: 0,
        own_seqs: vec![],
        owner: true,
        owner_key: rt.mls.as_ref().ok_or(ApiError::Internal)?.signature_key(),
        members: vec![Member { name: me, contact_id: None, pending: false, owner: true }],
        created_at: time::now(),
        removed: false,
        ttl_secs: None,
        unread: 0,
        last_activity: time::now(),
        preview: String::new(),
        preview_at: time::now(),
        pinned: false,
        archived: false,
    };
    system(&s, &mut g, "You made this group. Messages here are end-to-end encrypted.".into())?;
    for (id, name) in &contacts {
        g.members.push(Member { name: name.clone(), contact_id: Some(*id), pending: true, owner: false });
    }
    save(&s, &g)?;
    subscribe(&s, &mut rt, &g).await;
    drop(rt);
    // Each person is added once their client sends a key package back.
    for (id, _) in &contacts {
        let _ = s.engine.send_payload(id, &Payload::KeyPackageRequest { case_id: g.id }).await;
    }
    emit(&agent, &g);
    Ok(Json(group_json(&s, &g, &my_name_in(&s, &g).await)))
}

async fn show(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let g = record(&s, &gid(&id)?)?;
    Ok(Json(group_json(&s, &g, &my_name_in(&s, &g).await)))
}

#[derive(Deserialize)]
struct UpdateIn {
    title: Option<String>,
    /// Seconds; 0 turns disappearing messages off.
    ttl_secs: Option<u32>,
    pinned: Option<bool>,
    archived: Option<bool>,
    muted: Option<bool>,
}

/// Whether this desk may manage the group: its creator, or an admin the creator named.
async fn manages(s: &Session, g: &GroupRecord) -> Result<bool, ApiError> {
    if g.removed {
        return Ok(false);
    }
    if g.owner {
        return Ok(true);
    }
    let me = my_key(s).await?;
    Ok(extra(s, &g.id).admins.iter().any(|(_, k)| k == &me))
}

async fn update(State(agent): A, Path(id): Path<String>, Json(b): Json<UpdateIn>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    if (b.title.is_some() || b.ttl_secs.is_some()) && !manages(&s, &g).await? {
        return Err(ApiError::Invalid("Only the person who made the group, or an admin, can change its name or timer.".into()));
    }
    if let Some(m) = b.muted {
        let mut x = extra(&s, &g.id);
        x.muted = m;
        save_extra(&s, &g.id, &x)?;
    }
    if let Some(t) = b.title {
        let t: String = t.trim().chars().take(80).collect();
        if t.is_empty() {
            return Err(ApiError::Invalid("Give the group a name.".into()));
        }
        g.title = t.clone();
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::Title(t.clone())).await?;
        persist_mls(&s, &rt)?;
        drop(rt);
        system(&s, &mut g, format!("You renamed the group {t}."))?;
    }
    if let Some(t) = b.ttl_secs {
        g.ttl_secs = (t > 0).then_some(t.clamp(300, 30 * 86_400));
        let ttl = g.ttl_secs;
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::Timer(ttl)).await?;
        persist_mls(&s, &rt)?;
        drop(rt);
        system(&s, &mut g, timer_text(ttl))?;
    }
    if let Some(p) = b.pinned {
        g.pinned = p;
    }
    if let Some(a) = b.archived {
        g.archived = a;
    }
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(group_json(&s, &g, &my_name_in(&s, &g).await)))
}

/// Leave (a member) or delete for everyone (the creator). Either way this desk forgets the group.
async fn leave(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    {
        let rt = s.cases_rt.lock().await;
        if !g.removed {
            let msg = if g.owner { GroupMsg::Closed } else { GroupMsg::Leave };
            let _ = broadcast(&s, &rt, &mut g, &msg).await;
        }
        if let Some(m) = &rt.mls {
            let _ = m.forget(&g.group_id);
        }
        persist_mls(&s, &rt)?;
    }
    for n in s.cases.names(&msg_prefix(&g.id)) {
        s.cases.delete(&n)?;
    }
    crate::shared::remove_all(&s, &format!("g/{}/", hex(&g.id)));
    s.cases.delete(&format!("gx/{}", hex(&g.id)))?;
    let _ = s.cases.delete(&format!("gmention/{}", hex(&g.id)));
    s.cases.delete(&key(&g.id))?;
    agent.emit(json!({ "type": "group", "id": id }));
    Ok(Json(json!({ "ok": true })))
}

async fn read(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    let _ = s.cases.delete(&format!("gmention/{}", hex(&g.id)));
    if g.unread > 0 {
        g.unread = 0;
        save(&s, &g)?;
        emit(&agent, &g);
    }
    Ok(Json(json!({ "ok": true })))
}

/// The name this desk signed into one group. It's found by our signature key, so it stays right
/// after you rename yourself (a group keeps the name you joined with until you rejoin).
fn name_in(mls: &sotto_cases::Mls, group_id: &[u8]) -> String {
    let key = mls.signature_key();
    let named = mls.member_keys(group_id).ok().and_then(|ks| ks.into_iter().find(|(_, k)| *k == key)).map(|(n, _)| n);
    String::from_utf8_lossy(named.as_deref().unwrap_or(mls.identity())).to_string()
}

async fn my_name_in(s: &Session, g: &GroupRecord) -> String {
    let mut rt = s.cases_rt.lock().await;
    if ensure_mls(s, &mut rt).await.is_err() {
        return String::new();
    }
    rt.mls.as_ref().map(|m| name_in(m, &g.group_id)).unwrap_or_default()
}

async fn my_key(s: &Session) -> Result<Vec<u8>, ApiError> {
    let mut rt = s.cases_rt.lock().await;
    ensure_mls(s, &mut rt).await?;
    Ok(rt.mls.as_ref().ok_or(ApiError::Internal)?.signature_key())
}

async fn list_messages(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let g = record(&s, &gid(&id)?)?;
    let me = my_key(&s).await?;
    let mut all = messages(&s, &g.id);
    let start = all.len().saturating_sub(500);
    let v: Vec<Value> = all.drain(start..).map(|m| msg_json(&s, &g.id, &m, &me)).collect();
    Ok(Json(json!(v)))
}

#[derive(Deserialize)]
struct SendIn {
    text: String,
    reply_to: Option<String>,
}

async fn send(State(agent): A, Path(id): Path<String>, Json(b): Json<SendIn>) -> R {
    let s = agent.session().await?;
    let reply_to = b.reply_to.as_deref().map(gid).transpose()?;
    Ok(Json(post_text(&agent, &s, &gid(&id)?, b.text, reply_to, None).await?))
}

/// Post a text message as this desk. `via`: an integration's name, shown as the author
/// ("CI alerts (via Ada)") so nobody mistakes a script for a person.
pub(crate) async fn post_text(agent: &Arc<Agent>, s: &Arc<Session>, id: &Id, text: String, reply_to: Option<Id>, via: Option<&str>) -> Result<Value, ApiError> {
    let (agent, s) = (agent.clone(), s.clone());
    let mut g = record(&s, id)?;
    if g.removed {
        return Err(ApiError::Invalid("You're no longer in this group.".into()));
    }
    let body = text.trim_end().to_string();
    if body.trim().is_empty() {
        return Err(ApiError::Invalid("Write a message first.".into()));
    }
    if body.len() > 12_000 {
        return Err(ApiError::Invalid("That message is too long. Send it in parts.".into()));
    }
    let mid = new_id();
    let now_ms = time::now_ms();
    let me = match via {
        Some(v) => format!("{v} (via {})", my_name(&s).await),
        None => my_name(&s).await,
    };
    let my_key = {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::Text { id: mid, name: me.clone(), body: body.clone(), sent_at: now_ms, reply_to }).await?;
        persist_mls(&s, &rt)?;
        rt.mls.as_ref().map(|m| m.signature_key()).unwrap_or_default()
    };
    let m = Stored {
        id: mid,
        author: me,
        author_key: my_key.clone(),
        mine: true,
        system: false,
        body: body.clone(),
        sent_at: now_ms,
        received_at: now_ms,
        expires_at: g.ttl_secs.map(|t| time::now() + t as u64),
        reply_to,
        reactions: vec![],
        edited: false,
    };
    put_msg(&s, &g.id, &m)?;
    g.preview = format!("You: {}", body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect::<String>());
    g.preview_at = time::now();
    g.last_activity = time::now();
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(msg_json(&s, &g.id, &m, &my_key))
}

#[derive(Deserialize)]
struct EditIn {
    text: String,
}

async fn edit(State(agent): A, Path((id, mid)): Path<(String, String)>, Json(b): Json<EditIn>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    let mid = gid(&mid)?;
    let (n, mut m) = find_msg(&s, &g.id, &mid).ok_or(ApiError::NotFound)?;
    if !m.mine || m.system {
        return Err(ApiError::Invalid("You can only edit your own messages.".into()));
    }
    let body = b.text.trim_end().to_string();
    if body.trim().is_empty() {
        return Err(ApiError::Invalid("Write something, or delete the message instead.".into()));
    }
    {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::Edit { target: mid, body: body.clone() }).await?;
        persist_mls(&s, &rt)?;
    }
    m.body = body;
    m.edited = true;
    s.cases.put(&n, &m)?;
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct ReactIn {
    emoji: String,
    on: bool,
}

async fn react(State(agent): A, Path((id, mid)): Path<(String, String)>, Json(b): Json<ReactIn>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    let mid = gid(&mid)?;
    let (n, mut m) = find_msg(&s, &g.id, &mid).ok_or(ApiError::NotFound)?;
    if b.emoji.is_empty() || b.emoji.chars().count() > 8 {
        return Err(ApiError::Invalid("That isn't a reaction.".into()));
    }
    let me = {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::React { target: mid, emoji: b.emoji.clone(), on: b.on }).await?;
        persist_mls(&s, &rt)?;
        rt.mls.as_ref().map(|m| m.signature_key()).unwrap_or_default()
    };
    m.reactions.retain(|(e, k)| !(e == &b.emoji && k == &me));
    if b.on {
        m.reactions.push((b.emoji, me));
    }
    s.cases.put(&n, &m)?;
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(json!({ "ok": true })))
}

async fn delete_message(State(agent): A, Path((id, mid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    let mid = gid(&mid)?;
    let (n, m) = find_msg(&s, &g.id, &mid).ok_or(ApiError::NotFound)?;
    if m.mine && !g.removed {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::Delete { target: mid }).await?;
        persist_mls(&s, &rt)?;
    }
    s.cases.delete(&n)?;
    crate::shared::remove(&s, &file_at(&g.id, &mid));
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct MemberIn {
    contact_id: String,
}

async fn add_member(State(agent): A, Path(id): Path<String>, Json(b): Json<MemberIn>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    if !manages(&s, &g).await? {
        return Err(ApiError::Invalid("Only the person who made the group, or an admin, can add people.".into()));
    }
    if g.members.len() >= MAX_MEMBERS {
        return Err(ApiError::Invalid(format!("A group can have up to {MAX_MEMBERS} people.")));
    }
    let contact = gid(&b.contact_id)?;
    let v = s.engine.contact(&contact).await.ok_or(ApiError::NotFound)?;
    if v.state != "ready" {
        return Err(ApiError::Invalid("They need to accept your conversation invite first.".into()));
    }
    if g.members.iter().any(|m| m.contact_id == Some(contact)) {
        return Err(ApiError::Invalid("They're already in this group.".into()));
    }
    let name = shared_name(v.their_name);
    g.members.push(Member { name, contact_id: Some(contact), pending: true, owner: false });
    save(&s, &g)?;
    s.engine.send_payload(&contact, &Payload::KeyPackageRequest { case_id: g.id }).await?;
    emit(&agent, &g);
    Ok(Json(group_json(&s, &g, &my_name_in(&s, &g).await)))
}

/// `who` is the member's conversation id (if you have one with them) or their name in the group.
async fn remove_member(State(agent): A, Path((id, who)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    if !manages(&s, &g).await? {
        return Err(ApiError::Invalid("Only the person who made the group, or an admin, can remove people.".into()));
    }
    let contact = unhex::<16>(&who).ok();
    let Some(m) = g.members.iter().find(|m| !m.owner && (contact.is_some() && m.contact_id == contact || m.name == who)).cloned() else {
        return Err(ApiError::NotFound);
    };
    let rt = s.cases_rt.lock().await;
    if g.owner {
        let mut x = extra(&s, &g.id);
        expel(&s, &rt, &mut g, &mut x, &m.name).await?;
        system(&s, &mut g, format!("You removed {} from the group.", m.name))?;
    } else if m.pending {
        // Someone this admin proposed who hasn't joined yet: just stop waiting for them.
        g.members.retain(|x| !(x.pending && x.contact_id == m.contact_id));
        let mut x = extra(&s, &g.id);
        x.proposed.retain(|(_, c)| Some(*c) != m.contact_id);
        save_extra(&s, &g.id, &x)?;
    } else {
        broadcast(&s, &rt, &mut g, &GroupMsg::Propose(Proposal::Remove { name: m.name.clone() })).await?;
        persist_mls(&s, &rt)?;
        system(&s, &mut g, format!("You asked to remove {}. It happens when the group's creator is next online.", m.name))?;
    }
    drop(rt);
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(group_json(&s, &g, &my_name_in(&s, &g).await)))
}

async fn send_file(State(agent): A, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    if g.removed {
        return Err(ApiError::Invalid("You're no longer in this group.".into()));
    }
    let name = crate::api::header_text(&headers, "x-file-name");
    let caption: String = crate::api::header_text(&headers, "x-caption").chars().take(2000).collect();
    let mime = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
    let mid = new_id();
    // Everyone else in the group reads it once; a little room for a retry.
    let reads = (g.members.iter().filter(|m| !m.pending).count() + 1).min(255) as u8;
    let (f, refs) = crate::shared::send(&s, &file_at(&g.id, &mid), &name, &mime, &body, 1, reads).await?;
    let file = refs.into_iter().next().ok_or(ApiError::Internal)?;
    let now_ms = time::now_ms();
    let me = my_name(&s).await;
    let my_key = {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::File { id: mid, name: me.clone(), file: Box::new(file), caption: caption.clone(), sent_at: now_ms }).await?;
        persist_mls(&s, &rt)?;
        rt.mls.as_ref().map(|m| m.signature_key()).unwrap_or_default()
    };
    let m = Stored {
        id: mid,
        author: me,
        author_key: my_key.clone(),
        mine: true,
        system: false,
        body: caption,
        sent_at: now_ms,
        received_at: now_ms,
        expires_at: g.ttl_secs.map(|t| time::now() + t as u64),
        reply_to: None,
        reactions: vec![],
        edited: false,
    };
    put_msg(&s, &g.id, &m)?;
    g.preview = format!("You: {}", f.name);
    g.preview_at = time::now();
    g.last_activity = time::now();
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(msg_json(&s, &g.id, &m, &my_key)))
}

#[derive(Deserialize)]
struct PinIn {
    on: bool,
}

async fn pin(State(agent): A, Path((id, mid)): Path<(String, String)>, Json(b): Json<PinIn>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    if !manages(&s, &g).await? {
        return Err(ApiError::Invalid("Only the person who made the group, or an admin, can pin messages.".into()));
    }
    let mid = gid(&mid)?;
    find_msg(&s, &g.id, &mid).ok_or(ApiError::NotFound)?;
    {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut g, &GroupMsg::Pin { target: mid, on: b.on }).await?;
        persist_mls(&s, &rt)?;
    }
    let mut x = extra(&s, &g.id);
    x.pins.retain(|p| p != &mid);
    if b.on {
        x.pins.push(mid);
        system(&s, &mut g, "You pinned a message.".into())?;
    }
    save_extra(&s, &g.id, &x)?;
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, Default)]
struct ReviewIn {
    #[serde(default)]
    greyscale: bool,
}

async fn review_file(State(agent): A, Path((id, mid)): Path<(String, String)>, body: Option<Json<ReviewIn>>) -> R {
    let s = agent.session().await?;
    let g = record(&s, &gid(&id)?)?;
    let at = file_at(&g.id, &gid(&mid)?);
    let f = crate::shared::get(&s, &at).ok_or(ApiError::Invalid("Only files can be reviewed.".into()))?;
    if !matches!(f.status, crate::shared::Status::Ready) {
        return Err(ApiError::Invalid("This file hasn't finished downloading.".into()));
    }
    let greyscale = body.map(|b| b.0.greyscale).unwrap_or_default();
    let report = crate::review::run(&s, crate::review::Source::Shared { at, blob: f.blob }, greyscale).await?;
    agent.emit(json!({ "type": "review", "key": report["key"] }));
    emit(&agent, &g);
    Ok(Json(report))
}

#[derive(Deserialize)]
struct AdminIn {
    name: String,
    on: bool,
}

/// The creator names (or un-names) an admin, by their name in the group.
async fn set_admin(State(agent): A, Path(id): Path<String>, Json(b): Json<AdminIn>) -> R {
    let s = agent.session().await?;
    let mut g = record(&s, &gid(&id)?)?;
    if !g.owner || g.removed {
        return Err(ApiError::Invalid("Only the person who made the group can choose admins.".into()));
    }
    if !g.members.iter().any(|m| m.name == b.name && !m.owner && !m.pending) {
        return Err(ApiError::NotFound);
    }
    let mut rt = s.cases_rt.lock().await;
    ensure_mls(&s, &mut rt).await?;
    let keys = rt.mls.as_ref().ok_or(ApiError::Internal)?.member_keys(&g.group_id).map_err(|_| ApiError::Internal)?;
    let key = keys.into_iter().find(|(n, _)| n == b.name.as_bytes()).map(|(_, k)| k).ok_or_else(|| {
        // Groups made before members were named from their own key packages can hold a name that
        // isn't the one their desk signs with.
        ApiError::Invalid(format!("{} joined before names were checked, so they can't be made an admin in this group. Make a new group to use admins.", b.name))
    })?;
    let mut x = extra(&s, &g.id);
    x.admins.retain(|(n, _)| n != &b.name);
    if b.on {
        x.admins.push((b.name.clone(), key));
    }
    save_extra(&s, &g.id, &x)?;
    broadcast(&s, &rt, &mut g, &GroupMsg::Admins(x.admins.clone())).await?;
    persist_mls(&s, &rt)?;
    drop(rt);
    system(&s, &mut g, if b.on { format!("You made {} an admin.", b.name) } else { format!("{} is no longer an admin.", b.name) })?;
    save(&s, &g)?;
    emit(&agent, &g);
    Ok(Json(group_json(&s, &g, &my_name_in(&s, &g).await)))
}

/// The group's safety number, built from every member's signing key. Members compare it out of
/// band; it changes whenever someone joins or leaves.
async fn safety(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let g = record(&s, &gid(&id)?)?;
    let mut rt = s.cases_rt.lock().await;
    ensure_mls(&s, &mut rt).await?;
    let keys = rt.mls.as_ref().ok_or(ApiError::Internal)?.member_keys(&g.group_id).map_err(|_| ApiError::Invalid("You're no longer in this group.".into()))?;
    let n = keys.len();
    let number = sotto_core::safety::group_safety_number(&keys.into_iter().map(|(_, k)| k).collect::<Vec<_>>());
    Ok(Json(json!({ "number": number, "members": n })))
}

#[cfg(test)]
mod tests {
    use super::mentions;

    #[test]
    fn mentions_match_whole_names_in_any_case() {
        assert!(mentions("@Ben can you check?", "Ben"));
        assert!(mentions("thanks @ben.", "Ben"));
        assert!(mentions("cc @Ada Lovelace, please", "Ada Lovelace"));
        assert!(!mentions("@Benjamin is on it", "Ben"));
        assert!(!mentions("@Ben_ops", "Ben"));
        assert!(!mentions("mail ben@ben.dev", "Ben"));
        assert!(!mentions("Ben, no at sign", "Ben"));
        assert!(!mentions("@", ""));
    }
}
