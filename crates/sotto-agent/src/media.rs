//! Voice notes, audio and video. Like photos, everything shown or played is a clean copy made
//! from raw samples and frames that left the sandbox (see `sotto_review::media`):
//!
//! - Audio (voice notes included: a file can't prove Sotto's recorder made it) is cleaned the
//!   first time it's asked for: a waveform and a fresh AAC file.
//! - A video gets its poster frame and length the first time; the playable copy is made when
//!   someone presses play (it takes longer), at most 1280 pixels and 10 minutes.
//! - Your own videos play from the file itself. Held for Show in Extreme mode and for the
//!   anonymous inbox, like photos.
//!
//! Records: `media/<key>`; blobs: `mediaposter/<key>` (JPEG) and `mediaplay/<key>` (MP4).

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ApiError;
use crate::photos::source;
use crate::review::{Scratch, scratch_dir};
use crate::state::{Agent, RiskMode, Session};

type A = State<Arc<Agent>>;

/// Media jobs run one at a time (a video can keep ffmpeg busy for a while), apart from photos.
static JOBS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

#[derive(Serialize, Deserialize, Default)]
struct MediaRec {
    /// "audio" or "video".
    kind: String,
    duration: f64,
    width: u32,
    height: u32,
    peaks: Vec<u8>,
    failed: Option<String>,
    /// Your own video: it plays from the file itself.
    own: bool,
    /// A playable copy exists (or, for your own video, isn't needed).
    playable: bool,
}

fn rec_key(key: &str) -> String {
    format!("media/{key}")
}
fn poster_key(key: &str) -> String {
    format!("mediaposter/{key}")
}
fn play_key(key: &str) -> String {
    format!("mediaplay/{key}")
}

fn view(r: &MediaRec) -> Value {
    match &r.failed {
        Some(reason) => json!({ "state": "failed", "reason": reason }),
        None => json!({ "state": "ready", "kind": r.kind, "duration": r.duration, "width": r.width, "height": r.height, "peaks": r.peaks, "playable": r.playable }),
    }
}

/// Run `job` on the file at `key`, decrypted only into a memory-backed scratch folder.
async fn with_input<T: Send + 'static>(s: &Session, key: &str, job: impl FnOnce(&std::path::Path, &std::path::Path, &std::path::Path) -> Result<T, sotto_review::ReviewError> + Send + 'static) -> Result<Result<T, String>, ApiError> {
    let (store, blob, _) = source(s, key).ok_or(ApiError::NotFound)?;
    let scratch = Scratch(scratch_dir());
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&scratch.0).map_err(|_| ApiError::Internal)?;
    }
    let input = scratch.0.join("input");
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&input).map_err(|_| ApiError::Internal)?;
        store.read_blob(&blob, &mut f)?;
    }
    let work = scratch.0.join("work");
    let render = sotto_review::sandbox::default_render_path();
    let result = tokio::task::spawn_blocking(move || job(&input, &work, &render)).await.map_err(|_| ApiError::Internal)?;
    drop(scratch);
    Ok(result.map_err(|e| match e {
        sotto_review::ReviewError::NoSandbox => "Audio and video are opened in the safe review sandbox, which needs bubblewrap.".to_string(),
        other => format!("Couldn't open this safely: {other}"),
    }))
}

/// Sending "as a video": your file with every metadata tag removed (location, camera, times).
pub async fn clean_video(body: &[u8], mime: &str) -> Result<(Vec<u8>, &'static str), ApiError> {
    let ext = match mime {
        "video/mp4" | "video/quicktime" => "mp4",
        "video/webm" => "webm",
        "video/x-matroska" => "mkv",
        _ => return Err(ApiError::Invalid("Sotto can remove details from MP4, MOV, WebM and MKV videos. Send this one as a file instead.".into())),
    };
    let _job = JOBS.lock().await;
    let scratch = Scratch(scratch_dir());
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&scratch.0).map_err(|_| ApiError::Internal)?;
    }
    let input = scratch.0.join("input");
    std::fs::write(&input, body).map_err(|_| ApiError::Internal)?;
    let work = scratch.0.join("work");
    let render = sotto_review::sandbox::default_render_path();
    let out = tokio::task::spawn_blocking(move || sotto_review::media::remux(&input, &work, &render, ext, TIMEOUT)).await.map_err(|_| ApiError::Internal)?;
    drop(scratch);
    out.map(|b| (b, ext)).map_err(|e| ApiError::Invalid(format!("Couldn't remove the details from this video: {e} Send it as a file instead.")))
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/media/prepare", post(prepare))
        .route("/api/media/poster", get(poster))
        .route("/api/media/play", get(play))
        .route("/api/media/save", post(save))
}

#[derive(Deserialize)]
struct PrepareIn {
    key: String,
    /// "audio" or "video", from the file's type (the sandbox has the final say).
    kind: String,
    #[serde(default)]
    show: bool,
    #[serde(default)]
    own: bool,
    /// Make a video playable now.
    #[serde(default)]
    play: bool,
}

async fn prepare(State(agent): A, Json(b): Json<PrepareIn>) -> Result<Json<Value>, ApiError> {
    let s = agent.session().await?;
    let (store, blob, inbox) = source(&s, &b.key).ok_or(ApiError::NotFound)?;
    let existing = s.reviews.get::<MediaRec>(&rec_key(&b.key)).ok().flatten();
    if let Some(r) = &existing
        && (r.failed.is_none() || !b.show)
        && (r.playable || !b.play || r.failed.is_some())
    {
        return Ok(Json(view(r)));
    }
    let extreme = s.settings.lock().await.risk_mode == RiskMode::Extreme;
    if !b.show && !b.own && existing.is_none() && (extreme || inbox) {
        return Ok(Json(json!({ "state": "held", "reason": if inbox { "From the anonymous inbox" } else { "Extreme mode" } })));
    }
    if !store.has_blob(&blob) {
        return Ok(Json(json!({ "state": "missing" })));
    }
    let _job = JOBS.lock().await;
    if let Some(r) = s.reviews.get::<MediaRec>(&rec_key(&b.key)).ok().flatten()
        && r.failed.is_none()
        && (r.playable || !b.play)
    {
        return Ok(Json(view(&r)));
    }
    let mut rec = existing.filter(|r| r.failed.is_none()).unwrap_or_default();
    if b.kind == "audio" {
        match with_input(&s, &b.key, |i, w, r| sotto_review::media::audio(i, w, r, TIMEOUT)).await? {
            Ok(a) => {
                s.reviews.put_blob(&play_key(&b.key), &mut &a.bytes[..])?;
                rec = MediaRec { kind: "audio".into(), duration: a.duration, peaks: a.peaks, playable: true, ..Default::default() };
            }
            Err(reason) => rec = MediaRec { kind: "audio".into(), failed: Some(reason), ..Default::default() },
        }
    } else {
        if rec.kind.is_empty() {
            match with_input(&s, &b.key, |i, w, r| {
                let p = sotto_review::media::probe(i, w, r, TIMEOUT)?;
                let poster = sotto_review::media::poster(i, w, r, TIMEOUT)?;
                Ok((p, poster))
            })
            .await?
            {
                Ok((p, poster)) => {
                    s.reviews.put_blob(&poster_key(&b.key), &mut &poster.bytes[..])?;
                    let (width, height) = sotto_review::media::play_size(&p);
                    rec = MediaRec { kind: "video".into(), duration: p.duration, width, height, own: b.own && !inbox, playable: b.own && !inbox, ..Default::default() };
                }
                Err(reason) => rec = MediaRec { kind: "video".into(), failed: Some(reason), ..Default::default() },
            }
        }
        if rec.failed.is_none() && b.play && !rec.playable {
            match with_input(&s, &b.key, |i, w, r| {
                let p = sotto_review::media::probe(i, w, r, TIMEOUT)?;
                sotto_review::media::video(i, w, r, &p, TIMEOUT)
            })
            .await?
            {
                Ok(bytes) => {
                    s.reviews.put_blob(&play_key(&b.key), &mut &bytes[..])?;
                    rec.playable = true;
                }
                Err(reason) => rec.failed = Some(reason),
            }
        }
    }
    s.reviews.put(&rec_key(&b.key), &rec)?;
    Ok(Json(view(&rec)))
}

#[derive(Deserialize)]
struct KeyQ {
    key: String,
}

fn ok_rec(s: &Session, key: &str) -> Result<MediaRec, ApiError> {
    s.reviews.get::<MediaRec>(&rec_key(key)).ok().flatten().filter(|r| r.failed.is_none()).ok_or(ApiError::NotFound)
}

async fn poster(State(agent): A, Query(q): Query<KeyQ>) -> Result<Response, ApiError> {
    let s = agent.session().await?;
    ok_rec(&s, &q.key)?;
    let mut out = Vec::new();
    s.reviews.read_blob(&poster_key(&q.key), &mut out)?;
    Ok(([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "no-store")], out).into_response())
}

/// The bytes to play: the clean copy, or your own video as you sent it.
fn play_bytes(s: &Session, key: &str, r: &MediaRec, out: &mut impl std::io::Write) -> Result<&'static str, ApiError> {
    if r.own {
        let (store, blob, _) = source(s, key).ok_or(ApiError::NotFound)?;
        let mut bytes = Vec::new();
        store.read_blob(&blob, &mut bytes)?;
        let ct = if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) { "video/webm" } else { "video/mp4" };
        out.write_all(&bytes).map_err(|_| ApiError::Internal)?;
        return Ok(ct);
    }
    if !r.playable {
        return Err(ApiError::NotFound);
    }
    s.reviews.read_blob(&play_key(key), out)?;
    Ok(if r.kind == "audio" { "audio/mp4" } else { "video/mp4" })
}

async fn play(State(agent): A, Query(q): Query<KeyQ>) -> Result<Response, ApiError> {
    let s = agent.session().await?;
    let r = ok_rec(&s, &q.key)?;
    let mut out = Vec::new();
    let ct = play_bytes(&s, &q.key, &r, &mut out)?;
    Ok(([(header::CONTENT_TYPE, ct), (header::CACHE_CONTROL, "no-store")], out).into_response())
}

#[derive(Deserialize)]
struct SaveIn {
    key: String,
    path: String,
}

async fn save(State(agent): A, Json(b): Json<SaveIn>) -> Result<Json<Value>, ApiError> {
    let s = agent.session().await?;
    let path = std::path::PathBuf::from(&b.path);
    if !path.is_absolute() {
        return Err(ApiError::Invalid("Choose where to save it.".into()));
    }
    let r = ok_rec(&s, &b.key)?;
    let mut f = std::fs::File::create(&path).map_err(|_| ApiError::Invalid("Couldn't save there. Pick another folder.".into()))?;
    play_bytes(&s, &b.key, &r, &mut f)?;
    Ok(Json(json!({ "ok": true })))
}

/// Remove clean copies whose message is gone (called from the photo sweep).
pub fn sweep(s: &Session) {
    for name in s.reviews.names("media/") {
        let key = &name["media/".len()..];
        if source(s, key).is_some_and(|(store, blob, _)| !store.has_blob(&blob)) {
            let _ = s.reviews.delete_blob(&poster_key(key));
            let _ = s.reviews.delete_blob(&play_key(key));
            let _ = s.reviews.delete(&name);
        }
    }
}
