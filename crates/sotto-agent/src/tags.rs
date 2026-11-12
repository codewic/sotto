//! Tags: a short name like `@ada` that people can use to reach you, instead of passing an invite
//! link around. On your own relay `@ada` is enough; from anywhere else it's the full tag,
//! `ada@relay.example.org` (the tag, then the relay it lives on), like an email address.
//!
//! A tag is not an account and no server learns who you are. Your device keeps a small stock of
//! ordinary one-time invites on your relay under the tag. Someone who types the tag takes one and
//! connects through it exactly as through a link; each person gets their own fresh keys and
//! mailboxes. They arrive as a request you accept or remove, and your device restocks.
//!
//! What the relay learns: that the tag exists, and how often invites are taken. A tag is optional,
//! can be released at any time, and is refused in Extreme mode.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_core::keys::{sign, signing_public};
use sotto_core::wire::{Cmd, ErrCode, Resp, auth_message};
use sotto_core::{Id, hex, time, unhex};

use crate::ApiError;
use crate::state::{Agent, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

/// Invites kept ready on the relay.
const STOCK: u32 = 10;
/// Unused invites this device keeps open at most (each holds a mailbox until it expires).
const MAX_PENDING: usize = 60;

#[derive(Clone, Serialize, Deserialize)]
struct TagRecord {
    tag: String,
    relay: String,
    seed: [u8; 32],
    /// Contacts behind invites in stock (or taken but not yet used).
    pending: Vec<Id>,
    created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Request {
    tag: String,
    at: u64,
}

const KEY: &str = "tag/me";

/// Claiming, releasing and restocking take turns, so a restock can't bring back a released tag.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn record(s: &Session) -> Option<TagRecord> {
    s.cases.get::<TagRecord>(KEY).ok().flatten()
}

fn req_key(c: &Id) -> String {
    format!("tagreq/{}", hex(c))
}

/// Invites being made right now, before they're saved in the tag's record: hidden too, so they
/// never flash up in the conversation list.
static MAKING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Contacts that are only stocked invites: hidden from the conversation list.
pub fn hidden(s: &Session) -> Vec<String> {
    let mut v: Vec<String> = record(s).map(|r| r.pending.iter().map(|p| hex(p)).collect()).unwrap_or_default();
    v.extend(MAKING.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned());
    v
}

/// Stop hiding these as "being made" (they're saved in the record now, or deleted).
fn made(ids: &[Id]) {
    let gone: Vec<String> = ids.iter().map(|i| hex(i)).collect();
    MAKING.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| !gone.contains(x));
}

/// Whether a contact reached us through our tag and hasn't been accepted yet.
pub fn request_of(s: &Session, c: &Id) -> Option<String> {
    s.cases.get::<Request>(&req_key(c)).ok().flatten().map(|r| r.tag)
}

/// 3 to 24 characters: lowercase letters, digits, dot and underscore, starting with a letter.
/// The relay applies the same rule.
fn valid(t: &str) -> bool {
    let b = t.as_bytes();
    (3..=24).contains(&b.len()) && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'.' || *c == b'_')
}

/// `@Ada.M`, `ada.m` → `ada.m`. None if it can't be a tag.
pub fn normalize(t: &str) -> Option<String> {
    let t = t.trim().trim_start_matches('@').to_ascii_lowercase();
    valid(&t).then_some(t)
}

/// The relay part of a full tag (`relay.example.org:7400`) as a relay address, and back.
fn relay_of(host: &str) -> String {
    format!("ws://{}", host.trim().trim_start_matches("ws://").trim_end_matches('/'))
}

fn host_of(relay: &str) -> String {
    relay.trim_start_matches("ws://").trim_end_matches('/').to_string()
}

/// `@ada`, `ada`, `ada@relay.example.org` or `@ada@relay.example.org:7400` → (tag, relay if given).
pub fn parse_address(input: &str) -> Option<(String, Option<String>)> {
    let t = input.trim().trim_start_matches('@');
    let (tag, host) = match t.split_once('@') {
        Some((tag, host)) if !host.trim().is_empty() => (tag, Some(relay_of(host))),
        Some(_) => return None,
        None => (t, None),
    };
    Some((normalize(tag)?, host))
}

/// Make `n` fresh one-time invites (a fresh identity each), returning (contact id, link, expiry).
async fn make_invites(s: &Session, n: u32) -> Result<Vec<(Id, String, u64)>, ApiError> {
    let label = s.engine.profile().await.display_name;
    let mut out = Vec::new();
    for _ in 0..n {
        let (id, link) = s.engine.create_invite("", &label, false).await?;
        MAKING.lock().unwrap_or_else(|e| e.into_inner()).push(hex(&id));
        let exp = s.engine.contact(&id).await.and_then(|c| c.invite_expires).unwrap_or_else(|| time::now() + 7 * 86_400);
        out.push((id, link, exp));
    }
    Ok(out)
}

/// Add invites under the tag (an empty list just asks how many are left).
async fn tag_add(s: &Session, r: &TagRecord, invites: Vec<(Vec<u8>, u64)>) -> Result<u32, ApiError> {
    let mut digest = Vec::new();
    for (i, exp) in &invites {
        digest.extend_from_slice(&sotto_core::kdf::sha256(i));
        digest.extend_from_slice(&exp.to_be_bytes());
    }
    let digest = sotto_core::kdf::sha256(&digest);
    let (tag, seed, owner) = (r.tag.clone(), r.seed, signing_public(&r.seed));
    let conn = s.engine.relay(&r.relay).await;
    match conn.request(move |n| Cmd::TagAdd { sig: sign(&seed, &auth_message(n, "tagadd", &[tag.as_bytes(), &digest])), tag, owner, invites }).await {
        Ok(Resp::TagLeft { left }) => Ok(left),
        Err(sotto_client::Error::Relay(ErrCode::Auth)) => Err(ApiError::Invalid(format!("@{} is taken. Try another.", r.tag))),
        Err(e) => Err(e.into()),
        Ok(_) => Err(ApiError::Internal),
    }
}

/// Bring the stock back up, and turn used invites into requests.
async fn restock(agent: &Agent, s: &Session) -> Result<(), ApiError> {
    let _turn = TURN.lock().await;
    let Some(mut r) = record(s) else { return Ok(()) };
    let now = time::now();
    let mut keep = Vec::new();
    for id in r.pending.drain(..) {
        match s.engine.contact(&id).await {
            Some(c) if c.state == "invited" && c.invite_expires.is_some_and(|e| e > now) => keep.push(id),
            Some(c) if c.state == "ready" => {
                s.cases.put(&req_key(&id), &Request { tag: r.tag.clone(), at: now })?;
                agent.emit(json!({ "type": "contacts" }));
            }
            // Expired or broken: close its mailbox.
            Some(_) => {
                let _ = s.engine.delete_contact(&id).await;
            }
            None => {}
        }
    }
    r.pending = keep;
    s.cases.put(KEY, &r)?;
    let left = tag_add(s, &r, vec![]).await?;
    let room = MAX_PENDING.saturating_sub(r.pending.len()) as u32;
    let need = STOCK.saturating_sub(left).min(room);
    if need > 0 {
        let fresh = make_invites(s, need).await?;
        r.pending.extend(fresh.iter().map(|(id, _, _)| *id));
        s.cases.put(KEY, &r)?;
        made(&r.pending);
        tag_add(s, &r, fresh.into_iter().map(|(_, link, exp)| (link.into_bytes(), exp)).collect()).await?;
    }
    Ok(())
}

/// After unlock: restock now and twice a day.
pub fn start(agent: &Arc<Agent>, s: &Arc<Session>) {
    let (agent, s) = (agent.clone(), s.clone());
    tokio::spawn(async move {
        loop {
            let _ = restock(&agent, &s).await;
            tokio::time::sleep(std::time::Duration::from_secs(12 * 3600)).await;
        }
    });
}

/// A contact changed: if it was one of our stocked invites, someone used the tag.
pub async fn on_contact(agent: &Arc<Agent>, id: &str) {
    let Ok(s) = agent.session().await else { return };
    if hidden(&s).iter().any(|h| h == id) {
        let _ = restock(agent, &s).await;
    }
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/tag", get(show).put(claim).delete(release))
        .route("/api/tag/connect", post(connect))
        .route("/api/tag/requests/{id}/accept", post(accept))
        .route("/api/tag/requests/{id}", delete(decline))
}

fn view(s: &Session) -> Value {
    match record(s) {
        // `address` is the full tag: what someone on another relay types.
        Some(r) => json!({ "tag": r.tag, "address": format!("{}@{}", r.tag, host_of(&r.relay)), "relay": r.relay, "ready": r.pending.len(), "created_at": r.created_at }),
        None => json!({ "tag": null }),
    }
}

async fn show(State(agent): A) -> R {
    let s = agent.session().await?;
    Ok(Json(view(&s)))
}

#[derive(Deserialize)]
struct TagIn {
    tag: String,
}

async fn claim(State(agent): A, Json(b): Json<TagIn>) -> R {
    let s = agent.session().await?;
    if agent.calls_off().await {
        return Err(ApiError::Invalid("Tags are off in Extreme mode: anyone who knows a tag can ask to reach you. Use one-time invite links.".into()));
    }
    let tag = normalize(&b.tag).ok_or_else(|| ApiError::Invalid("A tag is 3 to 24 characters: lowercase letters, numbers, dots and underscores, starting with a letter.".into()))?;
    let _turn = TURN.lock().await;
    let old = record(&s);
    if old.as_ref().is_some_and(|o| o.tag == tag) {
        return Ok(Json(view(&s)));
    }
    let relay = s.engine.profile().await.default_relay;
    let mut r = TagRecord { tag, relay, seed: sotto_core::rand::bytes(), pending: vec![], created_at: time::now() };
    let fresh = make_invites(&s, STOCK).await?;
    r.pending = fresh.iter().map(|(id, _, _)| *id).collect();
    let result = tag_add(&s, &r, fresh.into_iter().map(|(_, link, exp)| (link.into_bytes(), exp)).collect()).await;
    if result.is_err() {
        for id in &r.pending {
            let _ = s.engine.delete_contact(id).await;
        }
    }
    let saved = match &result {
        Ok(_) => {
            // Only give up the old tag once the new one is ours.
            if let Some(o) = old {
                release_inner(&s, &o).await;
            }
            s.cases.put(KEY, &r).map_err(ApiError::from)
        }
        Err(_) => Ok(()),
    };
    made(&r.pending);
    saved?;
    match result {
        Ok(_) => {
            agent.emit(json!({ "type": "contacts" }));
            Ok(Json(view(&s)))
        }
        Err(e) => Err(e),
    }
}

async fn release_inner(s: &Session, r: &TagRecord) {
    let (tag, seed) = (r.tag.clone(), r.seed);
    let conn = s.engine.relay(&r.relay).await;
    let _ = conn.request(move |n| Cmd::TagDel { sig: sign(&seed, &auth_message(n, "tagdel", &[tag.as_bytes()])), tag }).await;
    for id in &r.pending {
        let _ = s.engine.delete_contact(id).await;
    }
    let _ = s.cases.delete(KEY);
}

/// Give the tag up: the relay forgets it, and unused invites are closed.
async fn release(State(agent): A) -> R {
    let s = agent.session().await?;
    let _turn = TURN.lock().await;
    if let Some(r) = record(&s) {
        release_inner(&s, &r).await;
    }
    agent.emit(json!({ "type": "contacts" }));
    Ok(Json(view(&s)))
}

/// Reach someone by their tag: take one of their invites and connect through it. A full tag
/// (`ada@relay.example.org`) is looked up on that relay; a bare one on yours.
async fn connect(State(agent): A, Json(b): Json<TagIn>) -> R {
    let s = agent.session().await?;
    let (tag, named) = parse_address(&b.tag).ok_or_else(|| ApiError::Invalid("That isn't a tag. Tags look like @ada, or ada@relay.example.org for someone on another relay.".into()))?;
    let mine = s.engine.profile().await.default_relay;
    let relay = named.unwrap_or_else(|| mine.clone());
    // Same rules as any relay someone else names: a real address, and an onion service when
    // this device only uses Tor.
    s.engine.check_remote_relay(&relay).map_err(|_| ApiError::Invalid(format!("Sotto can't reach {} from this device. In High risk and Extreme modes only onion relays are allowed.", host_of(&relay))))?;
    if record(&s).is_some_and(|r| r.tag == tag && r.relay == relay) {
        return Err(ApiError::Invalid("That's your own tag.".into()));
    }
    let where_ = if relay == mine { "your relay".to_string() } else { host_of(&relay) };
    let t = tag.clone();
    let conn = s.engine.relay(&relay).await;
    let take = conn.request(move |_| Cmd::TagTake { tag: t });
    // A relay that doesn't answer shouldn't keep someone waiting for minutes.
    let taken = tokio::time::timeout(std::time::Duration::from_secs(20), take).await.unwrap_or(Err(sotto_client::Error::Offline));
    let invite = match taken {
        Ok(Resp::TagInvite { invite }) => invite,
        Err(sotto_client::Error::Relay(ErrCode::NotFound)) => {
            return Err(ApiError::Invalid(format!("Nobody on {where_} has the tag @{tag}, or they aren't taking new contacts right now. Check the spelling and the relay, or ask them for an invite link.")));
        }
        Err(sotto_client::Error::Offline) => return Err(ApiError::Invalid(format!("Couldn't reach {where_}. Check the relay part of the tag, and that you're online."))),
        Err(e) => return Err(e.into()),
        Ok(_) => return Err(ApiError::Internal),
    };
    let link = String::from_utf8(invite).map_err(|_| ApiError::Internal)?;
    let me = s.engine.profile().await.display_name;
    let label = if relay == mine { format!("@{tag}") } else { format!("{tag}@{}", host_of(&relay)) };
    let id = s.engine.accept_invite(&link, &label, &me).await?;
    agent.emit(json!({ "type": "contacts" }));
    Ok(Json(json!({ "contact_id": hex(&id) })))
}

async fn accept(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let id = unhex::<16>(&id).map_err(|_| ApiError::NotFound)?;
    s.cases.delete(&req_key(&id))?;
    agent.emit(json!({ "type": "contacts" }));
    Ok(Json(json!({ "ok": true })))
}

/// Remove someone who reached us by tag: the conversation and its keys go.
async fn decline(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let id = unhex::<16>(&id).map_err(|_| ApiError::NotFound)?;
    s.cases.delete(&req_key(&id))?;
    s.engine.delete_contact(&id).await?;
    agent.emit(json!({ "type": "contacts" }));
    Ok(Json(json!({ "ok": true })))
}
