//! HTTP routes for the desktop UI.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use sotto_client::Transport;
use sotto_core::{Id, unhex};

use crate::ApiError;
use crate::drops;
use crate::review::{self, Source};
use crate::state::{Agent, RiskMode};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

pub fn router(agent: Arc<Agent>) -> Router {
    let api = Router::new()
        .route("/api/status", get(status))
        .route("/api/setup", post(setup))
        .route("/api/unlock", post(unlock))
        .route("/api/lock", post(lock))
        .route("/api/panic", post(panic))
        .route("/api/events", get(events))
        .route("/api/settings", get(get_settings).patch(patch_settings))
        .route("/api/passcode", post(change_passcode))
        .route("/api/duress", post(set_duress))
        .route("/api/contacts", get(contacts))
        .route("/api/invites", post(create_invite))
        .route("/api/invites/accept", post(accept_invite))
        .route("/api/contacts/{id}", get(contact).patch(patch_contact).delete(delete_contact))
        .route("/api/contacts/{id}/read", post(mark_read))
        .route("/api/contacts/{id}/messages", get(messages).post(send_text))
        .route("/api/contacts/{id}/files", post(send_file))
        .route("/api/contacts/{id}/messages/{mid}", delete(delete_message).patch(edit_message))
        .route("/api/contacts/{id}/messages/{mid}/react", post(react))
        .route("/api/contacts/{id}/messages/{mid}/retry", post(retry))
        .route("/api/contacts/{id}/messages/{mid}/review", post(review_message))
        .route("/api/drops", get(list_drops).post(create_drop))
        .route("/api/drops/{id}", delete(delete_drop))
        .route("/api/drops/{id}/sync", post(sync_drop))
        .route("/api/drops/{id}/sources", get(drop_sources))
        .route("/api/drops/{id}/sources/{sid}", get(drop_items).delete(forget_source))
        .route("/api/drops/{id}/sources/{sid}/reply", post(drop_reply))
        .route("/api/drops/{id}/sources/{sid}/items/{name}/review", post(review_drop_item))
        .route("/api/files", get(all_files))
        .route("/api/directory/check", post(check_directory))
        .route("/api/reviews/compare", post(compare))
        .route("/api/reviews/{key}", get(get_report))
        .route("/api/reviews/{key}/pages/{n}", get(get_page))
        .route("/api/reviews/{key}/export", post(export))
        .merge(crate::cases::routes())
        .merge(crate::calls::routes())
        .merge(crate::groups::routes())
        .merge(crate::channels::routes())
        .merge(crate::travel::routes())
        .merge(crate::hooks::routes())
        .merge(crate::secrets::routes())
        .merge(crate::tags::routes())
        .merge(crate::avatars::routes())
        .merge(crate::updates::routes())
        .merge(crate::scheduled::routes())
        .merge(crate::photos::routes())
        .merge(crate::media::routes())
        .layer(DefaultBodyLimit::max(512 << 20))
        .layer(axum::middleware::from_fn_with_state(agent.clone(), guard))
        .with_state(agent.clone());
    // Integrations post with their own tokens, so they sit outside the app's guard (and don't
    // count as someone using the app, which would hold off auto-lock).
    let hooks = crate::hooks::public_routes().layer(DefaultBodyLimit::max(64 << 10)).with_state(agent);
    api.merge(hooks)
}

/// Every request must carry the per-launch token, and Host must be local (blocks DNS rebinding).
async fn guard(State(agent): A, req: Request, next: Next) -> Response {
    let token_ok = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| sotto_core::kdf::sha256(t.as_bytes()) == sotto_core::kdf::sha256(agent.token.as_bytes()))
        .unwrap_or(false);
    let host_ok = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(|h| {
            let name = if h.starts_with('[') { h.split(']').next().map(|x| format!("{x}]")).unwrap_or_default() } else { h.split(':').next().unwrap_or("").to_string() };
            matches!(name.as_str(), "127.0.0.1" | "localhost" | "[::1]" | "sotto")
        })
        .unwrap_or(false);
    if !token_ok || !host_ok {
        return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Not allowed." }))).into_response();
    }
    if req.uri().path() != "/api/events" {
        agent.touch();
    }
    let mut res = next.run(req).await;
    res.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse().expect("static"));
    res
}

fn id(s: &str) -> Result<Id, ApiError> {
    unhex::<16>(s).map_err(|_| ApiError::NotFound)
}

fn ok() -> R {
    Ok(Json(json!({ "ok": true })))
}

// ---------- Lifecycle ----------

async fn status(State(agent): A) -> R {
    let state = agent.state().await;
    let mut out = json!({ "state": state, "version": env!("CARGO_PKG_VERSION"), "shipped_relay": agent.default_relay });
    if let Ok(s) = agent.session().await {
        let p = s.engine.profile().await;
        let settings = s.settings.lock().await.clone();
        let relays: Vec<Value> = s.engine.relay_statuses().await.into_iter().map(|(u, st)| json!({ "url": u, "status": st })).collect();
        out["profile"] = json!({
            "display_name": p.display_name,
            "default_relay": p.default_relay,
            "transport": transport_json(&p.transport),
            "fingerprint": p.identity.public().fingerprint(),
        });
        out["settings"] = serde_json::to_value(settings).unwrap_or(Value::Null);
        out["relays"] = json!(relays);
        // Whether anything answers at the Tor address, so "unreachable" can say why.
        if let Transport::Tor { socks } = &p.transport {
            let up = tokio::time::timeout(std::time::Duration::from_millis(400), tokio::net::TcpStream::connect(socks.as_str())).await.is_ok_and(|r| r.is_ok());
            out["tor_running"] = json!(up);
        }
        out["sandbox"] = json!(sotto_review::sandbox::available());
    }
    Ok(Json(out))
}

fn transport_json(t: &Transport) -> Value {
    match t {
        Transport::Direct => json!({ "kind": "direct" }),
        Transport::Tor { socks } => json!({ "kind": "tor", "socks": socks }),
    }
}

#[derive(Deserialize)]
struct TransportIn {
    kind: String,
    socks: Option<String>,
}

impl TransportIn {
    fn into_transport(self) -> Result<Transport, ApiError> {
        match self.kind.as_str() {
            "direct" => Ok(Transport::Direct),
            "tor" => Ok(Transport::Tor { socks: self.socks.unwrap_or_else(|| "127.0.0.1:9050".into()) }),
            _ => Err(ApiError::Invalid("Unknown connection type.".into())),
        }
    }
}

#[derive(Deserialize)]
struct SetupIn {
    passcode: String,
    duress: Option<String>,
    relay: Option<String>,
    transport: Option<TransportIn>,
}

async fn setup(State(agent): A, Json(b): Json<SetupIn>) -> R {
    if agent.state().await != "new" {
        return Err(ApiError::Invalid("Sotto is already set up on this device.".into()));
    }
    let transport = b.transport.map(TransportIn::into_transport).transpose()?;
    let duress = b.duress.filter(|d| !d.is_empty());
    agent.setup(&b.passcode, duress.as_deref(), b.relay.filter(|r| !r.trim().is_empty()), transport).await?;
    ok()
}

#[derive(Deserialize)]
struct PasscodeIn {
    passcode: String,
}

async fn unlock(State(agent): A, Json(b): Json<PasscodeIn>) -> R {
    // Each wrong guess makes the next answer slower (on top of Argon2id's own cost).
    let failures = agent.unlock_failures.load(std::sync::atomic::Ordering::Relaxed);
    if failures > 0 {
        tokio::time::sleep(std::time::Duration::from_millis(500 * failures.min(20) as u64)).await;
    }
    match agent.unlock(&b.passcode).await {
        Ok(()) => {
            agent.unlock_failures.store(0, std::sync::atomic::Ordering::Relaxed);
            ok()
        }
        Err(e) => {
            agent.unlock_failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(e)
        }
    }
}

async fn lock(State(agent): A) -> R {
    agent.lock().await;
    ok()
}

async fn panic(State(agent): A) -> R {
    agent.panic().await?;
    ok()
}

async fn events(State(agent): A) -> impl IntoResponse {
    let rx = agent.events.subscribe();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(|m| async move {
        let v = m.unwrap_or_else(|_| json!({ "type": "resync" }));
        Some(Ok::<_, std::convert::Infallible>(SseEvent::default().data(v.to_string())))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(20)))
}

// ---------- Settings ----------

async fn get_settings(State(agent): A) -> R {
    let s = agent.session().await?;
    let settings = s.settings.lock().await.clone();
    Ok(Json(serde_json::to_value(settings).unwrap_or(Value::Null)))
}

#[derive(Deserialize)]
struct SettingsIn {
    risk_mode: Option<RiskMode>,
    auto_lock_minutes: Option<u32>,
    delete_after_download: Option<bool>,
    display_name: Option<String>,
    default_relay: Option<String>,
    transport: Option<TransportIn>,
}

async fn patch_settings(State(agent): A, Json(b): Json<SettingsIn>) -> R {
    let s = agent.session().await?;
    {
        let mut st = s.settings.lock().await;
        if let Some(m) = b.risk_mode {
            st.risk_mode = m;
        }
        if let Some(m) = b.auto_lock_minutes {
            st.auto_lock_minutes = m.clamp(1, 240);
        }
        if let Some(d) = b.delete_after_download {
            st.delete_after_download = d;
        }
    }
    agent.save_settings(&s).await?;
    let transport = b.transport.map(TransportIn::into_transport).transpose()?;
    let restart = transport.is_some();
    s.engine.update_profile(b.display_name, b.default_relay, transport).await?;
    if restart {
        // A new transport applies to fresh connections; the simplest safe path is a lock and unlock.
        agent.emit(json!({ "type": "notice", "text": "Connection settings saved. They apply the next time you unlock Sotto." }));
    }
    get_settings(State(agent)).await
}

#[derive(Deserialize)]
struct ChangePasscodeIn {
    old: String,
    new: String,
}

async fn change_passcode(State(agent): A, Json(b): Json<ChangePasscodeIn>) -> R {
    if b.new.chars().count() < 8 {
        return Err(ApiError::Invalid("Use at least 8 characters for your passcode.".into()));
    }
    let s = agent.session().await?;
    let mut v = s.vault.lock().await;
    v.change_passcode(&b.old, &b.new)?;
    ok()
}

#[derive(Deserialize)]
struct DuressIn {
    passcode: String,
    duress: Option<String>,
}

async fn set_duress(State(agent): A, Json(b): Json<DuressIn>) -> R {
    let s = agent.session().await?;
    if let Some(d) = &b.duress
        && d.chars().count() < 8 {
            return Err(ApiError::Invalid("Use at least 8 characters for the duress passcode.".into()));
        }
    let mut v = s.vault.lock().await;
    v.set_duress(&b.passcode, b.duress.as_deref())?;
    ok()
}

// ---------- Contacts and messages ----------

async fn contacts(State(agent): A) -> R {
    let s = agent.session().await?;
    // Invites stocked for your tag aren't conversations yet; people who came through it are
    // marked as requests until you accept them.
    let hidden = crate::tags::hidden(&s);
    let mut out = Vec::new();
    for c in s.engine.contacts().await {
        if hidden.contains(&c.id) {
            continue;
        }
        let mut v = json!(c);
        if let Ok(cid) = id(&c.id) {
            v["request_tag"] = json!(crate::tags::request_of(&s, &cid));
            v["avatar"] = json!(crate::avatars::of(&s, &cid));
        }
        out.push(v);
    }
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
struct InviteIn {
    nickname: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    listed: bool,
}

fn qr_svg(data: &str) -> Option<String> {
    let code = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::L).ok()?;
    Some(code.render::<qrcode::render::svg::Color>().min_dimensions(240, 240).quiet_zone(true).build())
}

async fn create_invite(State(agent): A, Json(b): Json<InviteIn>) -> R {
    let s = agent.session().await?;
    let (cid, link) = s.engine.create_invite(&b.nickname, &b.label, b.listed).await?;
    Ok(Json(json!({ "contact_id": sotto_core::hex(&cid), "link": link, "qr": qr_svg(&link) })))
}

#[derive(Deserialize)]
struct AcceptIn {
    link: String,
    #[serde(default)]
    nickname: String,
    #[serde(default)]
    share_name: String,
}

async fn accept_invite(State(agent): A, Json(b): Json<AcceptIn>) -> R {
    let s = agent.session().await?;
    let cid = s.engine.accept_invite(&b.link, &b.nickname, &b.share_name).await?;
    Ok(Json(json!({ "contact_id": sotto_core::hex(&cid) })))
}

async fn contact(State(agent): A, Path(cid): Path<String>) -> R {
    let s = agent.session().await?;
    let c = s.engine.contact(&id(&cid)?).await.ok_or(ApiError::NotFound)?;
    Ok(Json(json!(c)))
}

#[derive(Deserialize)]
struct ReactIn {
    emoji: String,
    on: bool,
}

async fn react(State(agent): A, Path((cid, mid)): Path<(String, String)>, Json(b): Json<ReactIn>) -> R {
    let s = agent.session().await?;
    s.engine.react(&id(&cid)?, &id(&mid)?, &b.emoji, b.on).await?;
    ok()
}

#[derive(Deserialize)]
struct EditIn {
    text: String,
}

async fn edit_message(State(agent): A, Path((cid, mid)): Path<(String, String)>, Json(b): Json<EditIn>) -> R {
    let s = agent.session().await?;
    s.engine.edit(&id(&cid)?, &id(&mid)?, &b.text).await?;
    ok()
}

#[derive(Deserialize)]
struct PatchContactIn {
    pinned: Option<bool>,
    archived: Option<bool>,
    read_receipts: Option<bool>,
    nickname: Option<String>,
    verified: Option<bool>,
    /// Seconds; 0 means "keep until deleted".
    timer: Option<u32>,
}

async fn patch_contact(State(agent): A, Path(cid): Path<String>, Json(b): Json<PatchContactIn>) -> R {
    let s = agent.session().await?;
    let cid = id(&cid)?;
    if let Some(n) = b.nickname {
        s.engine.rename_contact(&cid, &n).await?;
    }
    if let Some(v) = b.verified {
        s.engine.set_verified(&cid, v).await?;
    }
    if let Some(v) = b.pinned {
        s.engine.set_pinned(&cid, v).await?;
    }
    if let Some(v) = b.archived {
        s.engine.set_archived(&cid, v).await?;
    }
    if let Some(v) = b.read_receipts {
        s.engine.set_read_receipts(&cid, v).await?;
    }
    if let Some(t) = b.timer {
        s.engine.set_timer(&cid, if t == 0 { None } else { Some(t.clamp(60, 365 * 86_400)) }).await?;
    }
    contact(State(agent), Path(sotto_core::hex(&cid))).await
}

async fn delete_contact(State(agent): A, Path(cid): Path<String>) -> R {
    let s = agent.session().await?;
    s.engine.delete_contact(&id(&cid)?).await?;
    ok()
}

async fn mark_read(State(agent): A, Path(cid): Path<String>) -> R {
    let s = agent.session().await?;
    s.engine.mark_read(&id(&cid)?).await?;
    ok()
}

async fn messages(State(agent): A, Path(cid): Path<String>) -> R {
    let s = agent.session().await?;
    Ok(Json(json!(s.engine.messages(&id(&cid)?, 500)?)))
}

#[derive(Deserialize)]
struct TextIn {
    text: String,
    #[serde(default)]
    reply_to: Option<String>,
}

async fn send_text(State(agent): A, Path(cid): Path<String>, Json(b): Json<TextIn>) -> R {
    let s = agent.session().await?;
    let reply_to = b.reply_to.as_deref().map(id).transpose()?;
    Ok(Json(json!(s.engine.send_text(&id(&cid)?, &b.text, reply_to).await?)))
}

pub(crate) fn header_text(h: &HeaderMap, name: &str) -> String {
    let raw = h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
    percent_decode(raw)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (bytes.get(i + 1).and_then(|c| (*c as char).to_digit(16)), bytes.get(i + 2).and_then(|c| (*c as char).to_digit(16))) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

async fn send_file(State(agent): A, Path(cid): Path<String>, headers: HeaderMap, body: Bytes) -> R {
    let s = agent.session().await?;
    if body.is_empty() {
        return Err(ApiError::Invalid("That file is empty.".into()));
    }
    let name = header_text(&headers, "x-file-name");
    let caption = header_text(&headers, "x-caption");
    let mime = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream").to_string();
    let (name, mime, body) = crate::photos::maybe_clean(&headers, name, mime, body).await?;
    Ok(Json(json!(s.engine.send_file(&id(&cid)?, &name, &mime, &caption, &mut &body[..]).await?)))
}

#[derive(Deserialize)]
struct DeleteQ {
    both: Option<u8>,
}

async fn delete_message(State(agent): A, Path((cid, mid)): Path<(String, String)>, Query(q): Query<DeleteQ>) -> R {
    let s = agent.session().await?;
    s.engine.delete_message(&id(&cid)?, &id(&mid)?, q.both == Some(1)).await?;
    ok()
}

async fn retry(State(agent): A, Path((cid, mid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    s.engine.retry(&id(&cid)?, &id(&mid)?).await?;
    ok()
}

#[derive(Deserialize, Default)]
struct ReviewIn {
    #[serde(default)]
    greyscale: bool,
}

async fn review_message(State(agent): A, Path((cid, mid)): Path<(String, String)>, body: Option<Json<ReviewIn>>) -> R {
    let s = agent.session().await?;
    let (contact, message) = (id(&cid)?, id(&mid)?);
    let m = s.engine.find_message(&contact, &message).ok_or(ApiError::NotFound)?;
    let blob = match &m.body {
        sotto_client::Body::File { meta, .. } => meta.blob_id.clone().ok_or(ApiError::Invalid("This file hasn't finished downloading.".into()))?,
        _ => return Err(ApiError::Invalid("Only files can be reviewed.".into())),
    };
    let greyscale = body.map(|b| b.0.greyscale).unwrap_or_default();
    let report = review::run(&s, Source::Message { contact, message, blob }, greyscale).await?;
    agent.emit(json!({ "type": "review", "key": report["key"] }));
    Ok(Json(report))
}

// ---------- Drops ----------

async fn list_drops(State(agent): A) -> R {
    let s = agent.session().await?;
    let views = drops::accounts(&s.drops)?.iter().map(|a| drops::view(&s.drops, a)).collect::<Result<Vec<_>, _>>()?;
    Ok(Json(json!(views)))
}

#[derive(Deserialize)]
struct DropIn {
    name: String,
    url: String,
}

async fn create_drop(State(agent): A, Json(b): Json<DropIn>) -> R {
    let s = agent.session().await?;
    let a = drops::create(&s.drops, &b.name, &b.url, None)?;
    Ok(Json(json!(drops::view(&s.drops, &a)?)))
}

async fn delete_drop(State(agent): A, Path(did): Path<String>) -> R {
    let s = agent.session().await?;
    let a = drops::account(&s.drops, &did)?;
    for src in drops::sources(&s.drops, &a.id)? {
        drops::forget_source(&s.drops, &a.id, &src.id)?;
    }
    s.drops.delete(&format!("drop/{}", a.id))?;
    ok()
}

async fn sync_drop(State(agent): A, Path(did): Path<String>) -> R {
    let s = agent.session().await?;
    let added = drops::sync(&agent, &s, &did).await?;
    Ok(Json(json!({ "added": added })))
}

async fn drop_sources(State(agent): A, Path(did): Path<String>) -> R {
    let s = agent.session().await?;
    let v: Vec<drops::SourceView> = drops::sources(&s.drops, &did)?
        .into_iter()
        .map(|x| drops::SourceView { id: x.id, pseudonym: x.pseudonym, first_day: x.first_day, last_day: x.last_day, unread: x.unread })
        .collect();
    Ok(Json(json!(v)))
}

async fn drop_items(State(agent): A, Path((did, sid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let items: Vec<drops::ItemView> = drops::items(&s.drops, &did, &sid)?.iter().map(|i| i.view()).collect();
    drops::mark_read(&s.drops, &did, &sid)?;
    Ok(Json(json!(items)))
}

async fn forget_source(State(agent): A, Path((did, sid)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    drops::forget_source(&s.drops, &did, &sid)?;
    ok()
}

async fn drop_reply(State(agent): A, Path((did, sid)): Path<(String, String)>, Json(b): Json<TextIn>) -> R {
    let s = agent.session().await?;
    drops::reply(&agent, &s, &did, &sid, &b.text).await?;
    ok()
}

async fn review_drop_item(State(agent): A, Path((did, sid, name)): Path<(String, String, String)>, body: Option<Json<ReviewIn>>) -> R {
    let s = agent.session().await?;
    let item = s.drops.get::<drops::DropItem>(&drops::item_key(&did, &sid, &name))?.ok_or(ApiError::NotFound)?;
    let blob = item.blob.ok_or(ApiError::Invalid("Only files can be reviewed.".into()))?;
    let greyscale = body.map(|b| b.0.greyscale).unwrap_or_default();
    let report = review::run(&s, Source::Drop { drop: &did, source: &sid, item: &name, blob }, greyscale).await?;
    agent.emit(json!({ "type": "review", "key": report["key"] }));
    Ok(Json(report))
}

/// Every file across conversations and drops, newest first, with its review state.
async fn all_files(State(agent): A) -> R {
    let s = agent.session().await?;
    let mut out: Vec<Value> = Vec::new();
    for c in s.engine.contacts().await {
        let cid = id(&c.id)?;
        for m in s.engine.messages(&cid, 10_000)? {
            if let Some(f) = &m.file {
                out.push(json!({
                    "origin": "conversation",
                    "contact_id": c.id,
                    "where": if c.nickname.is_empty() { c.their_name.clone() } else { c.nickname.clone() },
                    "message_id": m.id,
                    "key": format!("m:file/{}", m.id),
                    "outgoing": m.outgoing,
                    "name": f.name, "mime": f.mime, "size": f.size, "status": f.status, "review": f.review,
                    "at": m.received_at / 1000,
                }));
            }
        }
    }
    // Files in groups and channels.
    for (at, f) in crate::shared::all(&s) {
        let mut parts = at.split('/');
        let (Some(kind), Some(owner), Some(item)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Ok(oid) = id(owner) else { continue };
        let (where_, path) = match kind {
            "g" => (crate::groups::title_of(&s, &oid), format!("/api/groups/{owner}/messages/{item}/review")),
            "c" => (crate::channels::title_of(&s, &oid), format!("/api/channels/{owner}/posts/{item}/review")),
            _ => continue,
        };
        let Some(where_) = where_ else { continue };
        let at_ms = s.cases.names(&format!("{}/{owner}/", if kind == "g" { "gmsg" } else { "cpost" })).into_iter().find(|n| n.ends_with(item)).and_then(|n| n.rsplit('/').next()?.split('-').next()?.parse::<u64>().ok()).unwrap_or(0);
        let v = crate::shared::view(&f, "");
        out.push(json!({
            "origin": if kind == "g" { "group" } else { "channel" },
            "where": where_,
            "key": format!("s:{}", f.blob),
            "review_path": path,
            "outgoing": matches!(f.review, sotto_client::Review::Own),
            "name": f.name, "mime": f.mime, "size": f.size, "status": v["status"], "review": v["review"],
            "at": at_ms / 1000,
        }));
    }
    for a in drops::accounts(&s.drops)? {
        for src in drops::sources(&s.drops, &a.id)? {
            for it in drops::items(&s.drops, &a.id, &src.id)? {
                if let (drops::ItemKind::File { name, mime, size }, Some(blob)) = (&it.kind, &it.blob) {
                    out.push(json!({
                        "origin": "drop",
                        "drop_id": a.id, "source_id": src.id, "item": it.name,
                        "where": format!("{} · {}", a.name, src.pseudonym),
                        "key": format!("d:{blob}"),
                        "outgoing": false,
                        "name": name, "mime": mime, "size": size, "status": "ready", "review": it.review.view(),
                        "at": it.received_at,
                    }));
                }
            }
        }
    }
    out.sort_by(|a, b| b["at"].as_u64().cmp(&a["at"].as_u64()));
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
struct DirectoryIn {
    /// The contents of the outlet's /.well-known/sotto.json, as fetched in Tor Browser.
    document: String,
    domain: String,
    contact_id: String,
}

/// Check a contact's identity against an outlet's signed directory.
async fn check_directory(State(agent): A, Json(b): Json<DirectoryIn>) -> R {
    let s = agent.session().await?;
    let d = sotto_directory::Directory::verify(&b.document, &b.domain).map_err(|e| ApiError::Invalid(format!("{e}.")))?;
    let c = s.engine.contact(&id(&b.contact_id)?).await.ok_or(ApiError::NotFound)?;
    let fp = c.their_fingerprint.ok_or(ApiError::Invalid("This contact hasn't connected yet.".into()))?;
    match d.reporter(&fp) {
        Some(r) => Ok(Json(json!({ "match": true, "name": r.name, "outlet": d.outlet, "domain": d.domain }))),
        None => Ok(Json(json!({ "match": false, "outlet": d.outlet, "domain": d.domain }))),
    }
}

// ---------- Reviews ----------

async fn get_report(State(agent): A, Path(key): Path<String>) -> R {
    let s = agent.session().await?;
    Ok(Json(review::report(&s, &key)?))
}

async fn get_page(State(agent): A, Path((key, n)): Path<(String, u32)>) -> Result<Response, ApiError> {
    let s = agent.session().await?;
    let png = review::page(&s, &key, n)?;
    Ok(([(header::CONTENT_TYPE, "image/png")], png).into_response())
}

#[derive(Deserialize)]
struct ExportIn {
    path: String,
}

async fn export(State(agent): A, Path(key): Path<String>, Json(b): Json<ExportIn>) -> R {
    let s = agent.session().await?;
    let path = std::path::PathBuf::from(&b.path);
    if !path.is_absolute() {
        return Err(ApiError::Invalid("Choose a full path for the clean copy.".into()));
    }
    review::export_clean(&s, &key, &path)?;
    ok()
}

#[derive(Deserialize)]
struct CompareIn {
    a: String,
    b: String,
}

async fn compare(State(agent): A, Json(b): Json<CompareIn>) -> R {
    let s = agent.session().await?;
    Ok(Json(json!(review::compare(&s, &b.a, &b.b)?)))
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_percent_headers() {
        assert_eq!(super::percent_decode("contract%20v2%2Epdf"), "contract v2.pdf");
        assert_eq!(super::percent_decode("caf%C3%A9"), "café");
        assert_eq!(super::percent_decode("bad%zz%"), "bad%zz%");
    }
}
