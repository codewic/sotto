//! One long-lived connection per relay, with reconnect, re-subscription,
//! Tor stream isolation and optional cover traffic.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use sotto_core::keys::sign;
use sotto_core::wire::{ClientFrame, Cmd, FILE_FRAME_SIZE, Resp, ServerMsg, auth_message, decode_frame, encode_frame};
use sotto_core::{Id, hex};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use crate::{Error, Result};

/// How to reach relays.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Transport {
    /// Straight to the relay: TLS for `wss://` relays, plain TCP for local `ws://` ones. The relay
    /// sees your network address.
    Direct,
    /// Through a Tor SOCKS5 proxy, normally `127.0.0.1:9050` (tor) or `127.0.0.1:9150` (Tor Browser).
    Tor { socks: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Connecting,
    Online,
    Offline,
}

#[derive(Debug)]
pub enum Push {
    Msg { relay: String, rcv_id: Id, msg_id: Id, blob: Vec<u8> },
    GMsg { relay: String, read_id: Id, seq: u64, blob: Vec<u8> },
}

type Builder = Box<dyn FnOnce(&[u8; 32]) -> Cmd + Send>;

struct Request {
    build: Builder,
    reply: oneshot::Sender<Resp>,
}

struct Subs {
    queues: HashMap<Id, [u8; 32]>,
    groups: HashMap<Id, Arc<AtomicU64>>,
}

/// Handle to a relay connection. Cheap to clone.
#[derive(Clone)]
pub struct RelayConn {
    pub url: String,
    requests: mpsc::Sender<Request>,
    subs: Arc<Mutex<Subs>>,
    resub: mpsc::UnboundedSender<()>,
    status: watch::Receiver<Status>,
    cover: Arc<AtomicBool>,
}

struct Target {
    host: String,
    port: u16,
    tls: bool,
}

fn parse_url(url: &str) -> Result<Target> {
    let (rest, tls) = match (url.strip_prefix("wss://"), url.strip_prefix("ws://")) {
        (Some(r), _) => (r, true),
        (None, Some(r)) => (r, false),
        _ => return Err(Error::BadRelay),
    };
    let authority = rest.split('/').next().unwrap_or("");
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().map_err(|_| Error::BadRelay)?),
        None => (authority.to_string(), if tls { 443 } else { 80 }),
    };
    if host.is_empty() {
        return Err(Error::BadRelay);
    }
    Ok(Target { host, port, tls })
}

/// TLS for `wss://` relays, trusting the Mozilla root certificates built into Sotto (the same on
/// every computer, whatever the system has installed).
fn tls() -> tokio_rustls::TlsConnector {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        // Debug builds only: tests trust a throwaway certificate for a local TLS relay.
        if cfg!(debug_assertions)
            && let Some(path) = std::env::var_os("SOTTO_TEST_RELAY_CA")
        {
            use rustls::pki_types::pem::PemObject;
            for cert in rustls::pki_types::CertificateDer::pem_file_iter(path).into_iter().flatten().flatten() {
                let _ = roots.add(cert);
            }
        }
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("ring supports the default TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    });
    tokio_rustls::TlsConnector::from(config.clone())
}

/// Check a relay address without connecting.
pub fn validate_relay_url(url: &str) -> Result<()> {
    parse_url(url).map(|_| ())
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

async fn open(url: &str, transport: &Transport, isolation: &str) -> Result<Box<dyn Io>> {
    let t = parse_url(url)?;
    let stream: Box<dyn Io> = match transport {
        Transport::Direct => Box::new(TcpStream::connect((t.host.as_str(), t.port)).await.map_err(|_| Error::Offline)?),
        Transport::Tor { socks } => {
            // Distinct SOCKS credentials make Tor put each relay on its own circuit.
            let s = tokio_socks::tcp::Socks5Stream::connect_with_password(
                socks.as_str(),
                (t.host.as_str(), t.port),
                isolation,
                "sotto",
            )
            .await
            .map_err(|_| Error::Offline)?;
            Box::new(s)
        }
    };
    if !t.tls {
        return Ok(stream);
    }
    let name = rustls::pki_types::ServerName::try_from(t.host.clone()).map_err(|_| Error::BadRelay)?;
    Ok(Box::new(tls().connect(name, stream).await.map_err(|_| Error::Offline)?))
}

impl RelayConn {
    pub fn spawn(url: String, transport: Transport, pushes: mpsc::UnboundedSender<Push>) -> RelayConn {
        let (req_tx, req_rx) = mpsc::channel(64);
        let (resub_tx, resub_rx) = mpsc::unbounded_channel();
        let (status_tx, status_rx) = watch::channel(Status::Connecting);
        let subs = Arc::new(Mutex::new(Subs { queues: HashMap::new(), groups: HashMap::new() }));
        let cover = Arc::new(AtomicBool::new(false));
        let conn = RelayConn {
            url: url.clone(),
            requests: req_tx,
            subs: subs.clone(),
            resub: resub_tx,
            status: status_rx,
            cover: cover.clone(),
        };
        tokio::spawn(run(Ctx { url, transport, subs, pushes, status: status_tx, cover }, req_rx, resub_rx));
        conn
    }

    pub fn status(&self) -> Status {
        *self.status.borrow()
    }

    pub fn status_watch(&self) -> watch::Receiver<Status> {
        self.status.clone()
    }

    pub fn set_cover(&self, on: bool) {
        self.cover.store(on, Ordering::Relaxed);
    }

    /// Send a command built against the live connection nonce and wait for its response.
    pub async fn request(&self, build: impl FnOnce(&[u8; 32]) -> Cmd + Send + 'static) -> Result<Resp> {
        let (reply, rx) = oneshot::channel();
        self.requests.send(Request { build: Box::new(build), reply }).await.map_err(|_| Error::Offline)?;
        match tokio::time::timeout(Duration::from_secs(120), rx).await {
            Ok(Ok(Resp::Err(e))) => Err(Error::Relay(e)),
            Ok(Ok(r)) => Ok(r),
            _ => Err(Error::Offline),
        }
    }

    /// Keep a queue subscribed across reconnects.
    pub async fn subscribe_queue(&self, rcv_id: Id, seed: [u8; 32]) {
        self.subs.lock().await.queues.insert(rcv_id, seed);
        let _ = self.resub.send(());
    }

    pub async fn unsubscribe_queue(&self, rcv_id: &Id) {
        self.subs.lock().await.queues.remove(rcv_id);
    }

    /// Keep a group subscribed; `next` is the next sequence number the caller wants.
    pub async fn subscribe_group(&self, read_id: Id, next: Arc<AtomicU64>) {
        self.subs.lock().await.groups.insert(read_id, next);
        let _ = self.resub.send(());
    }

    pub async fn unsubscribe_group(&self, read_id: &Id) {
        self.subs.lock().await.groups.remove(read_id);
    }

    /// Acknowledge a delivered queue message so the relay deletes it and pushes the next.
    pub async fn ack(&self, rcv_id: Id, msg_id: Id, seed: [u8; 32]) -> Result<()> {
        self.request(move |nonce| Cmd::Ack { rcv_id, msg_id, sig: sign(&seed, &auth_message(nonce, "ack", &[&rcv_id, &msg_id])) })
            .await
            .map(|_| ())
    }
}

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(FILE_FRAME_SIZE)).max_frame_size(Some(FILE_FRAME_SIZE))
}

struct Ctx {
    url: String,
    transport: Transport,
    subs: Arc<Mutex<Subs>>,
    pushes: mpsc::UnboundedSender<Push>,
    status: watch::Sender<Status>,
    cover: Arc<AtomicBool>,
}

async fn run(ctx: Ctx, mut requests: mpsc::Receiver<Request>, mut resub: mpsc::UnboundedReceiver<()>) {
    let isolation = hex(&sotto_core::rand::bytes::<8>());
    let mut backoff = 1u64;
    loop {
        let _ = ctx.status.send(Status::Connecting);
        let attempt = async {
            let io = open(&ctx.url, &ctx.transport, &isolation).await?;
            let (ws, _) = tokio_tungstenite::client_async_with_config(ctx.url.as_str(), io, Some(ws_config()))
                .await
                .map_err(|_| Error::Offline)?;
            Ok::<_, Error>(ws)
        };
        if let Ok(Ok(ws)) = tokio::time::timeout(Duration::from_secs(60), attempt).await {
            backoff = 1;
            session(ws, &ctx, &mut requests, &mut resub).await;
        }
        if ctx.pushes.is_closed() {
            return;
        }
        let _ = ctx.status.send(Status::Offline);
        let jitter = sotto_core::rand::below(1000) as u64;
        let sleep = tokio::time::sleep(Duration::from_millis(backoff * 1000 + jitter));
        tokio::pin!(sleep);
        loop {
            tokio::select! {
                _ = &mut sleep => break,
                // Fail requests quickly while offline instead of letting them hang.
                req = requests.recv() => match req {
                    Some(r) => drop(r.reply),
                    None => return,
                },
            }
        }
        backoff = (backoff * 2).min(60);
    }
}

async fn session<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    ctx: &Ctx,
    requests: &mut mpsc::Receiver<Request>,
    resub: &mut mpsc::UnboundedReceiver<()>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut sink, mut source) = ws.split();
    let nonce = match tokio::time::timeout(Duration::from_secs(30), source.next()).await {
        Ok(Some(Ok(Message::Binary(b)))) => match decode_frame::<ServerMsg>(&b) {
            Ok(ServerMsg::Hello { nonce, .. }) => nonce,
            _ => return,
        },
        _ => return,
    };
    let _ = ctx.status.send(Status::Online);
    let mut corr: u32 = 0;
    let mut pending: HashMap<u32, oneshot::Sender<Resp>> = HashMap::new();
    let mut subscribed_queues: HashSet<Id> = HashSet::new();
    let mut subscribed_groups: HashSet<Id> = HashSet::new();
    let mut keepalive = tokio::time::interval(Duration::from_secs(45));
    keepalive.tick().await;
    let mut cover_timer = Box::pin(tokio::time::sleep(next_cover_delay()));
    while resub.try_recv().is_ok() {}
    let mut need_resub = true;

    loop {
        let mut outgoing: Vec<(Cmd, Option<oneshot::Sender<Resp>>)> = Vec::new();
        if need_resub {
            need_resub = false;
            let s = ctx.subs.lock().await;
            for (rcv_id, seed) in s.queues.iter() {
                if subscribed_queues.insert(*rcv_id) {
                    let sig = sign(seed, &auth_message(&nonce, "sub", &[rcv_id]));
                    outgoing.push((Cmd::Sub { rcv_id: *rcv_id, sig }, None));
                }
            }
            for (read_id, next) in s.groups.iter() {
                if subscribed_groups.insert(*read_id) {
                    outgoing.push((Cmd::GSub { read_id: *read_id, from: next.load(Ordering::SeqCst) }, None));
                }
            }
            subscribed_queues.retain(|q| s.queues.contains_key(q));
            subscribed_groups.retain(|g| s.groups.contains_key(g));
        }
        if outgoing.is_empty() {
            tokio::select! {
                req = requests.recv() => {
                    let Some(req) = req else { return };
                    outgoing.push(((req.build)(&nonce), Some(req.reply)));
                }
                _ = resub.recv() => { need_resub = true; }
                incoming = source.next() => {
                    let Some(Ok(msg)) = incoming else { return };
                    let Message::Binary(b) = msg else { continue };
                    let Ok(server) = decode_frame::<ServerMsg>(&b) else { return };
                    match server {
                        ServerMsg::Resp { corr, resp } => {
                            if let Some(r) = pending.remove(&corr) {
                                let _ = r.send(resp);
                            }
                        }
                        ServerMsg::Msg { rcv_id, msg_id, blob } => {
                            let _ = ctx.pushes.send(Push::Msg { relay: ctx.url.clone(), rcv_id, msg_id, blob });
                        }
                        ServerMsg::GMsg { read_id, seq, blob } => {
                            let _ = ctx.pushes.send(Push::GMsg { relay: ctx.url.clone(), read_id, seq, blob });
                        }
                        ServerMsg::Hello { .. } | ServerMsg::Cover => {}
                    }
                }
                _ = keepalive.tick() => outgoing.push((Cmd::Ping, None)),
                _ = &mut cover_timer => {
                    cover_timer = Box::pin(tokio::time::sleep(next_cover_delay()));
                    if ctx.cover.load(Ordering::Relaxed) {
                        outgoing.push((Cmd::Cover, None));
                    }
                }
            }
        }
        for (cmd, reply) in outgoing {
            corr = corr.wrapping_add(1);
            if let Some(r) = reply {
                pending.insert(corr, r);
            }
            let Ok(frame) = encode_frame(&ClientFrame { corr, cmd }) else { continue };
            if sink.send(Message::Binary(frame.into())).await.is_err() {
                return;
            }
        }
    }
}

/// Exponentially distributed gaps (mean about 3 s) so cover traffic has no fixed rhythm.
fn next_cover_delay() -> Duration {
    let u = (sotto_core::rand::below(1_000_000) as f64 + 1.0) / 1_000_001.0;
    Duration::from_millis((-u.ln() * 3000.0).clamp(200.0, 20_000.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_relay_urls() {
        let t = parse_url("ws://abcdefghijklmnop.onion").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("abcdefghijklmnop.onion", 80));
        let t = parse_url("wss://relay.example.org").unwrap();
        assert!(t.tls && t.port == 443 && t.host == "relay.example.org");
        let t = parse_url("wss://relay.example.org:8443/").unwrap();
        assert!(t.tls && t.port == 8443);
        let t = parse_url("ws://127.0.0.1:7400/").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("127.0.0.1", 7400));
        assert!(parse_url("http://x").is_err());
        assert!(parse_url("ws://:80").is_err());
        assert!(parse_url("ws://x:notaport").is_err());
    }

    #[test]
    fn cover_delays_vary() {
        let a: Vec<_> = (0..20).map(|_| next_cover_delay()).collect();
        assert!(a.iter().any(|d| *d != a[0]));
        assert!(a.iter().all(|d| *d >= Duration::from_millis(200) && *d <= Duration::from_secs(20)));
    }
}
