//! Double Ratchet (Signal specification, without header encryption).
//!
//! State is updated on a copy and committed only when a message decrypts, so
//! a forged or corrupted message can never desynchronise a session.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::kdf::{hkdf, hmac};
use crate::keys::{x25519, x25519_public};
use crate::{Error, Result, aead};

const MAX_SKIP: u32 = 1000;
const MAX_STORED_SKIPPED: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub dh: [u8; 32],
    pub pn: u32,
    pub n: u32,
}

impl Header {
    fn bytes(&self) -> [u8; 40] {
        let mut b = [0u8; 40];
        b[..32].copy_from_slice(&self.dh);
        b[32..36].copy_from_slice(&self.pn.to_le_bytes());
        b[36..].copy_from_slice(&self.n.to_le_bytes());
        b
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatchetMessage {
    pub header: Header,
    pub ciphertext: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct Skipped {
    dh: [u8; 32],
    n: u32,
    mk: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Ratchet {
    dhs_secret: [u8; 32],
    dhs_public: [u8; 32],
    dhr: Option<[u8; 32]>,
    rk: [u8; 32],
    cks: Option<[u8; 32]>,
    ckr: Option<[u8; 32]>,
    ns: u32,
    nr: u32,
    pn: u32,
    skipped: Vec<Skipped>,
    ad: Vec<u8>,
}

fn kdf_rk(rk: &[u8; 32], dh: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let mut out = [0u8; 64];
    hkdf(rk, dh, b"SottoRatchetRoot", &mut out);
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    a.copy_from_slice(&out[..32]);
    b.copy_from_slice(&out[32..]);
    out.zeroize();
    (a, b)
}

fn kdf_ck(ck: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    (hmac(ck, &[0x02]), hmac(ck, &[0x01]))
}

fn message_keys(mk: &[u8; 32]) -> ([u8; 32], [u8; 24]) {
    let mut out = [0u8; 56];
    hkdf(&[0u8; 32], mk, b"SottoMessageKeys", &mut out);
    let mut key = [0u8; 32];
    let mut nonce = [0u8; 24];
    key.copy_from_slice(&out[..32]);
    nonce.copy_from_slice(&out[32..]);
    out.zeroize();
    (key, nonce)
}

fn aad(ad: &[u8], header: &Header) -> Vec<u8> {
    let mut v = ad.to_vec();
    v.extend_from_slice(&header.bytes());
    v
}

impl Ratchet {
    /// The first sender (the joiner) knows the other side's ratchet key from the invite.
    pub fn init_sender(sk: [u8; 32], remote: [u8; 32], ad: Vec<u8>) -> Result<Self> {
        let dhs_secret: [u8; 32] = crate::rand::bytes();
        let dh = x25519(&dhs_secret, &remote)?;
        let (rk, cks) = kdf_rk(&sk, &dh);
        Ok(Self {
            dhs_public: x25519_public(&dhs_secret),
            dhs_secret,
            dhr: Some(remote),
            rk,
            cks: Some(cks),
            ckr: None,
            ns: 0,
            nr: 0,
            pn: 0,
            skipped: Vec::new(),
            ad,
        })
    }

    /// The receiver (the inviter) uses the invite's one-time key pair as its first ratchet key.
    pub fn init_receiver(sk: [u8; 32], own_secret: [u8; 32], ad: Vec<u8>) -> Self {
        Self {
            dhs_public: x25519_public(&own_secret),
            dhs_secret: own_secret,
            dhr: None,
            rk: sk,
            cks: None,
            ckr: None,
            ns: 0,
            nr: 0,
            pn: 0,
            skipped: Vec::new(),
            ad,
        }
    }

    /// Whether this side can send yet (the receiver must hear from the sender first).
    pub fn can_send(&self) -> bool {
        self.cks.is_some()
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<RatchetMessage> {
        let ck = self.cks.ok_or(Error::Ratchet("no sending chain yet"))?;
        let (next, mk) = kdf_ck(&ck);
        self.cks = Some(next);
        let header = Header { dh: self.dhs_public, pn: self.pn, n: self.ns };
        self.ns = self.ns.checked_add(1).ok_or(Error::Ratchet("chain exhausted"))?;
        let (key, nonce) = message_keys(&mk);
        let ciphertext = aead::encrypt(&key, &nonce, plaintext, &aad(&self.ad, &header));
        Ok(RatchetMessage { header, ciphertext })
    }

    pub fn decrypt(&mut self, msg: &RatchetMessage) -> Result<Vec<u8>> {
        let mut next = self.clone();
        let pt = next.decrypt_inner(msg)?;
        *self = next;
        Ok(pt)
    }

    fn decrypt_inner(&mut self, msg: &RatchetMessage) -> Result<Vec<u8>> {
        let h = &msg.header;
        if let Some(i) = self.skipped.iter().position(|s| s.dh == h.dh && s.n == h.n) {
            let mk = self.skipped.remove(i).mk;
            return open(&mk, &self.ad, msg);
        }
        if self.dhr != Some(h.dh) {
            self.skip(h.pn)?;
            self.dh_ratchet(h)?;
        }
        self.skip(h.n)?;
        let ck = self.ckr.ok_or(Error::Ratchet("no receiving chain"))?;
        let (next, mk) = kdf_ck(&ck);
        self.ckr = Some(next);
        self.nr += 1;
        open(&mk, &self.ad, msg)
    }

    fn skip(&mut self, until: u32) -> Result<()> {
        if until > self.nr.saturating_add(MAX_SKIP) {
            return Err(Error::Ratchet("too many skipped messages"));
        }
        if let (Some(mut ck), Some(dhr)) = (self.ckr, self.dhr) {
            while self.nr < until {
                let (next, mk) = kdf_ck(&ck);
                self.skipped.push(Skipped { dh: dhr, n: self.nr, mk });
                ck = next;
                self.nr += 1;
            }
            self.ckr = Some(ck);
            if self.skipped.len() > MAX_STORED_SKIPPED {
                let excess = self.skipped.len() - MAX_STORED_SKIPPED;
                self.skipped.drain(..excess);
            }
        }
        Ok(())
    }

    fn dh_ratchet(&mut self, h: &Header) -> Result<()> {
        self.pn = self.ns;
        self.ns = 0;
        self.nr = 0;
        self.dhr = Some(h.dh);
        let dh = x25519(&self.dhs_secret, &h.dh)?;
        let (rk, ckr) = kdf_rk(&self.rk, &dh);
        self.rk = rk;
        self.ckr = Some(ckr);
        self.dhs_secret = crate::rand::bytes();
        self.dhs_public = x25519_public(&self.dhs_secret);
        let dh = x25519(&self.dhs_secret, &h.dh)?;
        let (rk, cks) = kdf_rk(&self.rk, &dh);
        self.rk = rk;
        self.cks = Some(cks);
        Ok(())
    }
}

fn open(mk: &[u8; 32], ad: &[u8], msg: &RatchetMessage) -> Result<Vec<u8>> {
    let (key, nonce) = message_keys(mk);
    aead::decrypt(&key, &nonce, &msg.ciphertext, &aad(ad, &msg.header))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Ratchet, Ratchet) {
        let sk = crate::rand::bytes();
        let bob_secret: [u8; 32] = crate::rand::bytes();
        let bob_public = x25519_public(&bob_secret);
        let alice = Ratchet::init_sender(sk, bob_public, b"ad".to_vec()).unwrap();
        let bob = Ratchet::init_receiver(sk, bob_secret, b"ad".to_vec());
        (alice, bob)
    }

    #[test]
    fn ping_pong() {
        let (mut a, mut b) = pair();
        assert!(!b.can_send());
        for i in 0..5 {
            let m = a.encrypt(format!("a{i}").as_bytes()).unwrap();
            assert_eq!(b.decrypt(&m).unwrap(), format!("a{i}").as_bytes());
            let m = b.encrypt(format!("b{i}").as_bytes()).unwrap();
            assert_eq!(a.decrypt(&m).unwrap(), format!("b{i}").as_bytes());
        }
    }

    #[test]
    fn out_of_order_and_replay() {
        let (mut a, mut b) = pair();
        let m1 = a.encrypt(b"1").unwrap();
        let m2 = a.encrypt(b"2").unwrap();
        let m3 = a.encrypt(b"3").unwrap();
        assert_eq!(b.decrypt(&m3).unwrap(), b"3");
        assert_eq!(b.decrypt(&m1).unwrap(), b"1");
        // Replay of a consumed message fails.
        assert!(b.decrypt(&m1).is_err());
        assert_eq!(b.decrypt(&m2).unwrap(), b"2");
        // Messages from a previous chain still decrypt after a ratchet step.
        let r = b.encrypt(b"r").unwrap();
        let late = a.encrypt(b"late").unwrap();
        a.decrypt(&r).unwrap();
        let after = a.encrypt(b"after").unwrap();
        assert_eq!(b.decrypt(&after).unwrap(), b"after");
        assert_eq!(b.decrypt(&late).unwrap(), b"late");
    }

    #[test]
    fn tampering_does_not_corrupt_state() {
        let (mut a, mut b) = pair();
        let m = a.encrypt(b"hello").unwrap();
        let mut bad = m.clone();
        bad.ciphertext[0] ^= 1;
        assert!(b.decrypt(&bad).is_err());
        let mut bad_header = m.clone();
        bad_header.header.n = 5;
        assert!(b.decrypt(&bad_header).is_err());
        assert_eq!(b.decrypt(&m).unwrap(), b"hello");
    }

    #[test]
    fn rejects_huge_skip_and_survives_serialization() {
        let (mut a, mut b) = pair();
        let mut m = a.encrypt(b"x").unwrap();
        m.header.n = 5000;
        assert_eq!(b.decrypt(&m), Err(Error::Ratchet("too many skipped messages")));
        let m = a.encrypt(b"y").unwrap();
        let bytes = postcard::to_stdvec(&b).unwrap();
        let mut b2: Ratchet = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(b2.decrypt(&m).unwrap(), b"y");
    }

    #[test]
    fn wrong_associated_data_fails() {
        let sk = crate::rand::bytes();
        let s: [u8; 32] = crate::rand::bytes();
        let mut a = Ratchet::init_sender(sk, x25519_public(&s), b"one".to_vec()).unwrap();
        let mut b = Ratchet::init_receiver(sk, s, b"two".to_vec());
        let m = a.encrypt(b"hi").unwrap();
        assert!(b.decrypt(&m).is_err());
    }
}
