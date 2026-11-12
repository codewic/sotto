//! Phase 0 of docs/calls.md: does device-hosted, constant-rate voice over Tor work well enough?
//!
//! Two separate Tor instances run here: one hosts each call as a fresh one-time onion service with
//! client authorization (the caller's device in the design), the other connects to it (the person
//! being called). After a simulated ring, both sides send one 498-byte frame (exactly one Tor cell)
//! every 40 ms, as a constant-rate Opus call would. Both ends live in this process and share one
//! clock, so one-way delay is measured exactly, not estimated from round trips.
//!
//!   call-spike run --tor /usr/bin/tor --data DIR --calls 10 --seconds 60 --out results.jsonl
//!   call-spike summary results.jsonl
//!
//! Throwaway measurement code. Nothing here ships.

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{MissedTickBehavior, interval, sleep, timeout};
use x25519_dalek::{PublicKey, StaticSecret};

/// One Tor RELAY_DATA cell carries 498 bytes, so one frame costs exactly one cell.
const FRAME: usize = 498;
/// Opus frame length in the design.
const TICK: Duration = Duration::from_millis(40);
/// Capture of one 40 ms frame plus Opus lookahead.
const CODEC_MS: f64 = 46.5;
/// Audio output device buffer.
const DEVICE_MS: f64 = 20.0;
/// Go criteria from docs/calls.md.
const GO_M2E_MS: f64 = 700.0;
const GO_CONNECT_S: f64 = 20.0;

/// How each call is carried.
#[derive(Clone, Copy)]
struct Mode {
    /// Independent Tor circuits carrying every frame; the first copy to arrive wins.
    paths: usize,
    /// The engine: freshest frame wins, and a path that falls behind the best one is replaced
    /// by a fresh circuit mid-call. Off: every frame is queued and sent, as a naive
    /// implementation would, on the paths the call started with.
    engine: bool,
}

type Res<T> = Result<T, String>;

fn e(x: impl std::fmt::Display) -> String {
    x.to_string()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("run") => run(&args[2..]).await,
        Some("summary") => summary(Path::new(args.get(2).map(String::as_str).unwrap_or("results.jsonl"))).await,
        _ => Err("usage: call-spike run --tor PATH --data DIR [--calls N] [--seconds S] [--answer-after S] [--paths K] [--engine] --out FILE | call-spike summary FILE".into()),
    };
    if let Err(m) = r {
        eprintln!("call-spike: {m}");
        std::process::exit(1);
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

async fn run(args: &[String]) -> Res<()> {
    let tor = flag(args, "--tor").unwrap_or_else(|| "tor".into());
    let data = PathBuf::from(flag(args, "--data").ok_or("--data is required")?);
    let calls: usize = flag(args, "--calls").and_then(|v| v.parse().ok()).unwrap_or(10);
    let seconds: u64 = flag(args, "--seconds").and_then(|v| v.parse().ok()).unwrap_or(60);
    let answer_after: f64 = flag(args, "--answer-after").and_then(|v| v.parse().ok()).unwrap_or(6.0);
    let out = PathBuf::from(flag(args, "--out").ok_or("--out is required")?);
    let mode = Mode { paths: flag(args, "--paths").and_then(|v| v.parse().ok()).unwrap_or(1).clamp(1, 4), engine: args.iter().any(|a| a == "--engine") };

    // Two independent Tor instances: no shared guards, circuits or descriptor cache.
    let _host_tor = launch(&tor, &data.join("host"), 19050, 19060).await?;
    let _callee_tor = launch(&tor, &data.join("callee"), 19051, 19061).await?;
    let mut host = Ctl::connect(19060, &data.join("host")).await?;
    let mut callee = Ctl::connect(19061, &data.join("callee")).await?;
    log("waiting for both Tor instances to bootstrap");
    host.bootstrapped().await?;
    callee.bootstrapped().await?;
    log("both bootstrapped");

    // Descriptor upload events, timestamped as they arrive.
    let uploads: Arc<Mutex<Vec<(String, Instant)>>> = Arc::default();
    {
        let mut ev = Ctl::connect(19060, &data.join("host")).await?;
        ev.cmd("SETEVENTS HS_DESC").await?;
        let uploads = uploads.clone();
        tokio::spawn(async move {
            while let Ok(line) = ev.line().await {
                // 650 HS_DESC UPLOADED <ServiceID> ...
                let p: Vec<&str> = line.split(' ').collect();
                if p.len() > 3 && p[1] == "HS_DESC" && p[2] == "UPLOADED" {
                    uploads.lock().unwrap().push((p[3].to_string(), Instant::now()));
                }
            }
        });
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(e)?;
    let port = listener.local_addr().map_err(e)?.port();

    for n in 1..=calls {
        log(&format!("call {n} of {calls}"));
        let result = one_call(&mut host, &mut callee, &listener, port, &uploads, seconds, answer_after, mode).await;
        let line = match result {
            Ok(v) => v,
            Err(m) => json!({ "ok": false, "error": m, "started_unix": unix(), "paths": mode.paths, "engine": mode.engine }),
        };
        log(&format!("  {}", brief(&line)));
        let mut f = tokio::fs::OpenOptions::new().create(true).append(true).open(&out).await.map_err(e)?;
        f.write_all(format!("{line}\n").as_bytes()).await.map_err(e)?;
        sleep(Duration::from_secs(3)).await;
    }
    summary(&out).await
}

#[allow(clippy::too_many_arguments)]
async fn one_call(
    host: &mut Ctl,
    callee: &mut Ctl,
    listener: &TcpListener,
    port: u16,
    uploads: &Arc<Mutex<Vec<(String, Instant)>>>,
    seconds: u64,
    answer_after: f64,
    mode: Mode,
) -> Res<Value> {
    let started_unix = unix();
    // The callee's one-time client-authorization key travels inside the call offer in the design.
    let secret = StaticSecret::from(sotto_core::rand::bytes::<32>());
    let public = PublicKey::from(&secret);

    let t0 = Instant::now();
    let reply = host
        .cmd(&format!("ADD_ONION NEW:ED25519-V3 Flags=DiscardPK,V3Auth Port=9,127.0.0.1:{port} ClientAuthV3={}", base32(public.as_bytes())))
        .await?;
    let sid = reply.iter().find_map(|l| l.strip_prefix("250-ServiceID=")).ok_or("no ServiceID in ADD_ONION reply")?.to_string();
    callee.cmd(&format!("ONION_CLIENT_AUTH_ADD {sid} x25519:{}", STANDARD.encode(secret.to_bytes()))).await?;

    // Ringing: the callee answers after a few seconds, as a person would.
    sleep(Duration::from_secs_f64(answer_after)).await;
    let t_answer = Instant::now();

    let target = format!("{sid}.onion");
    let attempts = Arc::new(AtomicU32::new(0));
    // Each path uses its own SOCKS credentials, so Tor builds it a separate rendezvous circuit.
    let connect_one = |i: usize| {
        let target = target.clone();
        let attempts = attempts.clone();
        async move {
            loop {
                attempts.fetch_add(1, Ordering::Relaxed);
                let user = format!("path{i}");
                let r = timeout(
                    Duration::from_secs(30),
                    tokio_socks::tcp::Socks5Stream::connect_with_password(("127.0.0.1", 19051), (target.as_str(), 9), &user, "x"),
                )
                .await;
                match r {
                    Ok(Ok(s)) => return Ok::<TcpStream, String>(s.into_inner()),
                    _ if t_answer.elapsed() > Duration::from_secs(90) => return Err("could not connect within 90 s of answering".to_string()),
                    _ => sleep(Duration::from_millis(500)).await,
                }
            }
        }
    };
    let mut callee_ends = Vec::new();
    let mut host_ends = Vec::new();
    // The call starts as soon as one path is up; extra paths get 20 more seconds to join.
    let first = tokio::try_join!(async { listener.accept().await.map(|(s, _)| s).map_err(e) }, connect_one(0))?;
    let connect_s = t_answer.elapsed().as_secs_f64();
    host_ends.push(first.0);
    callee_ends.push(first.1);
    if mode.paths > 1 {
        let extra: Vec<_> = (1..mode.paths).map(|i| tokio::spawn(timeout(Duration::from_secs(20), connect_one(i)))).collect();
        for _ in 1..mode.paths {
            if let Ok(Ok((s, _))) = timeout(Duration::from_secs(22), listener.accept()).await {
                host_ends.push(s);
            }
        }
        for t in extra {
            if let Ok(Ok(Ok(s))) = t.await {
                callee_ends.push(s);
            }
        }
    }
    let paths_up = host_ends.len().min(callee_ends.len());
    host_ends.truncate(paths_up);
    callee_ends.truncate(paths_up);
    let publish_s = uploads.lock().unwrap().iter().find(|(s, _)| *s == sid).map(|(_, t)| t.duration_since(t0).as_secs_f64());

    // Media: constant rate, both directions at once, for `seconds`.
    let base = Instant::now();
    let deadline = base + Duration::from_secs(seconds);
    let host_side = Side::new(base, deadline, mode);
    let callee_side = Side::new(base, deadline, mode);
    for s in host_ends {
        host_side.add(s);
    }
    for s in callee_ends {
        callee_side.add(s);
    }
    // The host keeps accepting replacement paths for the whole call.
    let accept_more = async {
        while let Ok(Ok((s, _))) = tokio::time::timeout_at(deadline.into(), listener.accept()).await {
            host_side.add(s);
        }
    };
    let supervise = async {
        if mode.engine { supervise(&callee_side, &connect_one, deadline, mode.paths).await } else { 0 }
    };
    let (sent_h, sent_c, _, replaced) = tokio::join!(host_side.run(), callee_side.run(), accept_more, supervise);
    let sent = [sent_h, sent_c];
    let got = [callee_side.inbox.frames(), host_side.inbox.frames()];
    let per_path = [callee_side.inbox.per_path_p50(), host_side.inbox.per_path_p50()];
    let attempts = attempts.load(Ordering::Relaxed);

    host.cmd(&format!("DEL_ONION {sid}")).await.ok();
    callee.cmd(&format!("ONION_CLIENT_AUTH_REMOVE {sid}")).await.ok();

    let dirs: Vec<Value> = (0..2)
        .map(|i| {
            let mut d = direction(&got[i], sent[i]);
            d["per_path_oneway_p50_ms"] = json!(per_path[i]);
            d
        })
        .collect();
    let m2e = dirs.iter().filter_map(|d| d["m2e_p50_ms"].as_f64()).fold(0.0, f64::max);
    Ok(json!({
        "ok": true,
        "started_unix": started_unix,
        "paths": mode.paths,
        "paths_up": paths_up,
        "replaced": replaced,
        "engine": mode.engine,
        "publish_s": publish_s,
        "connect_after_answer_s": connect_s,
        "attempts": attempts,
        "seconds": seconds,
        "m2e_p50_ms": m2e,
        "host_to_callee": dirs[0],
        "callee_to_host": dirs[1],
    }))
}

/// Frames arriving in one direction, over every path. The first copy of each frame wins.
#[derive(Default)]
struct Merge {
    first: Mutex<HashMap<u32, (u64, f64)>>,
    per_path: Mutex<HashMap<usize, Vec<f64>>>,
}

impl Merge {
    fn arrive(&self, path: usize, seq: u32, capture_us: u64, delay_ms: f64) {
        self.first.lock().unwrap().entry(seq).or_insert((capture_us, delay_ms));
        self.per_path.lock().unwrap().entry(path).or_default().push(delay_ms);
    }
    /// (capture µs, one-way delay ms) in sending order.
    fn frames(&self) -> Vec<(u64, f64)> {
        let m = self.first.lock().unwrap();
        let mut v: Vec<(u32, (u64, f64))> = m.iter().map(|(k, v)| (*k, *v)).collect();
        v.sort_by_key(|x| x.0);
        v.into_iter().map(|x| x.1).collect()
    }
    fn per_path_p50(&self) -> Vec<f64> {
        let m = self.per_path.lock().unwrap();
        let mut keys: Vec<_> = m.keys().cloned().collect();
        keys.sort();
        keys.iter().map(|k| round(pct(&m[k], 50.0))).collect()
    }
}

/// Recent arrivals on one path, for judging its health.
#[derive(Default)]
struct PathState {
    recent: Mutex<Vec<(Instant, f64)>>,
}

struct PathHandle {
    id: usize,
    added: Instant,
    st: Arc<PathState>,
    read: JoinHandle<()>,
    write: JoinHandle<()>,
}

/// One side of a call: a 40 ms ticker (the microphone) feeding every path, and the frames
/// arriving from the other side merged across paths.
struct Side {
    base: Instant,
    deadline: Instant,
    mode: Mode,
    tx: watch::Sender<Option<u32>>,
    inbox: Arc<Merge>,
    paths: Mutex<Vec<PathHandle>>,
    next_id: AtomicUsize,
}

impl Side {
    fn new(base: Instant, deadline: Instant, mode: Mode) -> Self {
        let (tx, _) = watch::channel(None);
        Side { base, deadline, mode, tx, inbox: Arc::default(), paths: Mutex::default(), next_id: AtomicUsize::new(0) }
    }

    fn add(&self, s: TcpStream) {
        s.set_nodelay(true).ok();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let st = Arc::new(PathState::default());
        let (r, w) = s.into_split();
        let read = tokio::spawn(read_path(r, id, self.base, self.deadline, st.clone(), self.inbox.clone()));
        let write = tokio::spawn(write_path(w, self.tx.subscribe(), self.base, self.mode));
        self.paths.lock().unwrap().push(PathHandle { id, added: Instant::now(), st, read, write });
    }

    fn remove(&self, id: usize) {
        let mut p = self.paths.lock().unwrap();
        if let Some(i) = p.iter().position(|h| h.id == id) {
            let h = p.remove(i);
            h.read.abort();
            h.write.abort();
        }
    }

    /// (path id, median delay over `window`, frames in `window`) for paths old enough to judge.
    fn health(&self, window: Duration) -> Vec<(usize, Option<f64>, usize)> {
        let now = Instant::now();
        self.paths
            .lock()
            .unwrap()
            .iter()
            .filter(|h| now.duration_since(h.added) > Duration::from_secs(3))
            .map(|h| {
                let r = h.st.recent.lock().unwrap();
                let v: Vec<f64> = r.iter().filter(|(t, _)| now.duration_since(*t) < window).map(|x| x.1).collect();
                (h.id, if v.is_empty() { None } else { Some(pct(&v, 50.0)) }, v.len())
            })
            .collect()
    }

    /// Ticks until the deadline, then waits for the other side's last frames. Returns frames produced.
    async fn run(&self) -> u32 {
        let mut t = interval(TICK);
        t.set_missed_tick_behavior(MissedTickBehavior::Burst);
        let mut seq: u32 = 0;
        loop {
            let at = t.tick().await.into_std();
            if at >= self.deadline {
                break;
            }
            self.tx.send_replace(Some(seq));
            seq += 1;
        }
        self.tx.send_replace(None);
        let stop = self.deadline + Duration::from_secs(15);
        while Instant::now() < stop && self.paths.lock().unwrap().iter().any(|h| !h.read.is_finished()) {
            sleep(Duration::from_millis(200)).await;
        }
        for h in self.paths.lock().unwrap().iter() {
            h.read.abort();
            h.write.abort();
        }
        seq
    }
}

/// The engine's path supervisor (callee side). Every second it compares each path's recent
/// delay with the best one; a path that is 300 ms behind or silent gets a replacement circuit,
/// connected first and swapped in only once it works (make before break).
async fn supervise<F, Fut>(side: &Side, connect: &F, deadline: Instant, first_new: usize) -> u32
where
    F: Fn(usize) -> Fut,
    Fut: std::future::Future<Output = Res<TcpStream>> + Send + 'static,
{
    let mut replaced = 0;
    let mut n = first_new;
    let mut pending: Option<(usize, JoinHandle<Res<TcpStream>>)> = None;
    let started = Instant::now();
    loop {
        sleep(Duration::from_secs(1)).await;
        if Instant::now() + Duration::from_secs(5) > deadline {
            break;
        }
        if let Some((_, h)) = &pending {
            if h.is_finished() {
                let (bad, h) = pending.take().unwrap();
                if let Ok(Ok(s)) = h.await {
                    side.add(s);
                    side.remove(bad);
                    replaced += 1;
                }
            }
            continue;
        }
        if started.elapsed() < Duration::from_secs(4) {
            continue;
        }
        let health = side.health(Duration::from_secs(2));
        let best = health.iter().filter_map(|h| h.1).fold(f64::INFINITY, f64::min);
        let worst = health
            .iter()
            .filter(|h| h.1.is_none_or(|m| m > best + 300.0))
            .max_by(|a, b| a.1.unwrap_or(f64::INFINITY).partial_cmp(&b.1.unwrap_or(f64::INFINITY)).unwrap());
        if let Some(w) = worst {
            pending = Some((w.0, tokio::spawn(connect(n))));
            n += 1;
        }
    }
    if let Some((_, h)) = pending {
        h.abort();
    }
    replaced
}

/// Writes frames to one path. Every path sends at the constant rate. Naive mode sends every
/// frame produced since its last write, however late; the engine sends only the newest.
async fn write_path(mut w: OwnedWriteHalf, mut rx: watch::Receiver<Option<u32>>, base: Instant, mode: Mode) {
    let mut frame = [0u8; FRAME];
    let mut next: Option<u32> = None;
    while rx.changed().await.is_ok() {
        let Some(newest) = *rx.borrow_and_update() else { break };
        let from = if mode.engine { newest } else { next.unwrap_or(newest) };
        for seq in from..=newest {
            let capture = base + TICK * seq;
            frame[..4].copy_from_slice(&seq.to_le_bytes());
            frame[4..12].copy_from_slice(&(capture.duration_since(base).as_micros() as u64).to_le_bytes());
            if w.write_all(&frame).await.is_err() {
                return;
            }
        }
        next = Some(newest + 1);
    }
    let _ = w.shutdown().await;
}

async fn read_path(mut r: OwnedReadHalf, path: usize, base: Instant, deadline: Instant, st: Arc<PathState>, inbox: Arc<Merge>) {
    let mut frame = [0u8; FRAME];
    let stop = deadline + Duration::from_secs(15);
    while let Ok(Ok(_)) = timeout(stop.saturating_duration_since(Instant::now()), r.read_exact(&mut frame)).await {
        let now = base.elapsed().as_micros() as u64;
        let seq = u32::from_le_bytes(frame[..4].try_into().unwrap());
        let sent = u64::from_le_bytes(frame[4..12].try_into().unwrap());
        let delay = now.saturating_sub(sent) as f64 / 1000.0;
        {
            let mut rec = st.recent.lock().unwrap();
            let t = Instant::now();
            rec.retain(|(at, _)| t.duration_since(*at) < Duration::from_secs(5));
            rec.push((t, delay));
        }
        inbox.arrive(path, seq, sent, delay);
    }
}

/// Delay percentiles, plus a simulated adaptive jitter buffer: before each frame, the playout
/// delay is set to the 95th percentile of the previous three seconds plus a 20 ms margin.
/// A frame arriving after its playout time counts as late (it would be concealed).
fn direction(frames: &[(u64, f64)], sent: u32) -> Value {
    let delays: Vec<f64> = frames.iter().map(|f| f.1).collect();
    if delays.is_empty() {
        return json!({ "received": 0, "sent": sent });
    }
    let window = (3_000 / TICK.as_millis() as usize).max(1);
    let mut playout = Vec::with_capacity(delays.len());
    let mut late = 0usize;
    for i in 0..delays.len() {
        let d = if i < 5 {
            delays[0] + 120.0
        } else {
            let lo = i.saturating_sub(window);
            pct(&delays[lo..i], 95.0) + 20.0
        }
        .clamp(60.0, 2_000.0);
        if delays[i] > d {
            late += 1;
        }
        playout.push(d);
    }
    let p_play50 = pct(&playout, 50.0);
    let p_play90 = pct(&playout, 90.0);
    json!({
        "sent": sent,
        "received": delays.len(),
        "oneway_p50_ms": round(pct(&delays, 50.0)),
        "oneway_p90_ms": round(pct(&delays, 90.0)),
        "oneway_p99_ms": round(pct(&delays, 99.0)),
        "oneway_max_ms": round(delays.iter().cloned().fold(0.0, f64::max)),
        // Frames that never arrived on any path count as late too: they'd be concealed.
        "late_pct": round(100.0 * (late + (sent as usize).saturating_sub(delays.len())) as f64 / (sent as f64).max(1.0)),
        "playout_p50_ms": round(p_play50),
        "m2e_p50_ms": round(p_play50 + CODEC_MS + DEVICE_MS),
        "m2e_p90_ms": round(p_play90 + CODEC_MS + DEVICE_MS),
        "static_m2e_ms": round(pct(&delays, 98.0) + CODEC_MS + DEVICE_MS),
    })
}

async fn summary(path: &Path) -> Res<()> {
    let text = tokio::fs::read_to_string(path).await.map_err(e)?;
    let rows: Vec<Value> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    if rows.is_empty() {
        return Err("no results yet".into());
    }
    let ok: Vec<&Value> = rows.iter().filter(|r| r["ok"] == true).collect();
    let m2e: Vec<f64> = ok.iter().filter_map(|r| r["m2e_p50_ms"].as_f64()).collect();
    let connect: Vec<f64> = ok.iter().filter_map(|r| r["connect_after_answer_s"].as_f64()).collect();
    let publish: Vec<f64> = ok.iter().filter_map(|r| r["publish_s"].as_f64()).collect();
    let late: Vec<f64> = ok
        .iter()
        .flat_map(|r| [r["host_to_callee"]["late_pct"].as_f64(), r["callee_to_host"]["late_pct"].as_f64()])
        .flatten()
        .collect();
    let within = connect.iter().filter(|&&s| s <= GO_CONNECT_S).count();
    let within_pct = 100.0 * within as f64 / rows.len() as f64;
    let med = if m2e.is_empty() { f64::NAN } else { pct(&m2e, 50.0) };
    println!("calls: {} ({} connected, {} failed)", rows.len(), ok.len(), rows.len() - ok.len());
    if !m2e.is_empty() {
        println!("mouth-to-ear, per-call median: median {:.0} ms, p90 {:.0} ms, best {:.0} ms, worst {:.0} ms", med, pct(&m2e, 90.0), pct(&m2e, 0.0), pct(&m2e, 100.0));
    }
    if !connect.is_empty() {
        println!("connect after answering: median {:.1} s, p90 {:.1} s; within {GO_CONNECT_S:.0} s: {within} of {} ({within_pct:.0}%)", pct(&connect, 50.0), pct(&connect, 90.0), rows.len());
    }
    if !publish.is_empty() {
        println!("onion service published after: median {:.1} s, p90 {:.1} s", pct(&publish, 50.0), pct(&publish, 90.0));
    }
    if !late.is_empty() {
        println!("late frames (concealed): median {:.1}%, p90 {:.1}%", pct(&late, 50.0), pct(&late, 90.0));
    }
    let go = med < GO_M2E_MS && within_pct >= 90.0;
    println!(
        "criteria: median mouth-to-ear under {GO_M2E_MS:.0} ms: {}; 90% connect within {GO_CONNECT_S:.0} s: {}; verdict so far: {}",
        if med < GO_M2E_MS { "yes" } else { "no" },
        if within_pct >= 90.0 { "yes" } else { "no" },
        if go { "GO" } else { "NO GO" }
    );
    if rows.len() < 50 {
        println!("note: {} of the 50 calls the design asks for, spread across the day", rows.len());
    }
    Ok(())
}

// ---------- Tor processes and the control port ----------

async fn launch(tor: &str, dir: &Path, socks: u16, ctl: u16) -> Res<Child> {
    tokio::fs::create_dir_all(dir).await.map_err(e)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(e)?;
    }
    let torrc = dir.join("torrc");
    let conf = format!(
        "DataDirectory {d}\nSocksPort 127.0.0.1:{socks}\nControlPort 127.0.0.1:{ctl}\nCookieAuthentication 1\nAvoidDiskWrites 1\nLog notice file {d}/notice.log\n",
        d = dir.display()
    );
    tokio::fs::write(&torrc, conf).await.map_err(e)?;
    Command::new(tor)
        .arg("-f")
        .arg(&torrc)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|x| format!("could not start {tor}: {x}"))
}

struct Ctl {
    r: BufReader<OwnedReadHalf>,
    w: OwnedWriteHalf,
}

impl Ctl {
    async fn connect(port: u16, dir: &Path) -> Res<Ctl> {
        let started = Instant::now();
        let stream = loop {
            match TcpStream::connect(("127.0.0.1", port)).await {
                Ok(s) => break s,
                Err(_) if started.elapsed() < Duration::from_secs(60) => sleep(Duration::from_millis(300)).await,
                Err(x) => return Err(format!("control port {port}: {x}")),
            }
        };
        let (r, w) = stream.into_split();
        let mut c = Ctl { r: BufReader::new(r), w };
        let cookie = loop {
            match tokio::fs::read(dir.join("control_auth_cookie")).await {
                Ok(b) if b.len() == 32 => break b,
                _ if started.elapsed() < Duration::from_secs(60) => sleep(Duration::from_millis(300)).await,
                _ => return Err("no control cookie".into()),
            }
        };
        let hex: String = cookie.iter().map(|b| format!("{b:02x}")).collect();
        c.cmd(&format!("AUTHENTICATE {hex}")).await?;
        Ok(c)
    }

    async fn line(&mut self) -> Res<String> {
        let mut s = String::new();
        if self.r.read_line(&mut s).await.map_err(e)? == 0 {
            return Err("control connection closed".into());
        }
        Ok(s.trim_end().to_string())
    }

    /// Sends one command and returns its reply lines. Fails on a non-2xx reply.
    async fn cmd(&mut self, c: &str) -> Res<Vec<String>> {
        self.w.write_all(format!("{c}\r\n").as_bytes()).await.map_err(e)?;
        let mut lines = Vec::new();
        loop {
            let l = self.line().await?;
            if l.starts_with("650") {
                continue; // async events never arrive here except on the events connection
            }
            let done = l.len() >= 4 && l.as_bytes()[3] == b' ';
            lines.push(l);
            if done {
                break;
            }
        }
        let last = lines.last().cloned().unwrap_or_default();
        if last.starts_with('2') { Ok(lines) } else { Err(format!("{}: {last}", c.split(' ').next().unwrap_or(""))) }
    }

    async fn bootstrapped(&mut self) -> Res<()> {
        let started = Instant::now();
        loop {
            let r = self.cmd("GETINFO status/bootstrap-phase").await?;
            if r.iter().any(|l| l.contains("PROGRESS=100")) {
                return Ok(());
            }
            if started.elapsed() > Duration::from_secs(300) {
                return Err(format!("Tor did not bootstrap in 5 minutes: {}", r.join(" ")));
            }
            sleep(Duration::from_secs(1)).await;
        }
    }
}

// ---------- small helpers ----------

fn base32(b: &[u8]) -> String {
    const A: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0u32);
    for &x in b {
        buf = (buf << 8) | x as u32;
        bits += 8;
        while bits >= 5 {
            out.push(A[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn pct(v: &[f64], p: f64) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
    s[i.min(s.len() - 1)]
}

fn round(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn log(m: &str) {
    eprintln!("[{}] {m}", unix());
}

fn brief(v: &Value) -> String {
    if v["ok"] != true {
        return format!("failed: {}", v["error"]);
    }
    format!(
        "connected {:.1} s after answer, published {}, mouth-to-ear {} ms, one-way p50 {} / p99 {} ms, late {}%",
        v["connect_after_answer_s"].as_f64().unwrap_or(0.0),
        v["publish_s"].as_f64().map(|s| format!("{s:.1} s")).unwrap_or("?".into()),
        v["m2e_p50_ms"],
        v["host_to_callee"]["oneway_p50_ms"],
        v["host_to_callee"]["oneway_p99_ms"],
        v["host_to_callee"]["late_pct"]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_matches_rfc4648() {
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32(&[0u8; 32]).len(), 52);
    }

    #[test]
    fn jitter_buffer_counts_late_frames() {
        let mut f: Vec<(u64, f64)> = (0..200).map(|i| (i * 40_000, 300.0)).collect();
        f[150].1 = 900.0;
        let d = direction(&f, 200);
        assert_eq!(d["received"], 200);
        assert!(d["late_pct"].as_f64().unwrap() > 0.0);
        assert!((d["m2e_p50_ms"].as_f64().unwrap() - (320.0 + CODEC_MS + DEVICE_MS)).abs() < 1.0);
    }
}
