//! Journalist side of source drops: keys, sync, replies.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sotto_client::{Review, Transport};
use sotto_core::seal::{SealPublic, SealSecret};
use sotto_core::{hex, kdf};
use sotto_drop::proto::{self, SubmissionKind};
use sotto_drop::{SourceSummary, SubmissionSummary};
use sotto_vault::Store;

use crate::ApiError;
use crate::state::{Agent, Session};

#[derive(Clone, Serialize, Deserialize)]
pub struct DropAccount {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sign_seed: [u8; 32],
    pub newsroom: SealSecret,
    pub created_at: u64,
    pub last_sync: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DropSource {
    pub drop: String,
    pub id: String,
    pub pseudonym: String,
    pub first_day: u32,
    pub last_day: u32,
    pub unread: u32,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum ItemKind {
    Text(String),
    File { name: String, mime: String, size: u64 },
    Reply(String),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DropItem {
    pub name: String,
    pub day: u32,
    pub kind: ItemKind,
    pub blob: Option<String>,
    pub review: Review,
    pub received_at: u64,
}

#[derive(Serialize)]
pub struct DropView {
    pub id: String,
    pub name: String,
    pub url: String,
    pub newsroom_key: String,
    pub journalist_key: String,
    pub last_sync: u64,
    pub sources: usize,
    pub unread: u32,
}

#[derive(Serialize)]
pub struct SourceView {
    pub id: String,
    pub pseudonym: String,
    pub first_day: u32,
    pub last_day: u32,
    pub unread: u32,
}

#[derive(Serialize)]
pub struct ItemView {
    pub name: String,
    pub day: u32,
    pub kind: &'static str,
    pub text: Option<String>,
    pub file_name: Option<String>,
    pub mime: Option<String>,
    pub size: Option<u64>,
    pub review: sotto_client::ReviewView,
    pub received_at: u64,
    /// Key for the safe-review endpoints, when this item is a file.
    pub key: Option<String>,
}

impl DropItem {
    pub fn view(&self) -> ItemView {
        let (kind, text, file_name, mime, size) = match &self.kind {
            ItemKind::Text(t) => ("text", Some(t.clone()), None, None, None),
            ItemKind::Reply(t) => ("reply", Some(t.clone()), None, None, None),
            ItemKind::File { name, mime, size } => ("file", None, Some(name.clone()), Some(mime.clone()), Some(*size)),
        };
        ItemView { name: self.name.clone(), day: self.day, kind, text, file_name, mime, size, review: self.review.view(), received_at: self.received_at, key: self.blob.as_ref().map(|b| format!("d:{b}")) }
    }
}

/// "Sender QV4": stable, short, and meaningless outside this device.
pub fn pseudonym(source_id: &str) -> String {
    let d = kdf::sha256(source_id.as_bytes());
    const L: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
    format!("Sender {}{}{}", L[d[0] as usize % L.len()] as char, L[d[1] as usize % L.len()] as char, (d[2] % 9) + 1)
}

fn account_name(id: &str) -> String {
    format!("drop/{id}")
}

pub fn accounts(store: &Store) -> Result<Vec<DropAccount>, ApiError> {
    let mut v = Vec::new();
    for n in store.names("drop/") {
        if let Some(a) = store.get::<DropAccount>(&n)? {
            v.push(a);
        }
    }
    Ok(v)
}

pub fn account(store: &Store, id: &str) -> Result<DropAccount, ApiError> {
    store.get::<DropAccount>(&account_name(id))?.ok_or(ApiError::NotFound)
}

pub fn view(store: &Store, a: &DropAccount) -> Result<DropView, ApiError> {
    let sources = sources(store, &a.id)?;
    Ok(DropView {
        id: a.id.clone(),
        name: a.name.clone(),
        url: a.url.clone(),
        newsroom_key: proto::encode_key(&a.newsroom.public())?,
        journalist_key: hex(&sotto_core::keys::signing_public(&a.sign_seed)),
        last_sync: a.last_sync,
        unread: sources.iter().map(|s| s.unread).sum(),
        sources: sources.len(),
    })
}

pub fn sources(store: &Store, drop: &str) -> Result<Vec<DropSource>, ApiError> {
    let mut v = Vec::new();
    for n in store.names(&format!("dropsrc/{drop}/")) {
        if let Some(s) = store.get::<DropSource>(&n)? {
            v.push(s);
        }
    }
    v.sort_by(|a, b| b.last_day.cmp(&a.last_day));
    Ok(v)
}

pub fn items(store: &Store, drop: &str, source: &str) -> Result<Vec<DropItem>, ApiError> {
    let mut v = Vec::new();
    for n in store.names(&format!("dropitem/{drop}/{source}/")) {
        if let Some(i) = store.get::<DropItem>(&n)? {
            v.push(i);
        }
    }
    v.sort_by(|a, b| (a.day, a.received_at, &a.name).cmp(&(b.day, b.received_at, &b.name)));
    Ok(v)
}

pub fn item_key(drop: &str, source: &str, name: &str) -> String {
    format!("dropitem/{drop}/{source}/{name}")
}

/// Create a drop account. An existing newsroom key can be imported so colleagues share one.
pub fn create(store: &Store, name: &str, url: &str, newsroom: Option<SealSecret>) -> Result<DropAccount, ApiError> {
    let url = url.trim().trim_end_matches('/').to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(ApiError::Invalid("The inbox address should start with http:// (onion addresses use http).".into()));
    }
    let a = DropAccount {
        id: hex(&sotto_core::rand::bytes::<8>()),
        name: name.trim().chars().take(80).collect(),
        url,
        sign_seed: sotto_core::rand::bytes(),
        newsroom: newsroom.unwrap_or_else(SealSecret::generate),
        created_at: sotto_core::time::now(),
        last_sync: 0,
    };
    store.put(&account_name(&a.id), &a)?;
    Ok(a)
}

fn client(transport: &Transport) -> Result<reqwest::Client, ApiError> {
    let mut b = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300)).user_agent("sotto");
    if let Transport::Tor { socks } = transport {
        // socks5h: the proxy resolves names, so .onion addresses work and nothing leaks to local DNS.
        b = b.proxy(reqwest::Proxy::all(format!("socks5h://{socks}")).map_err(|_| ApiError::Invalid("Bad Tor address.".into()))?);
    }
    b.build().map_err(|_| ApiError::Internal)
}

async fn call(c: &reqwest::Client, a: &DropAccount, method: reqwest::Method, path: &str, body: Vec<u8>) -> Result<Vec<u8>, ApiError> {
    let auth = proto::auth_header(&a.sign_seed, method.as_str(), path, &body);
    let res = c
        .request(method, format!("{}{}", a.url, path))
        .header("authorization", auth)
        .body(body)
        .send()
        .await
        .map_err(|_| ApiError::Unavailable("The inbox server can't be reached. Check that Tor is running and the address is right.".into()))?;
    match res.status().as_u16() {
        200..=299 => Ok(res.bytes().await.map_err(|_| ApiError::Unavailable("The connection to the inbox server dropped.".into()))?.to_vec()),
        401 => Err(ApiError::Invalid("The inbox server doesn't recognise this device. Add this device's key to its configuration.".into())),
        404 => Err(ApiError::NotFound),
        _ => Err(ApiError::Unavailable("The inbox server returned an error.".into())),
    }
}

/// Download new submissions from a drop into the vault. Returns how many were added.
pub async fn sync(agent: &Arc<Agent>, s: &Arc<Session>, drop_id: &str) -> Result<usize, ApiError> {
    let mut a = account(&s.drops, drop_id)?;
    let transport = s.engine.profile().await.transport;
    let c = client(&transport)?;
    let delete_after = s.settings.lock().await.delete_after_download;
    let list: Vec<SourceSummary> = serde_json::from_slice(&call(&c, &a, reqwest::Method::GET, "/j/v1/sources", vec![]).await?).map_err(|_| ApiError::Internal)?;
    let mut added = 0;
    for src in list {
        if src.id.len() != 32 || !src.id.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let subs_path = format!("/j/v1/sources/{}/submissions", src.id);
        let subs: Vec<SubmissionSummary> = serde_json::from_slice(&call(&c, &a, reqwest::Method::GET, &subs_path, vec![]).await?).map_err(|_| ApiError::Internal)?;
        let src_key = format!("dropsrc/{}/{}", a.id, src.id);
        let mut local = s.drops.get::<DropSource>(&src_key)?.unwrap_or(DropSource {
            drop: a.id.clone(),
            id: src.id.clone(),
            pseudonym: pseudonym(&src.id),
            first_day: src.last_day,
            last_day: src.last_day,
            unread: 0,
        });
        for sub in subs {
            let key = item_key(&a.id, &src.id, &sub.name);
            if s.drops.get::<DropItem>(&key)?.is_some() {
                continue;
            }
            let path = format!("{subs_path}/{}", sub.name);
            let sealed = call(&c, &a, reqwest::Method::GET, &path, vec![]).await?;
            let newsroom = a.newsroom.clone();
            let plain = tokio::task::spawn_blocking(move || sotto_core::seal::open(&newsroom, &sealed)).await.map_err(|_| ApiError::Internal)?;
            let Ok(plain) = plain else { continue };
            let plain = zeroize::Zeroizing::new(plain);
            let Ok((meta, content)) = proto::parse_submission(&plain) else { continue };
            let (kind, blob, review) = match meta.kind {
                SubmissionKind::Message => (ItemKind::Text(String::from_utf8_lossy(content).chars().take(200_000).collect()), None, Review::Own),
                SubmissionKind::File { name, mime } => {
                    let blob = format!("dropfile/{}", hex(&sotto_core::rand::bytes::<16>()));
                    s.drops.put_blob(&blob, &mut &content[..])?;
                    (ItemKind::File { name, mime, size: content.len() as u64 }, Some(blob), Review::Quarantined)
                }
            };
            s.drops.put(&key, &DropItem { name: sub.name.clone(), day: meta.day, kind, blob, review, received_at: sotto_core::time::now() })?;
            local.unread += 1;
            local.last_day = local.last_day.max(meta.day);
            local.first_day = local.first_day.min(meta.day);
            added += 1;
            if delete_after {
                let _ = call(&c, &a, reqwest::Method::DELETE, &path, vec![]).await;
            }
        }
        s.drops.put(&src_key, &local)?;
    }
    a.last_sync = sotto_core::time::now();
    s.drops.put(&account_name(&a.id), &a)?;
    agent.emit(json!({ "type": "drop", "id": a.id, "added": added }));
    Ok(added)
}

/// Send a reply a source will see next time they log in with their codename.
pub async fn reply(agent: &Arc<Agent>, s: &Arc<Session>, drop_id: &str, source: &str, text: &str) -> Result<(), ApiError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ApiError::Invalid("Write a reply first.".into()));
    }
    if text.len() > 50_000 {
        return Err(ApiError::Invalid("That reply is too long.".into()));
    }
    let a = account(&s.drops, drop_id)?;
    let transport = s.engine.profile().await.transport;
    let c = client(&transport)?;
    let key = call(&c, &a, reqwest::Method::GET, &format!("/j/v1/sources/{source}/key"), vec![]).await?;
    let public: SealPublic = postcard::from_bytes(&key).map_err(|_| ApiError::Internal)?;
    // The source id is the hash of their key: a server that swaps the key would be caught here.
    if proto::source_id(&public) != source {
        return Err(ApiError::Unavailable("The inbox server returned the wrong key for this sender. Nothing was sent.".into()));
    }
    let reply = proto::Reply { day: proto::today(), text: text.to_string() };
    let sealed = sotto_core::seal::seal(&public, &postcard::to_stdvec(&reply).map_err(|_| ApiError::Internal)?)?;
    call(&c, &a, reqwest::Method::POST, &format!("/j/v1/sources/{source}/replies"), sealed).await?;
    let name = format!("r{}-{}", proto::today(), hex(&sotto_core::rand::bytes::<6>()));
    s.drops.put(
        &item_key(&a.id, source, &name),
        &DropItem { name, day: proto::today(), kind: ItemKind::Reply(text.to_string()), blob: None, review: Review::Own, received_at: sotto_core::time::now() },
    )?;
    agent.emit(json!({ "type": "drop", "id": a.id, "added": 0 }));
    Ok(())
}

pub fn mark_read(store: &Store, drop: &str, source: &str) -> Result<(), ApiError> {
    let key = format!("dropsrc/{drop}/{source}");
    if let Some(mut s) = store.get::<DropSource>(&key)? {
        s.unread = 0;
        store.put(&key, &s)?;
    }
    Ok(())
}

/// Forget a source locally (files included). Does not touch the server.
pub fn forget_source(store: &Store, drop: &str, source: &str) -> Result<(), ApiError> {
    for i in items(store, drop, source)? {
        if let Some(b) = &i.blob {
            store.delete_blob(b)?;
        }
        if let Review::Clean { clean_blob, .. } = &i.review {
            store.delete_blob(clean_blob)?;
        }
        store.delete(&item_key(drop, source, &i.name))?;
    }
    store.delete(&format!("dropsrc/{drop}/{source}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn pseudonyms_are_stable_and_short() {
        let a = super::pseudonym("0123456789abcdef0123456789abcdef");
        assert_eq!(a, super::pseudonym("0123456789abcdef0123456789abcdef"));
        assert!(a.starts_with("Sender ") && a.len() == 10);
    }
}
