//! Channels: one-way posts from a desk to the people who chose to follow it. Every signal travels
//! over the owner's existing 1:1 conversation with each subscriber, so subscribers never learn who
//! else follows, or how many. Replies go privately to the owner. See docs/groups.md.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, header};
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_core::envelope::{ChannelSignal, Payload};
use sotto_core::{Id, hex, new_id, time, unhex};
use tokio::sync::Mutex;

use crate::ApiError;
use crate::state::{Agent, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

/// Largest channel to start with: every post is sent once per subscriber.
const MAX_SUBSCRIBERS: usize = 500;
/// Posts a new subscriber receives when they join.
const HISTORY: usize = 20;
const MAX_POST: usize = 12_000;

/// Serialises read-modify-write of channel records between API calls and incoming signals.
static LOCK: Mutex<()> = Mutex::const_new(());

#[derive(Clone, Serialize, Deserialize)]
struct Subscriber {
    contact_id: Id,
    joined: bool,
    since: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct ChannelRecord {
    id: Id,
    title: String,
    about: String,
    owner: bool,
    /// Following side: the conversation this channel comes from.
    from: Option<Id>,
    /// Owner side only. Never leaves this desk.
    subscribers: Vec<Subscriber>,
    /// Following side: invited, not yet joined.
    invited: bool,
    /// Following side: removed by the owner, or the channel closed.
    closed: bool,
    removed: bool,
    created_at: u64,
    unread: u32,
    last_activity: u64,
    preview: String,
    preview_at: u64,
    pinned: bool,
    archived: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Post {
    id: Id,
    body: String,
    sent_at: u64,
    received_at: u64,
    edited: bool,
    replies: u32,
}

#[derive(Clone, Serialize, Deserialize)]
struct Reply {
    id: Id,
    post_id: Id,
    /// Owner side: who wrote it. Following side: `None`, it's your own.
    contact_id: Option<Id>,
    body: String,
    sent_at: u64,
}

fn key(id: &Id) -> String {
    format!("chan/{}", hex(id))
}

/// Whether this desk made the channel (only the owner posts).
pub fn owns(s: &Session, id: &Id) -> bool {
    record(s, id).map(|c| c.owner).unwrap_or(false)
}

/// A channel's name, for lists elsewhere (Safe review).
pub fn title_of(s: &Session, id: &Id) -> Option<String> {
    record(s, id).ok().map(|c| c.title)
}

/// Per-channel settings kept beside the record: mute, and (following side) whether the owner may
/// see when you've read posts, and up to which post we already told them.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Extra {
    muted: bool,
    receipts: bool,
    reported: u64,
}

fn extra(s: &Session, cid: &Id) -> Extra {
    s.cases.get::<Extra>(&format!("cx/{}", hex(cid))).ok().flatten().unwrap_or_default()
}

fn save_extra(s: &Session, cid: &Id, x: &Extra) -> Result<(), ApiError> {
    s.cases.put(&format!("cx/{}", hex(cid)), x)?;
    Ok(())
}

/// Owner side: which followers said they've read a post.
fn seen_key(cid: &Id, pid: &Id) -> String {
    format!("cseen/{}/{}", hex(cid), hex(pid))
}

fn seen(s: &Session, cid: &Id, pid: &Id) -> Vec<Id> {
    s.cases.get::<Vec<Id>>(&seen_key(cid, pid)).ok().flatten().unwrap_or_default()
}

fn file_at(cid: &Id, pid: &Id) -> String {
    format!("c/{}/{}", hex(cid), hex(pid))
}

/// Followers per uploaded copy of a file: each copy can be read at most 255 times.
const PER_COPY: usize = 250;

fn records(s: &Session) -> Vec<ChannelRecord> {
    s.cases.names("chan/").iter().filter_map(|n| s.cases.get::<ChannelRecord>(n).ok().flatten()).collect()
}

fn record(s: &Session, id: &Id) -> Result<ChannelRecord, ApiError> {
    s.cases.get(&key(id))?.ok_or(ApiError::NotFound)
}

fn save(s: &Session, c: &ChannelRecord) -> Result<(), ApiError> {
    s.cases.put(&key(&c.id), c)?;
    Ok(())
}

fn post_prefix(cid: &Id) -> String {
    format!("cpost/{}/", hex(cid))
}

fn reply_prefix(cid: &Id, pid: &Id) -> String {
    format!("creply/{}/{}/", hex(cid), hex(pid))
}

fn put_post(s: &Session, cid: &Id, p: &Post) -> Result<(), ApiError> {
    s.cases.put(&format!("{}{:020}-{}", post_prefix(cid), p.received_at, hex(&p.id)), p)?;
    Ok(())
}

fn find_post(s: &Session, cid: &Id, pid: &Id) -> Option<(String, Post)> {
    let suffix = format!("-{}", hex(pid));
    let name = s.cases.names(&post_prefix(cid)).into_iter().find(|n| n.ends_with(&suffix))?;
    let p = s.cases.get::<Post>(&name).ok().flatten()?;
    Some((name, p))
}

fn posts(s: &Session, cid: &Id) -> Vec<Post> {
    let mut names = s.cases.names(&post_prefix(cid));
    names.sort();
    names.iter().filter_map(|n| s.cases.get::<Post>(n).ok().flatten()).collect()
}

fn replies(s: &Session, cid: &Id, pid: &Id) -> Vec<Reply> {
    let mut names = s.cases.names(&reply_prefix(cid, pid));
    names.sort();
    names.iter().filter_map(|n| s.cases.get::<Reply>(n).ok().flatten()).collect()
}

/// Everything a channel left on this desk: posts, replies and the record.
fn wipe(s: &Session, cid: &Id) -> Result<(), ApiError> {
    for n in s.cases.names(&post_prefix(cid)) {
        s.cases.delete(&n)?;
    }
    for n in s.cases.names(&format!("creply/{}/", hex(cid))) {
        s.cases.delete(&n)?;
    }
    for n in s.cases.names(&format!("cseen/{}/", hex(cid))) {
        s.cases.delete(&n)?;
    }
    crate::shared::remove_all(s, &format!("c/{}/", hex(cid)));
    s.cases.delete(&format!("cx/{}", hex(cid)))?;
    s.cases.delete(&key(cid))?;
    Ok(())
}

fn touch(c: &mut ChannelRecord, preview: String) {
    c.preview = preview.chars().take(160).collect();
    c.preview_at = time::now();
    c.last_activity = time::now();
}

fn emit(agent: &Agent, id: &Id) {
    agent.emit(json!({ "type": "channel", "id": hex(id) }));
}

fn channel_json(s: &Session, c: &ChannelRecord) -> Value {
    let x = extra(s, &c.id);
    json!({
        "muted": x.muted,
        "receipts": x.receipts,
        "id": hex(&c.id),
        "title": c.title,
        "about": c.about,
        "owner": c.owner,
        "from": c.from.map(|f| hex(&f)),
        "invited": c.invited,
        "closed": c.closed,
        "removed": c.removed,
        // Only the owner's own desk knows who follows; a follower's desk never has the list.
        "subscribers": if c.owner {
            Some(c.subscribers.iter().map(|x| json!({ "contact_id": hex(&x.contact_id), "joined": x.joined, "since": x.since })).collect::<Vec<_>>())
        } else {
            None
        },
        "subscriber_count": c.owner.then(|| c.subscribers.iter().filter(|x| x.joined).count()),
        "unread": c.unread,
        "last_activity": c.last_activity,
        "preview": c.preview,
        "preview_at": c.preview_at,
        "pinned": c.pinned,
        "archived": c.archived,
        "created_at": c.created_at,
    })
}

fn post_json(s: &Session, c: &ChannelRecord, p: &Post) -> Value {
    let file = crate::shared::get(s, &file_at(&c.id, &p.id));
    json!({
        "id": hex(&p.id),
        "body": if file.is_some() { "" } else { p.body.as_str() },
        "file": file.map(|f| crate::shared::view(&f, &p.body)),
        "sent_at": p.sent_at,
        "edited": p.edited,
        "replies": p.replies,
        // Owner only: followers who chose to say they've read it.
        "seen": c.owner.then(|| seen(s, &c.id, &p.id).len()),
    })
}

fn reply_json(r: &Reply) -> Value {
    json!({ "id": hex(&r.id), "post_id": hex(&r.post_id), "contact_id": r.contact_id.map(|c| hex(&c)), "mine": r.contact_id.is_none(), "body": r.body, "sent_at": r.sent_at })
}

/// Send to each contact one after another; a failure for one never stops the rest.
fn fan_out(s: &Arc<Session>, to: Vec<Id>, signal: ChannelSignal) {
    if to.is_empty() {
        return;
    }
    let s = s.clone();
    tokio::spawn(async move {
        let payload = Payload::Channel(signal);
        for c in to {
            let _ = s.engine.send_payload(&c, &payload).await;
        }
    });
}

fn joined(c: &ChannelRecord) -> Vec<Id> {
    c.subscribers.iter().filter(|x| x.joined).map(|x| x.contact_id).collect()
}

fn clean(text: &str, max: usize) -> String {
    text.trim().chars().take(max).collect()
}

// ---------------------------------------------------------------------------------------------
// Incoming signals (from a 1:1 conversation)

pub async fn on_signal(agent: &Arc<Agent>, s: &Arc<Session>, contact: Id, signal: ChannelSignal) -> Result<(), ApiError> {
    let _g = LOCK.lock().await;
    match signal {
        ChannelSignal::Invite { channel_id, title, about } => {
            let now = time::now();
            let mut c = match record(s, &channel_id) {
                Ok(c) if c.owner || c.from != Some(contact) => return Ok(()),
                Ok(c) if !c.closed && !c.invited => return Ok(()),
                Ok(c) => c,
                Err(_) => ChannelRecord {
                    id: channel_id,
                    title: String::new(),
                    about: String::new(),
                    owner: false,
                    from: Some(contact),
                    subscribers: vec![],
                    invited: true,
                    closed: false,
                    removed: false,
                    created_at: now,
                    unread: 0,
                    last_activity: now,
                    preview: String::new(),
                    preview_at: now,
                    pinned: false,
                    archived: false,
                },
            };
            c.title = clean(&title, 80);
            c.about = clean(&about, 300);
            c.invited = true;
            c.closed = false;
            c.removed = false;
            c.unread = 1;
            touch(&mut c, "You're invited to follow this channel.".into());
            save(s, &c)?;
            emit(agent, &c.id);
        }
        ChannelSignal::Join { channel_id } => {
            let Ok(mut c) = record(s, &channel_id) else { return Ok(()) };
            if !c.owner {
                return Ok(());
            }
            // No join links: only someone the owner invited can join.
            let Some(sub) = c.subscribers.iter_mut().find(|x| x.contact_id == contact) else { return Ok(()) };
            if sub.joined {
                return Ok(());
            }
            sub.joined = true;
            sub.since = time::now();
            save(s, &c)?;
            emit(agent, &c.id);
            // Catch the new follower up on recent posts.
            let all = posts(s, &channel_id);
            let recent: Vec<Post> = all[all.len().saturating_sub(HISTORY)..].to_vec();
            let s2 = s.clone();
            tokio::spawn(async move {
                for p in recent {
                    let at = file_at(&channel_id, &p.id);
                    let signal = if crate::shared::get(&s2, &at).is_some() {
                        match crate::shared::copy_for_one(&s2, &at).await {
                            Ok(file) => ChannelSignal::File { channel_id, post_id: p.id, file, caption: p.body, sent_at: p.sent_at },
                            Err(_) => continue,
                        }
                    } else {
                        ChannelSignal::Post { channel_id, post_id: p.id, body: p.body, sent_at: p.sent_at }
                    };
                    let _ = s2.engine.send_payload(&contact, &Payload::Channel(signal)).await;
                }
            });
        }
        ChannelSignal::Leave { channel_id } => {
            let Ok(mut c) = record(s, &channel_id) else { return Ok(()) };
            if !c.owner {
                return Ok(());
            }
            let before = c.subscribers.len();
            c.subscribers.retain(|x| x.contact_id != contact);
            if c.subscribers.len() != before {
                save(s, &c)?;
                emit(agent, &c.id);
            }
        }
        ChannelSignal::Reply { channel_id, post_id, reply_id, body, sent_at } => {
            let Ok(mut c) = record(s, &channel_id) else { return Ok(()) };
            if !c.owner || !c.subscribers.iter().any(|x| x.contact_id == contact && x.joined) {
                return Ok(());
            }
            let Some((name, mut p)) = find_post(s, &channel_id, &post_id) else { return Ok(()) };
            let body = clean(&body, MAX_POST);
            if body.is_empty() {
                return Ok(());
            }
            let r = Reply { id: reply_id, post_id, contact_id: Some(contact), body: body.clone(), sent_at };
            s.cases.put(&format!("{}{:020}-{}", reply_prefix(&channel_id, &post_id), time::now_ms(), hex(&reply_id)), &r)?;
            p.replies += 1;
            s.cases.put(&name, &p)?;
            c.unread += 1;
            touch(&mut c, format!("New reply: {body}"));
            save(s, &c)?;
            emit(agent, &c.id);
        }
        ChannelSignal::Seen { channel_id, post_ids } => {
            let Ok(c) = record(s, &channel_id) else { return Ok(()) };
            if !c.owner || !c.subscribers.iter().any(|x| x.contact_id == contact && x.joined) {
                return Ok(());
            }
            for pid in post_ids.into_iter().take(500) {
                if find_post(s, &channel_id, &pid).is_some() {
                    let mut v = seen(s, &channel_id, &pid);
                    if !v.contains(&contact) {
                        v.push(contact);
                        s.cases.put(&seen_key(&channel_id, &pid), &v)?;
                    }
                }
            }
            emit(agent, &c.id);
        }
        owner_signal => from_owner(agent, s, contact, owner_signal)?,
    }
    Ok(())
}

/// Closed, Meta, Post, Edit and Delete: they count only from the conversation the channel came
/// from, and posts only once we follow it.
fn from_owner(agent: &Arc<Agent>, s: &Arc<Session>, contact: Id, signal: ChannelSignal) -> Result<(), ApiError> {
    let channel_id = match &signal {
        ChannelSignal::Closed { channel_id, .. }
        | ChannelSignal::Meta { channel_id, .. }
        | ChannelSignal::Post { channel_id, .. }
        | ChannelSignal::Edit { channel_id, .. }
        | ChannelSignal::Delete { channel_id, .. }
        | ChannelSignal::File { channel_id, .. } => *channel_id,
        _ => return Ok(()),
    };
    let Ok(mut c) = record(s, &channel_id) else { return Ok(()) };
    if c.owner || c.from != Some(contact) {
        return Ok(());
    }
    match signal {
        ChannelSignal::Closed { removed, .. } => {
            if c.invited {
                // A withdrawn invitation leaves nothing behind.
                wipe(s, &c.id)?;
                emit(agent, &c.id);
                return Ok(());
            }
            c.closed = true;
            c.removed = removed;
            touch(&mut c, if removed { "You no longer follow this channel." } else { "This channel was closed." }.into());
        }
        ChannelSignal::Meta { title, about, .. } => {
            c.title = clean(&title, 80);
            c.about = clean(&about, 300);
        }
        _ if c.invited || c.closed => return Ok(()),
        ChannelSignal::Post { post_id, body, sent_at, .. } => {
            if find_post(s, &channel_id, &post_id).is_some() {
                return Ok(());
            }
            let body = clean(&body, MAX_POST);
            put_post(s, &channel_id, &Post { id: post_id, body: body.clone(), sent_at, received_at: time::now_ms(), edited: false, replies: 0 })?;
            c.unread += 1;
            touch(&mut c, body);
        }
        ChannelSignal::File { post_id, file, caption, sent_at, .. } => {
            if find_post(s, &channel_id, &post_id).is_some() {
                return Ok(());
            }
            let name = file.name.clone();
            crate::shared::receive(agent, s, &file_at(&channel_id, &post_id), file, json!({ "type": "channel", "id": hex(&channel_id) }))?;
            put_post(s, &channel_id, &Post { id: post_id, body: clean(&caption, 2000), sent_at, received_at: time::now_ms(), edited: false, replies: 0 })?;
            c.unread += 1;
            touch(&mut c, name);
        }
        ChannelSignal::Edit { post_id, body, .. } => {
            if let Some((name, mut p)) = find_post(s, &channel_id, &post_id) {
                p.body = clean(&body, MAX_POST);
                p.edited = true;
                s.cases.put(&name, &p)?;
            }
        }
        ChannelSignal::Delete { post_id, .. } => {
            if let Some((name, _)) = find_post(s, &channel_id, &post_id) {
                s.cases.delete(&name)?;
                for n in s.cases.names(&reply_prefix(&channel_id, &post_id)) {
                    s.cases.delete(&n)?;
                }
                crate::shared::remove(s, &file_at(&channel_id, &post_id));
            }
        }
        _ => return Ok(()),
    }
    save(s, &c)?;
    emit(agent, &c.id);
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// API

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/channels", get(list).post(create))
        .route("/api/channels/{id}", get(one).patch(update).delete(remove))
        .route("/api/channels/{id}/read", post(read))
        .route("/api/channels/{id}/join", post(join))
        .route("/api/channels/{id}/posts", get(list_posts).post(send_post))
        .route("/api/channels/{id}/posts/{pid}", patch(edit_post).delete(delete_post))
        .route("/api/channels/{id}/posts/{pid}/replies", get(list_replies).post(send_reply))
        .route("/api/channels/{id}/subscribers", post(invite))
        .route("/api/channels/{id}/subscribers/{contact}", delete(remove_subscriber))
        .route("/api/channels/{id}/files", post(send_file))
        .route("/api/channels/{id}/posts/{pid}/review", post(review_file))
}

fn cid(s: &str) -> Result<Id, ApiError> {
    unhex::<16>(s).map_err(|_| ApiError::NotFound)
}

async fn list(State(agent): A) -> R {
    let s = agent.session().await?;
    let mut v = records(&s);
    v.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    Ok(Json(json!(v.iter().map(|c| channel_json(&s, c)).collect::<Vec<_>>())))
}

async fn one(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    Ok(Json(channel_json(&s, &record(&s, &cid(&id)?)?)))
}

/// Check each contact can be invited, and return their ids.
async fn ready_contacts(s: &Session, ids: &[String]) -> Result<Vec<Id>, ApiError> {
    let mut out = Vec::new();
    for c in ids {
        let id = cid(c)?;
        let v = s.engine.contact(&id).await.ok_or(ApiError::NotFound)?;
        if v.state != "ready" {
            let who = if v.nickname.is_empty() { "Someone" } else { &v.nickname };
            return Err(ApiError::Invalid(format!("{who} hasn't accepted your conversation invite yet.")));
        }
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
struct CreateIn {
    title: String,
    #[serde(default)]
    about: String,
    #[serde(default)]
    invite: Vec<String>,
}

async fn create(State(agent): A, Json(b): Json<CreateIn>) -> R {
    let s = agent.session().await?;
    let title = clean(&b.title, 80);
    if title.is_empty() {
        return Err(ApiError::Invalid("Give the channel a name.".into()));
    }
    if b.invite.len() > MAX_SUBSCRIBERS {
        return Err(ApiError::Invalid(format!("A channel can have up to {MAX_SUBSCRIBERS} followers.")));
    }
    let to = ready_contacts(&s, &b.invite).await?;
    let now = time::now();
    let mut c = ChannelRecord {
        id: new_id(),
        title,
        about: clean(&b.about, 300),
        owner: true,
        from: None,
        subscribers: to.iter().map(|id| Subscriber { contact_id: *id, joined: false, since: now }).collect(),
        invited: false,
        closed: false,
        removed: false,
        created_at: now,
        unread: 0,
        last_activity: now,
        preview: String::new(),
        preview_at: now,
        pinned: false,
        archived: false,
    };
    touch(&mut c, "You made this channel.".into());
    {
        let _g = LOCK.lock().await;
        save(&s, &c)?;
    }
    fan_out(&s, to, ChannelSignal::Invite { channel_id: c.id, title: c.title.clone(), about: c.about.clone() });
    emit(&agent, &c.id);
    Ok(Json(channel_json(&s, &c)))
}

#[derive(Deserialize)]
struct UpdateIn {
    title: Option<String>,
    about: Option<String>,
    pinned: Option<bool>,
    archived: Option<bool>,
    muted: Option<bool>,
    /// Following side: let the owner see when you've read posts.
    receipts: Option<bool>,
}

async fn update(State(agent): A, Path(id): Path<String>, Json(b): Json<UpdateIn>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let mut c = record(&s, &cid(&id)?)?;
    if b.title.is_some() || b.about.is_some() {
        if !c.owner {
            return Err(ApiError::Invalid("Only the person who made the channel can change it.".into()));
        }
        if let Some(t) = &b.title {
            let t = clean(t, 80);
            if t.is_empty() {
                return Err(ApiError::Invalid("Give the channel a name.".into()));
            }
            c.title = t;
        }
        if let Some(a) = &b.about {
            c.about = clean(a, 300);
        }
        fan_out(&s, joined(&c), ChannelSignal::Meta { channel_id: c.id, title: c.title.clone(), about: c.about.clone() });
    }
    if let Some(p) = b.pinned {
        c.pinned = p;
    }
    if let Some(a) = b.archived {
        c.archived = a;
    }
    if b.muted.is_some() || b.receipts.is_some() {
        let mut x = extra(&s, &c.id);
        if let Some(m) = b.muted {
            x.muted = m;
        }
        if let Some(r) = b.receipts {
            if c.owner {
                return Err(ApiError::Invalid("Read receipts are a follower's choice.".into()));
            }
            // Turning them on reports only what you read from now on.
            if r && !x.receipts {
                x.reported = time::now_ms();
            }
            x.receipts = r;
        }
        save_extra(&s, &c.id, &x)?;
    }
    save(&s, &c)?;
    emit(&agent, &c.id);
    Ok(Json(channel_json(&s, &c)))
}

/// The owner closes the channel for everyone; a follower leaves (or turns down the invitation).
/// Either way it's gone from this desk.
async fn remove(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let c = record(&s, &cid(&id)?)?;
    if c.owner {
        let everyone: Vec<Id> = c.subscribers.iter().map(|x| x.contact_id).collect();
        fan_out(&s, everyone, ChannelSignal::Closed { channel_id: c.id, removed: false });
    } else if let (Some(from), false) = (c.from, c.closed) {
        fan_out(&s, vec![from], ChannelSignal::Leave { channel_id: c.id });
    }
    wipe(&s, &c.id)?;
    emit(&agent, &c.id);
    Ok(Json(json!({ "ok": true })))
}

async fn read(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let mut c = record(&s, &cid(&id)?)?;
    if c.unread > 0 {
        c.unread = 0;
        save(&s, &c)?;
        emit(&agent, &c.id);
    }
    let mut x = extra(&s, &c.id);
    if let (Some(from), true, false, false) = (c.from, x.receipts, c.invited, c.closed) {
        let fresh: Vec<Post> = posts(&s, &c.id).into_iter().filter(|p| p.received_at > x.reported).collect();
        if let Some(last) = fresh.iter().map(|p| p.received_at).max() {
            x.reported = last;
            save_extra(&s, &c.id, &x)?;
            fan_out(&s, vec![from], ChannelSignal::Seen { channel_id: c.id, post_ids: fresh.iter().map(|p| p.id).collect() });
        }
    }
    Ok(Json(json!({ "ok": true })))
}

async fn join(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let mut c = record(&s, &cid(&id)?)?;
    let Some(from) = c.from else { return Err(ApiError::NotFound) };
    if !c.invited {
        return Ok(Json(channel_json(&s, &c)));
    }
    s.engine.send_payload(&from, &Payload::Channel(ChannelSignal::Join { channel_id: c.id })).await?;
    c.invited = false;
    c.unread = 0;
    touch(&mut c, "You follow this channel. New posts appear here.".into());
    save(&s, &c)?;
    emit(&agent, &c.id);
    Ok(Json(channel_json(&s, &c)))
}

async fn list_posts(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let c = record(&s, &cid(&id)?)?;
    Ok(Json(json!(posts(&s, &c.id).iter().map(|p| post_json(&s, &c, p)).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct TextIn {
    text: String,
}

async fn send_post(State(agent): A, Path(id): Path<String>, Json(b): Json<TextIn>) -> R {
    let s = agent.session().await?;
    Ok(Json(post_text(&agent, &s, &cid(&id)?, &b.text).await?))
}

/// Post to a channel this desk owns (from the app or an integration).
pub(crate) async fn post_text(agent: &Arc<Agent>, s: &Arc<Session>, id: &Id, text: &str) -> Result<Value, ApiError> {
    let _g = LOCK.lock().await;
    let mut c = record(s, id)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the person who made the channel can post in it.".into()));
    }
    let body = clean(text, MAX_POST);
    if body.is_empty() {
        return Err(ApiError::Invalid("Write something first.".into()));
    }
    let now_ms = time::now_ms();
    let p = Post { id: new_id(), body: body.clone(), sent_at: now_ms, received_at: now_ms, edited: false, replies: 0 };
    put_post(s, &c.id, &p)?;
    touch(&mut c, format!("You: {body}"));
    save(s, &c)?;
    fan_out(s, joined(&c), ChannelSignal::Post { channel_id: c.id, post_id: p.id, body, sent_at: now_ms });
    emit(agent, &c.id);
    Ok(post_json(s, &c, &p))
}

async fn edit_post(State(agent): A, Path((id, pid)): Path<(String, String)>, Json(b): Json<TextIn>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the person who made the channel can edit posts.".into()));
    }
    let pid = cid(&pid)?;
    let (name, mut p) = find_post(&s, &c.id, &pid).ok_or(ApiError::NotFound)?;
    let body = clean(&b.text, MAX_POST);
    if body.is_empty() {
        return Err(ApiError::Invalid("A post can't be empty. Delete it instead.".into()));
    }
    p.body = body.clone();
    p.edited = true;
    s.cases.put(&name, &p)?;
    fan_out(&s, joined(&c), ChannelSignal::Edit { channel_id: c.id, post_id: pid, body });
    emit(&agent, &c.id);
    Ok(Json(post_json(&s, &c, &p)))
}

async fn delete_post(State(agent): A, Path((id, pid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the person who made the channel can delete posts.".into()));
    }
    let pid = cid(&pid)?;
    let (name, _) = find_post(&s, &c.id, &pid).ok_or(ApiError::NotFound)?;
    s.cases.delete(&name)?;
    for n in s.cases.names(&reply_prefix(&c.id, &pid)) {
        s.cases.delete(&n)?;
    }
    fan_out(&s, joined(&c), ChannelSignal::Delete { channel_id: c.id, post_id: pid });
    emit(&agent, &c.id);
    Ok(Json(json!({ "ok": true })))
}

async fn list_replies(State(agent): A, Path((id, pid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let c = record(&s, &cid(&id)?)?;
    Ok(Json(json!(replies(&s, &c.id, &cid(&pid)?).iter().map(reply_json).collect::<Vec<_>>())))
}

async fn send_reply(State(agent): A, Path((id, pid)): Path<(String, String)>, Json(b): Json<TextIn>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let c = record(&s, &cid(&id)?)?;
    let (Some(from), false, false) = (c.from, c.invited, c.closed) else {
        return Err(ApiError::Invalid("You can reply only in a channel you follow.".into()));
    };
    let pid = cid(&pid)?;
    let (name, mut p) = find_post(&s, &c.id, &pid).ok_or(ApiError::NotFound)?;
    let body = clean(&b.text, MAX_POST);
    if body.is_empty() {
        return Err(ApiError::Invalid("Write something first.".into()));
    }
    let r = Reply { id: new_id(), post_id: pid, contact_id: None, body: body.clone(), sent_at: time::now_ms() };
    s.engine.send_payload(&from, &Payload::Channel(ChannelSignal::Reply { channel_id: c.id, post_id: pid, reply_id: r.id, body, sent_at: r.sent_at })).await?;
    s.cases.put(&format!("{}{:020}-{}", reply_prefix(&c.id, &pid), r.sent_at, hex(&r.id)), &r)?;
    p.replies += 1;
    s.cases.put(&name, &p)?;
    emit(&agent, &c.id);
    Ok(Json(reply_json(&r)))
}

#[derive(Deserialize)]
struct InviteIn {
    contact_ids: Vec<String>,
}

async fn invite(State(agent): A, Path(id): Path<String>, Json(b): Json<InviteIn>) -> R {
    let s = agent.session().await?;
    let to = ready_contacts(&s, &b.contact_ids).await?;
    let _g = LOCK.lock().await;
    let mut c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the person who made the channel can invite people.".into()));
    }
    let fresh: Vec<Id> = to.into_iter().filter(|id| !c.subscribers.iter().any(|x| &x.contact_id == id)).collect();
    if c.subscribers.len() + fresh.len() > MAX_SUBSCRIBERS {
        return Err(ApiError::Invalid(format!("A channel can have up to {MAX_SUBSCRIBERS} followers.")));
    }
    let now = time::now();
    c.subscribers.extend(fresh.iter().map(|id| Subscriber { contact_id: *id, joined: false, since: now }));
    save(&s, &c)?;
    fan_out(&s, fresh, ChannelSignal::Invite { channel_id: c.id, title: c.title.clone(), about: c.about.clone() });
    emit(&agent, &c.id);
    Ok(Json(channel_json(&s, &c)))
}

async fn remove_subscriber(State(agent): A, Path((id, contact)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let _g = LOCK.lock().await;
    let mut c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the person who made the channel can remove followers.".into()));
    }
    let contact = cid(&contact)?;
    let before = c.subscribers.len();
    c.subscribers.retain(|x| x.contact_id != contact);
    if c.subscribers.len() == before {
        return Err(ApiError::NotFound);
    }
    save(&s, &c)?;
    fan_out(&s, vec![contact], ChannelSignal::Closed { channel_id: c.id, removed: true });
    emit(&agent, &c.id);
    Ok(Json(channel_json(&s, &c)))
}

async fn send_file(State(agent): A, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> R {
    let s = agent.session().await?;
    let c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the person who made the channel can post in it.".into()));
    }
    let name = crate::api::header_text(&headers, "x-file-name");
    let caption = clean(&crate::api::header_text(&headers, "x-caption"), 2000);
    let mime = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
    let pid = new_id();
    let to = joined(&c);
    // One upload per 250 followers; each follower gets the copy meant for them.
    let copies = to.len().div_ceil(PER_COPY).max(1);
    let reads = (to.len().min(PER_COPY) + 2) as u8;
    let (f, refs) = crate::shared::send(&s, &file_at(&c.id, &pid), &name, &mime, &body, copies, reads).await?;
    let _g = LOCK.lock().await;
    let mut c = record(&s, &c.id)?;
    let now_ms = time::now_ms();
    let p = Post { id: pid, body: caption.clone(), sent_at: now_ms, received_at: now_ms, edited: false, replies: 0 };
    put_post(&s, &c.id, &p)?;
    touch(&mut c, format!("You: {}", f.name));
    save(&s, &c)?;
    let channel_id = c.id;
    for (chunk, file) in to.chunks(PER_COPY).zip(refs.into_iter()) {
        fan_out(&s, chunk.to_vec(), ChannelSignal::File { channel_id, post_id: pid, file: file.clone(), caption: caption.clone(), sent_at: now_ms });
    }
    emit(&agent, &c.id);
    Ok(Json(post_json(&s, &c, &p)))
}

#[derive(Deserialize, Default)]
struct ReviewIn {
    #[serde(default)]
    greyscale: bool,
}

async fn review_file(State(agent): A, Path((id, pid)): Path<(String, String)>, body: Option<Json<ReviewIn>>) -> R {
    let s = agent.session().await?;
    let c = record(&s, &cid(&id)?)?;
    let at = file_at(&c.id, &cid(&pid)?);
    let f = crate::shared::get(&s, &at).ok_or(ApiError::Invalid("Only files can be reviewed.".into()))?;
    if !matches!(f.status, crate::shared::Status::Ready) {
        return Err(ApiError::Invalid("This file hasn't finished downloading.".into()));
    }
    let greyscale = body.map(|b| b.0.greyscale).unwrap_or_default();
    let report = crate::review::run(&s, crate::review::Source::Shared { at, blob: f.blob }, greyscale).await?;
    agent.emit(json!({ "type": "review", "key": report["key"] }));
    emit(&agent, &c.id);
    Ok(Json(report))
}
