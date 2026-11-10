//! Outlet directories: `https://<outlet>/.well-known/sotto.json`, signed by the outlet's key,
//! listing reporters' identity fingerprints and the outlet's drops. A source checks a journalist
//! against the outlet's own website before trusting them.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sotto_core::keys::{Sig, sign, signing_public, verify};
use sotto_core::{hex, unhex};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DirectoryError {
    #[error("this isn't a Sotto directory file")]
    Malformed,
    #[error("the directory's signature doesn't check out")]
    BadSignature,
    #[error("this directory belongs to {0}, not the outlet you expected")]
    WrongDomain(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reporter {
    pub name: String,
    /// `PublicIdentity::fingerprint()` of the reporter's listed identity.
    pub fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropEntry {
    pub name: String,
    pub onion: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directory {
    pub version: u8,
    pub outlet: String,
    pub domain: String,
    pub outlet_key: String,
    pub issued_at: u64,
    pub reporters: Vec<Reporter>,
    pub drops: Vec<DropEntry>,
    #[serde(default)]
    pub signature: String,
}

impl Directory {
    fn signed_bytes(&self) -> Vec<u8> {
        let mut unsigned = self.clone();
        unsigned.signature.clear();
        let mut m = b"sotto-directory-v1".to_vec();
        m.extend(postcard::to_stdvec(&unsigned).expect("serializable"));
        m
    }

    /// Sign with the outlet's Ed25519 seed; sets `outlet_key` and `signature`.
    pub fn sign(mut self, seed: &[u8; 32]) -> Directory {
        self.version = 1;
        self.domain = self.domain.trim().to_lowercase();
        self.outlet_key = hex(&signing_public(seed));
        self.signature.clear();
        self.signature = hex(&sign(seed, &self.signed_bytes()).0);
        self
    }

    /// Parse and verify a directory file fetched from `expected_domain`.
    pub fn verify(json: &str, expected_domain: &str) -> Result<Directory, DirectoryError> {
        let d: Directory = serde_json::from_str(json).map_err(|_| DirectoryError::Malformed)?;
        if d.version != 1 {
            return Err(DirectoryError::Malformed);
        }
        let key: [u8; 32] = unhex(&d.outlet_key).map_err(|_| DirectoryError::Malformed)?;
        let sig: [u8; 64] = unhex(&d.signature).map_err(|_| DirectoryError::Malformed)?;
        verify(&key, &d.signed_bytes(), &Sig(sig)).map_err(|_| DirectoryError::BadSignature)?;
        let want = expected_domain.trim().trim_start_matches("https://").trim_end_matches('/').to_lowercase();
        if d.domain != want {
            return Err(DirectoryError::WrongDomain(d.domain.clone()));
        }
        Ok(d)
    }

    pub fn reporter(&self, fingerprint: &str) -> Option<&Reporter> {
        self.reporters.iter().find(|r| r.fingerprint.eq_ignore_ascii_case(fingerprint))
    }

    /// Where an outlet publishes this file.
    pub fn url_for(domain: &str) -> String {
        format!("https://{}/.well-known/sotto.json", domain.trim().trim_start_matches("https://").trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Directory {
        Directory {
            version: 1,
            outlet: "The Ledger".into(),
            domain: "TheLedger.example".into(),
            outlet_key: String::new(),
            issued_at: 1_790_000_000,
            reporters: vec![Reporter { name: "Ada Mensah".into(), fingerprint: "e905fd25db1945e605a6957ea4e996cc".into() }],
            drops: vec![DropEntry { name: "Tips".into(), onion: "http://abc.onion".into() }],
            signature: String::new(),
        }
    }

    #[test]
    fn sign_verify_and_lookup() {
        let seed = [4u8; 32];
        let json = serde_json::to_string(&draft().sign(&seed)).unwrap();
        let d = Directory::verify(&json, "https://theledger.example/").unwrap();
        assert_eq!(d.reporter("E905FD25DB1945E605A6957EA4E996CC").unwrap().name, "Ada Mensah");
        assert!(d.reporter("00").is_none());
        assert_eq!(Directory::verify(&json, "evil.example"), Err(DirectoryError::WrongDomain("theledger.example".into())));
        // Adding a reporter without re-signing is detected.
        let mut forged: Directory = serde_json::from_str(&json).unwrap();
        forged.reporters.push(Reporter { name: "Impostor".into(), fingerprint: "11".repeat(16) });
        assert_eq!(Directory::verify(&serde_json::to_string(&forged).unwrap(), "theledger.example"), Err(DirectoryError::BadSignature));
        assert_eq!(Directory::verify("{}", "x"), Err(DirectoryError::Malformed));
        assert_eq!(Directory::url_for("theledger.example"), "https://theledger.example/.well-known/sotto.json");
    }
}
