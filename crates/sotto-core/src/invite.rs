//! One-time invites: the only way two Sotto users meet.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::kem::{KEM_PK_LEN, KemSecret};
use crate::keys::{Identity, PublicIdentity, Sig, verify, x25519_public};
use crate::{Error, Id, QueueAddr, Result};

pub const LINK_PREFIX: &str = "sotto://invite/";
const MAX_LINK_LEN: usize = 8192;
const MAX_LABEL_LEN: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub version: u8,
    pub identity: PublicIdentity,
    pub ephemeral: [u8; 32],
    pub kem_pk: Vec<u8>,
    pub queue: QueueAddr,
    pub label: String,
    pub expires_at: u64,
    pub sig: Sig,
}

/// What the inviter keeps until the invite is used (then deletes).
#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct InviteSecret {
    pub ephemeral: [u8; 32],
    pub kem: KemSecret,
}

#[derive(Serialize)]
struct Signed<'a> {
    version: u8,
    identity: &'a PublicIdentity,
    ephemeral: &'a [u8; 32],
    kem_pk: &'a [u8],
    queue: &'a QueueAddr,
    label: &'a str,
    expires_at: u64,
}

impl Invite {
    pub fn create(identity: &Identity, queue: QueueAddr, label: &str, ttl_secs: u64) -> Result<(Invite, InviteSecret)> {
        let label: String = label.chars().take(MAX_LABEL_LEN).collect();
        let ephemeral: [u8; 32] = crate::rand::bytes();
        let kem = KemSecret::generate();
        let mut invite = Invite {
            version: 1,
            identity: identity.public(),
            ephemeral: x25519_public(&ephemeral),
            kem_pk: kem.public(),
            queue,
            label,
            expires_at: crate::time::now() + ttl_secs,
            sig: crate::keys::Sig([0u8; 64]),
        };
        invite.sig = identity.sign(&invite.signed_bytes()?);
        Ok((invite, InviteSecret { ephemeral, kem }))
    }

    fn signed_bytes(&self) -> Result<Vec<u8>> {
        let mut out = b"sotto-invite-v1".to_vec();
        out.extend(postcard::to_stdvec(&Signed {
            version: self.version,
            identity: &self.identity,
            ephemeral: &self.ephemeral,
            kem_pk: &self.kem_pk,
            queue: &self.queue,
            label: &self.label,
            expires_at: self.expires_at,
        })?);
        Ok(out)
    }

    /// Check structure, signature and expiry.
    pub fn verify(&self, now: u64) -> Result<()> {
        if self.version != 1 {
            return Err(Error::Malformed("unsupported invite version"));
        }
        if self.kem_pk.len() != KEM_PK_LEN || self.label.chars().count() > MAX_LABEL_LEN {
            return Err(Error::Malformed("invite fields"));
        }
        verify(&self.identity.sign, &self.signed_bytes()?, &self.sig)?;
        if now > self.expires_at {
            return Err(Error::Expired);
        }
        Ok(())
    }

    pub fn id(&self) -> Id {
        id_for(&self.ephemeral)
    }

    pub fn to_link(&self) -> Result<String> {
        Ok(format!("{LINK_PREFIX}{}", URL_SAFE_NO_PAD.encode(postcard::to_stdvec(self)?)))
    }

    /// Accepts a full link or just the encoded part; whitespace (from copying out of a QR reader or email) is ignored.
    pub fn from_link(input: &str) -> Result<Invite> {
        let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.len() > MAX_LINK_LEN {
            return Err(Error::TooLarge);
        }
        let encoded = compact.strip_prefix(LINK_PREFIX).unwrap_or(&compact);
        let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| Error::Malformed("invite encoding"))?;
        Ok(postcard::from_bytes(&bytes)?)
    }
}

/// Invite id from the invite's one-time public key.
pub fn id_for(ephemeral_public: &[u8; 32]) -> Id {
    let d = crate::kdf::sha256(ephemeral_public);
    d[..16].try_into().expect("16 bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> QueueAddr {
        QueueAddr { relay: "ws://relay.onion".into(), snd_id: [9u8; 16] }
    }

    #[test]
    fn link_round_trip_and_verify() {
        let id = Identity::generate();
        let (inv, secret) = Invite::create(&id, queue(), "Ada, Outlet", 3600).unwrap();
        let link = inv.to_link().unwrap();
        assert!(link.starts_with(LINK_PREFIX));
        let wrapped = format!("  {}\n{}  ", &link[..40], &link[40..]);
        let parsed = Invite::from_link(&wrapped).unwrap();
        assert_eq!(parsed, inv);
        parsed.verify(crate::time::now()).unwrap();
        assert_eq!(parsed.id(), id_for(&x25519_public(&secret.ephemeral)));
    }

    #[test]
    fn rejects_tampering_and_expiry() {
        let id = Identity::generate();
        let (inv, _) = Invite::create(&id, queue(), "x", 60).unwrap();
        let mut bad = inv.clone();
        bad.queue.relay = "ws://evil.onion".into();
        assert_eq!(bad.verify(crate::time::now()), Err(Error::Signature));
        assert_eq!(inv.verify(inv.expires_at + 1), Err(Error::Expired));
        assert!(Invite::from_link("sotto://invite/!!!").is_err());
        let long = "a".repeat(MAX_LINK_LEN + 1);
        assert_eq!(Invite::from_link(&long), Err(Error::TooLarge));
    }
}
