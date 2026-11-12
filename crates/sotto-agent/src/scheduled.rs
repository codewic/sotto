//! Send later: a message held on this device (encrypted, like everything else) and sent when it's
//! due. Nothing waits on a server, so it goes out only while Sotto is open and unlocked; anything
//! that came due while it was closed goes as soon as it's unlocked again.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::routing::{delete, get};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_core::{Id, hex, new_id, time, unhex};

use crate::ApiError;
use crate::state::{Agent, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

/// Furthest ahead a message can be scheduled: 30 days.
const MAX_AHEAD: u64 = 30 * 86_400;

/// Debug builds only: tests schedule a couple of seconds ahead and check every second.
fn quick() -> bool {
    cfg!(debug_assertions) && std::env::var_os("SOTTO_SCHEDULE_QUICK").is_some()
}

#[derive(Clone, Serialize, Deserialize)]
struct Scheduled {
    id: Id,
    /// "c" (conversation), "g" (group) or "h" (channel).
    kind: String,
    target: Id,
    text: String,
    at: u64,
    tries: u32,
}

fn key(id: &Id) -> String {
    format!("sched/{}", hex(id))
}

fn all(s: &Session) -> Vec<Scheduled> {
    s.cases.names("sched/").iter().filter_map(|n| s.cases.get::<Scheduled>(n).ok().flatten()).collect()
}

fn view(x: &Scheduled) -> Value {
    json!({ "id": hex(&x.id), "kind": x.kind, "target": hex(&x.target), "text": x.text, "at": x.at, "late": x.tries > 0 })
}

async fn deliver(agent: &Arc<Agent>, s: &Arc<Session>, x: &Scheduled) -> Result<(), ApiError> {
    match x.kind.as_str() {
        "c" => s.engine.send_text(&x.target, &x.text, None).await.map(|_| ()).map_err(ApiError::from),
        "g" => crate::groups::post_text(agent, s, &x.target, x.text.clone(), None, None).await.map(|_| ()),
        "h" => crate::channels::post_text(agent, s, &x.target, &x.text).await.map(|_| ()),
        _ => Err(ApiError::NotFound),
    }
}

/// After unlock: send whatever is due, checking every 10 seconds.
pub fn start(agent: &Arc<Agent>, s: &Arc<Session>) {
    let (agent, s) = (agent.clone(), s.clone());
    tokio::spawn(async move {
        loop {
            // One loop per unlocked session: after a lock (or a lock and unlock), this one stops.
            match agent.session().await {
                Ok(cur) if Arc::ptr_eq(&cur, &s) => {}
                _ => return,
            }
            let now = time::now();
            for mut x in all(&s).into_iter().filter(|x| x.at <= now) {
                match deliver(&agent, &s, &x).await {
                    Ok(()) => {
                        let _ = s.cases.delete(&key(&x.id));
                    }
                    // Gone (the conversation was deleted): nothing to send it to.
                    Err(ApiError::NotFound) => {
                        let _ = s.cases.delete(&key(&x.id));
                    }
                    // Offline or the other side isn't ready: try again shortly, giving up after a day.
                    Err(_) => {
                        x.tries += 1;
                        if x.tries > 8640 {
                            let _ = s.cases.delete(&key(&x.id));
                        } else {
                            let _ = s.cases.put(&key(&x.id), &x);
                        }
                    }
                }
                agent.emit(json!({ "type": "scheduled" }));
            }
            tokio::time::sleep(std::time::Duration::from_secs(if quick() { 1 } else { 10 })).await;
        }
    });
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new().route("/api/scheduled", get(list).post(create)).route("/api/scheduled/{id}", delete(cancel))
}

#[derive(Deserialize)]
struct ListQ {
    kind: Option<String>,
    target: Option<String>,
}

async fn list(State(agent): A, Query(q): Query<ListQ>) -> R {
    let s = agent.session().await?;
    let mut v: Vec<Scheduled> = all(&s)
        .into_iter()
        .filter(|x| q.kind.as_deref().is_none_or(|k| k == x.kind) && q.target.as_deref().is_none_or(|t| t == hex(&x.target)))
        .collect();
    v.sort_by_key(|x| x.at);
    Ok(Json(json!(v.iter().map(view).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct CreateIn {
    kind: String,
    target: String,
    text: String,
    at: u64,
}

async fn create(State(agent): A, Json(b): Json<CreateIn>) -> R {
    let s = agent.session().await?;
    if !matches!(b.kind.as_str(), "c" | "g" | "h") {
        return Err(ApiError::Invalid("Schedule a message in a conversation, group or channel.".into()));
    }
    let target = unhex::<16>(&b.target).map_err(|_| ApiError::NotFound)?;
    let text = b.text.trim_end().to_string();
    if text.trim().is_empty() {
        return Err(ApiError::Invalid("Write the message first.".into()));
    }
    if text.len() > 12_000 {
        return Err(ApiError::Invalid("That message is too long. Send it in parts.".into()));
    }
    let now = time::now();
    if b.at <= now + if quick() { 0 } else { 30 } {
        return Err(ApiError::Invalid("Pick a time at least a minute from now, or just send it.".into()));
    }
    if b.at > now + MAX_AHEAD {
        return Err(ApiError::Invalid("Messages can be scheduled up to 30 days ahead.".into()));
    }
    let x = Scheduled { id: new_id(), kind: b.kind, target, text, at: b.at, tries: 0 };
    s.cases.put(&key(&x.id), &x)?;
    agent.emit(json!({ "type": "scheduled" }));
    Ok(Json(view(&x)))
}

async fn cancel(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let id = unhex::<16>(&id).map_err(|_| ApiError::NotFound)?;
    s.cases.delete(&key(&id))?;
    agent.emit(json!({ "type": "scheduled" }));
    Ok(Json(json!({ "ok": true })))
}
