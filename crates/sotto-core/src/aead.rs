//! XChaCha20-Poly1305 helpers.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use crate::{Error, Result};

pub const TAG_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;

fn cipher(key: &[u8; 32]) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(&(*key).into())
}

pub fn encrypt(key: &[u8; 32], nonce: &[u8; 24], pt: &[u8], aad: &[u8]) -> Vec<u8> {
    let nonce: XNonce = (*nonce).into();
    cipher(key)
        .encrypt(&nonce, Payload { msg: pt, aad })
        .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory buffers")
}

pub fn decrypt(key: &[u8; 32], nonce: &[u8; 24], ct: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    let nonce: XNonce = (*nonce).into();
    cipher(key)
        .decrypt(&nonce, Payload { msg: ct, aad })
        .map_err(|_| Error::Decrypt)
}

/// Encrypt with a fresh random nonce, returned as `nonce || ciphertext`.
pub fn seal_random(key: &[u8; 32], pt: &[u8], aad: &[u8]) -> Vec<u8> {
    let nonce: [u8; 24] = crate::rand::bytes();
    let mut out = nonce.to_vec();
    out.extend(encrypt(key, &nonce, pt, aad));
    out
}

/// Inverse of [`seal_random`].
pub fn open_random(key: &[u8; 32], data: &[u8], aad: &[u8]) -> Result<Vec<u8>> {
    if data.len() < NONCE_LEN + TAG_LEN {
        return Err(Error::Decrypt);
    }
    let (nonce, ct) = data.split_at(NONCE_LEN);
    let nonce: [u8; 24] = nonce.try_into().map_err(|_| Error::Decrypt)?;
    decrypt(key, &nonce, ct, aad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper() {
        let key = crate::rand::bytes();
        let ct = seal_random(&key, b"hello", b"ad");
        assert_eq!(open_random(&key, &ct, b"ad").unwrap(), b"hello");
        assert_eq!(open_random(&key, &ct, b"other"), Err(Error::Decrypt));
        let mut bad = ct.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(open_random(&key, &bad, b"ad"), Err(Error::Decrypt));
        assert_eq!(open_random(&key, &ct[..10], b"ad"), Err(Error::Decrypt));
    }
}
