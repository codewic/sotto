//! ML-KEM-768 (FIPS 203) wrapper. Secrets are stored as 64-byte seeds.

use ml_kem::{Decapsulate, DecapsulationKey, EncapsulationKey, KeyExport, MlKem768};
use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{Error, Result};

pub const KEM_PK_LEN: usize = 1184;
pub const KEM_CT_LEN: usize = 1088;

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct KemSecret {
    #[serde(with = "BigArray")]
    seed: [u8; 64],
}

impl KemSecret {
    pub fn generate() -> Self {
        Self { seed: crate::rand::bytes() }
    }

    pub fn from_seed(seed: [u8; 64]) -> Self {
        Self { seed }
    }

    fn key(&self) -> DecapsulationKey<MlKem768> {
        DecapsulationKey::<MlKem768>::from_seed(self.seed.into())
    }

    /// Encapsulation (public) key, `KEM_PK_LEN` bytes.
    pub fn public(&self) -> Vec<u8> {
        self.key().encapsulation_key().to_bytes().to_vec()
    }

    pub fn decapsulate(&self, ct: &[u8]) -> Result<[u8; 32]> {
        if ct.len() != KEM_CT_LEN {
            return Err(Error::Malformed("kem ciphertext length"));
        }
        let ct = ml_kem::Ciphertext::<MlKem768>::try_from(ct).map_err(|_| Error::Key)?;
        let ss = self.key().decapsulate(&ct);
        Ok(ss.into())
    }
}

/// Encapsulate to a public key: returns `(ciphertext, shared secret)`.
pub fn encapsulate(public: &[u8]) -> Result<(Vec<u8>, [u8; 32])> {
    if public.len() != KEM_PK_LEN {
        return Err(Error::Malformed("kem public key length"));
    }
    let key = ml_kem::Key::<EncapsulationKey<MlKem768>>::try_from(public).map_err(|_| Error::Key)?;
    let ek = EncapsulationKey::<MlKem768>::new(&key).map_err(|_| Error::Key)?;
    let m: [u8; 32] = crate::rand::bytes();
    let (ct, ss) = ek.encapsulate_deterministic(&m.into());
    Ok((ct.to_vec(), ss.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encapsulate_decapsulate() {
        let sk = KemSecret::generate();
        let pk = sk.public();
        assert_eq!(pk.len(), KEM_PK_LEN);
        let (ct, ss) = encapsulate(&pk).unwrap();
        assert_eq!(ct.len(), KEM_CT_LEN);
        assert_eq!(sk.decapsulate(&ct).unwrap(), ss);
        // A different key yields a different (implicitly rejected) secret.
        assert_ne!(KemSecret::generate().decapsulate(&ct).unwrap(), ss);
        assert!(encapsulate(&pk[..100]).is_err());
        assert!(sk.decapsulate(&ct[..100]).is_err());
    }

    #[test]
    fn seed_is_deterministic() {
        let a = KemSecret::from_seed([3u8; 64]);
        let b = KemSecret::from_seed([3u8; 64]);
        assert_eq!(a.public(), b.public());
    }
}
