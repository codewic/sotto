//! Sotto call broker.
//!
//! The only server-side piece of Sotto calls besides LiveKit itself. On request it creates a
//! throwaway room name and signs one short-lived join token per participant. The room's limits
//! (participant cap, close-when-empty) travel inside the tokens and are applied by LiveKit when the
//! first person joins, so the broker never talks to LiveKit and keeps no state about calls.
//!
//! What it deliberately doesn't know: who is calling whom (requests arrive over Tor and carry no
//! Sotto identity), names (identities are random per call) and anything about media (encrypted end
//! to end with a key the server never sees). See docs/calls.md.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use livekit_protocol::RoomConfiguration;
use livekit_token::{AccessToken, VideoGrants};
use serde::{Deserialize, Serialize};

/// Seconds an empty room lives before LiveKit closes it.
pub const EMPTY_TIMEOUT_SECS: u32 = 30;
/// Seconds a room lives after the last person leaves.
pub const DEPARTURE_TIMEOUT_SECS: u32 = 20;

pub struct Config {
    /// What clients connect to, for example `wss://calls.example.org`.
    pub public_url: String,
    pub api_key: String,
    api_secret: String,
    /// How long a token can be used to join. Once joined, LiveKit keeps the session going.
    pub join_window: Duration,
    /// Largest case huddle.
    pub max_huddle: u32,
    pub global_per_min: u32,
    pub ip_per_min: u32,
    /// Trust X-Forwarded-For from private-network peers too (a reverse proxy in its own
    /// container, as on Coolify). By default only a proxy on this machine is trusted.
    pub trust_private_proxy: bool,
}

/// Accepts the LiveKit address as wss://, ws://, https://, http:// or a bare host name, and
/// returns the WebSocket form clients connect to.
pub fn websocket_url(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let (scheme, rest) = if let Some(rest) = raw.strip_prefix("https://") {
        ("wss", rest)
    } else if let Some(rest) = raw.strip_prefix("wss://") {
        ("wss", rest)
    } else if let Some(rest) = raw.strip_prefix("http://") {
        ("ws", rest)
    } else if let Some(rest) = raw.strip_prefix("ws://") {
        ("ws", rest)
    } else if !raw.contains("://") {
        ("wss", raw)
    } else {
        return Err("the LiveKit address must be a host name or a wss:// or https:// URL".into());
    };
    let host = rest.trim_end_matches('/');
    if host.is_empty() || host.starts_with('/') || host.contains(char::is_whitespace) {
        return Err("the LiveKit address has no host name".into());
    }
    Ok(format!("{scheme}://{host}"))
}

impl Config {
    pub fn new(public_url: String, api_key: String, api_secret: String) -> Result<Self, String> {
        let public_url = websocket_url(&public_url)?;
        if api_key.is_empty() {
            return Err("the LiveKit API key is empty".into());
        }
        if api_secret.len() < 32 {
            return Err("the LiveKit API secret must be at least 32 characters".into());
        }
        Ok(Config {
            public_url,
            api_key,
            api_secret,
            join_window: Duration::from_secs(120),
            max_huddle: 8,
            global_per_min: 120,
            ip_per_min: 20,
            trust_private_proxy: false,
        })
    }
}

// Never print the secret, not even in debug output.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("public_url", &self.public_url)
            .field("api_key", &self.api_key)
            .field("api_secret", &"[hidden]")
            .field("join_window", &self.join_window)
            .field("max_huddle", &self.max_huddle)
            .field("global_per_min", &self.global_per_min)
            .field("ip_per_min", &self.ip_per_min)
            .field("trust_private_proxy", &self.trust_private_proxy)
            .finish()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A 1:1 call: exactly two tokens.
    #[default]
    Pair,
    /// A case huddle: `participants` tokens, up to `max_huddle`.
    Huddle,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomRequest {
    #[serde(default)]
    pub kind: Kind,
    #[serde(default)]
    pub participants: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct RoomGrant {
    pub url: String,
    pub room: String,
    /// One per participant. The caller keeps the first and sends the rest inside the
    /// end-to-end encrypted call offer.
    pub tokens: Vec<String>,
    pub join_within_secs: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MintError {
    TooManyParticipants(u32),
    TooFewParticipants,
    BadRoom,
    Signing,
}

impl MintError {
    fn message(&self) -> String {
        match self {
            MintError::TooManyParticipants(max) => format!("A huddle can have at most {max} people."),
            MintError::TooFewParticipants => "A huddle needs at least two people.".into(),
            MintError::BadRoom => "That isn't a call room.".into(),
            MintError::Signing => "The room couldn't be created. Try again.".into(),
        }
    }
}

fn random_id(prefix: &str, bytes: usize) -> String {
    const A: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).expect("operating system randomness is unavailable");
    let mut out = String::from(prefix);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in &raw[..bytes] {
        buf = (buf << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(A[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    out
}

/// A fresh room and one token per participant. Pure apart from randomness: no I/O.
/// Every room has space for `max_huddle`, so people can be added to a call later.
pub fn mint(cfg: &Config, req: &RoomRequest) -> Result<RoomGrant, MintError> {
    let n = match req.kind {
        Kind::Pair => 2,
        Kind::Huddle => {
            let n = req.participants.unwrap_or(cfg.max_huddle);
            if n > cfg.max_huddle {
                return Err(MintError::TooManyParticipants(cfg.max_huddle));
            }
            if n < 2 {
                return Err(MintError::TooFewParticipants);
            }
            n
        }
    };
    // 128-bit room names and 80-bit identities: unguessable, and meaningless to the server.
    let room = random_id("r", 16);
    let tokens = tokens_for(cfg, &room, n)?;
    Ok(RoomGrant { url: cfg.public_url.clone(), room, tokens, join_within_secs: cfg.join_window.as_secs() })
}

/// A room name this broker could have made: "r" and 25 base32 letters.
pub fn valid_room(room: &str) -> bool {
    room.len() == 26 && room.starts_with('r') && room[1..].bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// More tokens for a room that already exists (adding people to a call). Only someone who already
/// knows the room's unguessable name can ask, which means someone already in the call.
pub fn mint_more(cfg: &Config, room: &str, count: u32) -> Result<RoomGrant, MintError> {
    if !valid_room(room) {
        return Err(MintError::BadRoom);
    }
    if count == 0 {
        return Err(MintError::TooFewParticipants);
    }
    if count >= cfg.max_huddle {
        return Err(MintError::TooManyParticipants(cfg.max_huddle));
    }
    let tokens = tokens_for(cfg, room, count)?;
    Ok(RoomGrant { url: cfg.public_url.clone(), room: room.to_string(), tokens, join_within_secs: cfg.join_window.as_secs() })
}

fn tokens_for(cfg: &Config, room: &str, n: u32) -> Result<Vec<String>, MintError> {
    // Applied by LiveKit when the first person joins: space for a full huddle, closes when empty.
    let config = RoomConfiguration {
        name: room.to_string(),
        empty_timeout: EMPTY_TIMEOUT_SECS,
        departure_timeout: DEPARTURE_TIMEOUT_SECS,
        max_participants: cfg.max_huddle,
        ..Default::default()
    };
    let mut tokens = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let grants = VideoGrants {
            room_join: true,
            room: room.to_string(),
            can_publish: Some(true),
            can_subscribe: Some(true),
            can_publish_data: Some(true),
            can_update_own_metadata: Some(false),
            ..Default::default()
        };
        let token = AccessToken::with_api_key(&cfg.api_key, &cfg.api_secret)
            .with_ttl(cfg.join_window)
            .with_identity(&random_id("p", 10))
            .with_grants(grants)
            .with_room_config(config.clone())
            .to_jwt()
            .map_err(|_| MintError::Signing)?;
        tokens.push(token);
    }
    Ok(tokens)
}

/// Token buckets: one shared by everyone, one per client address. Requests usually arrive from
/// Tor exits, where many people share an address, so the per-address limit is generous and the
/// global one is the real backstop.
pub struct Limiter {
    global: Mutex<Bucket>,
    per_ip: Mutex<HashMap<IpAddr, Bucket>>,
    global_per_min: f64,
    ip_per_min: f64,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn take(&mut self, per_min: f64, now: Instant) -> Result<(), Duration> {
        self.tokens = (self.tokens + now.saturating_duration_since(self.at).as_secs_f64() * per_min / 60.0).min(per_min);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - self.tokens) * 60.0 / per_min))
        }
    }
}

impl Limiter {
    pub fn new(global_per_min: u32, ip_per_min: u32) -> Self {
        let g = global_per_min.max(1) as f64;
        Limiter { global: Mutex::new(Bucket { tokens: g, at: Instant::now() }), per_ip: Mutex::default(), global_per_min: g, ip_per_min: ip_per_min.max(1) as f64 }
    }

    /// Ok, or how long to wait.
    pub fn check(&self, ip: IpAddr, now: Instant) -> Result<(), Duration> {
        let mut per_ip = self.per_ip.lock().unwrap();
        if per_ip.len() > 10_000 {
            // Forget addresses that have been quiet long enough to have a full bucket again.
            per_ip.retain(|_, b| now.saturating_duration_since(b.at) < Duration::from_secs(60));
        }
        let rate = self.ip_per_min;
        per_ip.entry(ip).or_insert(Bucket { tokens: rate, at: now }).take(rate, now)?;
        self.global.lock().unwrap().take(self.global_per_min, now)
    }
}

struct AppState {
    cfg: Config,
    limiter: Limiter,
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_private(),
        IpAddr::V6(v) => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00 || v.to_ipv4_mapped().is_some_and(|m| m.is_loopback() || m.is_private()),
    }
}

pub fn app(cfg: Config) -> Router {
    let limiter = Limiter::new(cfg.global_per_min, cfg.ip_per_min);
    let state = Arc::new(AppState { cfg, limiter });
    Router::new()
        .route("/v1/rooms", post(create_room))
        .route("/v1/rooms/{room}/tokens", post(more_tokens))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(1024))
        .with_state(state)
}

/// The client address. Behind a reverse proxy, the first X-Forwarded-For entry; the header is
/// only trusted when the connection itself comes from this machine (or, with
/// `trust_private_proxy`, from a private network such as a container network).
fn client_ip(peer: SocketAddr, headers: &HeaderMap, trust_private_proxy: bool) -> IpAddr {
    let trusted = peer.ip().is_loopback() || (trust_private_proxy && is_private(peer.ip()));
    if trusted
        && let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse().ok())
    {
        return ip;
    }
    peer.ip()
}

fn reply(status: StatusCode, body: serde_json::Value) -> Response {
    let mut r = (status, Json(body)).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoreRequest {
    count: u32,
}

async fn more_tokens(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, Path(room): Path<String>, body: axum::body::Bytes) -> Response {
    if let Err(wait) = st.limiter.check(client_ip(peer, &headers, st.cfg.trust_private_proxy), Instant::now()) {
        let secs = wait.as_secs().max(1);
        let mut r = reply(StatusCode::TOO_MANY_REQUESTS, serde_json::json!({ "error": "Too many calls are starting right now. Try again in a moment.", "retry_after_secs": secs }));
        r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(secs));
        return r;
    }
    let Ok(req) = serde_json::from_slice::<MoreRequest>(&body) else {
        return reply(StatusCode::BAD_REQUEST, serde_json::json!({ "error": "The request wasn't understood." }));
    };
    match mint_more(&st.cfg, &room, req.count) {
        Ok(grant) => reply(StatusCode::OK, serde_json::to_value(grant).unwrap_or_default()),
        Err(MintError::Signing) => reply(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({ "error": MintError::Signing.message() })),
        Err(e) => reply(StatusCode::BAD_REQUEST, serde_json::json!({ "error": e.message() })),
    }
}

async fn create_room(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, body: axum::body::Bytes) -> Response {
    if let Err(wait) = st.limiter.check(client_ip(peer, &headers, st.cfg.trust_private_proxy), Instant::now()) {
        let secs = wait.as_secs().max(1);
        let mut r = reply(StatusCode::TOO_MANY_REQUESTS, serde_json::json!({ "error": "Too many calls are starting right now. Try again in a moment.", "retry_after_secs": secs }));
        r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(secs));
        return r;
    }
    // An empty body means a 1:1 call.
    let req: RoomRequest = if body.iter().all(|b| b.is_ascii_whitespace()) {
        RoomRequest::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(_) => return reply(StatusCode::BAD_REQUEST, serde_json::json!({ "error": "The request wasn't understood." })),
        }
    };
    match mint(&st.cfg, &req) {
        Ok(grant) => reply(StatusCode::OK, serde_json::to_value(grant).unwrap_or_default()),
        Err(MintError::Signing) => reply(StatusCode::INTERNAL_SERVER_ERROR, serde_json::json!({ "error": MintError::Signing.message() })),
        Err(e) => reply(StatusCode::BAD_REQUEST, serde_json::json!({ "error": e.message() })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use livekit_token::TokenVerifier;
    use tower::ServiceExt;

    const SECRET: &str = "test-secret-that-is-long-enough-for-hs256-0123456789";

    fn cfg() -> Config {
        Config::new("wss://calls.example.test".into(), "APItest".into(), SECRET.into()).unwrap()
    }

    #[test]
    fn pair_gets_two_distinct_tokens_for_one_room() {
        let c = cfg();
        let g = mint(&c, &RoomRequest::default()).unwrap();
        assert_eq!(g.tokens.len(), 2);
        assert_eq!(g.url, "wss://calls.example.test");
        assert_eq!(g.room.len(), 1 + 25);
        let v = TokenVerifier::with_api_key("APItest", SECRET);
        let a = v.verify(&g.tokens[0]).unwrap();
        let b = v.verify(&g.tokens[1]).unwrap();
        assert_ne!(a.sub, b.sub, "each participant gets its own identity");
        for claims in [&a, &b] {
            assert!(claims.video.room_join);
            assert_eq!(claims.video.room, g.room);
            assert!(!claims.video.room_admin && !claims.video.room_create && !claims.video.room_record);
            assert_eq!(claims.name, "", "no names on the server");
            let rc = claims.room_config.as_ref().unwrap();
            assert_eq!(rc.max_participants, 8, "every room has space for people to be added");
            assert_eq!(rc.empty_timeout, EMPTY_TIMEOUT_SECS);
            assert_eq!(rc.name, g.room);
            assert!(claims.exp - claims.nbf <= 121);
        }
    }

    #[test]
    fn tokens_signed_with_another_secret_fail() {
        let g = mint(&cfg(), &RoomRequest::default()).unwrap();
        assert!(TokenVerifier::with_api_key("APItest", "a-different-secret-that-is-also-long-0000").verify(&g.tokens[0]).is_err());
    }

    #[test]
    fn rooms_are_never_reused() {
        let c = cfg();
        let rooms: std::collections::HashSet<String> = (0..200).map(|_| mint(&c, &RoomRequest::default()).unwrap().room).collect();
        assert_eq!(rooms.len(), 200);
    }

    #[test]
    fn more_tokens_for_an_existing_room() {
        let c = cfg();
        let g = mint(&c, &RoomRequest::default()).unwrap();
        let more = mint_more(&c, &g.room, 2).unwrap();
        assert_eq!(more.room, g.room);
        assert_eq!(more.tokens.len(), 2);
        let claims = TokenVerifier::with_api_key("APItest", SECRET).verify(&more.tokens[0]).unwrap();
        assert_eq!(claims.video.room, g.room);
        assert_eq!(mint_more(&c, "not-a-room", 1).unwrap_err(), MintError::BadRoom);
        assert_eq!(mint_more(&c, &g.room, 0).unwrap_err(), MintError::TooFewParticipants);
        assert_eq!(mint_more(&c, &g.room, 8).unwrap_err(), MintError::TooManyParticipants(8));
        assert!(valid_room(&g.room));
    }

    #[test]
    fn huddles_are_capped() {
        let c = cfg();
        let ok = mint(&c, &RoomRequest { kind: Kind::Huddle, participants: Some(5) }).unwrap();
        assert_eq!(ok.tokens.len(), 5);
        assert_eq!(mint(&c, &RoomRequest { kind: Kind::Huddle, participants: Some(9) }).unwrap_err(), MintError::TooManyParticipants(8));
        assert_eq!(mint(&c, &RoomRequest { kind: Kind::Huddle, participants: Some(1) }).unwrap_err(), MintError::TooFewParticipants);
    }

    #[test]
    fn config_rejects_weak_settings_and_hides_the_secret() {
        assert!(Config::new("ftp://x".into(), "k".into(), SECRET.into()).is_err());
        assert!(Config::new("wss://x".into(), "".into(), SECRET.into()).is_err());
        assert!(Config::new("wss://x".into(), "k".into(), "short".into()).is_err());
        let shown = format!("{:?}", cfg());
        assert!(!shown.contains(SECRET));
        assert!(shown.contains("[hidden]"));
    }

    #[test]
    fn limiter_holds_each_address_and_everyone() {
        let l = Limiter::new(5, 2);
        let now = Instant::now();
        let a: IpAddr = "203.0.113.1".parse().unwrap();
        let b: IpAddr = "203.0.113.2".parse().unwrap();
        assert!(l.check(a, now).is_ok());
        assert!(l.check(a, now).is_ok());
        assert!(l.check(a, now).is_err(), "third request from one address in a minute waits");
        assert!(l.check(b, now).is_ok());
        assert!(l.check(b, now).is_ok());
        assert!(l.check("203.0.113.3".parse().unwrap(), now).is_ok());
        assert!(l.check("203.0.113.4".parse().unwrap(), now).is_err(), "the global limit holds across addresses");
        assert!(l.check(a, now + Duration::from_secs(60)).is_ok(), "buckets refill with time");
    }

    #[test]
    fn forwarded_address_only_trusted_from_this_machine() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.7, 10.0.0.1"));
        assert_eq!(client_ip("127.0.0.1:5000".parse().unwrap(), &h, false), "198.51.100.7".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip("192.0.2.9:5000".parse().unwrap(), &h, false), "192.0.2.9".parse::<IpAddr>().unwrap());
        // A proxy in another container: trusted only when asked to.
        assert_eq!(client_ip("10.0.1.5:5000".parse().unwrap(), &h, false), "10.0.1.5".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip("10.0.1.5:5000".parse().unwrap(), &h, true), "198.51.100.7".parse::<IpAddr>().unwrap());
        // A public peer is never trusted, whatever it claims.
        assert_eq!(client_ip("192.0.2.9:5000".parse().unwrap(), &h, true), "192.0.2.9".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn livekit_address_accepts_every_form_coolify_gives() {
        assert_eq!(websocket_url("https://calls.example.org/").unwrap(), "wss://calls.example.org");
        assert_eq!(websocket_url("calls.example.org").unwrap(), "wss://calls.example.org");
        assert_eq!(websocket_url("wss://calls.example.org").unwrap(), "wss://calls.example.org");
        assert_eq!(websocket_url("http://127.0.0.1:7880").unwrap(), "ws://127.0.0.1:7880");
        assert!(websocket_url("").is_err());
        assert!(websocket_url("ftp://x").is_err());
        assert!(websocket_url("https://").is_err());
    }

    async fn call(app: Router, method: &str, path: &str, body: &str) -> (StatusCode, HeaderMap, serde_json::Value) {
        let mut req = Request::builder().method(method).uri(path).body(Body::from(body.to_string())).unwrap();
        req.extensions_mut().insert(ConnectInfo("127.0.0.1:40000".parse::<SocketAddr>().unwrap()));
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, headers, serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
    }

    #[tokio::test]
    async fn http_creates_rooms_and_never_caches() {
        let (s, h, v) = call(app(cfg()), "POST", "/v1/rooms", "").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "no-store");
        assert_eq!(v["tokens"].as_array().unwrap().len(), 2);
        assert_eq!(v["join_within_secs"], 120);

        let (s, _, v) = call(app(cfg()), "POST", "/v1/rooms", r#"{"kind":"huddle","participants":4}"#).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["tokens"].as_array().unwrap().len(), 4);

        let (s, _, v) = call(app(cfg()), "POST", "/v1/rooms", r#"{"kind":"huddle","participants":40}"#).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "A huddle can have at most 8 people.");

        let (s, _, _) = call(app(cfg()), "POST", "/v1/rooms", r#"{"who":"me"}"#).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "unknown fields are refused");

        let (s, _, _) = call(app(cfg()), "GET", "/v1/rooms", "").await;
        assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);

        let (_, _, v) = call(app(cfg()), "POST", "/v1/rooms", "").await;
        let room = v["room"].as_str().unwrap().to_string();
        let (s, _, v) = call(app(cfg()), "POST", &format!("/v1/rooms/{room}/tokens"), r#"{"count":1}"#).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!((v["room"].as_str().unwrap(), v["tokens"].as_array().unwrap().len()), (room.as_str(), 1));
        let (s, _, _) = call(app(cfg()), "POST", "/v1/rooms/../tokens", r#"{"count":1}"#).await;
        assert_ne!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn http_rate_limits_with_retry_after() {
        let mut c = cfg();
        c.ip_per_min = 1;
        let a = app(c);
        let (s, _, _) = call(a.clone(), "POST", "/v1/rooms", "").await;
        assert_eq!(s, StatusCode::OK);
        let (s, h, v) = call(a, "POST", "/v1/rooms", "").await;
        assert_eq!(s, StatusCode::TOO_MANY_REQUESTS);
        assert!(h.get(header::RETRY_AFTER).is_some());
        assert!(v["retry_after_secs"].as_u64().unwrap() >= 1);
    }
}
