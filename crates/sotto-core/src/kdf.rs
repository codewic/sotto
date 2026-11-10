//! Hashing, HMAC and HKDF helpers.

use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256, Sha512};

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn sha512(data: &[u8]) -> [u8; 64] {
    Sha512::digest(data).into()
}

pub fn hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(data);
    m.finalize().into_bytes().into()
}

/// Constant-time HMAC verification.
pub fn hmac_verify(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
    let mut m = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(data);
    m.verify_slice(tag).is_ok()
}

/// HKDF-SHA-256 extract-and-expand into `out`.
pub fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) {
    Hkdf::<Sha256>::new(Some(salt), ikm)
        .expand(info, out)
        .expect("HKDF output length is within limits");
}

pub fn hkdf32(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    hkdf(salt, ikm, info, &mut out);
    out
}

/// Argon2id v1.3 with parallelism 1. `m_kib` is memory in KiB.
pub fn argon2id(password: &[u8], salt: &[u8], m_kib: u32, t: u32, out: &mut [u8]) -> crate::Result<()> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(m_kib, t, 1, Some(out.len())).map_err(|_| crate::Error::Malformed("argon2 params"))?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, out)
        .map_err(|_| crate::Error::Malformed("argon2 input"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2id_is_deterministic() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        argon2id(b"pass", b"saltsaltsaltsalt", 64, 1, &mut a).unwrap();
        argon2id(b"pass", b"saltsaltsaltsalt", 64, 1, &mut b).unwrap();
        assert_eq!(a, b);
        argon2id(b"pass2", b"saltsaltsaltsalt", 64, 1, &mut b).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn known_answers() {
        // RFC 4231 test case 2.
        let tag = hmac(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            crate::hex(&tag),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert!(hmac_verify(b"Jefe", b"what do ya want for nothing?", &tag));
        assert!(!hmac_verify(b"Jefe", b"what do ya want for nothing!", &tag));
        assert_eq!(
            crate::hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hkdf_rfc5869_case1() {
        let ikm = [0x0bu8; 22];
        let salt: Vec<u8> = (0u8..=0x0c).collect();
        let info: Vec<u8> = (0xf0u8..=0xf9).collect();
        let mut okm = [0u8; 42];
        hkdf(&salt, &ikm, &info, &mut okm);
        assert_eq!(
            crate::hex(&okm),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
        );
    }
}
