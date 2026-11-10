//! PQXDH handshake adapted to one-time invites (see `docs/protocol.md` section 4).

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::invite::{Invite, InviteSecret, id_for};
use crate::kem::{KEM_CT_LEN, encapsulate};
use crate::keys::{Identity, PublicIdentity, Sig, verify, x25519, x25519_public};
use crate::ratchet::Ratchet;
use crate::{Error, Id, Result};

const INFO: &[u8] = b"Sotto_PQXDH_X25519_SHA-256_ML-KEM-768_v1";

/// Sent by the joiner in the first envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinHeader {
    pub invite_id: Id,
    pub identity: PublicIdentity,
    pub ephemeral: [u8; 32],
    pub kem_ct: Vec<u8>,
    pub sig: Sig,
}

/// Associated data binding both identities into every ratchet message.
pub fn associated_data(inviter: &PublicIdentity, joiner: &PublicIdentity) -> Vec<u8> {
    let mut ad = b"sotto-ad-v1".to_vec();
    for k in [&inviter.sign, &inviter.dh, &joiner.sign, &joiner.dh] {
        ad.extend_from_slice(k);
    }
    ad
}

fn transcript(invite_id: &Id, identity_dh: &[u8; 32], ephemeral: &[u8; 32], kem_ct: &[u8]) -> Vec<u8> {
    let mut t = b"sotto-join-v1".to_vec();
    t.extend_from_slice(invite_id);
    t.extend_from_slice(identity_dh);
    t.extend_from_slice(ephemeral);
    t.extend_from_slice(kem_ct);
    t
}

fn derive(dh1: [u8; 32], dh2: [u8; 32], dh3: [u8; 32], ss: [u8; 32]) -> [u8; 32] {
    let mut ikm = Vec::with_capacity(160);
    ikm.extend_from_slice(&[0xffu8; 32]);
    ikm.extend_from_slice(&dh1);
    ikm.extend_from_slice(&dh2);
    ikm.extend_from_slice(&dh3);
    ikm.extend_from_slice(&ss);
    let sk = crate::kdf::hkdf32(&[0u8; 32], &ikm, INFO);
    ikm.zeroize();
    sk
}

/// Joiner side. The invite must already be verified by the caller.
pub fn join(me: &Identity, invite: &Invite) -> Result<(JoinHeader, Ratchet)> {
    let ek: [u8; 32] = crate::rand::bytes();
    let ek_pub = x25519_public(&ek);
    let dh1 = me.dh(&invite.ephemeral)?;
    let dh2 = x25519(&ek, &invite.identity.dh)?;
    let dh3 = x25519(&ek, &invite.ephemeral)?;
    let (kem_ct, ss) = encapsulate(&invite.kem_pk)?;
    let sk = derive(dh1, dh2, dh3, ss);
    let my_pub = me.public();
    let invite_id = invite.id();
    let sig = me.sign(&transcript(&invite_id, &my_pub.dh, &ek_pub, &kem_ct));
    let ratchet = Ratchet::init_sender(sk, invite.ephemeral, associated_data(&invite.identity, &my_pub))?;
    Ok((JoinHeader { invite_id, identity: my_pub, ephemeral: ek_pub, kem_ct, sig }, ratchet))
}

/// Inviter side, on receiving a join for one of its invites.
pub fn accept(me: &Identity, secret: &InviteSecret, header: &JoinHeader) -> Result<Ratchet> {
    if header.kem_ct.len() != KEM_CT_LEN {
        return Err(Error::Malformed("kem ciphertext length"));
    }
    if id_for(&x25519_public(&secret.ephemeral)) != header.invite_id {
        return Err(Error::Malformed("join is for another invite"));
    }
    verify(
        &header.identity.sign,
        &transcript(&header.invite_id, &header.identity.dh, &header.ephemeral, &header.kem_ct),
        &header.sig,
    )?;
    let dh1 = x25519(&secret.ephemeral, &header.identity.dh)?;
    let dh2 = me.dh(&header.ephemeral)?;
    let dh3 = x25519(&secret.ephemeral, &header.ephemeral)?;
    let ss = secret.kem.decapsulate(&header.kem_ct)?;
    let sk = derive(dh1, dh2, dh3, ss);
    Ok(Ratchet::init_receiver(sk, secret.ephemeral, associated_data(&me.public(), &header.identity)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueueAddr;

    #[test]
    fn handshake_then_conversation() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let q = QueueAddr { relay: "ws://r".into(), snd_id: [1; 16] };
        let (invite, secret) = Invite::create(&alice, q, "", 60).unwrap();
        let (header, mut bob_r) = join(&bob, &invite).unwrap();
        let mut alice_r = accept(&alice, &secret, &header).unwrap();
        let m = bob_r.encrypt(b"hello alice").unwrap();
        assert_eq!(alice_r.decrypt(&m).unwrap(), b"hello alice");
        let m = alice_r.encrypt(b"hello bob").unwrap();
        assert_eq!(bob_r.decrypt(&m).unwrap(), b"hello bob");
    }

    #[test]
    fn forged_join_is_rejected() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let mallory = Identity::generate();
        let q = QueueAddr { relay: "ws://r".into(), snd_id: [1; 16] };
        let (invite, secret) = Invite::create(&alice, q, "", 60).unwrap();
        let (mut header, _) = join(&bob, &invite).unwrap();
        // Claiming someone else's signing key breaks the transcript signature.
        header.identity.sign = mallory.public().sign;
        assert_eq!(accept(&alice, &secret, &header).err(), Some(Error::Signature));
        // A join for a different invite is refused.
        let (other, _) = Invite::create(&alice, QueueAddr { relay: "ws://r".into(), snd_id: [2; 16] }, "", 60).unwrap();
        let (h2, _) = join(&bob, &other).unwrap();
        assert!(accept(&alice, &secret, &h2).is_err());
    }
}
