//! Hybrid sealed box (X25519 + ML-KEM-768) with STREAM chunking.
//!
//! Used where there is no session: drop submissions, replies to sources,
//! travel-mode backups and insurance files. Format:
//! `"SOS1" || ephemeral_pk[32] || kem_ct[1088] || chunk*` where every chunk
//! except the last holds `CHUNK` plaintext bytes plus a 16-byte tag.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::kem::{KEM_CT_LEN, KEM_PK_LEN, KemSecret, encapsulate};
use crate::keys::{x25519, x25519_public};
use crate::{Error, Result, aead};

const MAGIC: &[u8; 4] = b"SOS1";
pub const CHUNK: usize = 64 * 1024;
const HEADER_LEN: usize = 4 + 32 + KEM_CT_LEN;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealPublic {
    pub dh: [u8; 32],
    pub kem: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct SealSecret {
    dh: [u8; 32],
    kem: KemSecret,
}

impl SealSecret {
    pub fn generate() -> Self {
        Self { dh: crate::rand::bytes(), kem: KemSecret::generate() }
    }

    /// Deterministic key pair from a 32-byte seed (codename-derived source keys).
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        let mut kem_seed = [0u8; 64];
        crate::kdf::hkdf(b"sotto-seal-seed", seed, b"kem", &mut kem_seed);
        let s = Self { dh: crate::kdf::hkdf32(b"sotto-seal-seed", seed, b"dh"), kem: KemSecret::from_seed(kem_seed) };
        kem_seed.zeroize();
        s
    }

    pub fn public(&self) -> SealPublic {
        SealPublic { dh: x25519_public(&self.dh), kem: self.kem.public() }
    }
}

struct StreamKey {
    key: [u8; 32],
    prefix: [u8; 19],
}

impl Drop for StreamKey {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl StreamKey {
    fn derive(dh: &[u8; 32], ss: &[u8; 32], epk: &[u8; 32], ct: &[u8], rdh: &[u8; 32]) -> Self {
        let mut ikm = [0u8; 64];
        ikm[..32].copy_from_slice(dh);
        ikm[32..].copy_from_slice(ss);
        let mut info = b"SottoSeal_v1".to_vec();
        info.extend_from_slice(epk);
        info.extend_from_slice(ct);
        info.extend_from_slice(rdh);
        let mut out = [0u8; 51];
        crate::kdf::hkdf(&[0u8; 32], &ikm, &info, &mut out);
        ikm.zeroize();
        let mut key = [0u8; 32];
        let mut prefix = [0u8; 19];
        key.copy_from_slice(&out[..32]);
        prefix.copy_from_slice(&out[32..]);
        out.zeroize();
        Self { key, prefix }
    }

    fn nonce(&self, counter: u32, last: bool) -> [u8; 24] {
        let mut n = [0u8; 24];
        n[..19].copy_from_slice(&self.prefix);
        n[19..23].copy_from_slice(&counter.to_be_bytes());
        n[23] = last as u8;
        n
    }
}

/// Read up to `buf.len()` bytes, stopping only at EOF.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(filled)
}

/// Seal everything read from `input` to `recipient`, writing to `output`. Returns bytes written.
pub fn seal_stream(recipient: &SealPublic, input: &mut impl Read, output: &mut impl Write) -> Result<u64> {
    if recipient.kem.len() != KEM_PK_LEN {
        return Err(Error::Key);
    }
    let ek: [u8; 32] = crate::rand::bytes();
    let epk = x25519_public(&ek);
    let dh = x25519(&ek, &recipient.dh)?;
    let (ct, ss) = encapsulate(&recipient.kem)?;
    let sk = StreamKey::derive(&dh, &ss, &epk, &ct, &recipient.dh);
    output.write_all(MAGIC)?;
    output.write_all(&epk)?;
    output.write_all(&ct)?;
    let mut written = HEADER_LEN as u64;

    let mut cur = vec![0u8; CHUNK];
    let mut next = vec![0u8; CHUNK];
    let mut cur_len = read_full(input, &mut cur)?;
    let mut counter: u32 = 0;
    loop {
        let next_len = if cur_len == CHUNK { read_full(input, &mut next)? } else { 0 };
        let last = next_len == 0;
        let c = aead::encrypt(&sk.key, &sk.nonce(counter, last), &cur[..cur_len], &[]);
        output.write_all(&c)?;
        written += c.len() as u64;
        if last {
            break;
        }
        counter = counter.checked_add(1).ok_or(Error::TooLarge)?;
        std::mem::swap(&mut cur, &mut next);
        cur_len = next_len;
    }
    cur.zeroize();
    next.zeroize();
    Ok(written)
}

/// Open a sealed stream. Fails on any tampering, reordering or truncation.
pub fn open_stream(secret: &SealSecret, input: &mut impl Read, output: &mut impl Write) -> Result<u64> {
    let mut header = [0u8; HEADER_LEN];
    if read_full(input, &mut header)? != HEADER_LEN || &header[..4] != MAGIC {
        return Err(Error::Decrypt);
    }
    let epk: [u8; 32] = header[4..36].try_into().expect("32 bytes");
    let ct = &header[36..];
    let dh = x25519(&secret.dh, &epk).map_err(|_| Error::Decrypt)?;
    let ss = secret.kem.decapsulate(ct)?;
    let sk = StreamKey::derive(&dh, &ss, &epk, ct, &x25519_public(&secret.dh));

    let size = CHUNK + aead::TAG_LEN;
    let mut cur = vec![0u8; size];
    let mut next = vec![0u8; size];
    let mut cur_len = read_full(input, &mut cur)?;
    let mut counter: u32 = 0;
    let mut written = 0u64;
    loop {
        if cur_len < aead::TAG_LEN {
            return Err(Error::Decrypt);
        }
        let next_len = if cur_len == size { read_full(input, &mut next)? } else { 0 };
        let last = next_len == 0;
        let mut pt = aead::decrypt(&sk.key, &sk.nonce(counter, last), &cur[..cur_len], &[])?;
        output.write_all(&pt)?;
        written += pt.len() as u64;
        pt.zeroize();
        if last {
            break;
        }
        counter = counter.checked_add(1).ok_or(Error::TooLarge)?;
        std::mem::swap(&mut cur, &mut next);
        cur_len = next_len;
    }
    Ok(written)
}

/// Incremental sealing for data that arrives in pieces (for example an HTTP upload).
/// Produces exactly the same format as [`seal_stream`].
pub struct Sealer {
    key: StreamKey,
    buf: Vec<u8>,
    counter: u32,
}

impl Sealer {
    /// Returns the sealer and the header bytes that must be written first.
    pub fn new(recipient: &SealPublic) -> Result<(Sealer, Vec<u8>)> {
        if recipient.kem.len() != KEM_PK_LEN {
            return Err(Error::Key);
        }
        let ek: [u8; 32] = crate::rand::bytes();
        let epk = x25519_public(&ek);
        let dh = x25519(&ek, &recipient.dh)?;
        let (ct, ss) = encapsulate(&recipient.kem)?;
        let key = StreamKey::derive(&dh, &ss, &epk, &ct, &recipient.dh);
        let mut header = MAGIC.to_vec();
        header.extend_from_slice(&epk);
        header.extend_from_slice(&ct);
        Ok((Sealer { key, buf: Vec::new(), counter: 0 }, header))
    }

    /// Add plaintext; returns any ciphertext that is ready to be written.
    pub fn push(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        // Keep the last full chunk back until we know whether more data follows.
        while self.buf.len() > CHUNK {
            let c = aead::encrypt(&self.key.key, &self.key.nonce(self.counter, false), &self.buf[..CHUNK], &[]);
            out.extend(c);
            self.buf.drain(..CHUNK);
            self.counter = self.counter.checked_add(1).ok_or(Error::TooLarge)?;
        }
        Ok(out)
    }

    /// Seal the final chunk.
    pub fn finish(mut self) -> Vec<u8> {
        let c = aead::encrypt(&self.key.key, &self.key.nonce(self.counter, true), &self.buf, &[]);
        self.buf.zeroize();
        c
    }
}

pub fn seal(recipient: &SealPublic, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() + HEADER_LEN + 64);
    seal_stream(recipient, &mut &data[..], &mut out)?;
    Ok(out)
}

pub fn open(secret: &SealSecret, data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len());
    open_stream(secret, &mut &data[..], &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_at_chunk_boundaries() {
        let s = SealSecret::generate();
        let p = s.public();
        for len in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK] {
            let data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let sealed = seal(&p, &data).unwrap();
            assert_eq!(open(&s, &sealed).unwrap(), data, "len {len}");
        }
    }

    #[test]
    fn detects_tampering_truncation_and_wrong_key() {
        let s = SealSecret::generate();
        let data = vec![7u8; CHUNK * 2 + 10];
        let sealed = seal(&s.public(), &data).unwrap();
        let mut bad = sealed.clone();
        bad[HEADER_LEN + 5] ^= 1;
        assert!(open(&s, &bad).is_err());
        // Dropping the final chunk must not look like a shorter valid file.
        let truncated = &sealed[..HEADER_LEN + 2 * (CHUNK + 16)];
        assert!(open(&s, truncated).is_err());
        assert!(open(&SealSecret::generate(), &sealed).is_err());
        assert!(open(&s, b"SOS1short").is_err());
    }

    #[test]
    fn incremental_sealer_matches_stream_format() {
        let s = SealSecret::generate();
        for len in [0, 10, CHUNK, CHUNK * 2 + 3] {
            let data: Vec<u8> = (0..len).map(|i| (i % 13) as u8).collect();
            let (mut sealer, mut out) = Sealer::new(&s.public()).unwrap();
            for piece in data.chunks(7000) {
                out.extend(sealer.push(piece).unwrap());
            }
            out.extend(sealer.finish());
            assert_eq!(open(&s, &out).unwrap(), data, "len {len}");
        }
    }

    #[test]
    fn seeded_keys_match() {
        let a = SealSecret::from_seed(&[5u8; 32]);
        let sealed = seal(&a.public(), b"reply").unwrap();
        assert_eq!(open(&SealSecret::from_seed(&[5u8; 32]), &sealed).unwrap(), b"reply");
    }
}
