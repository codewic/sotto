//! Formats shared by the drop server and the journalist's desktop agent.

use serde::{Deserialize, Serialize};
use sotto_core::keys::{Sig, sign, verify};
use sotto_core::seal::{SealPublic, SealSecret};
use sotto_core::{Error, Result, hex, kdf, unhex};

/// What a sealed submission contains before its content bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubmissionKind {
    Message,
    File { name: String, mime: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionMeta {
    pub kind: SubmissionKind,
    /// Days since the Unix epoch. The drop never records finer timing.
    pub day: u32,
}

/// `u32 LE meta length || postcard(meta) || content`.
pub fn submission_prefix(meta: &SubmissionMeta) -> Result<Vec<u8>> {
    let m = postcard::to_stdvec(meta)?;
    let mut out = (m.len() as u32).to_le_bytes().to_vec();
    out.extend(m);
    Ok(out)
}

/// Split an opened submission into its metadata and content.
pub fn parse_submission(plain: &[u8]) -> Result<(SubmissionMeta, &[u8])> {
    if plain.len() < 4 {
        return Err(Error::Malformed("submission"));
    }
    let len = u32::from_le_bytes(plain[..4].try_into().expect("4")) as usize;
    let meta_bytes = plain.get(4..4 + len).ok_or(Error::Malformed("submission"))?;
    Ok((postcard::from_bytes(meta_bytes)?, &plain[4 + len..]))
}

/// Replies from the newsroom are plain text sealed to the source's key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub day: u32,
    pub text: String,
}

pub fn today() -> u32 {
    (sotto_core::time::now() / 86_400) as u32
}

/// Derive a source's key pair from their codename. Slow on purpose (Argon2id).
pub fn source_secret(codename: &str, instance_id: &[u8; 16], m_kib: u32, t: u32) -> Result<SealSecret> {
    let mut salt = b"sotto-drop-v1".to_vec();
    salt.extend_from_slice(instance_id);
    let mut seed = zeroize::Zeroizing::new([0u8; 32]);
    kdf::argon2id(codename.as_bytes(), &salt, m_kib, t, &mut *seed)?;
    Ok(SealSecret::from_seed(&seed))
}

/// Public identifier of a source on this drop: derived from their public key, never from the codename directly.
pub fn source_id(public: &SealPublic) -> String {
    hex(&kdf::sha256(&public.dh)[..16])
}

/// Encode a newsroom public key for the drop configuration.
pub fn encode_key(public: &SealPublic) -> Result<String> {
    use base64::Engine;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(postcard::to_stdvec(public)?))
}

pub fn decode_key(s: &str) -> Result<SealPublic> {
    use base64::Engine;
    let b = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s.trim()).map_err(|_| Error::Malformed("key encoding"))?;
    Ok(postcard::from_bytes(&b)?)
}

/// Message a journalist signs for each API request.
pub fn auth_message(method: &str, path: &str, ts: u64, body: &[u8]) -> Vec<u8> {
    format!("sotto-drop-v1\n{method}\n{path}\n{ts}\n{}", hex(&kdf::sha256(body))).into_bytes()
}

/// `Sotto <public hex>.<unix seconds>.<signature hex>`
pub fn auth_header(seed: &[u8; 32], method: &str, path: &str, body: &[u8]) -> String {
    let ts = sotto_core::time::now();
    let public = sotto_core::keys::signing_public(seed);
    let sig = sign(seed, &auth_message(method, path, ts, body));
    format!("Sotto {}.{}.{}", hex(&public), ts, hex(&sig.0))
}

/// Verify a header; returns `(public key, signature bytes)` for replay tracking.
pub fn check_auth_header(header: &str, method: &str, path: &str, body: &[u8], now: u64, allowed: &[[u8; 32]]) -> Result<([u8; 32], [u8; 64])> {
    let rest = header.strip_prefix("Sotto ").ok_or(Error::Signature)?;
    let mut parts = rest.split('.');
    let (Some(pk), Some(ts), Some(sig), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(Error::Signature);
    };
    let pk: [u8; 32] = unhex(pk)?;
    let ts: u64 = ts.parse().map_err(|_| Error::Signature)?;
    let sig: [u8; 64] = unhex(sig)?;
    if !allowed.contains(&pk) {
        return Err(Error::Signature);
    }
    if ts + 300 < now || ts > now + 300 {
        return Err(Error::Expired);
    }
    verify(&pk, &auth_message(method, path, ts, body), &Sig(sig))?;
    Ok((pk, sig))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_round_trip() {
        let meta = SubmissionMeta { kind: SubmissionKind::File { name: "a.pdf".into(), mime: "application/pdf".into() }, day: 20_000 };
        let mut plain = submission_prefix(&meta).unwrap();
        plain.extend_from_slice(b"%PDF");
        let (m, content) = parse_submission(&plain).unwrap();
        assert_eq!(m, meta);
        assert_eq!(content, b"%PDF");
        assert!(parse_submission(&[1, 0]).is_err());
    }

    #[test]
    fn auth_header_checks() {
        let (seed, public) = sotto_core::keys::signing_keypair();
        let h = auth_header(&seed, "GET", "/j/v1/sources", b"");
        let now = sotto_core::time::now();
        assert!(check_auth_header(&h, "GET", "/j/v1/sources", b"", now, &[public]).is_ok());
        assert!(check_auth_header(&h, "GET", "/j/v1/other", b"", now, &[public]).is_err());
        assert!(check_auth_header(&h, "GET", "/j/v1/sources", b"", now + 1000, &[public]).is_err());
        assert!(check_auth_header(&h, "GET", "/j/v1/sources", b"", now, &[[0u8; 32]]).is_err());
    }

    #[test]
    fn codename_keys_are_stable_per_instance() {
        let a = source_secret("abandon ability", &[1; 16], 64, 1).unwrap().public();
        let b = source_secret("abandon ability", &[1; 16], 64, 1).unwrap().public();
        let c = source_secret("abandon ability", &[2; 16], 64, 1).unwrap().public();
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(source_id(&a).len(), 32);
        assert_eq!(decode_key(&encode_key(&a).unwrap()).unwrap(), a);
    }
}
