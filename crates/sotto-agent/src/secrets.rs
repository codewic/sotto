//! One-time secrets in a 1:1 conversation: a password, an API key, a recovery code.
//!
//! The secret travels end-to-end encrypted like any message. The sender's device keeps only its
//! label. The receiver's device keeps the text until it's revealed once (then deletes it and tells
//! the sender it was opened) or until it expires (then deletes it unread).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_core::envelope::{Hidden, Payload};
use sotto_core::{Id, hex, new_id, time, unhex};

use crate::ApiError;
use crate::state::{Agent, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

const MAX_TEXT: usize = 8000;
/// Longest an unopened secret may wait: 7 days.
const MAX_TTL: u64 = 7 * 86_400;

#[derive(Clone, Serialize, Deserialize)]
struct Secret {
    id: Id,
    contact: Id,
    label: String,
    /// Receiver only, until it's revealed or expires.
    body: Option<String>,
    mine: bool,
    sent_at: u64,
    expires_at: u64,
    opened_at: Option<u64>,
}

fn key(contact: &Id, id: &Id) -> String {
    format!("secret/{}/{}", hex(contact), hex(id))
}

fn of(s: &Session, contact: &Id) -> Vec<Secret> {
    s.cases.names(&format!("secret/{}/", hex(contact))).iter().filter_map(|n| s.cases.get::<Secret>(n).ok().flatten()).collect()
}

/// Drop the text of anything past its time.
fn sweep(s: &Session, mut x: Secret) -> Secret {
    if x.body.is_some() && time::now() >= x.expires_at {
        x.body = None;
        let _ = s.cases.put(&key(&x.contact, &x.id), &x);
    }
    x
}

fn view(x: &Secret) -> Value {
    let state = if x.opened_at.is_some() {
        "opened"
    } else if time::now() >= x.expires_at {
        "expired"
    } else {
        "waiting"
    };
    json!({
        "id": hex(&x.id),
        "label": x.label,
        "mine": x.mine,
        "sent_at": x.sent_at,
        "expires_at": x.expires_at,
        "opened_at": x.opened_at,
        "state": state,
    })
}

fn emit(agent: &Agent, contact: &Id) {
    agent.emit(json!({ "type": "secret", "contact_id": hex(contact) }));
}

/// Called after unlock: clear expired secrets every minute, so their text never outlives them.
pub fn start(s: &Arc<Session>) {
    let s = s.clone();
    tokio::spawn(async move {
        loop {
            for n in s.cases.names("secret/") {
                if let Ok(Some(x)) = s.cases.get::<Secret>(&n) {
                    sweep(&s, x);
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

/// Payloads from a 1:1 conversation.
pub fn on_payload(agent: &Arc<Agent>, s: &Session, contact: Id, p: Payload) -> Result<(), ApiError> {
    match p {
        Payload::Secret { id, label, body, sent_at, expires_at } => {
            if s.cases.get::<Secret>(&key(&contact, &id))?.is_some() {
                return Ok(());
            }
            // Never hold one longer than the limit, whatever the sender asked for.
            let expires_at = expires_at.min(time::now() + MAX_TTL);
            let x = Secret { id, contact, label: label.chars().take(80).collect(), body: Some(body.0.chars().take(MAX_TEXT).collect()), mine: false, sent_at, expires_at, opened_at: None };
            s.cases.put(&key(&contact, &id), &x)?;
            emit(agent, &contact);
        }
        Payload::SecretOpened { id } => {
            if let Some(mut x) = s.cases.get::<Secret>(&key(&contact, &id))? {
                if x.mine && x.opened_at.is_none() {
                    x.opened_at = Some(time::now());
                    s.cases.put(&key(&contact, &id), &x)?;
                    emit(agent, &contact);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/contacts/{id}/secrets", get(list).post(send))
        .route("/api/contacts/{id}/secrets/{sid}", axum::routing::delete(forget))
        .route("/api/contacts/{id}/secrets/{sid}/open", post(open))
}

fn ids(c: &str) -> Result<Id, ApiError> {
    unhex::<16>(c).map_err(|_| ApiError::NotFound)
}

async fn list(State(agent): A, Path(cid): Path<String>) -> R {
    let s = agent.session().await?;
    let mut v: Vec<Secret> = of(&s, &ids(&cid)?).into_iter().map(|x| sweep(&s, x)).collect();
    v.sort_by_key(|x| x.sent_at);
    Ok(Json(json!(v.iter().map(view).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct SendIn {
    label: String,
    text: String,
    /// Seconds an unopened secret waits before it's destroyed (up to 7 days).
    ttl_secs: u64,
}

async fn send(State(agent): A, Path(cid): Path<String>, Json(b): Json<SendIn>) -> R {
    let s = agent.session().await?;
    let contact = ids(&cid)?;
    let label: String = b.label.trim().chars().take(80).collect();
    if label.is_empty() {
        return Err(ApiError::Invalid("Say what it is, for example Staging database password. The label isn't secret.".into()));
    }
    if b.text.trim().is_empty() {
        return Err(ApiError::Invalid("There's no secret to send.".into()));
    }
    if b.text.chars().count() > MAX_TEXT {
        return Err(ApiError::Invalid("That's too long for a one-time secret. Send it as a file instead.".into()));
    }
    let now_ms = time::now_ms();
    let expires_at = time::now() + b.ttl_secs.clamp(300, MAX_TTL);
    let id = new_id();
    s.engine.send_payload(&contact, &Payload::Secret { id, label: label.clone(), body: Hidden(b.text), sent_at: now_ms, expires_at }).await?;
    let x = Secret { id, contact, label, body: None, mine: true, sent_at: now_ms, expires_at, opened_at: None };
    s.cases.put(&key(&contact, &id), &x)?;
    emit(&agent, &contact);
    Ok(Json(view(&x)))
}

/// Reveal a received secret. It's deleted from this device in the same step, so this works once.
async fn open(State(agent): A, Path((cid, sid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let (contact, id) = (ids(&cid)?, ids(&sid)?);
    let x = s.cases.get::<Secret>(&key(&contact, &id))?.ok_or(ApiError::NotFound)?;
    let mut x = sweep(&s, x);
    if x.mine {
        return Err(ApiError::Invalid("You sent this one; only they can open it.".into()));
    }
    let Some(text) = x.body.take() else {
        return Err(ApiError::Invalid(if x.opened_at.is_some() { "This secret was already opened, and it's gone." } else { "This secret expired before it was opened, and it's gone." }.into()));
    };
    x.opened_at = Some(time::now());
    s.cases.put(&key(&contact, &id), &x)?;
    let _ = s.engine.send_payload(&contact, &Payload::SecretOpened { id }).await;
    emit(&agent, &contact);
    let mut v = view(&x);
    v["text"] = json!(text);
    Ok(Json(v))
}

/// Remove a secret's record from this device (its text, if any, goes with it).
async fn forget(State(agent): A, Path((cid, sid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let (contact, id) = (ids(&cid)?, ids(&sid)?);
    s.cases.delete(&key(&contact, &id))?;
    emit(&agent, &contact);
    Ok(Json(json!({ "ok": true })))
}
