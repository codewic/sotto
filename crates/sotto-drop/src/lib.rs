//! Sotto drop: a no-install, no-JavaScript intake for sources, served as a Tor onion service.
//!
//! Sources get a codename instead of an account. Everything they send is sealed
//! to the newsroom's public key (X25519 + ML-KEM-768) as it streams in, so
//! plaintext never reaches the disk. Replies are sealed by the journalist's app
//! to a key derived from the source's codename, and opened here only in memory
//! for the length of a page view. See `docs/threat-model.md` section 6.

#![forbid(unsafe_code)]

pub mod pages;
pub mod proto;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use serde::{Deserialize, Serialize};
use sotto_core::seal::{SealPublic, SealSecret, Sealer};
use sotto_core::{codename, hex, kdf, unhex};
use tokio::io::AsyncWriteExt;

use proto::{SubmissionKind, SubmissionMeta, source_id, today};

const SESSION_IDLE: u64 = 30 * 60;
const MAX_MESSAGE: usize = 200_000;
const LOGINS_PER_MINUTE: u32 = 30;
const COOKIE: &str = "sotto_s";
/// At most this many Argon2id derivations run at once (each uses `argon_m_kib` of memory).
const KDF_CONCURRENCY: usize = 4;
/// Upper bound on live sessions; beyond it, new visitors are asked to try again shortly.
const MAX_SESSIONS: usize = 20_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalistKey {
    pub name: String,
    pub sign_pk: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DropConfig {
    pub name: String,
    /// 16 random bytes in hex; keeps codename-derived keys unique to this drop.
    pub instance_id: String,
    /// Newsroom public key, from the Sotto desktop app.
    pub newsroom_key: String,
    pub journalists: Vec<JournalistKey>,
    pub data_dir: PathBuf,
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_upload")]
    pub max_upload_mb: u64,
    #[serde(default = "default_m")]
    pub argon_m_kib: u32,
    #[serde(default = "default_t")]
    pub argon_t: u32,
    #[serde(default)]
    pub intro: String,
    /// Where sources can confirm this onion address, for example "outlet.example/contact".
    #[serde(default)]
    pub verify_hint: String,
}

fn default_listen() -> String {
    "127.0.0.1:7500".into()
}
fn default_upload() -> u64 {
    500
}
fn default_m() -> u32 {
    64 * 1024
}
fn default_t() -> u32 {
    3
}

pub fn load_config(path: &Path) -> Result<DropConfig, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("invalid config: {e}"))
}

enum SessionKind {
    Anonymous,
    Pending { codename: String },
    Source { id: String, secret: Arc<SealSecret> },
}

struct Session {
    kind: SessionKind,
    csrf: String,
    last: u64,
}

pub struct App {
    cfg: DropConfig,
    newsroom: SealPublic,
    instance: [u8; 16],
    journalists: Vec<[u8; 32]>,
    sessions: Mutex<HashMap<String, Session>>,
    logins: Mutex<(u64, u32)>,
    replay: Mutex<HashMap<[u8; 32], u64>>,
    kdf: tokio::sync::Semaphore,
}

impl App {
    pub fn new(cfg: DropConfig) -> Result<Arc<App>, String> {
        let newsroom = proto::decode_key(&cfg.newsroom_key).map_err(|_| "newsroom_key is not a valid Sotto key".to_string())?;
        let instance = unhex::<16>(&cfg.instance_id).map_err(|_| "instance_id must be 32 hex characters".to_string())?;
        let journalists = cfg
            .journalists
            .iter()
            .map(|j| unhex::<32>(&j.sign_pk).map_err(|_| format!("journalist {} has an invalid key", j.name)))
            .collect::<Result<Vec<_>, _>>()?;
        std::fs::create_dir_all(cfg.data_dir.join("sources")).map_err(|e| format!("can't create data dir: {e}"))?;
        Ok(Arc::new(App {
            cfg,
            newsroom,
            instance,
            journalists,
            sessions: Mutex::new(HashMap::new()),
            logins: Mutex::new((0, 0)),
            replay: Mutex::new(HashMap::new()),
            kdf: tokio::sync::Semaphore::new(KDF_CONCURRENCY),
        }))
    }

    fn source_dir(&self, id: &str) -> PathBuf {
        self.cfg.data_dir.join("sources").join(id)
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<String, Session>> {
        self.sessions.lock().unwrap_or_else(|p| p.into_inner())
    }
}

pub fn router(app: Arc<App>) -> Router {
    let limit = (app.cfg.max_upload_mb as usize).saturating_mul(1 << 20) + MAX_MESSAGE + 64 * 1024;
    Router::new()
        .route("/", get(landing))
        .route("/new", get(new_get).post(new_post))
        .route("/login", get(login_get).post(login_post))
        .route("/inbox", get(inbox))
        .route("/submit", post(submit))
        .route("/replies/delete", post(delete_reply))
        .route("/logout", post(logout))
        .route("/assets/drop.css", get(css))
        .route("/j/v1/sources", get(j_sources))
        .route("/j/v1/sources/{id}", axum::routing::delete(j_delete_source))
        .route("/j/v1/sources/{id}/key", get(j_key))
        .route("/j/v1/sources/{id}/submissions", get(j_submissions))
        .route("/j/v1/sources/{id}/submissions/{name}", get(j_get_submission).delete(j_delete_submission))
        .route("/j/v1/sources/{id}/replies", post(j_reply))
        .layer(DefaultBodyLimit::max(limit))
        .layer(axum::middleware::map_response(security_headers))
        .with_state(app)
}

async fn security_headers(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"),
    );
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert("permissions-policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=()"));
    h.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    h.remove(header::SERVER);
    res
}

fn now() -> u64 {
    sotto_core::time::now()
}

fn cookie_sid(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    let prefix = format!("{COOKIE}=");
    raw.split(';').map(str::trim).find_map(|c| c.strip_prefix(&prefix)).map(str::to_string)
}

fn set_cookie(sid: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("{COOKIE}={sid}; Path=/; HttpOnly; SameSite=Strict")).expect("ascii cookie")
}

/// Find or create the visitor's session. Returns `(sid, csrf, created)`.
fn session(app: &App, headers: &HeaderMap) -> (String, String, bool) {
    let t = now();
    let mut s = app.sessions();
    s.retain(|_, v| v.last + SESSION_IDLE > t);
    if let Some(sid) = cookie_sid(headers)
        && let Some(sess) = s.get_mut(&sid) {
            sess.last = t;
            return (sid, sess.csrf.clone(), false);
        }
    let sid = hex(&sotto_core::rand::bytes::<32>());
    let csrf = hex(&sotto_core::rand::bytes::<16>());
    if s.len() >= MAX_SESSIONS {
        // Under a flood, hand out a session that is not stored; forms will ask the visitor to retry.
        return (sid, csrf, true);
    }
    s.insert(sid.clone(), Session { kind: SessionKind::Anonymous, csrf: csrf.clone(), last: t });
    (sid, csrf, true)
}

fn with_cookie(sid: &str, created: bool, body: impl IntoResponse) -> Response {
    let mut r = body.into_response();
    if created {
        r.headers_mut().insert(header::SET_COOKIE, set_cookie(sid));
    }
    r
}

fn csrf_ok(app: &App, sid: &str, token: &str) -> bool {
    let s = app.sessions();
    // Compare digests so the comparison time does not depend on how many characters match.
    s.get(sid).map(|v| kdf::sha256(v.csrf.as_bytes()) == kdf::sha256(token.as_bytes())).unwrap_or(false)
}

fn source_of(app: &App, headers: &HeaderMap) -> Option<(String, String, Arc<SealSecret>)> {
    let sid = cookie_sid(headers)?;
    let t = now();
    let mut s = app.sessions();
    let sess = s.get_mut(&sid)?;
    if sess.last + SESSION_IDLE <= t {
        return None;
    }
    sess.last = t;
    match &sess.kind {
        SessionKind::Source { id, secret } => Some((sid.clone(), id.clone(), secret.clone())),
        _ => None,
    }
}

async fn css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], pages::CSS)
}

async fn landing(State(app): State<Arc<App>>) -> Html<String> {
    Html(pages::landing(&app.cfg.name, &app.cfg.intro, &app.cfg.verify_hint))
}

async fn new_get(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let (sid, csrf, created) = session(&app, &headers);
    let code = codename::generate();
    if let Some(s) = app.sessions().get_mut(&sid) {
        s.kind = SessionKind::Pending { codename: code.clone() };
    }
    with_cookie(&sid, created, Html(pages::new_codename(&app.cfg.name, &code, &csrf)))
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
}

async fn derive(app: &Arc<App>, code: String) -> Option<SealSecret> {
    let _permit = app.kdf.acquire().await.ok()?;
    let (instance, m, t) = (app.instance, app.cfg.argon_m_kib, app.cfg.argon_t);
    tokio::task::spawn_blocking(move || proto::source_secret(&code, &instance, m, t).ok()).await.ok().flatten()
}

async fn new_post(State(app): State<Arc<App>>, headers: HeaderMap, Form(f): Form<CsrfForm>) -> Response {
    let Some(sid) = cookie_sid(&headers) else { return Redirect::to("/new").into_response() };
    if !csrf_ok(&app, &sid, &f.csrf) {
        return expired(&app);
    }
    let code = match app.sessions().get(&sid).map(|v| &v.kind) {
        Some(SessionKind::Pending { codename }) => codename.clone(),
        _ => return Redirect::to("/new").into_response(),
    };
    let Some(secret) = derive(&app, code).await else { return server_error(&app) };
    let public = secret.public();
    let id = source_id(&public);
    let dir = app.source_dir(&id);
    let write = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(dir.join("sub"))?;
        std::fs::create_dir_all(dir.join("rep"))?;
        let key = postcard::to_stdvec(&public).map_err(std::io::Error::other)?;
        std::fs::write(dir.join("key.bin"), key)?;
        set_day_mtime(&dir.join("key.bin"));
        Ok(())
    })();
    if write.is_err() {
        return server_error(&app);
    }
    if let Some(s) = app.sessions().get_mut(&sid) {
        s.kind = SessionKind::Source { id, secret: Arc::new(secret) };
    }
    Redirect::to("/inbox").into_response()
}

async fn login_get(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let (sid, csrf, created) = session(&app, &headers);
    with_cookie(&sid, created, Html(pages::login(&app.cfg.name, &csrf, None)))
}

#[derive(Deserialize)]
struct LoginForm {
    csrf: String,
    codename: String,
}

fn allow_login(app: &App) -> bool {
    let minute = now() / 60;
    let mut l = app.logins.lock().unwrap_or_else(|p| p.into_inner());
    if l.0 != minute {
        *l = (minute, 0);
    }
    l.1 += 1;
    l.1 <= LOGINS_PER_MINUTE
}

async fn login_post(State(app): State<Arc<App>>, headers: HeaderMap, Form(f): Form<LoginForm>) -> Response {
    let (sid, csrf, created) = session(&app, &headers);
    if created || !csrf_ok(&app, &sid, &f.csrf) {
        return with_cookie(&sid, created, Html(pages::login(&app.cfg.name, &csrf, Some("Your session expired. Enter your codename again."))));
    }
    if !allow_login(&app) {
        return Html(pages::login(&app.cfg.name, &csrf, Some("Too many attempts right now. Wait a minute, then try again."))).into_response();
    }
    let not_found = "That codename doesn't match a conversation. Check the words and their order, then try again.";
    let Ok(code) = codename::normalize(&f.codename) else {
        return Html(pages::login(&app.cfg.name, &csrf, Some(not_found))).into_response();
    };
    let Some(secret) = derive(&app, code).await else { return server_error(&app) };
    let id = source_id(&secret.public());
    if !app.source_dir(&id).join("key.bin").exists() {
        return Html(pages::login(&app.cfg.name, &csrf, Some(not_found))).into_response();
    }
    if let Some(s) = app.sessions().get_mut(&sid) {
        s.kind = SessionKind::Source { id, secret: Arc::new(secret) };
    }
    Redirect::to("/inbox").into_response()
}

#[derive(Deserialize)]
struct InboxQuery {
    sent: Option<u8>,
    error: Option<String>,
}

fn valid_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('d').and_then(|r| r.strip_suffix(".sealed")) else { return false };
    let Some((day, rand)) = rest.split_once('-') else { return false };
    !day.is_empty() && day.len() <= 8 && day.bytes().all(|b| b.is_ascii_digit()) && rand.len() == 16 && rand.bytes().all(|b| b.is_ascii_hexdigit())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

fn day_of(name: &str) -> u32 {
    name.strip_prefix('d').and_then(|r| r.split('-').next()).and_then(|d| d.parse().ok()).unwrap_or(0)
}

fn list_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|it| it.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| valid_name(n)).collect())
        .unwrap_or_default();
    v.sort();
    v
}

async fn inbox(State(app): State<Arc<App>>, headers: HeaderMap, Query(q): Query<InboxQuery>) -> Response {
    let Some((sid, id, secret)) = source_of(&app, &headers) else { return Redirect::to("/login").into_response() };
    let csrf = app.sessions().get(&sid).map(|s| s.csrf.clone()).unwrap_or_default();
    let dir = app.source_dir(&id).join("rep");
    let mut replies = Vec::new();
    for name in list_names(&dir).into_iter().rev() {
        let Ok(sealed) = std::fs::read(dir.join(&name)) else { continue };
        let Ok(plain) = sotto_core::seal::open(&secret, &sealed) else { continue };
        let Ok(r) = postcard::from_bytes::<proto::Reply>(&plain) else { continue };
        replies.push(pages::ReplyItem { name, day: r.day, text: r.text });
    }
    let error = match q.error.as_deref() {
        Some("empty") => Some("Write a message or attach a file first."),
        Some("failed") => Some("That didn't go through, and nothing was saved. Try again."),
        _ => None,
    };
    Html(pages::inbox(&app.cfg.name, &csrf, &replies, q.sent == Some(1), app.cfg.max_upload_mb, error)).into_response()
}

/// Round file times down to the day so the filesystem keeps no finer timing than the names do.
fn set_day_mtime(path: &Path) {
    let t = UNIX_EPOCH + Duration::from_secs(today() as u64 * 86_400);
    if let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_times(std::fs::FileTimes::new().set_modified(t).set_accessed(t));
    }
}

fn new_name() -> String {
    format!("d{}-{}.sealed", today(), hex(&sotto_core::rand::bytes::<8>()))
}

fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.trim().is_empty() { "file".into() } else { clean }
}

async fn write_sealed_message(app: &App, dir: &Path, text: &str) -> Result<(), ()> {
    let meta = SubmissionMeta { kind: SubmissionKind::Message, day: today() };
    let mut plain = proto::submission_prefix(&meta).map_err(|_| ())?;
    plain.extend_from_slice(text.as_bytes());
    let sealed = sotto_core::seal::seal(&app.newsroom, &plain).map_err(|_| ())?;
    let path = dir.join(new_name());
    tokio::fs::write(&path, sealed).await.map_err(|_| ())?;
    set_day_mtime(&path);
    Ok(())
}

/// Read a text field without buffering more than `cap` bytes.
async fn bounded_text(field: &mut axum::extract::multipart::Field<'_>, cap: usize) -> Result<String, ()> {
    let mut buf = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(|_| ())? {
        if buf.len() + chunk.len() > cap {
            return Err(());
        }
        buf.extend_from_slice(&chunk);
    }
    String::from_utf8(buf).map_err(|_| ())
}

async fn submit(State(app): State<Arc<App>>, headers: HeaderMap, mut form: Multipart) -> Response {
    let Some((sid, id, _)) = source_of(&app, &headers) else { return Redirect::to("/login").into_response() };
    let dir = app.source_dir(&id).join("sub");
    let mut csrf_checked = false;
    let mut wrote = false;
    let result: Result<(), ()> = async {
        while let Some(mut field) = form.next_field().await.map_err(|_| ())? {
            match field.name().unwrap_or("") {
                "csrf" => {
                    let token = bounded_text(&mut field, 128).await?;
                    csrf_checked = csrf_ok(&app, &sid, &token);
                    if !csrf_checked {
                        return Err(());
                    }
                }
                "message" if csrf_checked => {
                    let text = bounded_text(&mut field, MAX_MESSAGE).await?;
                    if !text.trim().is_empty() {
                        write_sealed_message(&app, &dir, &text).await?;
                        wrote = true;
                    }
                }
                "file" if csrf_checked => {
                    if field.file_name().map(str::is_empty).unwrap_or(true) {
                        continue;
                    }
                    let name = sanitize_file_name(field.file_name().unwrap_or(""));
                    let mime: String = field.content_type().unwrap_or("application/octet-stream").chars().take(100).collect();
                    let meta = SubmissionMeta { kind: SubmissionKind::File { name, mime }, day: today() };
                    let (mut sealer, header) = Sealer::new(&app.newsroom).map_err(|_| ())?;
                    let path = dir.join(new_name());
                    let tmp = path.with_extension("part");
                    let mut out = tokio::fs::File::create(&tmp).await.map_err(|_| ())?;
                    let stream = async {
                        out.write_all(&header).await?;
                        let prefix = proto::submission_prefix(&meta).map_err(std::io::Error::other)?;
                        out.write_all(&sealer.push(&prefix).map_err(std::io::Error::other)?).await?;
                        while let Some(chunk) = field.chunk().await.map_err(std::io::Error::other)? {
                            out.write_all(&sealer.push(&chunk).map_err(std::io::Error::other)?).await?;
                        }
                        out.write_all(&sealer.finish()).await?;
                        out.sync_all().await?;
                        Ok::<(), std::io::Error>(())
                    }
                    .await;
                    if stream.is_err() {
                        let _ = tokio::fs::remove_file(&tmp).await;
                        return Err(());
                    }
                    tokio::fs::rename(&tmp, &path).await.map_err(|_| ())?;
                    set_day_mtime(&path);
                    wrote = true;
                }
                _ => return Err(()),
            }
        }
        Ok(())
    }
    .await;
    match (result, wrote) {
        (Ok(()), true) => Redirect::to("/inbox?sent=1").into_response(),
        (Ok(()), false) => Redirect::to("/inbox?error=empty").into_response(),
        (Err(()), _) if !csrf_checked => expired(&app),
        (Err(()), _) => Redirect::to("/inbox?error=failed").into_response(),
    }
}

#[derive(Deserialize)]
struct DeleteForm {
    csrf: String,
    name: String,
}

async fn delete_reply(State(app): State<Arc<App>>, headers: HeaderMap, Form(f): Form<DeleteForm>) -> Response {
    let Some((sid, id, _)) = source_of(&app, &headers) else { return Redirect::to("/login").into_response() };
    if !csrf_ok(&app, &sid, &f.csrf) || !valid_name(&f.name) {
        return expired(&app);
    }
    let _ = std::fs::remove_file(app.source_dir(&id).join("rep").join(&f.name));
    Redirect::to("/inbox").into_response()
}

async fn logout(State(app): State<Arc<App>>, headers: HeaderMap, Form(f): Form<CsrfForm>) -> Response {
    if let Some(sid) = cookie_sid(&headers)
        && csrf_ok(&app, &sid, &f.csrf) {
            app.sessions().remove(&sid);
        }
    let mut r = Html(pages::message(
        &app.cfg.name,
        "You've left",
        "This session is gone from our server. Close Tor Browser to clear it from this device too.",
    ))
    .into_response();
    r.headers_mut().insert(header::SET_COOKIE, HeaderValue::from_static("sotto_s=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict"));
    r
}

fn expired(app: &App) -> Response {
    (
        StatusCode::FORBIDDEN,
        Html(pages::message(&app.cfg.name, "Your session expired", "For your safety, sessions end after 30 minutes without activity. Start again from the beginning.")),
    )
        .into_response()
}

fn server_error(app: &App) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Html(pages::message(&app.cfg.name, "Something went wrong", "Nothing was saved. Try again in a few minutes."))).into_response()
}

// ---------- Journalist API ----------

fn journalist(app: &App, method: &Method, path: &str, headers: &HeaderMap, body: &[u8]) -> Result<(), StatusCode> {
    let h = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).ok_or(StatusCode::UNAUTHORIZED)?;
    let t = now();
    let (_, sig) = proto::check_auth_header(h, method.as_str(), path, body, t, &app.journalists).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let mut seen = app.replay.lock().unwrap_or_else(|p| p.into_inner());
    seen.retain(|_, ts| *ts + 600 > t);
    if seen.insert(kdf::sha256(&sig), t).is_some() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SourceSummary {
    pub id: String,
    pub submissions: usize,
    pub replies: usize,
    pub last_day: u32,
}

async fn j_sources(State(app): State<Arc<App>>, method: Method, headers: HeaderMap) -> Response {
    if let Err(s) = journalist(&app, &method, "/j/v1/sources", &headers, b"") {
        return s.into_response();
    }
    let mut out = Vec::new();
    if let Ok(it) = std::fs::read_dir(app.cfg.data_dir.join("sources")) {
        for e in it.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            if !valid_id(&id) {
                continue;
            }
            let subs = list_names(&e.path().join("sub"));
            let reps = list_names(&e.path().join("rep"));
            let last_day = subs.iter().chain(reps.iter()).map(|n| day_of(n)).max().unwrap_or(0);
            out.push(SourceSummary { id, submissions: subs.len(), replies: reps.len(), last_day });
        }
    }
    out.sort_by(|a, b| b.last_day.cmp(&a.last_day).then(a.id.cmp(&b.id)));
    Json(out).into_response()
}

async fn j_key(State(app): State<Arc<App>>, method: Method, headers: HeaderMap, UrlPath(id): UrlPath<String>) -> Response {
    if !valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(s) = journalist(&app, &method, &format!("/j/v1/sources/{id}/key"), &headers, b"") {
        return s.into_response();
    }
    match std::fs::read(app.source_dir(&id).join("key.bin")) {
        Ok(b) => b.into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SubmissionSummary {
    pub name: String,
    pub size: u64,
    pub day: u32,
}

async fn j_submissions(State(app): State<Arc<App>>, method: Method, headers: HeaderMap, UrlPath(id): UrlPath<String>) -> Response {
    if !valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(s) = journalist(&app, &method, &format!("/j/v1/sources/{id}/submissions"), &headers, b"") {
        return s.into_response();
    }
    let dir = app.source_dir(&id).join("sub");
    let out: Vec<SubmissionSummary> = list_names(&dir)
        .into_iter()
        .map(|name| SubmissionSummary { size: std::fs::metadata(dir.join(&name)).map(|m| m.len()).unwrap_or(0), day: day_of(&name), name })
        .collect();
    Json(out).into_response()
}

async fn j_get_submission(State(app): State<Arc<App>>, method: Method, headers: HeaderMap, UrlPath((id, name)): UrlPath<(String, String)>) -> Response {
    if !valid_id(&id) || !valid_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(s) = journalist(&app, &method, &format!("/j/v1/sources/{id}/submissions/{name}"), &headers, b"") {
        return s.into_response();
    }
    match tokio::fs::read(app.source_dir(&id).join("sub").join(&name)).await {
        Ok(b) => ([(header::CONTENT_TYPE, "application/octet-stream")], b).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn j_delete_submission(State(app): State<Arc<App>>, method: Method, headers: HeaderMap, UrlPath((id, name)): UrlPath<(String, String)>) -> Response {
    if !valid_id(&id) || !valid_name(&name) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(s) = journalist(&app, &method, &format!("/j/v1/sources/{id}/submissions/{name}"), &headers, b"") {
        return s.into_response();
    }
    let _ = std::fs::remove_file(app.source_dir(&id).join("sub").join(&name));
    StatusCode::NO_CONTENT.into_response()
}

async fn j_reply(State(app): State<Arc<App>>, method: Method, headers: HeaderMap, UrlPath(id): UrlPath<String>, body: Bytes) -> Response {
    if !valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(s) = journalist(&app, &method, &format!("/j/v1/sources/{id}/replies"), &headers, &body) {
        return s.into_response();
    }
    if body.len() > 1 << 20 || !body.starts_with(b"SOS1") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let dir = app.source_dir(&id).join("rep");
    if !dir.exists() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = dir.join(new_name());
    if std::fs::write(&path, &body).is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    set_day_mtime(&path);
    StatusCode::CREATED.into_response()
}

async fn j_delete_source(State(app): State<Arc<App>>, method: Method, headers: HeaderMap, UrlPath(id): UrlPath<String>) -> Response {
    if !valid_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if let Err(s) = journalist(&app, &method, &format!("/j/v1/sources/{id}"), &headers, b"") {
        return s.into_response();
    }
    let _ = std::fs::remove_dir_all(app.source_dir(&id));
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_validated() {
        assert!(valid_name("d20404-0123456789abcdef.sealed"));
        assert!(!valid_name("d20404-0123456789abcdef.sealed/../x"));
        assert!(!valid_name("../key.bin"));
        assert!(!valid_name("d-0123456789abcdef.sealed"));
        assert!(valid_id(&"a".repeat(32)));
        assert!(!valid_id("../../etc"));
        assert_eq!(day_of("d20404-0123456789abcdef.sealed"), 20404);
        assert_eq!(sanitize_file_name("C:\\a\\b.doc"), "b.doc");
    }
}
