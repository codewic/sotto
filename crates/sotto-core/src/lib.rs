//! Sotto core: cryptography, handshake, ratchet, sealed boxes and wire formats.
//!
//! Everything in this crate is pure computation with no I/O beyond reading OS
//! randomness, so it can be shared by the relay, the drop server, the desktop
//! agent and the mobile bindings. The design is documented in `docs/protocol.md`.

#![forbid(unsafe_code)]

pub mod aead;
pub mod codename;
pub mod envelope;
pub mod error;
pub mod invite;
pub mod kdf;
pub mod kem;
pub mod keys;
pub mod pad;
pub mod pqxdh;
pub mod rand;
pub mod ratchet;
pub mod release;
pub mod safety;
pub mod seal;
pub mod time;
pub mod wire;

pub use error::{Error, Result};

/// 16-byte random identifier used for queues, files, messages and cases.
pub type Id = [u8; 16];

/// Address of a queue as seen by a sender.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QueueAddr {
    /// Relay URL, for example `ws://abc...xyz.onion` or `ws://127.0.0.1:7400` in tests.
    pub relay: String,
    /// Send-side id of the queue. Knowing it lets you write, never read.
    pub snd_id: Id,
}

/// Lowercase hex, used for ids in storage keys.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    s
}

/// Parse hex into a fixed array.
pub fn unhex<const N: usize>(s: &str) -> Result<[u8; N]> {
    let s = s.as_bytes();
    if s.len() != N * 2 {
        return Err(Error::Malformed("hex length"));
    }
    let mut out = [0u8; N];
    for (i, pair) in s.chunks(2).enumerate() {
        let hi = (pair[0] as char).to_digit(16).ok_or(Error::Malformed("hex"))?;
        let lo = (pair[1] as char).to_digit(16).ok_or(Error::Malformed("hex"))?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Ok(out)
}

/// A fresh random id.
pub fn new_id() -> Id {
    rand::bytes()
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_round_trip() {
        let id: [u8; 4] = [0, 15, 160, 255];
        assert_eq!(super::hex(&id), "000fa0ff");
        assert_eq!(super::unhex::<4>("000fa0ff").unwrap(), id);
        assert!(super::unhex::<4>("000fa0f").is_err());
        assert!(super::unhex::<4>("000fa0fz").is_err());
    }
}
