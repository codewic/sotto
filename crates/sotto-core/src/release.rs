//! Signed releases. A release's manifest (`latest.json`) is signed with the release key, which
//! lives offline with the owner; its public half is pinned here, so neither GitHub nor CI can
//! publish an update the app will install.

use crate::keys::{Sig, verify};

/// The release key's public half (Ed25519). Changing it is a deliberate, reviewed act.
pub const RELEASE_PUBLIC_KEY: &str = "7f1ed181be7057fd7cc171dadafff768aeda09c5f7ddbba2839e21a65f518a3d";

/// What the signature covers: a fixed label, then the manifest's exact bytes.
pub fn signed_message(manifest: &[u8]) -> Vec<u8> {
    let mut m = b"sotto-release-manifest-v1\n".to_vec();
    m.extend_from_slice(manifest);
    m
}

/// Check `sig_hex` (128 hex characters) over `manifest` with `public` (32 bytes).
pub fn verify_manifest(manifest: &[u8], sig_hex: &str, public: &[u8; 32]) -> bool {
    let Ok(sig) = crate::unhex::<64>(sig_hex.trim()) else { return false };
    verify(public, &signed_message(manifest), &Sig(sig)).is_ok()
}

/// The pinned key as bytes.
pub fn pinned_key() -> Option<[u8; 32]> {
    crate::unhex::<32>(RELEASE_PUBLIC_KEY).ok()
}

/// `0.10.2` > `0.9.9`. Anything that isn't three numbers compares as 0.0.0, so it never wins.
pub fn version_tuple(v: &str) -> (u64, u64, u64) {
    let mut p = v.trim().trim_start_matches('v').split('.').map(|x| x.parse::<u64>().ok());
    match (p.next().flatten(), p.next().flatten(), p.next().flatten(), p.next()) {
        (Some(a), Some(b), Some(c), None) => (a, b, c),
        _ => (0, 0, 0),
    }
}

pub fn newer(candidate: &str, current: &str) -> bool {
    version_tuple(candidate) > version_tuple(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{sign, signing_public};

    #[test]
    fn signatures_and_versions() {
        let seed = [7u8; 32];
        let public = signing_public(&seed);
        let manifest = br#"{"version":"0.2.0"}"#;
        let sig = crate::hex(&sign(&seed, &signed_message(manifest)).0);
        assert!(verify_manifest(manifest, &sig, &public));
        assert!(!verify_manifest(br#"{"version":"9.9.9"}"#, &sig, &public), "a changed manifest fails");
        assert!(!verify_manifest(manifest, &sig, &signing_public(&[8u8; 32])), "another key fails");
        assert!(newer("0.10.0", "0.9.9") && !newer("0.2.0", "0.2.0") && !newer("0.1.9", "0.2.0"));
        assert!(!newer("1.0", "0.1.0") && !newer("0.3.0-rc1", "0.2.0"), "only plain x.y.z counts");
    }
}
