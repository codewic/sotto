//! Integrations: let scripts, CI jobs and monitoring post into a group or a channel.
//!
//! Each integration has its own token, shown once when it's made and stored here only as a hash.
//! Posts arrive on the local agent (127.0.0.1, like the rest of the API) at `POST /hooks/{id}`
//! with `Authorization: Bearer <token>`, and go out end-to-end encrypted like any message. In a
//! group they're labelled "<integration> (via <you>)" so nobody mistakes a script for a person.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_core::{Id, hex, new_id, time, unhex};

use crate::ApiError;
use crate::state::{Agent, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

/// Posts each integration may make per minute.
const PER_MINUTE: usize = 30;
const MAX_TEXT: usize = 4000;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
enum Target {
    Group(Id),
    Channel(Id),
}

#[derive(Clone, Serialize, Deserialize)]
struct Hook {
    id: Id,
    name: String,
    target: Target,
    token_hash: [u8; 32],
    created_at: u64,
    last_used: Option<u64>,
    posts: u64,
}

fn key(id: &Id) -> String {
    format!("hook/{}", hex(id))
}

fn hooks(s: &Session) -> Vec<Hook> {
    s.cases.names("hook/").iter().filter_map(|n| s.cases.get::<Hook>(n).ok().flatten()).collect()
}

fn hook_json(agent: &Agent, s: &Session, h: &Hook) -> Value {
    let (kind, id, title) = match h.target {
        Target::Group(g) => ("group", g, crate::groups::title_of(s, &g)),
        Target::Channel(c) => ("channel", c, crate::channels::title_of(s, &c)),
    };
    json!({
        "id": hex(&h.id),
        "name": h.name,
        "target": { "kind": kind, "id": hex(&id), "title": title },
        // What a script sets as SOTTO_HOOK_URL.
        "url": agent.endpoint.get().map(|e| format!("{e}/hooks/{}", hex(&h.id))),
        "created_at": h.created_at,
        "last_used": h.last_used,
        "posts": h.posts,
    })
}

/// Managed from the app (behind the app's own token).
pub fn routes() -> Router<Arc<Agent>> {
    Router::new().route("/api/hooks", get(list).post(create)).route("/api/hooks/{id}", delete(remove))
}

/// Where integrations post. Not behind the app's token: each integration's own token is checked.
pub fn public_routes() -> Router<Arc<Agent>> {
    Router::new().route("/hooks/{id}", post(deliver))
}

async fn list(State(agent): A) -> R {
    let s = agent.session().await?;
    let mut v = hooks(&s);
    v.sort_by_key(|h| h.created_at);
    Ok(Json(json!(v.iter().map(|h| hook_json(&agent, &s, h)).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct CreateIn {
    name: String,
    /// "group" or "channel"
    kind: String,
    id: String,
}

async fn create(State(agent): A, Json(b): Json<CreateIn>) -> R {
    let s = agent.session().await?;
    let name: String = b.name.trim().chars().take(40).collect();
    if name.is_empty() {
        return Err(ApiError::Invalid("Give the integration a name, for example CI alerts.".into()));
    }
    let tid = unhex::<16>(&b.id).map_err(|_| ApiError::NotFound)?;
    let target = match b.kind.as_str() {
        "group" => {
            crate::groups::title_of(&s, &tid).ok_or(ApiError::NotFound)?;
            Target::Group(tid)
        }
        "channel" => {
            if !crate::channels::owns(&s, &tid) {
                return Err(ApiError::Invalid("Integrations can post only to channels you made.".into()));
            }
            Target::Channel(tid)
        }
        _ => return Err(ApiError::Invalid("Choose a group or a channel.".into())),
    };
    let secret: [u8; 32] = sotto_core::rand::bytes();
    let token = format!("sotto_hook_{}", hex(&secret));
    let h = Hook { id: new_id(), name, target, token_hash: sotto_core::kdf::sha256(token.as_bytes()), created_at: time::now(), last_used: None, posts: 0 };
    s.cases.put(&key(&h.id), &h)?;
    let mut v = hook_json(&agent, &s, &h);
    // Shown once; only its hash is kept.
    v["token"] = json!(token);
    Ok(Json(v))
}

async fn remove(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let id = unhex::<16>(&id).map_err(|_| ApiError::NotFound)?;
    s.cases.get::<Hook>(&key(&id))?.ok_or(ApiError::NotFound)?;
    s.cases.delete(&key(&id))?;
    Ok(Json(json!({ "ok": true })))
}

static RECENT: Mutex<Option<HashMap<Id, VecDeque<Instant>>>> = Mutex::new(None);

/// Allow at most `PER_MINUTE` posts per integration in any minute.
fn allow(id: &Id) -> bool {
    let mut g = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    let q = g.get_or_insert_with(HashMap::new).entry(*id).or_default();
    let now = Instant::now();
    while q.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
        q.pop_front();
    }
    if q.len() >= PER_MINUTE {
        return false;
    }
    q.push_back(now);
    true
}

fn refuse(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

/// `POST /hooks/{id}`: the body is JSON `{"text": "..."}` or plain text.
async fn deliver(State(agent): A, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    // Same rule as the app's API: only this computer may call it.
    let local = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| {
            let name = if h.starts_with('[') { h.split(']').next().map(|x| format!("{x}]")).unwrap_or_default() } else { h.split(':').next().unwrap_or("").to_string() };
            matches!(name.as_str(), "127.0.0.1" | "localhost" | "[::1]")
        })
        .unwrap_or(false);
    if !local {
        return refuse(StatusCode::UNAUTHORIZED, "Not allowed.");
    }
    let Ok(id) = unhex::<16>(&id) else { return refuse(StatusCode::UNAUTHORIZED, "Not allowed.") };
    let token = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    let Ok(s) = agent.session().await else { return refuse(StatusCode::SERVICE_UNAVAILABLE, "Sotto is locked, so nothing was posted. Unlock it and try again.") };
    let Some(mut h) = s.cases.get::<Hook>(&key(&id)).ok().flatten() else { return refuse(StatusCode::UNAUTHORIZED, "Not allowed.") };
    if sotto_core::kdf::sha256(token.as_bytes()) != h.token_hash {
        return refuse(StatusCode::UNAUTHORIZED, "Not allowed.");
    }
    if !allow(&h.id) {
        return refuse(StatusCode::TOO_MANY_REQUESTS, "Too many posts from this integration. Wait a minute.");
    }
    let is_json = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("application/json"));
    let text = if is_json {
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => v["text"].as_str().unwrap_or_default().to_string(),
            Err(_) => return refuse(StatusCode::BAD_REQUEST, "The body isn't valid JSON. Send {\"text\": \"...\"} or plain text."),
        }
    } else {
        String::from_utf8_lossy(&body).to_string()
    };
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return refuse(StatusCode::BAD_REQUEST, "There's nothing to post. Send {\"text\": \"...\"} or plain text.");
    }
    let result = match h.target {
        Target::Group(g) => crate::groups::post_text(&agent, &s, &g, text, None, Some(&h.name)).await,
        Target::Channel(c) => crate::channels::post_text(&agent, &s, &c, &text).await,
    };
    match result {
        Ok(_) => {
            h.last_used = Some(time::now());
            h.posts += 1;
            let _ = s.cases.put(&key(&h.id), &h);
            (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
        }
        Err(ApiError::NotFound) => refuse(StatusCode::NOT_FOUND, "The group or channel this integration posts to is gone."),
        Err(e) => e.into_response(),
    }
}
