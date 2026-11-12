//! Photos. Every photo shown or sent is a clean copy: decoded only inside the review sandbox,
//! which hands back raw pixels, and encoded again here. Location, camera details and times can't
//! survive that, and a booby-trapped image never reaches the app's own decoders.
//!
//! - Sending "as a photo" cleans the bytes before upload; "as a file" sends them untouched.
//! - A received photo is cleaned the first time the app asks for it, automatically except in
//!   Extreme mode or for the anonymous inbox, where it waits for **Show**.
//! - Clean copies live in the reviews store (`photo/<key>` record, `photoimg/<key>` bytes) and
//!   are removed by a sweep once the message they came from is gone, as are Safe review results.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_vault::Store;

use crate::ApiError;
use crate::review::{Scratch, scratch_dir};
use crate::state::{Agent, RiskMode, Session};

type A = State<Arc<Agent>>;

/// One photo is cleaned at a time: it bounds memory, and a second request for the same photo
/// finds the first one's result.
static JOBS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Serialize, Deserialize)]
struct PhotoRec {
    width: u32,
    height: u32,
    mime: String,
    /// Why it couldn't be cleaned (it's then shown as an ordinary file).
    failed: Option<String>,
    /// Your own photo, shown from the file itself (you chose it; nothing to clean).
    own: bool,
}

fn rec_key(key: &str) -> String {
    format!("photo/{key}")
}
fn img_key(key: &str) -> String {
    format!("photoimg/{key}")
}

/// Which store and blob a review or photo key points at: `m:file/<mid>` (a 1:1 message),
/// `s:sblob/...` (a group or channel file) or `d:<blob>` (the anonymous inbox).
fn source<'a>(s: &'a Session, key: &str) -> Option<(&'a Arc<Store>, String, bool)> {
    let (kind, blob) = key.split_once(':')?;
    if blob.is_empty() || blob.contains("..") {
        return None;
    }
    match kind {
        "m" if blob.starts_with("file/") => Some((s.engine.store(), blob.to_string(), false)),
        "s" if blob.starts_with("sblob/") => Some((&s.cases, blob.to_string(), false)),
        "d" => Some((&s.drops, blob.to_string(), true)),
        _ => None,
    }
}

/// Clean photo bytes in the sandbox. The input sits only in a memory-backed scratch folder.
async fn clean_from(write_input: impl FnOnce(&mut std::fs::File) -> Result<(), ApiError>) -> Result<sotto_review::Photo, ApiError> {
    let scratch = Scratch(scratch_dir());
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&scratch.0).map_err(|_| ApiError::Internal)?;
    }
    let input = scratch.0.join("input");
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&input).map_err(|_| ApiError::Internal)?;
        write_input(&mut f)?;
    }
    let work = scratch.0.join("work");
    let render = sotto_review::sandbox::default_render_path();
    let result = tokio::task::spawn_blocking(move || sotto_review::photo(&input, &work, &render, std::time::Duration::from_secs(60))).await.map_err(|_| ApiError::Internal)?;
    drop(scratch);
    result.map_err(|e| match e {
        sotto_review::ReviewError::NoSandbox => ApiError::Invalid("Photos are cleaned in the safe review sandbox, which needs bubblewrap. Send it as a file instead, or install bubblewrap.".into()),
        other => ApiError::Invalid(format!("Couldn't clean this photo: {other} Send it as a file instead.")),
    })
}

/// The upload routes call this: when the app asked to send as a photo (`x-send-as: photo`),
/// the clean copy is what gets sent, named for its new format.
pub async fn maybe_clean(headers: &HeaderMap, name: String, mime: String, body: Bytes) -> Result<(String, String, Bytes), ApiError> {
    if headers.get("x-send-as").and_then(|v| v.to_str().ok()) != Some("photo") {
        return Ok((name, mime, body));
    }
    let _job = JOBS.lock().await;
    let p = clean_from(|f| {
        use std::io::Write;
        f.write_all(&body).map_err(|_| ApiError::Internal)
    })
    .await?;
    let stem = std::path::Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "photo".into());
    let ext = if p.mime == "image/png" { "png" } else { "jpg" };
    Ok((format!("{stem}.{ext}"), p.mime.to_string(), Bytes::from(p.bytes)))
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/photos/prepare", post(prepare))
        .route("/api/photos/image", get(image))
        .route("/api/photos/save", post(save))
}

#[derive(Deserialize)]
struct PrepareIn {
    key: String,
    /// You asked to see it (Extreme mode, the anonymous inbox, or a retry).
    #[serde(default)]
    show: bool,
    /// Your own photo: nothing to hold back.
    #[serde(default)]
    own: bool,
}

/// Size and type of your own photo, read from its header only (no decoding), turned the way
/// its orientation tag says so the box has the right shape.
fn own_dims(bytes: &[u8]) -> Option<(u32, u32, &'static str)> {
    use image::ImageDecoder;
    let format = image::guess_format(bytes).ok()?;
    let mime = match format {
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Gif => "image/gif",
        image::ImageFormat::WebP => "image/webp",
        _ => return None,
    };
    let mut dec = image::ImageReader::with_format(std::io::Cursor::new(bytes), format).into_decoder().ok()?;
    let (w, h) = dec.dimensions();
    let turned = matches!(dec.orientation().ok()?, image::metadata::Orientation::Rotate90 | image::metadata::Orientation::Rotate270 | image::metadata::Orientation::Rotate90FlipH | image::metadata::Orientation::Rotate270FlipH);
    Some(if turned { (h, w, mime) } else { (w, h, mime) })
}

fn view(r: &PhotoRec) -> Value {
    match &r.failed {
        Some(reason) => json!({ "state": "failed", "reason": reason }),
        None => json!({ "state": "ready", "width": r.width, "height": r.height, "mime": r.mime }),
    }
}

async fn prepare(State(agent): A, Json(b): Json<PrepareIn>) -> Result<Json<Value>, ApiError> {
    let s = agent.session().await?;
    let (store, blob, inbox) = source(&s, &b.key).ok_or(ApiError::NotFound)?;
    if let Some(r) = s.reviews.get::<PhotoRec>(&rec_key(&b.key)).ok().flatten()
        && (r.failed.is_none() || !b.show)
    {
        return Ok(Json(view(&r)));
    }
    let extreme = s.settings.lock().await.risk_mode == RiskMode::Extreme;
    if !b.show && !b.own && (extreme || inbox) {
        return Ok(Json(json!({ "state": "held", "reason": if inbox { "From the anonymous inbox" } else { "Extreme mode" } })));
    }
    if !store.has_blob(&blob) {
        return Ok(Json(json!({ "state": "missing" })));
    }
    if b.own && !inbox {
        let mut bytes = Vec::new();
        store.read_blob(&blob, &mut bytes)?;
        if let Some((width, height, mime)) = own_dims(&bytes) {
            let rec = PhotoRec { width, height, mime: mime.into(), failed: None, own: true };
            s.reviews.put(&rec_key(&b.key), &rec)?;
            return Ok(Json(view(&rec)));
        }
    }
    let _job = JOBS.lock().await;
    // Someone else may have finished it while we waited.
    if let Some(r) = s.reviews.get::<PhotoRec>(&rec_key(&b.key)).ok().flatten()
        && r.failed.is_none()
    {
        return Ok(Json(view(&r)));
    }
    let store = store.clone();
    let rec = match clean_from(move |f| store.read_blob(&blob, f).map(|_| ()).map_err(ApiError::from)).await {
        Ok(p) => {
            s.reviews.put_blob(&img_key(&b.key), &mut &p.bytes[..])?;
            PhotoRec { width: p.width as u32, height: p.height as u32, mime: p.mime.to_string(), failed: None, own: false }
        }
        Err(ApiError::Invalid(reason)) => PhotoRec { width: 0, height: 0, mime: String::new(), failed: Some(reason), own: false },
        Err(e) => return Err(e),
    };
    s.reviews.put(&rec_key(&b.key), &rec)?;
    Ok(Json(view(&rec)))
}

#[derive(Deserialize)]
struct KeyQ {
    key: String,
}

async fn image(State(agent): A, Query(q): Query<KeyQ>) -> Result<Response, ApiError> {
    let s = agent.session().await?;
    let r = s.reviews.get::<PhotoRec>(&rec_key(&q.key)).ok().flatten().ok_or(ApiError::NotFound)?;
    if r.failed.is_some() {
        return Err(ApiError::NotFound);
    }
    let mut out = Vec::new();
    if r.own {
        let (store, blob, _) = source(&s, &q.key).ok_or(ApiError::NotFound)?;
        store.read_blob(&blob, &mut out)?;
    } else {
        s.reviews.read_blob(&img_key(&q.key), &mut out)?;
    }
    // Only types this module recorded.
    let ct = match r.mime.as_str() {
        "image/png" => "image/png",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        _ => "image/jpeg",
    };
    Ok(([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "no-store")], out).into_response())
}

#[derive(Deserialize)]
struct SaveIn {
    key: String,
    path: String,
}

/// Save the clean copy outside Sotto, where you chose.
async fn save(State(agent): A, Json(b): Json<SaveIn>) -> Result<Json<Value>, ApiError> {
    let s = agent.session().await?;
    let path = std::path::PathBuf::from(&b.path);
    if !path.is_absolute() {
        return Err(ApiError::Invalid("Choose where to save it.".into()));
    }
    let r = s.reviews.get::<PhotoRec>(&rec_key(&b.key)).ok().flatten().filter(|r| r.failed.is_none()).ok_or(ApiError::NotFound)?;
    let mut f = std::fs::File::create(&path).map_err(|_| ApiError::Invalid("Couldn't save there. Pick another folder.".into()))?;
    if r.own {
        let (store, blob, _) = source(&s, &b.key).ok_or(ApiError::NotFound)?;
        store.read_blob(&blob, &mut f)?;
    } else {
        s.reviews.read_blob(&img_key(&b.key), &mut f)?;
    }
    Ok(Json(json!({ "ok": true })))
}

/// After unlock: every minute, remove clean photo copies and Safe review results whose original
/// is gone (deleted, or a disappearing message that expired), so nothing outlives its message.
pub fn start(s: &Arc<Session>) {
    let s = s.clone();
    tokio::spawn(async move {
        loop {
            sweep(&s);
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

pub fn sweep(s: &Session) {
    for name in s.reviews.names("photo/") {
        let key = &name["photo/".len()..];
        if source(s, key).is_some_and(|(store, blob, _)| !store.has_blob(&blob)) {
            let _ = s.reviews.delete_blob(&img_key(key));
            let _ = s.reviews.delete(&name);
        }
    }
    for name in s.reviews.names("review/") {
        let key = &name["review/".len()..];
        if source(s, key).is_some_and(|(store, blob, _)| !store.has_blob(&blob)) {
            let pages = s.reviews.get::<String>(&name).ok().flatten().and_then(|j| serde_json::from_str::<Value>(&j).ok()).and_then(|v| v["pages"].as_u64()).unwrap_or(0);
            for n in 1..=pages {
                let _ = s.reviews.delete_blob(&format!("clean/{key}/{n}.png"));
            }
            let _ = s.reviews.delete_blob(&format!("clean/{key}.pdf"));
            let _ = s.reviews.delete(&format!("reviewtext/{key}"));
            let _ = s.reviews.delete(&name);
        }
    }
}
