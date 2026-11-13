//! Sotto desktop agent: owns the vault and engine, serves the UI over a local,
//! token-authenticated API, and pushes live updates over server-sent events.

#![forbid(unsafe_code)]

pub mod api;
pub mod avatars;
pub mod calls;
pub mod cases;
pub mod channels;
pub mod drops;
pub mod groups;
pub mod media;
pub mod photos;
pub mod hooks;
pub mod review;
pub mod scheduled;
pub mod secrets;
pub mod shared;
pub mod state;
pub mod tags;
pub mod travel;
pub mod updates;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub use api::router;
pub use state::{Agent, spawn_background};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("Sotto is locked.")]
    Locked,
    #[error("That passcode isn't right.")]
    WrongPasscode,
    #[error("Not found.")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("Something went wrong inside Sotto.")]
    Internal,
}

impl From<sotto_vault::VaultError> for ApiError {
    fn from(e: sotto_vault::VaultError) -> Self {
        match e {
            sotto_vault::VaultError::WrongPasscode => ApiError::WrongPasscode,
            sotto_vault::VaultError::Missing => ApiError::NotFound,
            sotto_vault::VaultError::SamePasscode | sotto_vault::VaultError::Exists => ApiError::Invalid(e.to_string()),
            _ => ApiError::Internal,
        }
    }
}

impl From<sotto_client::Error> for ApiError {
    fn from(e: sotto_client::Error) -> Self {
        use sotto_client::Error as E;
        match e {
            E::Relay(sotto_core::wire::ErrCode::NotFound) => {
                ApiError::Invalid("This conversation's mailbox no longer exists on the relay. Ask them for a new invite.".into())
            }
            E::Offline | E::Relay(_) | E::BadResponse => ApiError::Unavailable(e.to_string()),
            E::NotFound => ApiError::NotFound,
            E::Vault(v) => v.into(),
            E::Core(sotto_core::Error::Expired) => ApiError::Invalid("This invite has expired. Ask for a new one.".into()),
            E::Core(sotto_core::Error::Signature) | E::Core(sotto_core::Error::Malformed(_)) | E::Core(sotto_core::Error::Encoding) => {
                ApiError::Invalid("That doesn't look like a valid Sotto invite. Copy the whole link and try again.".into())
            }
            other => ApiError::Invalid(other.to_string()),
        }
    }
}

impl From<sotto_core::Error> for ApiError {
    fn from(e: sotto_core::Error) -> Self {
        sotto_client::Error::Core(e).into()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            ApiError::Locked => StatusCode::LOCKED,
            ApiError::WrongPasscode => StatusCode::UNAUTHORIZED,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(serde_json::json!({ "error": self.to_string() }))).into_response()
    }
}
