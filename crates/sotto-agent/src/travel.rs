//! Travel mode: before a border crossing, seal the whole vault into one archive that only a
//! 24-word recovery phrase can open, then erase the device. Restore it afterwards.
//!
//! Archive: hybrid sealed stream (see `docs/protocol.md` section 9) of
//! `u16 path length || path || u64 size || bytes` for every vault file.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use sotto_core::codename::{recovery_entropy, recovery_words};
use sotto_core::seal::{SealSecret, Sealer};
use sotto_vault::Vault;
use zeroize::Zeroizing;

use crate::ApiError;
use crate::state::Agent;

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/travel/prepare", post(prepare))
        .route("/api/travel/start", post(start))
        .route("/api/travel/restore", post(restore))
}

/// Which files make up a vault, as paths relative to it.
fn vault_files(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut out = vec!["vault.hdr".to_string()];
    for sub in ["records", "blobs"] {
        if let Ok(it) = std::fs::read_dir(dir.join(sub)) {
            for e in it.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if valid_name(&name) {
                    out.push(format!("{sub}/{name}"));
                }
            }
        }
    }
    Ok(out)
}

fn valid_name(name: &str) -> bool {
    name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Only paths a vault actually contains may come out of an archive.
fn valid_rel(path: &str) -> bool {
    path == "vault.hdr" || path.strip_prefix("records/").map(valid_name).unwrap_or(false) || path.strip_prefix("blobs/").map(valid_name).unwrap_or(false)
}

async fn prepare(State(agent): A) -> R {
    agent.session().await?;
    let entropy = Zeroizing::new(sotto_core::rand::bytes::<32>());
    let words = recovery_words(&entropy);
    *agent.travel.lock().await = Some(entropy);
    Ok(Json(json!({ "words": words })))
}

#[derive(Deserialize)]
struct StartIn {
    path: String,
    /// Three words from the phrase, by position, to prove it was written down: [[index, word], ...].
    check: Vec<(usize, String)>,
}

async fn start(State(agent): A, Json(b): Json<StartIn>) -> R {
    agent.session().await?;
    let entropy = agent.travel.lock().await.clone().ok_or(ApiError::Invalid("Start again: the recovery phrase expired.".into()))?;
    let words: Vec<String> = recovery_words(&entropy).split(' ').map(str::to_string).collect();
    if b.check.len() < 3 || b.check.iter().any(|(i, w)| words.get(*i).map(|x| x != &w.trim().to_lowercase()).unwrap_or(true)) {
        return Err(ApiError::Invalid("Those words don't match your recovery phrase. Check what you wrote down.".into()));
    }
    let out = PathBuf::from(&b.path);
    if !out.is_absolute() || out.starts_with(&agent.dir) {
        return Err(ApiError::Invalid("Choose a full path outside Sotto's own folder, such as a USB drive.".into()));
    }
    // Stop everything so the files are quiet while they are copied.
    agent.lock().await;
    let dir = agent.dir.clone();
    let target = out.clone();
    let e1 = entropy.clone();
    let result = tokio::task::spawn_blocking(move || write_archive(&dir, &target, &e1)).await.map_err(|_| ApiError::Internal)?;
    // Read the backup back and check it opens and parses before anything is erased.
    let check = match result {
        Ok(()) => {
            let (target, e) = (out.clone(), entropy.clone());
            tokio::task::spawn_blocking(move || parse_archive_file(&target, &e).map(|_| ())).await.map_err(|_| ApiError::Internal)?
        }
        Err(e) => Err(e),
    };
    if check.is_err() {
        let _ = std::fs::remove_file(&out);
        return Err(ApiError::Invalid("Couldn't write a readable travel backup there. Nothing was erased.".into()));
    }
    agent.panic().await?;
    *agent.travel.lock().await = None;
    Ok(Json(json!({ "ok": true, "path": b.path })))
}

fn write_archive(dir: &Path, target: &Path, entropy: &[u8; 32]) -> Result<(), ApiError> {
    use std::io::Write;
    let public = SealSecret::from_seed(entropy).public();
    let (mut sealer, header) = Sealer::new(&public)?;
    let mut f = std::io::BufWriter::new(std::fs::File::create(target).map_err(|_| ApiError::Internal)?);
    f.write_all(&header).map_err(|_| ApiError::Internal)?;
    for rel in vault_files(dir).map_err(|_| ApiError::Internal)? {
        let data = Zeroizing::new(std::fs::read(dir.join(&rel)).map_err(|_| ApiError::Internal)?);
        let mut rec = Vec::with_capacity(rel.len() + 10);
        rec.extend_from_slice(&(rel.len() as u16).to_le_bytes());
        rec.extend_from_slice(rel.as_bytes());
        rec.extend_from_slice(&(data.len() as u64).to_le_bytes());
        f.write_all(&sealer.push(&rec)?).map_err(|_| ApiError::Internal)?;
        f.write_all(&sealer.push(&data)?).map_err(|_| ApiError::Internal)?;
    }
    f.write_all(&sealer.finish()).map_err(|_| ApiError::Internal)?;
    f.flush().map_err(|_| ApiError::Internal)?;
    f.into_inner().map_err(|_| ApiError::Internal)?.sync_all().map_err(|_| ApiError::Internal)?;
    Ok(())
}

#[derive(Deserialize)]
struct RestoreIn {
    path: String,
    words: String,
}

async fn restore(State(agent): A, Json(b): Json<RestoreIn>) -> R {
    if agent.state().await != "new" {
        return Err(ApiError::Invalid("Sotto already has a vault on this device.".into()));
    }
    let entropy = Zeroizing::new(recovery_entropy(&b.words).map_err(|_| ApiError::Invalid("That recovery phrase isn't valid. Check every word and their order.".into()))?);
    let (dir, path) = (agent.dir.clone(), PathBuf::from(&b.path));
    tokio::task::spawn_blocking(move || read_archive(&path, &dir, &entropy)).await.map_err(|_| ApiError::Internal)??;
    agent.emit(json!({ "type": "state", "state": "locked" }));
    Ok(Json(json!({ "ok": true })))
}

fn parse_archive_file(path: &Path, entropy: &[u8; 32]) -> Result<usize, ApiError> {
    let secret = SealSecret::from_seed(entropy);
    let mut sealed = std::fs::File::open(path).map_err(|_| ApiError::Internal)?;
    let mut plain = Zeroizing::new(Vec::new());
    sotto_core::seal::open_stream(&secret, &mut sealed, &mut *plain).map_err(|_| ApiError::Internal)?;
    let mut r = &plain[..];
    let mut count = 0;
    let mut has_header = false;
    while !r.is_empty() {
        let plen = u16::from_le_bytes(r.get(..2).ok_or(ApiError::Internal)?.try_into().map_err(|_| ApiError::Internal)?) as usize;
        let p = std::str::from_utf8(r.get(2..2 + plen).ok_or(ApiError::Internal)?).map_err(|_| ApiError::Internal)?;
        has_header |= p == "vault.hdr";
        if !valid_rel(p) {
            return Err(ApiError::Internal);
        }
        let n = u64::from_le_bytes(r.get(2 + plen..10 + plen).ok_or(ApiError::Internal)?.try_into().map_err(|_| ApiError::Internal)?) as usize;
        r = r.get(10 + plen + n..).ok_or(ApiError::Internal)?;
        count += 1;
    }
    if has_header { Ok(count) } else { Err(ApiError::Internal) }
}

fn read_archive(path: &Path, dir: &Path, entropy: &[u8; 32]) -> Result<(), ApiError> {
    let secret = SealSecret::from_seed(entropy);
    let mut sealed = std::fs::File::open(path).map_err(|_| ApiError::Invalid("Couldn't open that file.".into()))?;
    let mut plain = Zeroizing::new(Vec::new());
    sotto_core::seal::open_stream(&secret, &mut sealed, &mut *plain).map_err(|_| ApiError::Invalid("This backup doesn't open with that recovery phrase.".into()))?;
    // Parse everything before writing anything, so a damaged archive leaves no partial vault.
    let mut files: Vec<(String, &[u8])> = Vec::new();
    let mut r = &plain[..];
    while !r.is_empty() {
        let mut len2 = [0u8; 2];
        r.read_exact(&mut len2).map_err(|_| ApiError::Internal)?;
        let plen = u16::from_le_bytes(len2) as usize;
        let p = std::str::from_utf8(r.get(..plen).ok_or(ApiError::Internal)?).map_err(|_| ApiError::Internal)?.to_string();
        r = &r[plen..];
        let mut len8 = [0u8; 8];
        r.read_exact(&mut len8).map_err(|_| ApiError::Internal)?;
        let n = u64::from_le_bytes(len8) as usize;
        let data = r.get(..n).ok_or(ApiError::Internal)?;
        r = &r[n..];
        if !valid_rel(&p) {
            return Err(ApiError::Invalid("This file isn't a Sotto travel backup.".into()));
        }
        files.push((p, data));
    }
    if !files.iter().any(|(p, _)| p == "vault.hdr") || Vault::exists(dir) {
        return Err(ApiError::Invalid("This file isn't a Sotto travel backup.".into()));
    }
    for sub in ["records", "blobs"] {
        std::fs::create_dir_all(dir.join(sub)).map_err(|_| ApiError::Internal)?;
    }
    for (p, data) in files.iter().filter(|(p, _)| p != "vault.hdr") {
        std::fs::write(dir.join(p), data).map_err(|_| ApiError::Internal)?;
    }
    // The header goes last: until it exists, the restored files are inert.
    let hdr = files.iter().find(|(p, _)| p == "vault.hdr").map(|(_, d)| *d).ok_or(ApiError::Internal)?;
    std::fs::write(dir.join("vault.hdr"), hdr).map_err(|_| ApiError::Internal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn archive_paths_are_restricted() {
        assert!(super::valid_rel("vault.hdr"));
        assert!(super::valid_rel(&format!("records/{}", "a".repeat(32))));
        assert!(!super::valid_rel("records/../../etc/passwd"));
        assert!(!super::valid_rel("/etc/passwd"));
        assert!(!super::valid_rel("blobs/short"));
    }
}
