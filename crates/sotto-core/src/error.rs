/// Errors are deliberately coarse: callers should not be able to tell *why*
/// a decryption failed, only that it did.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("decryption failed")]
    Decrypt,
    #[error("invalid signature")]
    Signature,
    #[error("invalid key")]
    Key,
    #[error("malformed data: {0}")]
    Malformed(&'static str),
    #[error("expired")]
    Expired,
    #[error("data too large")]
    TooLarge,
    #[error("ratchet: {0}")]
    Ratchet(&'static str),
    #[error("encoding failed")]
    Encoding,
    #[error("io: {0}")]
    Io(String),
}

impl From<postcard::Error> for Error {
    fn from(_: postcard::Error) -> Self {
        Error::Encoding
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.kind().to_string())
    }
}

pub type Result<T> = core::result::Result<T, Error>;
