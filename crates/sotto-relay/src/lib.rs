//! Sotto relay: store-and-forward queues, ordered case queues, file chunks
//! and time-locked insurance packages (`docs/protocol.md` section 7).
//!
//! Privacy rules enforced here:
//! - message data lives in memory only and is deleted on acknowledgement or expiry;
//! - nothing about requests is logged;
//! - every frame has one of two fixed sizes;
//! - time-locked packages (already end-to-end encrypted) are the only data written to disk.

#![forbid(unsafe_code)]

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use sotto_core::keys::{Sig, verify};
use sotto_core::wire::{
    BLOB_SIZE, ClientFrame, Cmd, ErrCode, FILE_BLOB_SIZE, FILE_FRAME_SIZE, PROTOCOL_VERSION, Resp, ServerMsg, auth_message,
    decode_frame, encode_frame,
};
use sotto_core::{Id, kdf, new_id, time::now};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

const MAX_QUEUE_MSGS: usize = 2000;
const MAX_GROUP_MSGS: usize = 20_000;
const MAX_TTL_DAYS: u8 = 30;
const MAX_FILE_TTL_DAYS: u8 = 30;
const MAX_TL_CHUNKS: u32 = 4096;
/// Queues, groups, files and time locks one connection may create. Onion-service proof-of-work
/// (deploy/relay/torrc) makes opening many connections expensive.
const MAX_CREATES_PER_CONN: u32 = 2000;
/// Queues with nothing waiting and no activity for this long are removed.
const QUEUE_IDLE_SECS: u64 = 90 * DAY;
const DAY: u64 = 86_400;
/// Tags: ready invites kept per tag, and the largest invite accepted.
const MAX_TAG_INVITES: usize = 32;
const MAX_INVITE_BYTES: usize = 4096;

#[derive(Clone, Debug)]
pub struct Config {
    /// Where the relay keeps what must survive a restart: queue and group metadata (ids, auth keys,
    /// TTLs; never message content) and time-locked insurance packages. `None` keeps everything in
    /// memory and disables insurance files.
    pub data_dir: Option<PathBuf>,
    /// Upper bound on bytes held in memory across queues, groups and files.
    pub max_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { data_dir: None, max_bytes: 1 << 30 }
    }
}

type ConnId = u64;
type Tx = mpsc::Sender<ServerMsg>;

struct StoredMsg {
    id: Id,
    blob: Vec<u8>,
    expires: u64,
}

struct Queue {
    last_used: u64,
    rcv_auth: [u8; 32],
    ttl: u64,
    msgs: VecDeque<StoredMsg>,
    subscriber: Option<(ConnId, Tx)>,
    inflight: Option<Id>,
}

struct Group {
    ttl: u64,
    next_seq: u64,
    msgs: VecDeque<(u64, Vec<u8>, u64)>,
    subscribers: Vec<(ConnId, Tx)>,
    last_used: u64,
}

struct FileEntry {
    read_auth: [u8; 32],
    data: Vec<u8>,
    reads_left: u8,
    expires: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct TimeLock {
    owner: [u8; 32],
    read_id: Id,
    deadline: u64,
    chunks: Vec<Vec<u8>>,
}

/// A short name someone claimed, and the one-time invites waiting under it.
#[derive(serde::Serialize, serde::Deserialize)]
struct Tag {
    owner: [u8; 32],
    invites: VecDeque<(Vec<u8>, u64)>,
}

/// 3 to 24 characters: lowercase letters, digits, dot and underscore, starting with a letter.
pub fn valid_tag(t: &str) -> bool {
    let b = t.as_bytes();
    (3..=24).contains(&b.len()) && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'.' || *c == b'_')
}

#[derive(Default)]
struct State {
    queues: HashMap<Id, Queue>,
    snd_index: HashMap<Id, Id>,
    groups: HashMap<Id, Group>,
    group_write: HashMap<Id, Id>,
    files: HashMap<Id, FileEntry>,
    timelocks: HashMap<Id, TimeLock>,
    tl_read: HashMap<Id, Id>,
    tags: HashMap<String, Tag>,
    bytes: usize,
    next_conn: ConnId,
    creates: HashMap<ConnId, u32>,
}

struct Shared {
    state: Mutex<State>,
    config: Config,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// Run the relay on an already bound listener until the process ends.
pub async fn serve(listener: TcpListener, config: Config) -> std::io::Result<()> {
    let shared = Arc::new(Shared { state: Mutex::new(State::default()), config });
    load_meta(&shared)?;
    load_timelocks(&shared)?;
    load_tags(&shared);
    let sweeper = shared.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            sweep(&sweeper);
        }
    });
    loop {
        let (stream, _) = listener.accept().await?;
        let shared = shared.clone();
        tokio::spawn(async move {
            let _ = handle(stream, shared).await;
        });
    }
}

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(FILE_FRAME_SIZE)).max_frame_size(Some(FILE_FRAME_SIZE))
}

type BoxError = Box<dyn std::error::Error + Send + Sync>;

async fn handle(stream: TcpStream, shared: Arc<Shared>) -> Result<(), BoxError> {
    let _ = stream.set_nodelay(true);
    let ws = tokio::time::timeout(Duration::from_secs(30), tokio_tungstenite::accept_async_with_config(stream, Some(ws_config()))).await??;
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<ServerMsg>(256);
    let conn = {
        let mut st = shared.lock();
        st.next_conn += 1;
        st.next_conn
    };
    let nonce: [u8; 32] = sotto_core::rand::bytes();
    sink.send(Message::Binary(encode_frame(&ServerMsg::Hello { version: PROTOCOL_VERSION, nonce })?.into())).await?;

    let result: Result<(), BoxError> = async {
        loop {
            tokio::select! {
                incoming = source.next() => {
                    let Some(msg) = incoming else { break };
                    match msg? {
                        Message::Binary(frame) => {
                            let Ok(frame) = decode_frame::<ClientFrame>(&frame) else { break };
                            if let Some(out) = dispatch(&shared, conn, &nonce, &tx, frame) {
                                sink.send(Message::Binary(encode_frame(&out)?.into())).await?;
                            }
                        }
                        Message::Ping(p) => sink.send(Message::Pong(p)).await?,
                        _ => break,
                    }
                }
                outgoing = rx.recv() => {
                    let Some(out) = outgoing else { break };
                    sink.send(Message::Binary(encode_frame(&out)?.into())).await?;
                }
            }
        }
        Ok(())
    }
    .await;
    disconnect(&shared, conn);
    result
}

fn disconnect(shared: &Shared, conn: ConnId) {
    let mut st = shared.lock();
    st.creates.remove(&conn);
    for q in st.queues.values_mut() {
        if q.subscriber.as_ref().map(|(c, _)| *c) == Some(conn) {
            q.subscriber = None;
            q.inflight = None;
        }
    }
    for g in st.groups.values_mut() {
        g.subscribers.retain(|(c, _)| *c != conn);
    }
}

fn resp(corr: u32, r: Resp) -> Option<ServerMsg> {
    Some(ServerMsg::Resp { corr, resp: r })
}

fn err(corr: u32, e: ErrCode) -> Option<ServerMsg> {
    resp(corr, Resp::Err(e))
}

fn check(key: &[u8; 32], nonce: &[u8; 32], tag: &str, fields: &[&[u8]], sig: &Sig) -> bool {
    verify(key, &auth_message(nonce, tag, fields), sig).is_ok()
}

/// Push the head of a queue to its subscriber if nothing is in flight.
fn pump(q: &mut Queue, rcv_id: Id) {
    if q.inflight.is_some() {
        return;
    }
    let (Some((_, tx)), Some(head)) = (&q.subscriber, q.msgs.front()) else { return };
    let msg = ServerMsg::Msg { rcv_id, msg_id: head.id, blob: head.blob.clone() };
    if tx.try_send(msg).is_ok() {
        q.inflight = Some(head.id);
    }
}

fn ttl_secs(days: u8, max: u8) -> u64 {
    days.clamp(1, max) as u64 * DAY
}

fn dispatch(shared: &Shared, conn: ConnId, nonce: &[u8; 32], tx: &Tx, frame: ClientFrame) -> Option<ServerMsg> {
    let corr = frame.corr;
    let t = now();
    let mut st = shared.lock();
    let max_bytes = shared.config.max_bytes;
    if matches!(frame.cmd, Cmd::New { .. } | Cmd::GNew { .. } | Cmd::FPut { .. } | Cmd::TNew { .. }) {
        let n = st.creates.entry(conn).or_insert(0);
        *n += 1;
        if *n > MAX_CREATES_PER_CONN {
            return err(corr, ErrCode::Full);
        }
    }
    match frame.cmd {
        Cmd::Ping => resp(corr, Resp::Pong),
        Cmd::Cover => Some(ServerMsg::Cover),

        Cmd::New { rcv_auth, ttl_days } => {
            let (rcv_id, snd_id) = (new_id(), new_id());
            st.queues.insert(
                rcv_id,
                Queue { last_used: t, rcv_auth, ttl: ttl_secs(ttl_days, MAX_TTL_DAYS), msgs: VecDeque::new(), subscriber: None, inflight: None },
            );
            st.snd_index.insert(snd_id, rcv_id);
            save_meta(shared, &st);
            resp(corr, Resp::Ids { rcv_id, snd_id })
        }
        Cmd::Send { snd_id, blob } => {
            if blob.len() != BLOB_SIZE {
                return err(corr, ErrCode::BadRequest);
            }
            let Some(rcv_id) = st.snd_index.get(&snd_id).copied() else { return err(corr, ErrCode::NotFound) };
            if st.bytes + blob.len() > max_bytes {
                return err(corr, ErrCode::Full);
            }
            let bytes = blob.len();
            let q = st.queues.get_mut(&rcv_id).expect("index points at a queue");
            if q.msgs.len() >= MAX_QUEUE_MSGS {
                return err(corr, ErrCode::Full);
            }
            q.last_used = t;
            q.msgs.push_back(StoredMsg { id: new_id(), blob, expires: t + q.ttl });
            pump(q, rcv_id);
            st.bytes += bytes;
            resp(corr, Resp::Ok)
        }
        Cmd::Sub { rcv_id, sig } => {
            let Some(q) = st.queues.get_mut(&rcv_id) else { return err(corr, ErrCode::NotFound) };
            if !check(&q.rcv_auth, nonce, "sub", &[&rcv_id], &sig) {
                return err(corr, ErrCode::Auth);
            }
            q.subscriber = Some((conn, tx.clone()));
            q.inflight = None;
            q.last_used = t;
            // Reply first so the client sees Ok before the first pushed message.
            let _ = tx.try_send(ServerMsg::Resp { corr, resp: Resp::Ok });
            pump(q, rcv_id);
            None
        }
        Cmd::Ack { rcv_id, msg_id, sig } => {
            let Some(q) = st.queues.get_mut(&rcv_id) else { return err(corr, ErrCode::NotFound) };
            if !check(&q.rcv_auth, nonce, "ack", &[&rcv_id, &msg_id], &sig) {
                return err(corr, ErrCode::Auth);
            }
            let mut freed = 0;
            if let Some(pos) = q.msgs.iter().position(|m| m.id == msg_id) {
                freed = q.msgs.remove(pos).map(|m| m.blob.len()).unwrap_or(0);
            }
            if q.inflight == Some(msg_id) {
                q.inflight = None;
            }
            pump(q, rcv_id);
            st.bytes -= freed;
            resp(corr, Resp::Ok)
        }
        Cmd::Del { rcv_id, sig } => {
            let Some(q) = st.queues.get(&rcv_id) else { return err(corr, ErrCode::NotFound) };
            if !check(&q.rcv_auth, nonce, "del", &[&rcv_id], &sig) {
                return err(corr, ErrCode::Auth);
            }
            let q = st.queues.remove(&rcv_id).expect("present");
            let freed: usize = q.msgs.iter().map(|m| m.blob.len()).sum();
            st.bytes -= freed;
            st.snd_index.retain(|_, r| *r != rcv_id);
            save_meta(shared, &st);
            resp(corr, Resp::Ok)
        }

        Cmd::GNew { ttl_days } => {
            let (read_id, write_id) = (new_id(), new_id());
            st.groups.insert(
                read_id,
                Group { ttl: ttl_secs(ttl_days, MAX_TTL_DAYS), next_seq: 0, msgs: VecDeque::new(), subscribers: Vec::new(), last_used: t },
            );
            st.group_write.insert(write_id, read_id);
            save_meta(shared, &st);
            resp(corr, Resp::GIds { read_id, write_id })
        }
        Cmd::GSend { write_id, blob } => {
            if blob.len() != BLOB_SIZE {
                return err(corr, ErrCode::BadRequest);
            }
            let Some(read_id) = st.group_write.get(&write_id).copied() else { return err(corr, ErrCode::NotFound) };
            if st.bytes + blob.len() > max_bytes {
                return err(corr, ErrCode::Full);
            }
            let g = st.groups.get_mut(&read_id).expect("index points at a group");
            if g.msgs.len() >= MAX_GROUP_MSGS {
                return err(corr, ErrCode::Full);
            }
            let seq = g.next_seq;
            g.next_seq += 1;
            g.last_used = t;
            for (_, sub) in &g.subscribers {
                let _ = sub.try_send(ServerMsg::GMsg { read_id, seq, blob: blob.clone() });
            }
            let len = blob.len();
            g.msgs.push_back((seq, blob, t + g.ttl));
            let crossed = (seq + 1).is_multiple_of(SEQ_BLOCK);
            st.bytes += len;
            if crossed {
                save_meta(shared, &st);
            }
            resp(corr, Resp::GSeq { seq })
        }
        Cmd::GSub { read_id, from } => {
            let Some(g) = st.groups.get_mut(&read_id) else { return err(corr, ErrCode::NotFound) };
            g.last_used = t;
            g.subscribers.retain(|(c, _)| *c != conn);
            g.subscribers.push((conn, tx.clone()));
            let _ = tx.try_send(ServerMsg::Resp { corr, resp: Resp::Ok });
            for (seq, blob, _) in g.msgs.iter().filter(|(s, _, _)| *s >= from) {
                let _ = tx.try_send(ServerMsg::GMsg { read_id, seq: *seq, blob: blob.clone() });
            }
            None
        }

        Cmd::FPut { read_auth, max_reads, ttl_days, data } => {
            if data.len() != FILE_BLOB_SIZE || max_reads == 0 {
                return err(corr, ErrCode::BadRequest);
            }
            if st.bytes + data.len() > max_bytes {
                return err(corr, ErrCode::Full);
            }
            let fid = new_id();
            st.bytes += data.len();
            st.files.insert(fid, FileEntry { read_auth, data, reads_left: max_reads, expires: t + ttl_secs(ttl_days, MAX_FILE_TTL_DAYS) });
            resp(corr, Resp::FId { fid })
        }
        Cmd::FGet { fid, sig } => {
            let Some(f) = st.files.get_mut(&fid) else { return err(corr, ErrCode::NotFound) };
            if !check(&f.read_auth, nonce, "fget", &[&fid], &sig) {
                return err(corr, ErrCode::Auth);
            }
            f.reads_left -= 1;
            let (data, done) = if f.reads_left == 0 { (std::mem::take(&mut f.data), true) } else { (f.data.clone(), false) };
            if done {
                st.files.remove(&fid);
                st.bytes -= data.len();
            }
            resp(corr, Resp::FData { data })
        }
        Cmd::FDel { fid, sig } => {
            let Some(f) = st.files.get(&fid) else { return err(corr, ErrCode::NotFound) };
            if !check(&f.read_auth, nonce, "fdel", &[&fid], &sig) {
                return err(corr, ErrCode::Auth);
            }
            let f = st.files.remove(&fid).expect("present");
            st.bytes -= f.data.len();
            resp(corr, Resp::Ok)
        }

        Cmd::TNew { owner, deadline } => {
            if shared.config.data_dir.is_none() {
                return err(corr, ErrCode::NotFound);
            }
            let (tl_id, read_id) = (new_id(), new_id());
            let tl = TimeLock { owner, read_id, deadline, chunks: Vec::new() };
            if persist(shared, &tl_id, &tl).is_err() {
                return err(corr, ErrCode::Full);
            }
            st.tl_read.insert(read_id, tl_id);
            st.timelocks.insert(tl_id, tl);
            resp(corr, Resp::TIds { tl_id, read_id })
        }
        Cmd::TPut { tl_id, index, blob, sig } => {
            if blob.len() != FILE_BLOB_SIZE || index >= MAX_TL_CHUNKS {
                return err(corr, ErrCode::BadRequest);
            }
            let Some(tl) = st.timelocks.get_mut(&tl_id) else { return err(corr, ErrCode::NotFound) };
            let digest = kdf::sha256(&blob);
            if !check(&tl.owner, nonce, "tput", &[&tl_id, &index.to_le_bytes(), &digest], &sig) {
                return err(corr, ErrCode::Auth);
            }
            if index as usize != tl.chunks.len() {
                return err(corr, ErrCode::BadRequest);
            }
            let len = blob.len();
            if st.bytes + len > max_bytes {
                return err(corr, ErrCode::Full);
            }
            let tl = st.timelocks.get_mut(&tl_id).expect("checked above");
            tl.chunks.push(blob);
            if persist(shared, &tl_id, tl).is_err() {
                tl.chunks.pop();
                return err(corr, ErrCode::Full);
            }
            st.bytes += len;
            resp(corr, Resp::Ok)
        }
        Cmd::TCheckIn { tl_id, deadline, sig } => {
            let Some(tl) = st.timelocks.get_mut(&tl_id) else { return err(corr, ErrCode::NotFound) };
            if !check(&tl.owner, nonce, "tcheckin", &[&tl_id, &deadline.to_le_bytes()], &sig) {
                return err(corr, ErrCode::Auth);
            }
            tl.deadline = deadline;
            if persist(shared, &tl_id, tl).is_err() {
                return err(corr, ErrCode::Full);
            }
            resp(corr, Resp::Ok)
        }
        Cmd::TGet { read_id, index } => {
            let Some(tl_id) = st.tl_read.get(&read_id).copied() else { return err(corr, ErrCode::NotFound) };
            let tl = &st.timelocks[&tl_id];
            if t < tl.deadline {
                return resp(corr, Resp::Locked { deadline: tl.deadline });
            }
            match tl.chunks.get(index as usize) {
                Some(b) => resp(corr, Resp::TData { blob: b.clone(), count: tl.chunks.len() as u32 }),
                None => err(corr, ErrCode::NotFound),
            }
        }
        Cmd::TDel { tl_id, sig } => {
            let Some(tl) = st.timelocks.get(&tl_id) else { return err(corr, ErrCode::NotFound) };
            if !check(&tl.owner, nonce, "tdel", &[&tl_id], &sig) {
                return err(corr, ErrCode::Auth);
            }
            let tl = st.timelocks.remove(&tl_id).expect("present");
            st.tl_read.remove(&tl.read_id);
            st.bytes = st.bytes.saturating_sub(tl.chunks.iter().map(Vec::len).sum());
            if let Some(dir) = &shared.config.data_dir {
                let _ = std::fs::remove_file(dir.join("timelocks").join(format!("{}.tl", sotto_core::hex(&tl_id))));
            }
            resp(corr, Resp::Ok)
        }

        Cmd::TagAdd { tag, owner, invites, sig } => {
            if !valid_tag(&tag) || invites.iter().any(|(i, _)| i.len() > MAX_INVITE_BYTES) {
                return err(corr, ErrCode::BadRequest);
            }
            let mut digest = Vec::new();
            for (i, exp) in &invites {
                digest.extend_from_slice(&sotto_core::kdf::sha256(i));
                digest.extend_from_slice(&exp.to_be_bytes());
            }
            let digest = sotto_core::kdf::sha256(&digest);
            // The first claim decides the owner; after that only the owner may add.
            let key = st.tags.get(&tag).map(|x| x.owner).unwrap_or(owner);
            if key != owner || !check(&owner, nonce, "tagadd", &[tag.as_bytes(), &digest], &sig) {
                return err(corr, ErrCode::Auth);
            }
            let entry = st.tags.entry(tag).or_insert_with(|| Tag { owner, invites: VecDeque::new() });
            entry.invites.retain(|(_, exp)| *exp > t);
            for inv in invites {
                if entry.invites.len() >= MAX_TAG_INVITES {
                    break;
                }
                if inv.1 > t {
                    entry.invites.push_back(inv);
                }
            }
            let left = entry.invites.len() as u32;
            save_tags(shared, &st);
            resp(corr, Resp::TagLeft { left })
        }
        Cmd::TagTake { tag } => {
            // Counted like creates, so one connection can't drain a tag in a loop.
            let n = st.creates.entry(conn).or_insert(0);
            *n += 50;
            if *n > MAX_CREATES_PER_CONN {
                return err(corr, ErrCode::Full);
            }
            let Some(entry) = st.tags.get_mut(&tag) else { return err(corr, ErrCode::NotFound) };
            entry.invites.retain(|(_, exp)| *exp > t);
            let Some((invite, _)) = entry.invites.pop_front() else { return err(corr, ErrCode::NotFound) };
            save_tags(shared, &st);
            resp(corr, Resp::TagInvite { invite })
        }
        Cmd::TagDel { tag, sig } => {
            let Some(entry) = st.tags.get(&tag) else { return err(corr, ErrCode::NotFound) };
            if !check(&entry.owner, nonce, "tagdel", &[tag.as_bytes()], &sig) {
                return err(corr, ErrCode::Auth);
            }
            st.tags.remove(&tag);
            save_tags(shared, &st);
            resp(corr, Resp::Ok)
        }
    }
}

/// Tags survive a restart (their invites are public material: an invite only opens a mailbox).
fn save_tags(shared: &Shared, st: &State) {
    let Some(dir) = &shared.config.data_dir else { return };
    let all: Vec<(&String, &Tag)> = st.tags.iter().collect();
    if let Ok(bytes) = postcard::to_stdvec(&all) {
        let tmp = dir.join(format!("tags.{}.tmp", sotto_core::hex(&sotto_core::rand::bytes::<4>())));
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, dir.join("tags.bin"));
        }
    }
}

fn load_tags(shared: &Shared) {
    let Some(dir) = &shared.config.data_dir else { return };
    let Ok(bytes) = std::fs::read(dir.join("tags.bin")) else { return };
    let Ok(all) = postcard::from_bytes::<Vec<(String, Tag)>>(&bytes) else { return };
    shared.lock().tags = all.into_iter().collect();
}

fn persist(shared: &Shared, tl_id: &Id, tl: &TimeLock) -> std::io::Result<()> {
    let Some(dir) = &shared.config.data_dir else { return Ok(()) };
    let bytes = postcard::to_stdvec(tl).map_err(std::io::Error::other)?;
    let path = dir.join("timelocks").join(format!("{}.tl", sotto_core::hex(tl_id)));
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(tmp, path)
}

/// Queue and group metadata that must survive a restart. No message content, no timing.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Meta {
    queues: Vec<(Id, Id, [u8; 32], u64)>,
    groups: Vec<(Id, Id, u64, u64)>,
}

const SEQ_BLOCK: u64 = 1024;

/// Group sequence numbers are persisted in blocks: after a restart each group resumes at the next
/// block, so numbers only ever grow without writing to disk on every message.
fn save_meta(shared: &Shared, st: &State) {
    let Some(dir) = &shared.config.data_dir else { return };
    let snd_of: HashMap<&Id, &Id> = st.snd_index.iter().map(|(s, r)| (r, s)).collect();
    let write_of: HashMap<&Id, &Id> = st.group_write.iter().map(|(w, r)| (r, w)).collect();
    let meta = Meta {
        queues: st.queues.iter().filter_map(|(rcv, q)| snd_of.get(rcv).map(|s| (*rcv, **s, q.rcv_auth, q.ttl))).collect(),
        groups: st.groups.iter().filter_map(|(read, g)| write_of.get(read).map(|w| (*read, **w, g.ttl, (g.next_seq / SEQ_BLOCK + 1) * SEQ_BLOCK))).collect(),
    };
    if let Ok(bytes) = postcard::to_stdvec(&meta) {
        let path = dir.join("meta.bin");
        let tmp = dir.join(format!("meta.{}.tmp", sotto_core::hex(&sotto_core::rand::bytes::<4>())));
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

fn load_meta(shared: &Shared) -> std::io::Result<()> {
    let Some(dir) = &shared.config.data_dir else { return Ok(()) };
    let Ok(bytes) = std::fs::read(dir.join("meta.bin")) else { return Ok(()) };
    let Ok(meta) = postcard::from_bytes::<Meta>(&bytes) else { return Ok(()) };
    let mut st = shared.lock();
    let t = now();
    for (rcv, snd, auth, ttl) in meta.queues {
        st.queues.insert(rcv, Queue { last_used: t, rcv_auth: auth, ttl, msgs: VecDeque::new(), subscriber: None, inflight: None });
        st.snd_index.insert(snd, rcv);
    }
    for (read, write, ttl, next_seq) in meta.groups {
        st.groups.insert(read, Group { ttl, next_seq, msgs: VecDeque::new(), subscribers: Vec::new(), last_used: t });
        st.group_write.insert(write, read);
    }
    Ok(())
}

fn load_timelocks(shared: &Shared) -> std::io::Result<()> {
    let Some(dir) = &shared.config.data_dir else { return Ok(()) };
    let dir = &dir.join("timelocks");
    std::fs::create_dir_all(dir)?;
    let mut st = shared.lock();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(hexid) = name.strip_suffix(".tl") else { continue };
        let Ok(tl_id) = sotto_core::unhex::<16>(hexid) else { continue };
        let Ok(bytes) = std::fs::read(entry.path()) else { continue };
        let Ok(tl) = postcard::from_bytes::<TimeLock>(&bytes) else { continue };
        st.tl_read.insert(tl.read_id, tl_id);
        st.timelocks.insert(tl_id, tl);
    }
    Ok(())
}

/// Remove expired messages, files and idle groups.
fn sweep(shared: &Shared) {
    let t = now();
    let mut st = shared.lock();
    let mut freed = 0usize;
    for q in st.queues.values_mut() {
        q.msgs.retain(|m| {
            let keep = m.expires > t;
            if !keep {
                freed += m.blob.len();
            }
            keep
        });
    }
    for g in st.groups.values_mut() {
        while g.msgs.front().map(|(_, _, e)| *e <= t).unwrap_or(false) {
            freed += g.msgs.pop_front().map(|(_, b, _)| b.len()).unwrap_or(0);
        }
    }
    let stale: Vec<Id> = st.queues.iter().filter(|(_, q)| q.msgs.is_empty() && q.subscriber.is_none() && q.last_used + QUEUE_IDLE_SECS < t).map(|(k, _)| *k).collect();
    if !stale.is_empty() {
        for k in &stale {
            st.queues.remove(k);
        }
        st.snd_index.retain(|_, r| !stale.contains(r));
        save_meta(shared, &st);
    }
    let idle: Vec<Id> = st.groups.iter().filter(|(_, g)| g.msgs.is_empty() && g.last_used + g.ttl < t).map(|(k, _)| *k).collect();
    for k in idle {
        st.groups.remove(&k);
        st.group_write.retain(|_, r| *r != k);
    }
    st.files.retain(|_, f| {
        let keep = f.expires > t;
        if !keep {
            freed += f.data.len();
        }
        keep
    });
    st.bytes -= freed;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_frees_expired() {
        let shared = Shared { state: Mutex::new(State::default()), config: Config::default() };
        let _ = SEQ_BLOCK;
        {
            let mut st = shared.lock();
            let mut q = Queue { last_used: u64::MAX / 2, rcv_auth: [0; 32], ttl: 1, msgs: VecDeque::new(), subscriber: None, inflight: None };
            q.msgs.push_back(StoredMsg { id: [1; 16], blob: vec![0; 10], expires: 0 });
            q.msgs.push_back(StoredMsg { id: [2; 16], blob: vec![0; 10], expires: u64::MAX });
            st.queues.insert([9; 16], q);
            st.files.insert([3; 16], FileEntry { read_auth: [0; 32], data: vec![0; 5], reads_left: 1, expires: 0 });
            st.bytes = 25;
        }
        sweep(&shared);
        let st = shared.lock();
        assert_eq!(st.bytes, 10);
        assert_eq!(st.queues[&[9; 16]].msgs.len(), 1);
        assert!(st.files.is_empty());
    }
}
