//! Identity keys: Ed25519 for signatures, X25519 for key agreement.

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{Error, Result};

/// A 64-byte Ed25519 signature with serde support.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sig(#[serde(with = "BigArray")] pub [u8; 64]);

impl core::fmt::Debug for Sig {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Sig({}..)", crate::hex(&self.0[..4]))
    }
}

/// The public half of an identity, safe to share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PublicIdentity {
    pub sign: [u8; 32],
    pub dh: [u8; 32],
}

impl PublicIdentity {
    /// Short fingerprint for display and directory lookups: hex of SHA-256 over both keys.
    pub fn fingerprint(&self) -> String {
        let mut data = Vec::with_capacity(64);
        data.extend_from_slice(&self.sign);
        data.extend_from_slice(&self.dh);
        crate::hex(&crate::kdf::sha256(&data)[..16])
    }
}

/// A long-term identity held only on the device.
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Identity {
    sign_seed: [u8; 32],
    dh_secret: [u8; 32],
}

impl Identity {
    pub fn generate() -> Self {
        Self { sign_seed: crate::rand::bytes(), dh_secret: crate::rand::bytes() }
    }

    /// Deterministic identity from a 32-byte seed (used for codename-derived drop keys).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            sign_seed: crate::kdf::hkdf32(b"sotto-identity", seed, b"sign"),
            dh_secret: crate::kdf::hkdf32(b"sotto-identity", seed, b"dh"),
        }
    }

    pub fn public(&self) -> PublicIdentity {
        PublicIdentity { sign: signing_public(&self.sign_seed), dh: x25519_public(&self.dh_secret) }
    }

    pub fn sign(&self, msg: &[u8]) -> Sig {
        sign(&self.sign_seed, msg)
    }

    pub fn dh(&self, their: &[u8; 32]) -> Result<[u8; 32]> {
        x25519(&self.dh_secret, their)
    }
}

/// Generate a fresh Ed25519 key, returning `(seed, public)`.
pub fn signing_keypair() -> ([u8; 32], [u8; 32]) {
    let seed = crate::rand::bytes();
    let public = signing_public(&seed);
    (seed, public)
}

pub fn signing_public(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

pub fn sign(seed: &[u8; 32], msg: &[u8]) -> Sig {
    Sig(SigningKey::from_bytes(seed).sign(msg).to_bytes())
}

/// Strict Ed25519 verification (rejects malleable and small-order encodings).
pub fn verify(public: &[u8; 32], msg: &[u8], sig: &Sig) -> Result<()> {
    let key = VerifyingKey::from_bytes(public).map_err(|_| Error::Key)?;
    key.verify_strict(msg, &Signature::from_bytes(&sig.0)).map_err(|_| Error::Signature)
}

pub fn x25519_public(secret: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

/// X25519 that rejects non-contributory results (all-zero shared secrets).
pub fn x25519(secret: &[u8; 32], their: &[u8; 32]) -> Result<[u8; 32]> {
    let shared = StaticSecret::from(*secret).diffie_hellman(&PublicKey::from(*their));
    if !shared.was_contributory() {
        return Err(Error::Key);
    }
    Ok(shared.to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_dh() {
        let a = Identity::generate();
        let b = Identity::generate();
        let sig = a.sign(b"msg");
        verify(&a.public().sign, b"msg", &sig).unwrap();
        assert_eq!(verify(&a.public().sign, b"other", &sig), Err(Error::Signature));
        assert_eq!(verify(&b.public().sign, b"msg", &sig), Err(Error::Signature));
        assert_eq!(a.dh(&b.public().dh).unwrap(), b.dh(&a.public().dh).unwrap());
        assert_eq!(a.dh(&[0u8; 32]), Err(Error::Key));
    }

    #[test]
    fn seeded_identity_is_deterministic() {
        let seed = [7u8; 32];
        assert_eq!(Identity::from_seed(&seed).public(), Identity::from_seed(&seed).public());
        assert_ne!(Identity::from_seed(&seed).public(), Identity::from_seed(&[8u8; 32]).public());
        assert_eq!(Identity::from_seed(&seed).public().fingerprint().len(), 32);
    }
}
