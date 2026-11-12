//! Signed automatic updates.
//!
//! The app fetches the public manifest (`latest.json`) and its signature from the releases
//! repository, checks the signature against the release key pinned in `sotto-core`, and only then
//! looks at what's inside. A newer version's package is downloaded to this device and checked
//! against the hash in the signed manifest; the app's shell installs it when you choose to restart.
//!
//! Privacy: the request is the same public file everyone fetches, with no identifiers or version.
//! Checks run about every 6 hours at random offsets, through Tor when Sotto uses Tor, and in
//! Extreme mode only when you ask.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use sotto_core::release::{newer, pinned_key, verify_manifest};
use sotto_core::{hex, kdf};

use crate::ApiError;
use crate::state::{Agent, RiskMode};

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

/// Where releases are published. `SOTTO_UPDATE_BASE` points elsewhere for tests.
const RELEASES: &str = "https://github.com/codewic/sotto-releases/releases";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Default)]
struct Status {
    state: &'static str,
    version: Option<String>,
    notes: String,
    error: Option<String>,
    checked_at: Option<u64>,
    /// Ready to install: (file, kind, signed manifest, signature).
    staged: Option<(PathBuf, String, String, String)>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static RUNNING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn status() -> Status {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or(Status { state: "idle", ..Default::default() })
}

fn set(f: impl FnOnce(&mut Status)) {
    let mut g = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert(Status { state: "idle", ..Default::default() });
    f(s);
}

/// The pinned release key. Debug builds (tests, development) may use a test key instead; a
/// release build only ever trusts the pinned one.
fn trusted_key() -> Option<[u8; 32]> {
    #[cfg(debug_assertions)]
    if let Ok(k) = std::env::var("SOTTO_UPDATE_TEST_KEY") {
        return sotto_core::unhex::<32>(&k).ok();
    }
    pinned_key()
}

fn base() -> String {
    std::env::var("SOTTO_UPDATE_BASE").unwrap_or_else(|_| RELEASES.to_string())
}

/// How this copy was installed (the app's shell says so): which package can replace it.
fn package_kind() -> Option<String> {
    std::env::var("SOTTO_PACKAGE").ok().filter(|k| matches!(k.as_str(), "appimage" | "deb" | "exe"))
}

fn view() -> Value {
    let s = status();
    json!({
        "current": VERSION,
        "state": s.state,
        "version": s.version,
        "notes": s.notes,
        "error": s.error,
        "checked_at": s.checked_at,
        "installable": package_kind().is_some(),
    })
}

fn emit(agent: &Agent) {
    agent.emit(json!({ "type": "update" }));
}

/// Fetch, verify, and (if newer and installable) download and verify the package.
pub async fn check(agent: &Arc<Agent>) -> Result<(), ApiError> {
    let _one = RUNNING.lock().await;
    set(|s| {
        s.state = "checking";
        s.error = None;
    });
    emit(agent);
    let result = run(agent).await;
    match &result {
        Ok(()) => {}
        Err(e) => {
            let msg = match e {
                ApiError::Invalid(m) => m.clone(),
                _ => "Couldn't reach the update server. Sotto will try again later.".into(),
            };
            set(|s| {
                s.state = "error";
                s.error = Some(msg);
            });
        }
    }
    set(|s| s.checked_at = Some(sotto_core::time::now()));
    emit(agent);
    result
}

async fn run(agent: &Arc<Agent>) -> Result<(), ApiError> {
    let transport = match agent.session().await {
        Ok(s) => s.engine.profile().await.transport,
        Err(_) => agent.default_transport.clone(),
    };
    let client = crate::calls::http(&transport)?;
    let get = |url: String| {
        let c = client.clone();
        async move {
            let r = c.get(&url).send().await.map_err(|_| ApiError::Invalid("Couldn't reach the update server. Sotto will try again later.".into()))?;
            if !r.status().is_success() {
                return Err(ApiError::Invalid("The update server didn't have a release for Sotto yet.".into()));
            }
            r.bytes().await.map(|b| b.to_vec()).map_err(|_| ApiError::Invalid("The update download was cut off.".into()))
        }
    };
    let manifest = get(format!("{}/latest/download/latest.json", base())).await?;
    let sig = String::from_utf8(get(format!("{}/latest/download/latest.json.sig", base())).await?).unwrap_or_default();
    let key = trusted_key().ok_or(ApiError::Internal)?;
    if !verify_manifest(&manifest, &sig, &key) {
        // Don't trust anything in it, not even the version.
        return Err(ApiError::Invalid("An update was offered without a valid Sotto signature, so it was ignored.".into()));
    }
    let m: Value = serde_json::from_slice(&manifest).map_err(|_| ApiError::Invalid("The update's manifest was unreadable.".into()))?;
    let version = m["version"].as_str().unwrap_or_default().to_string();
    if m["app"] != "sotto" || !newer(&version, VERSION) {
        set(|s| {
            s.state = "current";
            s.version = None;
            s.staged = None;
        });
        return Ok(());
    }
    let notes = m["notes"].as_str().unwrap_or_default().to_string();
    let Some(kind) = package_kind() else {
        set(|s| {
            s.state = "available";
            s.version = Some(version.clone());
            s.notes = notes.clone();
        });
        return Ok(());
    };
    if status().staged.as_ref().is_some_and(|(_, _, mm, _)| mm.as_bytes() == manifest.as_slice()) {
        return Ok(());
    }
    let file = m["files"]
        .as_array()
        .and_then(|fs| fs.iter().find(|f| f["kind"] == kind.as_str()))
        .cloned()
        .ok_or_else(|| ApiError::Invalid(format!("Version {version} has no {kind} package. Download it from the releases page.")))?;
    let name = file["name"].as_str().unwrap_or_default().to_string();
    // A name from the manifest is signed, but still never allowed to point outside the folder.
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return Err(ApiError::Invalid("The update's file name was not acceptable.".into()));
    }
    set(|s| {
        s.state = "downloading";
        s.version = Some(version.clone());
        s.notes = notes.clone();
    });
    emit(agent);
    let bytes = get(format!("{}/download/v{version}/{name}", base())).await?;
    if hex(&kdf::sha256(&bytes)) != file["sha256"].as_str().unwrap_or_default() || Some(bytes.len() as u64) != file["size"].as_u64() {
        return Err(ApiError::Invalid("The downloaded update didn't match its signed fingerprint, so it was thrown away.".into()));
    }
    let dir = agent.dir.parent().map(|p| p.join("updates")).unwrap_or_else(|| PathBuf::from("updates"));
    std::fs::create_dir_all(&dir).map_err(|_| ApiError::Internal)?;
    let path = dir.join(&name);
    let tmp = dir.join(format!("{name}.part"));
    std::fs::write(&tmp, &bytes).map_err(|_| ApiError::Internal)?;
    std::fs::rename(&tmp, &path).map_err(|_| ApiError::Internal)?;
    let manifest_s = String::from_utf8(manifest).unwrap_or_default();
    set(|s| {
        s.state = "ready";
        s.staged = Some((path, kind.clone(), manifest_s, sig.trim().to_string()));
    });
    Ok(())
}

/// After unlock: check a few minutes in, then about every 6 hours, never in Extreme mode.
pub fn start(agent: &Arc<Agent>) {
    let agent = agent.clone();
    tokio::spawn(async move {
        let mut wait = 60 + u64::from(sotto_core::rand::bytes::<2>()[0]) * 2;
        loop {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            let Ok(s) = agent.session().await else { return };
            let extreme = s.settings.lock().await.risk_mode == RiskMode::Extreme;
            if !extreme && std::env::var("SOTTO_UPDATES").as_deref() != Ok("off") {
                let _ = check(&agent).await;
            }
            let jitter = u64::from(u16::from_le_bytes(sotto_core::rand::bytes::<2>())) % 7200;
            wait = 5 * 3600 + jitter;
        }
    });
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/updates", get(show))
        .route("/api/updates/check", post(check_now))
        .route("/api/updates/staged", get(staged))
}

async fn show(State(_agent): A) -> R {
    Ok(Json(view()))
}

async fn check_now(State(agent): A) -> R {
    let _ = check(&agent).await;
    Ok(Json(view()))
}

/// For the app's shell: the verified package and the signed manifest, so it can check again.
async fn staged(State(_agent): A) -> R {
    match status().staged {
        Some((path, kind, manifest, sig)) => Ok(Json(json!({ "path": path.to_string_lossy(), "kind": kind, "manifest": manifest, "sig": sig }))),
        None => Err(ApiError::NotFound),
    }
}
