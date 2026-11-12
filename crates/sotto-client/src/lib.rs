//! Sotto client engine: relay connections, contacts, sessions, messages and files.
//!
//! The engine owns no UI. The desktop agent and the mobile bindings drive it
//! and render its [`Event`]s.

#![forbid(unsafe_code)]

pub mod calls;
pub mod conn;
pub mod engine;
pub mod files;
pub mod model;

pub use calls::{CallView, JoinInfo, RoomGrant};
pub use conn::{RelayConn, Status, Transport};
pub use engine::{Engine, EngineConfig, Event};
pub use model::*;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] sotto_core::Error),
    #[error(transparent)]
    Vault(#[from] sotto_vault::VaultError),
    #[error("The relay can't be reached right now. Sotto keeps retrying in the background.")]
    Offline,
    #[error("The relay refused the request ({0:?}).")]
    Relay(sotto_core::wire::ErrCode),
    #[error("That relay address isn't valid. It should look like ws://example.onion")]
    BadRelay,
    #[error("Not found.")]
    NotFound,
    #[error("This contact hasn't accepted your invite yet.")]
    NotReady,
    #[error("The relay sent an unexpected reply.")]
    BadResponse,
    #[error("The file didn't arrive intact and was discarded.")]
    Integrity,
    #[error("This is one of your own invites. Send it to the other person instead.")]
    OwnInvite,
    #[error("{0}")]
    Invalid(&'static str),
}

pub type Result<T> = core::result::Result<T, Error>;
