//! Cases: an investigation's encrypted team space (MLS group over an ordered relay queue),
//! plus the insurance file. See `docs/protocol.md` sections 10 and 11.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::{Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_cases::{Mls, MlsState, Processed};
use sotto_client::conn::Push;
use sotto_core::envelope::Payload;
use sotto_core::keys::{sign, signing_keypair};
use sotto_core::wire::{BLOB_SIZE, Cmd, FILE_CHUNK, Resp, auth_message};
use sotto_core::{Id, hex, kdf, new_id, pad, time, unhex};
use tokio::sync::mpsc;

use crate::ApiError;
use crate::state::{Agent, Session};

#[derive(Default)]
pub struct Runtime {
    pub(crate) mls: Option<Mls>,
    pub(crate) seqs: HashMap<Id, Arc<AtomicU64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Editor,
    Reporter,
    Lawyer,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Member {
    pub name: String,
    pub contact_id: Option<Id>,
    pub role: Role,
    pub pending: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Insurance {
    seed: [u8; 32],
    tl_id: Id,
    read_id: Id,
    relay: String,
    key: [u8; 32],
    chunks: u32,
    deadline: u64,
    interval_days: u32,
    recipients: Vec<Id>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CaseRecord {
    id: Id,
    title: String,
    group_id: Vec<u8>,
    relay: String,
    read_id: Id,
    write_id: Id,
    next_seq: u64,
    own_seqs: Vec<u64>,
    owner: bool,
    members: Vec<Member>,
    key: [u8; 32],
    created_at: u64,
    destroy_at: Option<u64>,
    legal_hold: bool,
    insurance: Option<Insurance>,
    removed: bool,
    /// The owner's MLS signature key. Control messages and commits from anyone else are ignored.
    owner_key: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
enum EntryKind {
    Note(String),
    Source { alias: String, note: String },
    Audit(String),
    System(String),
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: Id,
    author: String,
    at: u64,
    kind: EntryKind,
}

/// Application messages inside the MLS group.
#[derive(Serialize, Deserialize)]
enum CaseMsg {
    Note { id: Id, text: String },
    Source { id: Id, alias: String, note: String },
    Audit { id: Id, action: String },
    Members(Vec<(String, Role)>),
    Retention { destroy_at: Option<u64>, legal_hold: bool },
    Closed,
}

/// A colleague's insurance file that this desk holds the key for.
#[derive(Clone, Serialize, Deserialize)]
struct Grant {
    id: Id,
    from: Id,
    case_title: String,
    relay: String,
    read_id: Id,
    key: [u8; 32],
    deadline: u64,
    chunks: u32,
    received_at: u64,
}

#[derive(Serialize, Deserialize)]
struct Package {
    title: String,
    exported_at: u64,
    entries: Vec<Entry>,
}

fn case_key(id: &Id) -> String {
    format!("case/{}", hex(id))
}

fn records(s: &Session) -> Vec<CaseRecord> {
    s.cases.names("case/").iter().filter_map(|n| s.cases.get::<CaseRecord>(n).ok().flatten()).collect()
}

fn record(s: &Session, id: &Id) -> Result<CaseRecord, ApiError> {
    s.cases.get(&case_key(id))?.ok_or(ApiError::NotFound)
}

fn save(s: &Session, c: &CaseRecord) -> Result<(), ApiError> {
    s.cases.put(&case_key(&c.id), c)?;
    Ok(())
}

fn entries_store(s: &Session, c: &CaseRecord) -> Result<sotto_vault::Store, ApiError> {
    Ok(s.vault.try_lock().map_err(|_| ApiError::Unavailable("Busy, try again.".into()))?.scope(c.key)?)
}

fn add_entry(s: &Session, c: &CaseRecord, author: &str, kind: EntryKind, id: Option<Id>) -> Result<(), ApiError> {
    let store = entries_store(s, c)?;
    let id = id.unwrap_or_else(new_id);
    let at = time::now_ms();
    store.put(&format!("entry/{at:020}-{}", hex(&id)), &Entry { id, author: author.chars().take(80).collect(), at, kind })?;
    Ok(())
}

pub(crate) async fn my_name(s: &Session) -> String {
    let n = s.engine.profile().await.display_name;
    if n.trim().is_empty() { "This device".into() } else { n }
}

/// Load (or create) the desk's MLS client.
pub(crate) async fn ensure_mls(s: &Session, rt: &mut Runtime) -> Result<(), ApiError> {
    if rt.mls.is_some() {
        return Ok(());
    }
    let mls = match s.cases.get::<MlsState>("mls")? {
        Some(state) => Mls::restore(&state).map_err(|_| ApiError::Internal)?,
        None => {
            let m = Mls::new(my_name(s).await.as_bytes()).map_err(|_| ApiError::Internal)?;
            s.cases.put("mls", &m.snapshot())?;
            m
        }
    };
    rt.mls = Some(mls);
    Ok(())
}

pub(crate) fn persist_mls(s: &Session, rt: &Runtime) -> Result<(), ApiError> {
    if let Some(m) = &rt.mls {
        s.cases.put("mls", &m.snapshot())?;
    }
    Ok(())
}

async fn gsend(s: &Session, c: &mut CaseRecord, bytes: Vec<u8>) -> Result<(), ApiError> {
    let blob = pad::pad(&bytes, BLOB_SIZE).map_err(|_| ApiError::Invalid("That's too long for one project message.".into()))?;
    let write_id = c.write_id;
    match s.engine.relay(&c.relay).await.request(move |_| Cmd::GSend { write_id, blob }).await? {
        Resp::GSeq { seq } => {
            c.own_seqs.push(seq);
            if c.own_seqs.len() > 500 {
                c.own_seqs.drain(..100);
            }
            Ok(())
        }
        _ => Err(ApiError::Internal),
    }
}

async fn broadcast(s: &Session, rt: &Runtime, c: &mut CaseRecord, msg: &CaseMsg) -> Result<(), ApiError> {
    let mls = rt.mls.as_ref().ok_or(ApiError::Internal)?;
    let data = mls.encrypt(&c.group_id, &postcard::to_stdvec(msg).map_err(|_| ApiError::Internal)?).map_err(|_| ApiError::Internal)?;
    gsend(s, c, data).await
}

async fn subscribe(s: &Session, rt: &mut Runtime, c: &CaseRecord) {
    let next = rt.seqs.entry(c.read_id).or_insert_with(|| Arc::new(AtomicU64::new(c.next_seq))).clone();
    s.engine.relay(&c.relay).await.subscribe_group(c.read_id, next).await;
}

/// Called after unlock: route group pushes here, subscribe every case, check in insurance files.
pub async fn start(agent: &Arc<Agent>, s: &Arc<Session>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Push>();
    s.engine.set_group_sink(tx).await;
    {
        let mut rt = s.cases_rt.lock().await;
        for c in records(s) {
            if !c.removed {
                subscribe(s, &mut rt, &c).await;
            }
        }
    }
    crate::groups::start(agent, s).await;
    crate::secrets::start(s);
    crate::tags::start(agent, s);
    crate::updates::start(agent);
    let (a, sess) = (agent.clone(), s.clone());
    tokio::spawn(async move {
        while let Some(p) = rx.recv().await {
            if let Push::GMsg { read_id, seq, blob, .. } = p {
                let _ = on_group(&a, &sess, read_id, seq, blob).await;
            }
        }
    });
    // Unlocking proves the journalist still holds this device and passcode: extend insurance deadlines.
    let (a, sess) = (agent.clone(), s.clone());
    tokio::spawn(async move {
        for c in records(&sess) {
            if c.insurance.is_some() {
                let _ = check_in(&a, &sess, &c.id).await;
            }
        }
        // Retention: destroy cases whose time has come (unless on legal hold).
        loop {
            for c in records(&sess) {
                if let Some(t) = c.destroy_at
                    && !c.legal_hold && t <= time::now() {
                        let _ = destroy_local(&a, &sess, &c.id).await;
                    }
            }
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        }
    });
}

async fn on_group(agent: &Arc<Agent>, s: &Arc<Session>, read_id: Id, seq: u64, blob: Vec<u8>) -> Result<(), ApiError> {
    let mut rt = s.cases_rt.lock().await;
    let Some(mut c) = records(s).into_iter().find(|c| c.read_id == read_id) else {
        // Not a case: maybe a group chat.
        drop(rt);
        return crate::groups::on_group(agent, s, read_id, seq, blob).await;
    };
    if seq < c.next_seq {
        return Ok(());
    }
    c.next_seq = seq + 1;
    if let Some(n) = rt.seqs.get(&read_id) {
        n.store(c.next_seq, Ordering::SeqCst);
    }
    if c.own_seqs.contains(&seq) {
        save(s, &c)?;
        return Ok(());
    }
    ensure_mls(s, &mut rt).await?;
    let committer = (!c.owner_key.is_empty()).then_some(c.owner_key.as_slice());
    let processed = pad::unpad(&blob).ok().and_then(|b| rt.mls.as_ref()?.process(&c.group_id, b, committer).ok());
    match processed {
        Some(Processed::Application { sender, sender_key, data }) => {
            let author = String::from_utf8_lossy(&sender).to_string();
            let from_owner = !c.owner_key.is_empty() && sender_key == c.owner_key;
            match postcard::from_bytes::<CaseMsg>(&data) {
                Ok(CaseMsg::Note { id, text }) => add_entry(s, &c, &author, EntryKind::Note(text.chars().take(20_000).collect()), Some(id))?,
                Ok(CaseMsg::Source { id, alias, note }) => add_entry(s, &c, &author, EntryKind::Source { alias: alias.chars().take(80).collect(), note: note.chars().take(5000).collect() }, Some(id))?,
                Ok(CaseMsg::Audit { id, action }) => add_entry(s, &c, &author, EntryKind::Audit(action.chars().take(300).collect()), Some(id))?,
                Ok(CaseMsg::Members(list)) if !c.owner && from_owner => {
                    c.members = list.into_iter().take(100).map(|(name, role)| Member { name, contact_id: None, role, pending: false }).collect();
                }
                Ok(CaseMsg::Retention { destroy_at, legal_hold }) if !c.owner && from_owner => {
                    c.destroy_at = destroy_at;
                    c.legal_hold = legal_hold;
                    add_entry(s, &c, &author, EntryKind::System(retention_text(destroy_at, legal_hold)), None)?;
                }
                Ok(CaseMsg::Closed) if !c.owner && from_owner => {
                    persist_mls(s, &rt)?;
                    drop(rt);
                    return destroy_local(agent, s, &c.id).await;
                }
                _ => {}
            }
        }
        Some(Processed::Commit { added, removed }) => {
            for a in added {
                add_entry(s, &c, "Sotto", EntryKind::System(format!("{} joined the project.", String::from_utf8_lossy(&a))), None)?;
            }
            for r in removed {
                add_entry(s, &c, "Sotto", EntryKind::System(format!("{} was removed from the project.", String::from_utf8_lossy(&r))), None)?;
            }
            if !rt.mls.as_ref().map(|m| m.is_active(&c.group_id)).unwrap_or(false) {
                c.removed = true;
                add_entry(s, &c, "Sotto", EntryKind::System("You were removed from this project. You can still read what you had.".into()), None)?;
            }
        }
        _ => {}
    }
    persist_mls(s, &rt)?;
    save(s, &c)?;
    agent.emit(json!({ "type": "case", "id": hex(&c.id) }));
    Ok(())
}

/// Case payloads that arrive over a 1:1 conversation.
pub async fn on_extension(agent: &Arc<Agent>, contact: Id, payload: Payload) {
    let Ok(s) = agent.session().await else { return };
    let _ = handle_extension(agent, &s, contact, payload).await;
}

async fn handle_extension(agent: &Arc<Agent>, s: &Arc<Session>, contact: Id, payload: Payload) -> Result<(), ApiError> {
    match payload {
        Payload::KeyPackageRequest { case_id } => {
            let kp = {
                let mut rt = s.cases_rt.lock().await;
                ensure_mls(s, &mut rt).await?;
                let kp = rt.mls.as_ref().ok_or(ApiError::Internal)?.key_package().map_err(|_| ApiError::Internal)?;
                persist_mls(s, &rt)?;
                kp
            };
            s.engine.send_payload(&contact, &Payload::KeyPackage { case_id, data: kp }).await?;
        }
        Payload::KeyPackage { case_id, data } => {
            if s.cases.get::<CaseRecord>(&case_key(&case_id))?.is_none() {
                return crate::groups::on_key_package(agent, s, contact, case_id, data).await;
            }
            let mut rt = s.cases_rt.lock().await;
            let mut c = record(s, &case_id)?;
            let Some(i) = c.members.iter().position(|m| m.pending && m.contact_id == Some(contact)) else { return Ok(()) };
            if !c.owner {
                return Ok(());
            }
            ensure_mls(s, &mut rt).await?;
            let (commit, welcome) = rt.mls.as_ref().ok_or(ApiError::Internal)?.add_member(&c.group_id, &data).map_err(|_| ApiError::Invalid("Their key package was invalid.".into()))?;
            persist_mls(s, &rt)?;
            gsend(s, &mut c, commit).await?;
            c.members[i].pending = false;
            let owner_key = rt.mls.as_ref().ok_or(ApiError::Internal)?.signature_key();
            let invite = Payload::CaseInvite { case_id: c.id, title: c.title.clone(), welcome, relay: c.relay.clone(), read_id: c.read_id, write_id: c.write_id, owner_key };
            s.engine.send_payload(&contact, &invite).await?;
            let list = c.members.iter().filter(|m| !m.pending).map(|m| (m.name.clone(), m.role)).collect();
            broadcast(s, &rt, &mut c, &CaseMsg::Members(list)).await?;
            persist_mls(s, &rt)?;
            save(s, &c)?;
            agent.emit(json!({ "type": "case", "id": hex(&c.id) }));
        }
        Payload::CaseInvite { case_id, title, welcome, relay, read_id, write_id, owner_key } => {
            s.engine.check_remote_relay(&relay)?;
            if owner_key.is_empty() {
                return Ok(());
            }
            let mut rt = s.cases_rt.lock().await;
            ensure_mls(s, &mut rt).await?;
            let group_id = rt.mls.as_ref().ok_or(ApiError::Internal)?.join(&welcome).map_err(|_| ApiError::Invalid("The project invitation couldn't be opened.".into()))?;
            persist_mls(s, &rt)?;
            let c = CaseRecord {
                id: case_id,
                title: title.chars().take(120).collect(),
                group_id,
                relay,
                read_id,
                write_id,
                next_seq: 0,
                own_seqs: vec![],
                owner: false,
                members: vec![],
                key: sotto_core::rand::bytes(),
                created_at: time::now(),
                destroy_at: None,
                legal_hold: false,
                insurance: None,
                removed: false,
                owner_key,
            };
            save(s, &c)?;
            add_entry(s, &c, "Sotto", EntryKind::System("You joined this project.".into()), None)?;
            subscribe(s, &mut rt, &c).await;
            agent.emit(json!({ "type": "case", "id": hex(&c.id) }));
        }
        Payload::GroupInvite { group_id, title, welcome, relay, read_id, write_id, owner_key } => {
            crate::groups::on_invite(agent, s, contact, group_id, title, welcome, relay, read_id, write_id, owner_key).await?;
        }
        Payload::Channel(signal) => crate::channels::on_signal(agent, s, contact, signal).await?,
        p @ (Payload::Secret { .. } | Payload::SecretOpened { .. }) => crate::secrets::on_payload(agent, s, contact, p)?,
        Payload::Avatar { id } => crate::avatars::on_payload(agent, s, contact, id)?,
        Payload::InsuranceGrant { case_title, relay, read_id, deadline, key, chunks } => {
            s.engine.check_remote_relay(&relay)?;
            let g = Grant { id: new_id(), from: contact, case_title: case_title.chars().take(120).collect(), relay, read_id, key, deadline, chunks: chunks.min(4096), received_at: time::now() };
            s.cases.put(&format!("grant/{}", hex(&read_id)), &g)?;
            agent.emit(json!({ "type": "case" }));
        }
        Payload::InsuranceUpdate { read_id, deadline } => {
            let k = format!("grant/{}", hex(&read_id));
            if let Some(mut g) = s.cases.get::<Grant>(&k)?
                && g.from == contact {
                    g.deadline = deadline;
                    s.cases.put(&k, &g)?;
                    agent.emit(json!({ "type": "case" }));
                }
        }
        _ => {}
    }
    Ok(())
}

fn retention_text(destroy_at: Option<u64>, hold: bool) -> String {
    match (destroy_at, hold) {
        (_, true) => "Legal hold is on: nothing in this project will be destroyed automatically.".into(),
        (Some(_), false) => "A destroy date was set for this project.".into(),
        (None, false) => "Automatic destruction was turned off for this project.".into(),
    }
}

async fn destroy_local(agent: &Arc<Agent>, s: &Session, id: &Id) -> Result<(), ApiError> {
    let mut rt = s.cases_rt.lock().await;
    let c = record(s, id)?;
    if let Ok(store) = entries_store(s, &c) {
        store.destroy(&[])?;
    }
    if let Some(m) = &rt.mls {
        let _ = m.forget(&c.group_id);
    }
    s.engine.relay(&c.relay).await.unsubscribe_group(&c.read_id).await;
    rt.seqs.remove(&c.read_id);
    if let Some(ins) = &c.insurance {
        let (tl_id, seed) = (ins.tl_id, ins.seed);
        let _ = s.engine.relay(&ins.relay).await.request(move |n| Cmd::TDel { tl_id, sig: sign(&seed, &auth_message(n, "tdel", &[&tl_id])) }).await;
    }
    s.cases.delete(&case_key(id))?;
    persist_mls(s, &rt)?;
    agent.emit(json!({ "type": "case", "id": hex(id) }));
    Ok(())
}

// ---------- Insurance ----------

async fn setup_insurance(agent: &Arc<Agent>, s: &Arc<Session>, id: &Id, interval_days: u32, recipients: Vec<Id>) -> Result<(), ApiError> {
    let mut c = record(s, id)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the project owner can set up an insurance file.".into()));
    }
    if recipients.is_empty() {
        return Err(ApiError::Invalid("Choose at least one colleague to hold it.".into()));
    }
    let interval_days = interval_days.clamp(1, 90);
    let package = Package { title: c.title.clone(), exported_at: time::now(), entries: list_entries(s, &c)? };
    let bytes = postcard::to_stdvec(&package).map_err(|_| ApiError::Internal)?;
    let key: [u8; 32] = sotto_core::rand::bytes();
    let (seed, owner) = signing_keypair();
    let deadline = time::now() + interval_days as u64 * 86_400;
    let relay = s.engine.profile().await.default_relay;
    let conn = s.engine.relay(&relay).await;
    let (tl_id, read_id) = match conn.request(move |_| Cmd::TNew { owner, deadline }).await? {
        Resp::TIds { tl_id, read_id } => (tl_id, read_id),
        _ => return Err(ApiError::Unavailable("This relay doesn't keep insurance files.".into())),
    };
    let mut chunks = 0u32;
    for (i, piece) in bytes.chunks(FILE_CHUNK).enumerate() {
        let blob = sotto_client::files::encrypt_chunk(&key, i as u32, piece).map_err(|_| ApiError::Internal)?;
        let digest = kdf::sha256(&blob);
        let index = i as u32;
        conn.request(move |n| Cmd::TPut { tl_id, index, sig: sign(&seed, &auth_message(n, "tput", &[&tl_id, &index.to_le_bytes(), &digest])), blob }).await?;
        chunks += 1;
    }
    for r in &recipients {
        s.engine.send_payload(r, &Payload::InsuranceGrant { case_title: c.title.clone(), relay: relay.clone(), read_id, deadline, key, chunks }).await?;
    }
    c.insurance = Some(Insurance { seed, tl_id, read_id, relay, key, chunks, deadline, interval_days, recipients });
    save(s, &c)?;
    let _ = audit(s, &mut c, "set up an insurance file").await;
    agent.emit(json!({ "type": "case", "id": hex(id) }));
    Ok(())
}

async fn check_in(agent: &Arc<Agent>, s: &Session, id: &Id) -> Result<u64, ApiError> {
    let mut c = record(s, id)?;
    let ins = c.insurance.as_mut().ok_or(ApiError::NotFound)?;
    let deadline = time::now() + ins.interval_days as u64 * 86_400;
    let (tl_id, seed) = (ins.tl_id, ins.seed);
    s.engine.relay(&ins.relay).await.request(move |n| Cmd::TCheckIn { tl_id, deadline, sig: sign(&seed, &auth_message(n, "tcheckin", &[&tl_id, &deadline.to_le_bytes()])) }).await?;
    ins.deadline = deadline;
    let (read_id, recipients) = (ins.read_id, ins.recipients.clone());
    save(s, &c)?;
    for r in recipients {
        let _ = s.engine.send_payload(&r, &Payload::InsuranceUpdate { read_id, deadline }).await;
    }
    agent.emit(json!({ "type": "case", "id": hex(id) }));
    Ok(deadline)
}

async fn open_grant(s: &Session, read_id: &Id) -> Result<Value, ApiError> {
    let g: Grant = s.cases.get(&format!("grant/{}", hex(read_id)))?.ok_or(ApiError::NotFound)?;
    let conn = s.engine.relay(&g.relay).await;
    let mut bytes = Vec::new();
    for index in 0..g.chunks {
        let rid = g.read_id;
        match conn.request(move |_| Cmd::TGet { read_id: rid, index }).await? {
            Resp::Locked { deadline } => return Ok(json!({ "locked": true, "deadline": deadline })),
            Resp::TData { blob, .. } => bytes.extend_from_slice(&sotto_client::files::decrypt_chunk(&g.key, index, &blob).map_err(|_| ApiError::Internal)?),
            _ => return Err(ApiError::Internal),
        }
    }
    let p: Package = postcard::from_bytes(&bytes).map_err(|_| ApiError::Internal)?;
    Ok(json!({ "locked": false, "title": p.title, "exported_at": p.exported_at, "entries": p.entries.iter().map(entry_json).collect::<Vec<_>>() }))
}

// ---------- Views and routes ----------

fn list_entries(s: &Session, c: &CaseRecord) -> Result<Vec<Entry>, ApiError> {
    let store = entries_store(s, c)?;
    Ok(store.names("entry/").iter().filter_map(|n| store.get::<Entry>(n).ok().flatten()).collect())
}

fn entry_json(e: &Entry) -> Value {
    let (kind, text, alias) = match &e.kind {
        EntryKind::Note(t) => ("note", t.clone(), None),
        EntryKind::Source { alias, note } => ("source", note.clone(), Some(alias.clone())),
        EntryKind::Audit(t) => ("audit", t.clone(), None),
        EntryKind::System(t) => ("system", t.clone(), None),
    };
    json!({ "id": hex(&e.id), "author": e.author, "at": e.at, "kind": kind, "text": text, "alias": alias })
}

fn case_json(c: &CaseRecord) -> Value {
    json!({
        "id": hex(&c.id), "title": c.title, "owner": c.owner, "removed": c.removed, "created_at": c.created_at,
        "destroy_at": c.destroy_at, "legal_hold": c.legal_hold,
        "members": c.members.iter().map(|m| json!({ "name": m.name, "role": m.role, "pending": m.pending })).collect::<Vec<_>>(),
        "insurance": c.insurance.as_ref().map(|i| json!({ "deadline": i.deadline, "interval_days": i.interval_days, "recipients": i.recipients.len() })),
    })
}

async fn audit(s: &Session, c: &mut CaseRecord, action: &str) -> Result<(), ApiError> {
    let me = my_name(s).await;
    let id = new_id();
    add_entry(s, c, &me, EntryKind::Audit(action.into()), Some(id))?;
    let rt = s.cases_rt.lock().await;
    broadcast(s, &rt, c, &CaseMsg::Audit { id, action: action.into() }).await
}

type A = State<Arc<Agent>>;
type R = Result<Json<Value>, ApiError>;

fn cid(s: &str) -> Result<Id, ApiError> {
    unhex::<16>(s).map_err(|_| ApiError::NotFound)
}

pub fn routes() -> Router<Arc<Agent>> {
    Router::new()
        .route("/api/cases", get(list).post(create))
        .route("/api/cases/{id}", get(show).patch(update).delete(destroy))
        .route("/api/cases/{id}/notes", post(note))
        .route("/api/cases/{id}/sources", post(source))
        .route("/api/cases/{id}/members", post(invite))
        .route("/api/cases/{id}/members/{name}", delete(remove))
        .route("/api/cases/{id}/insurance", post(insurance).delete(stop_insurance))
        .route("/api/cases/{id}/insurance/checkin", post(checkin))
        .route("/api/grants", get(grants))
        .route("/api/grants/{read_id}/open", post(open))
}

async fn list(State(agent): A) -> R {
    let s = agent.session().await?;
    let mut v: Vec<CaseRecord> = records(&s);
    v.sort_by_key(|x| std::cmp::Reverse(x.created_at));
    Ok(Json(json!(v.iter().map(case_json).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct CreateIn {
    title: String,
}

async fn create(State(agent): A, Json(b): Json<CreateIn>) -> R {
    let s = agent.session().await?;
    let title: String = b.title.trim().chars().take(120).collect();
    if title.is_empty() {
        return Err(ApiError::Invalid("Give the project a working title.".into()));
    }
    let mut rt = s.cases_rt.lock().await;
    ensure_mls(&s, &mut rt).await?;
    let group_id = rt.mls.as_ref().ok_or(ApiError::Internal)?.create_group().map_err(|_| ApiError::Internal)?;
    persist_mls(&s, &rt)?;
    let relay = s.engine.profile().await.default_relay;
    let (read_id, write_id) = match s.engine.relay(&relay).await.request(|_| Cmd::GNew { ttl_days: 30 }).await? {
        Resp::GIds { read_id, write_id } => (read_id, write_id),
        _ => return Err(ApiError::Internal),
    };
    let me = my_name(&s).await;
    let c = CaseRecord {
        id: new_id(),
        title,
        group_id,
        relay,
        read_id,
        write_id,
        next_seq: 0,
        own_seqs: vec![],
        owner: true,
        members: vec![Member { name: me.clone(), contact_id: None, role: Role::Owner, pending: false }],
        key: sotto_core::rand::bytes(),
        created_at: time::now(),
        destroy_at: None,
        legal_hold: false,
        insurance: None,
        removed: false,
        owner_key: rt.mls.as_ref().ok_or(ApiError::Internal)?.signature_key(),
    };
    save(&s, &c)?;
    add_entry(&s, &c, &me, EntryKind::System("Project opened.".into()), None)?;
    subscribe(&s, &mut rt, &c).await;
    agent.emit(json!({ "type": "case", "id": hex(&c.id) }));
    Ok(Json(case_json(&c)))
}

async fn show(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let c = record(&s, &cid(&id)?)?;
    let entries: Vec<Value> = list_entries(&s, &c)?.iter().map(entry_json).collect();
    let mut v = case_json(&c);
    v["entries"] = json!(entries);
    Ok(Json(v))
}

#[derive(Deserialize)]
struct UpdateIn {
    /// Days from now; 0 removes the destroy date.
    destroy_in_days: Option<u32>,
    legal_hold: Option<bool>,
}

async fn update(State(agent): A, Path(id): Path<String>, Json(b): Json<UpdateIn>) -> R {
    let s = agent.session().await?;
    let mut c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the project owner can change retention.".into()));
    }
    if let Some(d) = b.destroy_in_days {
        c.destroy_at = if d == 0 { None } else { Some(time::now() + d.min(3650) as u64 * 86_400) };
    }
    if let Some(h) = b.legal_hold {
        c.legal_hold = h;
    }
    {
        let rt = s.cases_rt.lock().await;
        let msg = CaseMsg::Retention { destroy_at: c.destroy_at, legal_hold: c.legal_hold };
        broadcast(&s, &rt, &mut c, &msg).await?;
        persist_mls(&s, &rt)?;
    }
    add_entry(&s, &c, &my_name(&s).await, EntryKind::System(retention_text(c.destroy_at, c.legal_hold)), None)?;
    save(&s, &c)?;
    show(State(agent), Path(id)).await
}

async fn destroy(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let mut c = record(&s, &cid(&id)?)?;
    if c.owner {
        let rt = s.cases_rt.lock().await;
        let _ = broadcast(&s, &rt, &mut c, &CaseMsg::Closed).await;
    }
    destroy_local(&agent, &s, &c.id).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct NoteIn {
    text: String,
}

async fn note(State(agent): A, Path(id): Path<String>, Json(b): Json<NoteIn>) -> R {
    let s = agent.session().await?;
    let text = b.text.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::Invalid("Write a note first.".into()));
    }
    let mut c = record(&s, &cid(&id)?)?;
    if c.removed {
        return Err(ApiError::Invalid("You're no longer a member of this project.".into()));
    }
    let nid = new_id();
    {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut c, &CaseMsg::Note { id: nid, text: text.clone() }).await?;
        persist_mls(&s, &rt)?;
    }
    add_entry(&s, &c, &my_name(&s).await, EntryKind::Note(text), Some(nid))?;
    save(&s, &c)?;
    show(State(agent), Path(id)).await
}

#[derive(Deserialize)]
struct SourceIn {
    alias: String,
    note: String,
}

async fn source(State(agent): A, Path(id): Path<String>, Json(b): Json<SourceIn>) -> R {
    let s = agent.session().await?;
    let alias: String = b.alias.trim().chars().take(80).collect();
    if alias.is_empty() {
        return Err(ApiError::Invalid("Give them a pseudonym, never their real name.".into()));
    }
    let mut c = record(&s, &cid(&id)?)?;
    let sid = new_id();
    {
        let rt = s.cases_rt.lock().await;
        broadcast(&s, &rt, &mut c, &CaseMsg::Source { id: sid, alias: alias.clone(), note: b.note.clone() }).await?;
        persist_mls(&s, &rt)?;
    }
    add_entry(&s, &c, &my_name(&s).await, EntryKind::Source { alias, note: b.note }, Some(sid))?;
    save(&s, &c)?;
    show(State(agent), Path(id)).await
}

#[derive(Deserialize)]
struct InviteIn {
    contact_id: String,
    role: Role,
}

async fn invite(State(agent): A, Path(id): Path<String>, Json(b): Json<InviteIn>) -> R {
    let s = agent.session().await?;
    let mut c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the project owner can add people.".into()));
    }
    if b.role == Role::Owner {
        return Err(ApiError::Invalid("A project has one owner.".into()));
    }
    let contact = cid(&b.contact_id)?;
    let view = s.engine.contact(&contact).await.ok_or(ApiError::NotFound)?;
    if view.state != "ready" {
        return Err(ApiError::Invalid("They need to accept your conversation invite first.".into()));
    }
    if c.members.iter().any(|m| m.contact_id == Some(contact)) {
        return Err(ApiError::Invalid("They're already in this project.".into()));
    }
    let name = if !view.their_name.is_empty() { view.their_name.clone() } else { view.nickname.clone() };
    c.members.push(Member { name, contact_id: Some(contact), role: b.role, pending: true });
    save(&s, &c)?;
    s.engine.send_payload(&contact, &Payload::KeyPackageRequest { case_id: c.id }).await?;
    let action = format!("invited {} as {}", c.members.last().map(|m| m.name.clone()).unwrap_or_default(), format!("{:?}", b.role).to_lowercase());
    let _ = audit(&s, &mut c, &action).await;
    save(&s, &c)?;
    show(State(agent), Path(id)).await
}

async fn remove(State(agent): A, Path((id, name)): Path<(String, String)>) -> R {
    let s = agent.session().await?;
    let mut c = record(&s, &cid(&id)?)?;
    if !c.owner {
        return Err(ApiError::Invalid("Only the project owner can remove people.".into()));
    }
    let Some(i) = c.members.iter().position(|m| m.name == name && m.role != Role::Owner) else { return Err(ApiError::NotFound) };
    let m = c.members.remove(i);
    if !m.pending {
        let mut rt = s.cases_rt.lock().await;
        ensure_mls(&s, &mut rt).await?;
        let commit = rt.mls.as_ref().ok_or(ApiError::Internal)?.remove_members(&c.group_id, &[m.name.clone().into_bytes()]).map_err(|_| ApiError::Internal)?;
        persist_mls(&s, &rt)?;
        gsend(&s, &mut c, commit).await?;
        let list = c.members.iter().filter(|m| !m.pending).map(|m| (m.name.clone(), m.role)).collect();
        broadcast(&s, &rt, &mut c, &CaseMsg::Members(list)).await?;
        persist_mls(&s, &rt)?;
    }
    add_entry(&s, &c, &my_name(&s).await, EntryKind::System(format!("{} was removed from the project.", m.name)), None)?;
    save(&s, &c)?;
    show(State(agent), Path(id)).await
}

#[derive(Deserialize)]
struct InsuranceIn {
    interval_days: u32,
    contact_ids: Vec<String>,
}

async fn insurance(State(agent): A, Path(id): Path<String>, Json(b): Json<InsuranceIn>) -> R {
    let s = agent.session().await?;
    let ids = b.contact_ids.iter().map(|c| cid(c)).collect::<Result<Vec<_>, _>>()?;
    setup_insurance(&agent, &s, &cid(&id)?, b.interval_days, ids).await?;
    show(State(agent), Path(id)).await
}

async fn stop_insurance(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let mut c = record(&s, &cid(&id)?)?;
    if let Some(ins) = c.insurance.take() {
        let (tl_id, seed) = (ins.tl_id, ins.seed);
        let _ = s.engine.relay(&ins.relay).await.request(move |n| Cmd::TDel { tl_id, sig: sign(&seed, &auth_message(n, "tdel", &[&tl_id])) }).await;
    }
    save(&s, &c)?;
    show(State(agent), Path(id)).await
}

async fn checkin(State(agent): A, Path(id): Path<String>) -> R {
    let s = agent.session().await?;
    let deadline = check_in(&agent, &s, &cid(&id)?).await?;
    Ok(Json(json!({ "deadline": deadline })))
}

async fn grants(State(agent): A) -> R {
    let s = agent.session().await?;
    let mut out = Vec::new();
    for n in s.cases.names("grant/") {
        if let Some(g) = s.cases.get::<Grant>(&n)? {
            let from = s.engine.contact(&g.from).await.map(|c| if c.nickname.is_empty() { c.their_name } else { c.nickname }).unwrap_or_else(|| "A former contact".into());
            out.push(json!({ "read_id": hex(&g.read_id), "case_title": g.case_title, "from": from, "deadline": g.deadline, "received_at": g.received_at, "id": hex(&g.id) }));
        }
    }
    Ok(Json(json!(out)))
}

async fn open(State(agent): A, Path(read_id): Path<String>) -> R {
    let s = agent.session().await?;
    Ok(Json(open_grant(&s, &cid(&read_id)?).await?))
}
