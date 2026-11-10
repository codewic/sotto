//! Safe review inside the agent: plaintext only ever sits on a RAM-backed scratch directory.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use sotto_client::Review;
use sotto_vault::Store;

use crate::ApiError;
use crate::state::Session;

/// A scratch directory on tmpfs when possible ($XDG_RUNTIME_DIR, then /dev/shm).
pub fn scratch_dir() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| Some(PathBuf::from("/dev/shm")).filter(|p| p.is_dir()))
        .unwrap_or_else(std::env::temp_dir);
    base.join(format!("sotto-review-{}", sotto_core::hex(&sotto_core::rand::bytes::<8>())))
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        // Overwrite the decrypted input before unlinking, in case scratch is not RAM-backed.
        let input = self.0.join("input");
        if let Ok(len) = std::fs::metadata(&input).map(|m| m.len()) {
            let _ = std::fs::write(&input, vec![0u8; len.min(1 << 30) as usize]);
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where a reviewable file lives.
pub enum Source<'a> {
    Message { contact: [u8; 16], message: [u8; 16], blob: String },
    Drop { drop: &'a str, source: &'a str, item: &'a str, blob: String },
}

impl Source<'_> {
    pub fn key(&self) -> String {
        match self {
            Source::Message { blob, .. } => format!("m:{blob}"),
            Source::Drop { blob, .. } => format!("d:{blob}"),
        }
    }
}

fn store_for<'a>(s: &'a Session, src: &Source) -> &'a Arc<Store> {
    match src {
        Source::Message { .. } => s.engine.store(),
        Source::Drop { .. } => &s.drops,
    }
}

pub async fn run(s: &Arc<Session>, src: Source<'_>, greyscale: bool) -> Result<Value, ApiError> {
    let key = src.key();
    let scratch = Scratch(scratch_dir());
    if cfg!(target_os = "linux") && !(scratch.0.starts_with("/dev/shm") || std::env::var_os("XDG_RUNTIME_DIR").map(|d| scratch.0.starts_with(d)).unwrap_or(false)) {
        return Err(ApiError::Invalid("Safe review needs a memory-backed scratch folder ($XDG_RUNTIME_DIR or /dev/shm), so decrypted files never touch the disk.".into()));
    }
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&scratch.0).map_err(|_| ApiError::Internal)?;
    }
    let input = scratch.0.join("input");
    {
        let blob = match &src {
            Source::Message { blob, .. } | Source::Drop { blob, .. } => blob.clone(),
        };
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&input).map_err(|_| ApiError::Internal)?;
        store_for(s, &src).read_blob(&blob, &mut f)?;
    }
    let work = scratch.0.join("work");
    let (inp, wk) = (input.clone(), work.clone());
    let opts = sotto_review::Options { greyscale, ..Default::default() };
    let result = tokio::task::spawn_blocking(move || sotto_review::review(&inp, &wk, &opts)).await.map_err(|_| ApiError::Internal)?;
    let review = match result {
        Ok(report) => {
            let clean_blob = format!("clean/{key}.pdf");
            s.reviews.put_blob(&clean_blob, &mut std::fs::File::open(&report.clean_pdf).map_err(|_| ApiError::Internal)?)?;
            for (i, p) in report.previews.iter().enumerate() {
                s.reviews.put_blob(&format!("clean/{key}/{}.png", i + 1), &mut std::fs::File::open(p).map_err(|_| ApiError::Internal)?)?;
            }
            let json = serde_json::to_string(&report).map_err(|_| ApiError::Internal)?;
            s.reviews.put(&format!("review/{key}"), &json)?;
            s.reviews.put(&format!("reviewtext/{key}"), &report.text)?;
            Review::Clean { clean_blob, pages: report.pages as u32 }
        }
        Err(e) => Review::Failed { reason: e.to_string() },
    };
    match &src {
        Source::Message { contact, message, .. } => s.engine.set_review(contact, message, review.clone())?,
        Source::Drop { drop, source, item, .. } => {
            let k = crate::drops::item_key(drop, source, item);
            if let Some(mut it) = s.drops.get::<crate::drops::DropItem>(&k)? {
                it.review = review.clone();
                s.drops.put(&k, &it)?;
            }
        }
    }
    match review {
        Review::Failed { reason } => Err(ApiError::Invalid(reason)),
        _ => report(s, &key),
    }
}

pub fn report(s: &Session, key: &str) -> Result<Value, ApiError> {
    let json: String = s.reviews.get(&format!("review/{key}"))?.ok_or(ApiError::NotFound)?;
    let mut v: Value = serde_json::from_str(&json).map_err(|_| ApiError::Internal)?;
    v["key"] = json!(key);
    Ok(v)
}

pub fn page(s: &Session, key: &str, n: u32) -> Result<Vec<u8>, ApiError> {
    let mut out = Vec::new();
    s.reviews.read_blob(&format!("clean/{key}/{n}.png"), &mut out)?;
    Ok(out)
}

pub fn compare(s: &Session, a: &str, b: &str) -> Result<Vec<sotto_review::Passage>, ApiError> {
    let ta: String = s.reviews.get(&format!("reviewtext/{a}"))?.ok_or(ApiError::NotFound)?;
    let tb: String = s.reviews.get(&format!("reviewtext/{b}"))?.ok_or(ApiError::NotFound)?;
    if ta.trim().is_empty() || tb.trim().is_empty() {
        return Err(ApiError::Invalid("One of these files has no text to compare (it may be a scan or photo).".into()));
    }
    Ok(sotto_review::compare_texts(&ta, &tb))
}

/// Write the clean copy to a path the journalist chose.
pub fn export_clean(s: &Session, key: &str, path: &Path) -> Result<(), ApiError> {
    let mut f = std::fs::File::create(path).map_err(|_| ApiError::Invalid("Couldn't write to that location.".into()))?;
    s.reviews.read_blob(&format!("clean/{key}.pdf"), &mut f)?;
    Ok(())
}
