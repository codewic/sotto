//! Vault and engine lifecycle: setup, unlock, lock, panic, auto-lock and settings.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sotto_client::{Engine, EngineConfig, Transport};
use sotto_vault::{Params, Store, Unlocked, Vault};
use tokio::sync::{Mutex, RwLock, broadcast};

use crate::ApiError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskMode {
    Standard,
    High,
    Extreme,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub risk_mode: RiskMode,
    pub auto_lock_minutes: u32,
    pub delete_after_download: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { risk_mode: RiskMode::Standard, auto_lock_minutes: 15, delete_after_download: true }
    }
}

impl Settings {
    /// High and Extreme lock after one idle minute regardless of the chosen value.
    pub fn effective_lock_minutes(&self) -> u32 {
        match self.risk_mode {
            RiskMode::Standard => self.auto_lock_minutes.clamp(1, 240),
            _ => 1,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
struct Scopes {
    engine: [u8; 32],
    drops: [u8; 32],
    reviews: [u8; 32],
    cases: [u8; 32],
}

/// Everything that exists only while the vault is open.
pub struct Session {
    pub vault: Mutex<Vault>,
    pub engine: Engine,
    pub how: Unlocked,
    pub main: Arc<Store>,
    pub drops: Arc<Store>,
    pub reviews: Arc<Store>,
    pub cases: Arc<Store>,
    pub settings: Mutex<Settings>,
    /// MLS client and per-case sequence counters; one lock serialises all case work.
    pub cases_rt: Mutex<crate::cases::Runtime>,
}

pub struct Agent {
    pub dir: PathBuf,
    pub params: Params,
    pub default_relay: String,
    pub default_transport: Transport,
    pub token: String,
    pub session: RwLock<Option<Arc<Session>>>,
    pub events: broadcast::Sender<Value>,
    pub last_activity: AtomicU64,
    /// Serialises setup, unlock, lock and panic so they cannot interleave.
    pub lifecycle: Mutex<()>,
    /// Travel-mode recovery entropy between "prepare" and "start". Never written to disk.
    pub travel: Mutex<Option<zeroize::Zeroizing<[u8; 32]>>>,
    pub unlock_failures: std::sync::atomic::AtomicU32,
    /// Where this agent listens (`unix:/path/agent.sock` or `http://127.0.0.1:PORT`), so the app
    /// can tell integrations where to post. Unset in tests.
    pub endpoint: std::sync::OnceLock<String>,
}

impl Agent {
    pub fn new(dir: PathBuf, params: Params, default_relay: String, default_transport: Transport, token: String) -> Arc<Agent> {
        // TLS for the call broker: install the crypto provider once, whoever builds the agent
        // (the binary or a test). Installing twice is harmless.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let (events, _) = broadcast::channel(1024);
        Arc::new(Agent {
            dir,
            params,
            default_relay,
            default_transport,
            token,
            session: RwLock::new(None),
            events,
            last_activity: AtomicU64::new(sotto_core::time::now()),
            lifecycle: Mutex::new(()),
            travel: Mutex::new(None),
            unlock_failures: std::sync::atomic::AtomicU32::new(0),
            endpoint: std::sync::OnceLock::new(),
        })
    }

    pub fn emit(&self, v: Value) {
        let _ = self.events.send(v);
    }

    pub fn touch(&self) {
        self.last_activity.store(sotto_core::time::now(), Ordering::Relaxed);
    }

    /// True when the current protection level turns calls off.
    pub async fn calls_off(&self) -> bool {
        match self.session.read().await.clone() {
            Some(s) => s.settings.lock().await.risk_mode == RiskMode::Extreme,
            None => true,
        }
    }

    pub async fn session(&self) -> Result<Arc<Session>, ApiError> {
        self.session.read().await.clone().ok_or(ApiError::Locked)
    }

    pub async fn state(&self) -> &'static str {
        if self.session.read().await.is_some() {
            "unlocked"
        } else if Vault::exists(&self.dir) {
            "locked"
        } else {
            "new"
        }
    }

    pub async fn setup(self: &Arc<Self>, passcode: &str, duress: Option<&str>, relay: Option<String>, transport: Option<Transport>) -> Result<(), ApiError> {
        let _g = self.lifecycle.lock().await;
        if passcode.chars().count() < 8 {
            return Err(ApiError::Invalid("Use at least 8 characters for your passcode.".into()));
        }
        if let Some(d) = duress {
            if d.chars().count() < 8 {
                return Err(ApiError::Invalid("Use at least 8 characters for the duress passcode.".into()));
            }
        }
        if let Some(r) = &relay {
            sotto_client::conn::validate_relay_url(r)?;
        }
        let (dir, params, p, d) = (self.dir.clone(), self.params, passcode.to_string(), duress.map(str::to_string));
        let vault = tokio::task::spawn_blocking(move || Vault::create(&dir, &p, d.as_deref(), params)).await.map_err(|_| ApiError::Internal)??;
        let relay = relay.unwrap_or_else(|| self.default_relay.clone());
        let transport = transport.unwrap_or_else(|| self.default_transport.clone());
        self.open(vault, Unlocked::Real, relay, transport).await
    }

    pub async fn unlock(self: &Arc<Self>, passcode: &str) -> Result<(), ApiError> {
        let _g = self.lifecycle.lock().await;
        if self.session.read().await.is_some() {
            return Ok(());
        }
        let (dir, p) = (self.dir.clone(), passcode.to_string());
        let (vault, how) = tokio::task::spawn_blocking(move || Vault::unlock(&dir, &p)).await.map_err(|_| ApiError::Internal)??;
        self.open(vault, how, self.default_relay.clone(), self.default_transport.clone()).await
    }

    async fn open(self: &Arc<Self>, vault: Vault, how: Unlocked, relay: String, transport: Transport) -> Result<(), ApiError> {
        let main = Arc::new(vault.main_handle()?);
        let scopes = match main.get::<Scopes>("scopes")? {
            Some(s) => s,
            None => {
                let s = Scopes {
                    engine: sotto_core::rand::bytes(),
                    drops: sotto_core::rand::bytes(),
                    reviews: sotto_core::rand::bytes(),
                    cases: sotto_core::rand::bytes(),
                };
                main.put("scopes", &s)?;
                s
            }
        };
        let settings: Settings = main.get("settings")?.unwrap_or_default();
        let engine_store = Arc::new(vault.scope(scopes.engine)?);
        let drops = Arc::new(vault.scope(scopes.drops)?);
        let reviews = Arc::new(vault.scope(scopes.reviews)?);
        let cases = Arc::new(vault.scope(scopes.cases)?);
        let engine = Engine::start(engine_store, EngineConfig { default_relay: relay, transport }).await?;
        engine.set_cover_traffic(settings.risk_mode != RiskMode::Standard).await;
        let session = Arc::new(Session {
            vault: Mutex::new(vault),
            engine: engine.clone(),
            how,
            main,
            drops,
            reviews,
            cases,
            settings: Mutex::new(settings),
            cases_rt: Mutex::new(crate::cases::Runtime::default()),
        });
        *self.session.write().await = Some(session.clone());
        self.touch();
        // Forward engine events to the UI stream; case payloads go to the case module.
        let me = self.clone();
        let mut rx = engine.events();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(sotto_client::Event::Extension { contact_id, payload }) => crate::cases::on_extension(&me, contact_id, payload).await,
                    // Calls are off in Extreme mode: refuse before anything rings.
                    Ok(sotto_client::Event::Call { call }) if call.phase == sotto_client::calls::Phase::Ringing && me.calls_off().await => {
                        if let (Ok(s), Ok(id)) = (me.session().await, sotto_core::unhex::<16>(&call.id)) {
                            s.engine.refuse_call(&id).await;
                        }
                    }
                    Ok(e) => {
                        if matches!(e, sotto_client::Event::Contacts) {
                            let me2 = me.clone();
                            tokio::spawn(async move { crate::avatars::sync(&me2).await });
                        }
                        if let sotto_client::Event::Contact { id } = &e {
                            let (me2, id) = (me.clone(), id.clone());
                            tokio::spawn(async move {
                                crate::tags::on_contact(&me2, &id).await;
                                crate::avatars::on_contact(&me2, &id).await;
                            });
                        }
                        me.emit(serde_json::to_value(&e).unwrap_or(Value::Null))
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => me.emit(json!({ "type": "resync" })),
                    Err(_) => break,
                }
            }
        });
        crate::cases::start(self, &session).await;
        self.emit(json!({ "type": "state", "state": "unlocked" }));
        Ok(())
    }

    /// Forget all keys. The engine stops and the vault handle is dropped (keys are zeroized).
    pub async fn lock(&self) {
        let _g = self.lifecycle.lock().await;
        if let Some(s) = self.session.write().await.take() {
            s.engine.shutdown().await;
        }
        self.emit(json!({ "type": "state", "state": "locked" }));
    }

    /// Destroy everything on this device.
    pub async fn panic(&self) -> Result<(), ApiError> {
        let _g = self.lifecycle.lock().await;
        if let Some(s) = self.session.write().await.take() {
            s.engine.shutdown().await;
        }
        Vault::panic_wipe(&self.dir)?;
        self.emit(json!({ "type": "state", "state": "new" }));
        Ok(())
    }

    pub async fn save_settings(&self, s: &Session) -> Result<(), ApiError> {
        let settings = s.settings.lock().await.clone();
        s.main.put("settings", &settings)?;
        s.engine.set_cover_traffic(settings.risk_mode != RiskMode::Standard).await;
        Ok(())
    }
}

/// Background work: disappearing messages and auto-lock.
pub fn spawn_background(agent: Arc<Agent>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            let Some(s) = agent.session.read().await.clone() else { continue };
            let _ = s.engine.sweep_expired().await;
            let minutes = s.settings.lock().await.effective_lock_minutes() as u64;
            let idle = sotto_core::time::now().saturating_sub(agent.last_activity.load(Ordering::Relaxed));
            // Someone talking on a call isn't idle: don't lock under them.
            let on_call = !s.engine.calls().await.is_empty();
            if idle >= minutes * 60 && !on_call {
                drop(s);
                agent.lock().await;
            }
        }
    });
}
