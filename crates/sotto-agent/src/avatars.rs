//! Avatars from Sotto's built-in collection. Only the choice travels (`v1:shape:hue:turn`), never
//! an image, so nothing about it can carry a payload or identify a device. Your choice goes to every
//! contact end-to-end encrypted, and to new contacts once they connect.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_core::envelope::Payload;
use sotto_core::{Id, hex, unhex};

use crate::ApiError;
use crate::state::{Agent, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

/// Sizes of the collection; the app draws the same shapes and hues.
const SHAPES: u32 = 20;
const HUES: u32 = 12;

/// `v1:<shape>:<hue>:<turn>` within the collection.
pub fn valid(id: &str) -> bool {
    let p: Vec<&str> = id.split(':').collect();
    let n = |i: usize, max: u32| p.get(i).and_then(|x| x.parse::<u32>().ok()).is_some_and(|v| v < max);
    p.len() == 4 && p[0] == "v1" && n(1, SHAPES) && n(2, HUES) && n(3, 4)
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Mine {
    id: Option<String>,
    /// Bumped on every change, so each contact is told once per change.
    version: u64,
}

fn mine(s: &Session) -> Mine {
    s.cases.get::<Mine>("avatar/me").ok().flatten().unwrap_or_default()
}

/// A contact's chosen avatar, if they picked one.
pub fn of(s: &Session, contact: &Id) -> Option<String> {
    s.cases.get::<String>(&format!("avatar/c/{}", hex(contact))).ok().flatten()
}

pub fn on_payload(agent: &Agent, s: &Session, contact: Id, id: Option<String>) -> Result<(), ApiError> {
    let key = format!("avatar/c/{}", hex(&contact));
    match id.filter(|i| valid(i)) {
        Some(i) => s.cases.put(&key, &i)?,
        None => s.cases.delete(&key)?,
    }
    agent.emit(json!({ "type": "contacts" }));
    Ok(())
}

/// Tell a contact our current avatar if they haven't heard this version yet.
async fn tell(s: &Session, contact: &Id, m: &Mine) {
    let key = format!("avatar/sent/{}", hex(contact));
    if s.cases.get::<u64>(&key).ok().flatten() == Some(m.version) {
        return;
    }
    if s.engine.send_payload(contact, &Payload::Avatar { id: m.id.clone() }).await.is_ok() {
        let _ = s.cases.put(&key, &m.version);
    }
}

/// A contact changed (for example a new one connected): bring them up to date.
pub async fn on_contact(agent: &Arc<Agent>, id: &str) {
    let Ok(s) = agent.session().await else { return };
    let m = mine(&s);
    if m.version == 0 {
        return;
    }
    let Ok(cid) = unhex::<16>(id) else { return };
    if s.engine.contact(&cid).await.is_some_and(|c| c.state == "ready") {
        tell(&s, &cid, &m).await;
    }
}

/// Contacts were added or changed: tell anyone who hasn't heard the current choice.
pub async fn sync(agent: &Arc<Agent>) {
    let Ok(s) = agent.session().await else { return };
    let m = mine(&s);
    if m.version == 0 {
        return;
    }
    for c in s.engine.contacts().await {
        if c.state == "ready"
            && let Ok(id) = unhex::<16>(&c.id) {
                tell(&s, &id, &m).await;
            }
    }
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new().route("/api/me/avatar", get(show).put(set))
}

async fn show(State(agent): A) -> R {
    let s = agent.session().await?;
    Ok(Json(json!({ "id": mine(&s).id })))
}

#[derive(Deserialize)]
struct SetIn {
    id: Option<String>,
}

async fn set(State(agent): A, Json(b): Json<SetIn>) -> R {
    let s = agent.session().await?;
    if b.id.as_deref().is_some_and(|i| !valid(i)) {
        return Err(ApiError::Invalid("That avatar isn't in the collection.".into()));
    }
    let m = Mine { id: b.id, version: mine(&s).version + 1 };
    s.cases.put("avatar/me", &m)?;
    // Everyone we talk to hears about it in the background.
    let s2 = s.clone();
    tokio::spawn(async move {
        for c in s2.engine.contacts().await {
            if c.state == "ready"
                && let Ok(id) = unhex::<16>(&c.id) {
                    tell(&s2, &id, &m).await;
                }
        }
    });
    agent.emit(json!({ "type": "me" }));
    Ok(Json(json!({ "id": mine(&s).id })))
}
