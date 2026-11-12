//! Mobile bindings (UniFFI) for iOS and Android: the same engine and vault as the desktop app,
//! behind a small surface. Lists come back as JSON strings so the native UI layer stays thin.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sotto_client::{Engine, EngineConfig, Transport};
use sotto_core::unhex;
use sotto_vault::{Params, Vault};

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SottoError {
    #[error("{message}")]
    Failed { message: String },
}

fn err(e: impl std::fmt::Display) -> SottoError {
    SottoError::Failed { message: e.to_string() }
}

/// What a user sees before accepting an invite.
#[derive(uniffi::Record)]
pub struct InviteInfo {
    pub label: String,
    pub fingerprint: String,
    pub relay: String,
    pub expires_at: u64,
}

#[uniffi::export]
pub fn generate_codename() -> String {
    sotto_core::codename::generate()
}

#[uniffi::export]
pub fn safety_number(a_sign_hex: String, b_sign_hex: String) -> Result<String, SottoError> {
    let a: [u8; 32] = unhex(&a_sign_hex).map_err(err)?;
    let b: [u8; 32] = unhex(&b_sign_hex).map_err(err)?;
    Ok(sotto_core::safety::safety_number(&a, &b))
}

#[uniffi::export]
pub fn inspect_invite(link: String) -> Result<InviteInfo, SottoError> {
    let inv = sotto_core::invite::Invite::from_link(&link).map_err(err)?;
    inv.verify(sotto_core::time::now()).map_err(err)?;
    Ok(InviteInfo { label: inv.label.clone(), fingerprint: inv.identity.fingerprint(), relay: inv.queue.relay.clone(), expires_at: inv.expires_at })
}

/// Receives live events (the same JSON the desktop agent streams).
#[uniffi::export(callback_interface)]
pub trait EventListener: Send + Sync {
    fn on_event(&self, json: String);
}

struct Open {
    _vault: Vault,
    engine: Engine,
}

#[derive(uniffi::Object)]
pub struct SottoClient {
    rt: tokio::runtime::Runtime,
    dir: PathBuf,
    relay: String,
    transport: Transport,
    open: Mutex<Option<Open>>,
    listener: Mutex<Option<Arc<dyn EventListener>>>,
}

#[uniffi::export]
impl SottoClient {
    /// `socks` is the embedded Tor SOCKS address (for example from Orbot or Tor for iOS); `None` connects directly (testing only).
    #[uniffi::constructor]
    pub fn new(data_dir: String, relay: String, socks: Option<String>) -> Result<Arc<Self>, SottoError> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().map_err(err)?;
        let transport = socks.map(|s| Transport::Tor { socks: s }).unwrap_or(Transport::Direct);
        Ok(Arc::new(Self { rt, dir: PathBuf::from(data_dir).join("vault"), relay, transport, open: Mutex::new(None), listener: Mutex::new(None) }))
    }

    pub fn state(&self) -> String {
        if self.open.lock().map(|o| o.is_some()).unwrap_or(false) {
            "unlocked".into()
        } else if Vault::exists(&self.dir) {
            "locked".into()
        } else {
            "new".into()
        }
    }

    pub fn setup(&self, passcode: String) -> Result<(), SottoError> {
        if passcode.chars().count() < 8 {
            return Err(err("Use at least 8 characters for your passcode."));
        }
        let vault = Vault::create(&self.dir, &passcode, None, Params::default()).map_err(err)?;
        self.start(vault)
    }

    pub fn unlock(&self, passcode: String) -> Result<(), SottoError> {
        let (vault, _) = Vault::unlock(&self.dir, &passcode).map_err(err)?;
        self.start(vault)
    }

    pub fn lock(&self) {
        if let Some(o) = self.open.lock().ok().and_then(|mut g| g.take()) {
            self.rt.block_on(o.engine.shutdown());
        }
    }

    pub fn panic_wipe(&self) -> Result<(), SottoError> {
        self.lock();
        Vault::panic_wipe(&self.dir).map_err(err)
    }

    pub fn set_listener(&self, listener: Box<dyn EventListener>) {
        if let Ok(mut l) = self.listener.lock() {
            *l = Some(Arc::from(listener));
        }
    }

    pub fn contacts_json(&self) -> Result<String, SottoError> {
        let e = self.engine()?;
        serde_json::to_string(&self.rt.block_on(e.contacts())).map_err(err)
    }

    pub fn messages_json(&self, contact_id: String) -> Result<String, SottoError> {
        let e = self.engine()?;
        let id: [u8; 16] = unhex(&contact_id).map_err(err)?;
        serde_json::to_string(&e.messages(&id, 500).map_err(err)?).map_err(err)
    }

    pub fn send_text(&self, contact_id: String, text: String) -> Result<(), SottoError> {
        let e = self.engine()?;
        let id: [u8; 16] = unhex(&contact_id).map_err(err)?;
        self.rt.block_on(e.send_text(&id, &text, None)).map(|_| ()).map_err(err)
    }

    pub fn create_invite(&self, nickname: String, label: String) -> Result<String, SottoError> {
        let e = self.engine()?;
        Ok(self.rt.block_on(e.create_invite(&nickname, &label, false)).map_err(err)?.1)
    }

    pub fn accept_invite(&self, link: String, nickname: String) -> Result<String, SottoError> {
        let e = self.engine()?;
        Ok(sotto_core::hex(&self.rt.block_on(e.accept_invite(&link, &nickname, "")).map_err(err)?))
    }

    pub fn mark_verified(&self, contact_id: String, verified: bool) -> Result<(), SottoError> {
        let e = self.engine()?;
        let id: [u8; 16] = unhex(&contact_id).map_err(err)?;
        self.rt.block_on(e.set_verified(&id, verified)).map_err(err)
    }
}

impl SottoClient {
    fn engine(&self) -> Result<Engine, SottoError> {
        self.open.lock().map_err(err)?.as_ref().map(|o| o.engine.clone()).ok_or_else(|| err("Sotto is locked."))
    }

    fn start(&self, vault: Vault) -> Result<(), SottoError> {
        let key: [u8; 32] = match vault.store().get::<[u8; 32]>("mobile-engine-scope").map_err(err)? {
            Some(k) => k,
            None => {
                let k = sotto_core::rand::bytes();
                vault.store().put("mobile-engine-scope", &k).map_err(err)?;
                k
            }
        };
        let store = Arc::new(vault.scope(key).map_err(err)?);
        let engine =
            self.rt.block_on(Engine::start(store, EngineConfig { default_relay: self.relay.clone(), transport: self.transport.clone() })).map_err(err)?;
        let mut events = engine.events();
        let listener = self.listener.lock().map_err(err)?.clone();
        self.rt.spawn(async move {
            while let Ok(e) = events.recv().await {
                if let (Some(l), Ok(json)) = (&listener, serde_json::to_string(&e)) {
                    l.on_event(json);
                }
            }
        });
        *self.open.lock().map_err(err)? = Some(Open { _vault: vault, engine });
        Ok(())
    }
}
