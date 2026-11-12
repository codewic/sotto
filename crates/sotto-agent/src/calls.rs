//! Calls: the call server setting, room requests to the call broker, risk-mode rules, and the
//! routes the main window and the call window use. See docs/calls.md.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_client::{RoomGrant, Transport};
use sotto_core::{Id, unhex};

use crate::ApiError;
use crate::state::{Agent, RiskMode, Session};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

const KEY: &str = "call-settings";

/// Kept in the vault under its own name, so the main settings record never changes shape.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CallSettings {
    /// The call broker, for example `https://calls.example.org`. Empty: calls aren't set up.
    pub server: String,
    /// Acknowledged that in High risk mode the call server sees your network address.
    pub high_risk_ack: bool,
}

pub fn load(s: &Session) -> CallSettings {
    s.main.get::<CallSettings>(KEY).ok().flatten().unwrap_or_default()
}

fn save(s: &Session, c: &CallSettings) -> Result<(), ApiError> {
    s.main.put(KEY, c)?;
    Ok(())
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/calls", get(list))
        .route("/api/calls/settings", get(get_settings).patch(patch_settings))
        .route("/api/calls/test", post(test))
        .route("/api/contacts/{id}/call", post(start))
        .route("/api/calls/{id}/answer", post(answer))
        .route("/api/calls/{id}/decline", post(decline))
        .route("/api/calls/{id}/end", post(end))
        .route("/api/calls/{id}/joined", post(joined))
        .route("/api/calls/{id}/join", get(join))
        .route("/api/calls/{id}/people", get(people))
        .route("/api/calls/{id}/invite", post(invite))
        .route("/api/calls/{id}/group", post(group))
}

fn id(s: &str) -> Result<Id, ApiError> {
    unhex::<16>(s).map_err(|_| ApiError::NotFound)
}

/// Accepts what people type: `calls.example.org`, `https://calls.example.org/`, or an onion
/// address. Plain http is only allowed for onion services and this machine (development).
pub fn broker_url(raw: &str) -> Result<String, ApiError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let bad = || ApiError::Invalid("That call server address isn't valid. It should look like https://calls.example.org".into());
    let (scheme, rest) = match raw.split_once("://") {
        Some((scheme, rest)) => (scheme, rest.trim_end_matches('/')),
        None => {
            let rest = raw.trim_end_matches('/');
            (if rest.ends_with(".onion") { "http" } else { "https" }, rest)
        }
    };
    let with_scheme = format!("{scheme}://{rest}");
    let host = rest.split(['/', ':']).next().unwrap_or("");
    if host.is_empty() || host.contains(char::is_whitespace) {
        return Err(bad());
    }
    match scheme {
        "https" => {}
        "http" if host.ends_with(".onion") || host == "127.0.0.1" || host == "localhost" => {}
        "http" => return Err(ApiError::Invalid("Use https:// for the call server, so nobody on the network can read or change what it sends.".into())),
        _ => return Err(bad()),
    }
    Ok(with_scheme)
}

fn http(transport: &Transport) -> Result<reqwest::Client, ApiError> {
    let mut b = reqwest::Client::builder().timeout(Duration::from_secs(40)).user_agent("sotto");
    if let Transport::Tor { socks } = transport {
        // socks5h: the call server's name is resolved inside Tor, not by the local network.
        b = b.proxy(reqwest::Proxy::all(format!("socks5h://{socks}")).map_err(|_| ApiError::Invalid("Bad Tor address.".into()))?);
    }
    b.build().map_err(|_| ApiError::Internal)
}

#[derive(Deserialize)]
struct GrantOut {
    url: String,
    #[serde(default)]
    room: String,
    tokens: Vec<String>,
}

/// One more join token for a room that exists (adding someone to a call).
async fn request_token(s: &Session, server: &str, room: &str) -> Result<String, ApiError> {
    let transport = s.engine.profile().await.transport;
    let res = http(&transport)?
        .post(format!("{server}/v1/rooms/{room}/tokens"))
        .header("content-type", "application/json")
        .body(r#"{"count":1}"#)
        .send()
        .await
        .map_err(|_| ApiError::Unavailable("The call server can't be reached right now. Try adding them again.".into()))?;
    match res.status().as_u16() {
        200 => {}
        429 => return Err(ApiError::Unavailable("The call server is busy right now. Try again in a minute.".into())),
        _ => return Err(ApiError::Unavailable("The call server wouldn't add anyone to this call.".into())),
    }
    let g: GrantOut = res.json().await.map_err(|_| ApiError::Unavailable("The call server sent a reply Sotto doesn't understand.".into()))?;
    g.tokens.into_iter().next().ok_or_else(|| ApiError::Unavailable("The call server sent a reply Sotto doesn't understand.".into()))
}

/// Ask the call broker for a fresh room. Goes through Tor when Sotto uses Tor, so the broker
/// doesn't learn who is about to call.
async fn request_room(s: &Session, server: &str) -> Result<RoomGrant, ApiError> {
    if server.is_empty() {
        return Err(ApiError::Invalid("Add a call server in Settings first (Protection and account, Calls).".into()));
    }
    let transport = s.engine.profile().await.transport;
    let res = http(&transport)?
        .post(format!("{server}/v1/rooms"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|_| ApiError::Unavailable("The call server can't be reached. Check its address in Settings and your connection, then try again.".into()))?;
    match res.status().as_u16() {
        200 => {}
        429 => return Err(ApiError::Unavailable("The call server is busy right now. Try again in a minute.".into())),
        _ => return Err(ApiError::Unavailable("The call server refused to start a call. Check its address in Settings.".into())),
    }
    let g: GrantOut = res.json().await.map_err(|_| ApiError::Unavailable("The call server sent a reply Sotto doesn't understand. Check its address in Settings.".into()))?;
    let media_ok = g.url.starts_with("wss://") || (server.starts_with("http://") && g.url.starts_with("ws://"));
    if !media_ok || g.tokens.len() != 2 {
        return Err(ApiError::Unavailable("The call server sent a reply Sotto doesn't understand. Check its address in Settings.".into()));
    }
    let mut t = g.tokens.into_iter();
    Ok(RoomGrant { url: g.url, room: g.room, my_token: t.next().unwrap_or_default(), their_token: t.next().unwrap_or_default() })
}

async fn list(State(agent): A) -> R {
    let s = agent.session().await?;
    Ok(Json(json!(s.engine.calls().await)))
}

fn settings_json(c: &CallSettings, risk: RiskMode) -> Value {
    json!({
        "server": c.server,
        "high_risk_ack": c.high_risk_ack,
        // What the UI should do before the next call.
        "allowed": risk != RiskMode::Extreme,
        "needs_notice": risk == RiskMode::High && !c.high_risk_ack,
    })
}

async fn get_settings(State(agent): A) -> R {
    let s = agent.session().await?;
    let risk = s.settings.lock().await.risk_mode;
    Ok(Json(settings_json(&load(&s), risk)))
}

#[derive(Deserialize)]
struct SettingsIn {
    server: Option<String>,
    high_risk_ack: Option<bool>,
}

async fn patch_settings(State(agent): A, Json(b): Json<SettingsIn>) -> R {
    let s = agent.session().await?;
    let mut c = load(&s);
    if let Some(server) = b.server {
        c.server = broker_url(&server)?;
    }
    if let Some(ack) = b.high_risk_ack {
        c.high_risk_ack = ack;
    }
    save(&s, &c)?;
    let risk = s.settings.lock().await.risk_mode;
    Ok(Json(settings_json(&c, risk)))
}

/// Check the call server by asking it for a room (nobody joins it; it closes by itself).
async fn test(State(agent): A) -> R {
    let s = agent.session().await?;
    let grant = request_room(&s, &load(&s).server).await?;
    let media = grant.url.split("://").nth(1).unwrap_or("").to_string();
    Ok(Json(json!({ "ok": true, "media": media })))
}

async fn start(State(agent): A, Path(cid): Path<String>) -> R {
    let s = agent.session().await?;
    let contact = id(&cid)?;
    let c = load(&s);
    let risk = s.settings.lock().await.risk_mode;
    match risk {
        RiskMode::Extreme => return Err(ApiError::Invalid("Calls are off in Extreme mode, because the call server would see your network address. Send a message instead.".into())),
        RiskMode::High if !c.high_risk_ack => return Err(ApiError::Invalid("Read the High risk call notice first.".into())),
        _ => {}
    }
    let grant = request_room(&s, &c.server).await?;
    let view = s.engine.start_call(&contact, grant).await?;
    Ok(Json(json!(view)))
}

async fn answer(State(agent): A, Path(call): Path<String>) -> R {
    let s = agent.session().await?;
    if s.settings.lock().await.risk_mode == RiskMode::Extreme {
        return Err(ApiError::Invalid("Calls are off in Extreme mode.".into()));
    }
    Ok(Json(json!(s.engine.answer_call(&id(&call)?).await?)))
}

#[derive(Deserialize, Default)]
struct DeclineIn {
    /// Tell the caller "can't talk now". Off: decline silently.
    #[serde(default)]
    say: bool,
}

async fn decline(State(agent): A, Path(call): Path<String>, body: Option<Json<DeclineIn>>) -> R {
    let s = agent.session().await?;
    s.engine.decline_call(&id(&call)?, body.map(|b| b.0).unwrap_or_default().say).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn end(State(agent): A, Path(call): Path<String>) -> R {
    let s = agent.session().await?;
    s.engine.end_call(&id(&call)?).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn joined(State(agent): A, Path(call): Path<String>) -> R {
    let s = agent.session().await?;
    s.engine.call_peer_joined(&id(&call)?).await?;
    Ok(Json(json!({ "ok": true })))
}

/// For the call window only: where to connect, the join token and the media key.
async fn join(State(agent): A, Path(call): Path<String>) -> R {
    let s = agent.session().await?;
    let j = s.engine.join_info(&id(&call)?).await?;
    let name = s.engine.contact(&j.contact_id).await.map(|c| if c.nickname.is_empty() { c.their_name } else { c.nickname }).unwrap_or_default();
    // The name this desk shares with others in a group call (end-to-end encrypted, inside the call).
    let me = s.engine.profile().await.display_name;
    Ok(Json(json!({
        "call_id": call,
        "contact_id": sotto_core::hex(&j.contact_id),
        "name": name,
        "url": j.url,
        "token": j.token,
        "key": base64::engine::general_purpose::STANDARD.encode(j.key),
        "outgoing": j.outgoing,
        "group": j.group,
        "me": if me.trim().is_empty() { "Someone".to_string() } else { me },
    })))
}

/// People who can be added to this call: ready conversations not already in it.
async fn people(State(agent): A, Path(call): Path<String>) -> R {
    let s = agent.session().await?;
    let cid = id(&call)?;
    let view = s.engine.calls().await.into_iter().find(|c| c.id == call).ok_or(ApiError::NotFound)?;
    let can_add = s.engine.call_room(&cid).await.is_some();
    let list: Vec<Value> = s
        .engine
        .contacts()
        .await
        .into_iter()
        .filter(|c| c.state == "ready" && c.id != view.contact_id && !view.invited.contains(&c.id))
        .map(|c| json!({ "id": c.id, "name": if c.nickname.is_empty() { c.their_name } else { c.nickname }, "verified": c.verified }))
        .collect();
    Ok(Json(json!({ "can_add": can_add, "people": list })))
}

#[derive(Deserialize)]
struct InviteIn {
    contact_id: String,
}

/// Add someone to a call that's going: one more join token for the same room, then an ordinary
/// end-to-end encrypted invitation carrying it and the call's media key.
async fn invite(State(agent): A, Path(call): Path<String>, Json(b): Json<InviteIn>) -> R {
    let s = agent.session().await?;
    if s.settings.lock().await.risk_mode == RiskMode::Extreme {
        return Err(ApiError::Invalid("Calls are off in Extreme mode.".into()));
    }
    let cid = id(&call)?;
    let (_, room) = s.engine.call_room(&cid).await.ok_or_else(|| ApiError::Invalid("People can't be added to this call.".into()))?;
    let token = request_token(&s, &load(&s).server, &room).await?;
    let view = s.engine.invite_to_call(&cid, &id(&b.contact_id)?, token).await?;
    Ok(Json(json!(view)))
}

async fn group(State(agent): A, Path(call): Path<String>) -> R {
    let s = agent.session().await?;
    s.engine.mark_group_call(&id(&call)?).await?;
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broker_addresses() {
        assert_eq!(broker_url("calls.example.org").unwrap(), "https://calls.example.org");
        assert_eq!(broker_url(" https://calls.example.org/ ").unwrap(), "https://calls.example.org");
        assert_eq!(broker_url("abcd.onion").unwrap(), "http://abcd.onion");
        assert_eq!(broker_url("http://127.0.0.1:7890").unwrap(), "http://127.0.0.1:7890");
        assert_eq!(broker_url("").unwrap(), "");
        assert!(broker_url("http://calls.example.org").is_err(), "no plain http on the internet");
        assert!(broker_url("ftp://x").is_err());
        assert!(broker_url("https://").is_err());
    }
}
