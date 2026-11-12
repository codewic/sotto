//! Files shared into a group or a channel. A file is uploaded to the relay once per copy
//! (encrypted, chunked, each chunk readable a fixed number of times), and kept encrypted in this
//! desk's store. A received file is fetched in the background and stays quarantined until Safe
//! review makes a clean copy, exactly like a file in a 1:1 conversation.
//!
//! Records live under `sfile/<where>`, where `<where>` is `g/<group>/<message>` or
//! `c/<channel>/<post>`; the bytes under the blob `sblob/<where>`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_client::{Review, files};
use sotto_core::envelope::FileRef;
use sotto_core::new_id;

use crate::ApiError;
use crate::state::{Agent, Session};

/// The largest file a group or channel takes.
pub const MAX_SIZE: usize = 100 << 20;

#[derive(Clone, Serialize, Deserialize)]
pub enum Status {
    Sent,
    Downloading,
    Ready,
    Failed,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Shared {
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub blob: String,
    pub status: Status,
    pub review: Review,
    /// Where to fetch it from, until it's stored here.
    pub remote: Option<FileRef>,
}

fn key(at: &str) -> String {
    format!("sfile/{at}")
}

pub fn get(s: &Session, at: &str) -> Option<Shared> {
    s.cases.get::<Shared>(&key(at)).ok().flatten()
}

pub fn put(s: &Session, at: &str, f: &Shared) -> Result<(), ApiError> {
    s.cases.put(&key(at), f)?;
    Ok(())
}

/// Forget one file: its record and its bytes.
pub fn remove(s: &Session, at: &str) {
    if let Some(f) = get(s, at) {
        let _ = s.cases.delete_blob(&f.blob);
    }
    let _ = s.cases.delete(&key(at));
}

/// Forget every file under a prefix (a whole group or channel).
pub fn remove_all(s: &Session, prefix: &str) {
    for n in s.cases.names(&key(prefix)) {
        if let Some(at) = n.strip_prefix("sfile/") {
            remove(s, at);
        }
    }
}

/// Every shared file on this desk: (where, file).
pub fn all(s: &Session) -> Vec<(String, Shared)> {
    s.cases
        .names("sfile/")
        .into_iter()
        .filter_map(|n| {
            let f = s.cases.get::<Shared>(&n).ok().flatten()?;
            Some((n.strip_prefix("sfile/")?.to_string(), f))
        })
        .collect()
}

fn clean_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    if clean.trim().is_empty() { "file".into() } else { clean }
}

async fn upload_copy(s: &Session, relay: &str, name: &str, mime: &str, bytes: &[u8], reads: u8) -> Result<FileRef, ApiError> {
    let conn = s.engine.relay(relay).await;
    let file_key: [u8; 32] = sotto_core::rand::bytes();
    let up = files::upload(&conn, &file_key, &mut &bytes[..], None, reads.max(1)).await?;
    let chunks = files::chunk_list(&conn, up.chunks, reads.max(1)).await?;
    Ok(FileRef { id: new_id(), name: name.into(), mime: mime.into(), size: up.size, sha256: up.sha256, key: file_key, relay: relay.into(), chunks })
}

/// Keep `bytes` here under `at`, and upload `copies` copies, each readable `reads` times.
pub async fn send(s: &Session, at: &str, name: &str, mime: &str, bytes: &[u8], copies: usize, reads: u8) -> Result<(Shared, Vec<FileRef>), ApiError> {
    if bytes.is_empty() {
        return Err(ApiError::Invalid("That file is empty.".into()));
    }
    if bytes.len() > MAX_SIZE {
        return Err(ApiError::Invalid("That file is over 100 MB. Send it in a 1:1 conversation instead.".into()));
    }
    let name = clean_name(name);
    let mime: String = mime.chars().take(100).collect();
    let relay = s.engine.profile().await.default_relay;
    let mut refs = Vec::with_capacity(copies);
    for _ in 0..copies.max(1) {
        refs.push(upload_copy(s, &relay, &name, &mime, bytes, reads).await?);
    }
    let blob = format!("sblob/{at}");
    s.cases.put_blob(&blob, &mut &bytes[..])?;
    let f = Shared { name, mime, size: bytes.len() as u64, blob, status: Status::Sent, review: Review::Own, remote: None };
    put(s, at, &f)?;
    Ok((f, refs))
}

/// One fresh copy of a file we have, readable once (for a channel follower who joined late).
pub async fn copy_for_one(s: &Session, at: &str) -> Result<FileRef, ApiError> {
    let f = get(s, at).ok_or(ApiError::NotFound)?;
    let mut bytes = Vec::new();
    s.cases.read_blob(&f.blob, &mut bytes)?;
    let relay = s.engine.profile().await.default_relay;
    upload_copy(s, &relay, &f.name, &f.mime, &bytes, 1).await
}

/// Record a received file and fetch it in the background; `event` is emitted when it lands.
pub fn receive(agent: &Arc<Agent>, s: &Arc<Session>, at: &str, file: FileRef, event: Value) -> Result<(), ApiError> {
    if get(s, at).is_some() {
        return Ok(());
    }
    let f = Shared {
        name: clean_name(&file.name),
        mime: file.mime.chars().take(100).collect(),
        size: file.size,
        blob: format!("sblob/{at}"),
        status: Status::Downloading,
        review: Review::Quarantined,
        remote: Some(file),
    };
    put(s, at, &f)?;
    let (agent, s, at) = (agent.clone(), s.clone(), at.to_string());
    tokio::spawn(async move {
        let _ = fetch(&s, &at).await;
        agent.emit(event);
    });
    Ok(())
}

/// Download a received file into this desk's store.
pub async fn fetch(s: &Session, at: &str) -> Result<(), ApiError> {
    let mut f = get(s, at).ok_or(ApiError::NotFound)?;
    let Some(file) = f.remote.clone() else { return Ok(()) };
    let conn = s.engine.relay(&file.relay).await;
    let result = async {
        let mut w = s.cases.blob_writer(&f.blob)?;
        files::download(&conn, &file.key, &file.chunks, file.size, &file.sha256, &mut w).await?;
        w.finish()?;
        Ok::<_, ApiError>(())
    }
    .await;
    f.status = if result.is_ok() { Status::Ready } else { Status::Failed };
    if result.is_ok() {
        // The file key isn't needed once the file is stored.
        f.remote = None;
    }
    put(s, at, &f)?;
    result
}

/// The same shape as a 1:1 file, so the conversation's file card renders it.
pub fn view(f: &Shared, caption: &str) -> Value {
    let status = match f.status {
        Status::Sent => "sent",
        Status::Downloading => "downloading",
        Status::Ready => "ready",
        Status::Failed => "failed",
    };
    json!({ "name": f.name, "mime": f.mime, "size": f.size, "status": status, "review": f.review.view(), "caption": caption })
}
