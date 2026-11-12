//! Encrypted on-device storage with duress passcode and panic wipe.
//!
//! Layout of a vault directory:
//!
//! ```text
//! vault.hdr        magic | version | argon2 m,t | salt[32] | 4 key slots
//! records/<hex>    nonce[24] | XChaCha20-Poly1305(padded postcard value), aad = file name
//! blobs/<hex>      chunked STREAM ciphertext of a large file
//! ```
//!
//! Record and blob file names are HMACs of their logical names, so the
//! directory reveals only how many items exist and their padded sizes.
//! Each slot wraps `role || store key` under a key derived from a passcode.
//! Unused slots are random, so nobody can tell whether a duress passcode is set.
//! Destroying the header destroys every key, which is what panic wipe does first.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use serde::de::DeserializeOwned;
use sotto_core::{aead, hex, kdf};
use zeroize::{Zeroize, Zeroizing};

const MAGIC: &[u8; 8] = b"SOTTOVL1";
const SLOTS: usize = 4;
const SLOT_PT: usize = 1 + 32;
const SLOT_LEN: usize = aead::NONCE_LEN + SLOT_PT + aead::TAG_LEN;
const PREFIX_LEN: usize = 8 + 1 + 4 + 4 + 32;
const HEADER_LEN: usize = PREFIX_LEN + SLOTS * SLOT_LEN;
const ROLE_REAL: u8 = 1;
const ROLE_DURESS: u8 = 2;
const INDEX: &str = "\u{0}index";
const BLOB_CHUNK: usize = 64 * 1024;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VaultError {
    #[error("no vault here")]
    Missing,
    #[error("a vault already exists here")]
    Exists,
    #[error("wrong passcode")]
    WrongPasscode,
    #[error("vault data is damaged")]
    Corrupt,
    #[error("passcodes must differ")]
    SamePasscode,
    #[error("storage error: {0}")]
    Io(String),
}

impl From<std::io::Error> for VaultError {
    fn from(e: std::io::Error) -> Self {
        VaultError::Io(e.to_string())
    }
}

impl From<sotto_core::Error> for VaultError {
    fn from(e: sotto_core::Error) -> Self {
        match e {
            sotto_core::Error::Io(s) => VaultError::Io(s),
            _ => VaultError::Corrupt,
        }
    }
}

pub type Result<T> = core::result::Result<T, VaultError>;

/// Argon2id cost. Desktop default: 64 MiB, 3 passes.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub m_kib: u32,
    pub t: u32,
}

impl Default for Params {
    fn default() -> Self {
        Self { m_kib: 64 * 1024, t: 3 }
    }
}

impl Params {
    /// Cheap parameters for tests only.
    pub const fn testing() -> Self {
        Self { m_kib: 64, t: 1 }
    }
}

/// How the vault was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unlocked {
    Real,
    /// The duress passcode was used: the decoy is open and the real data has been destroyed.
    Duress,
}

struct Header {
    params: Params,
    salt: [u8; 32],
    slots: [[u8; SLOT_LEN]; SLOTS],
}

impl Header {
    fn prefix(&self) -> [u8; PREFIX_LEN] {
        let mut p = [0u8; PREFIX_LEN];
        p[..8].copy_from_slice(MAGIC);
        p[8] = 1;
        p[9..13].copy_from_slice(&self.params.m_kib.to_le_bytes());
        p[13..17].copy_from_slice(&self.params.t.to_le_bytes());
        p[17..].copy_from_slice(&self.salt);
        p
    }

    fn encode(&self) -> Vec<u8> {
        let mut v = self.prefix().to_vec();
        for s in &self.slots {
            v.extend_from_slice(s);
        }
        v
    }

    fn decode(b: &[u8]) -> Result<Self> {
        if b.len() != HEADER_LEN || &b[..8] != MAGIC || b[8] != 1 {
            return Err(VaultError::Corrupt);
        }
        let params = Params {
            m_kib: u32::from_le_bytes(b[9..13].try_into().expect("4")),
            t: u32::from_le_bytes(b[13..17].try_into().expect("4")),
        };
        let mut slots = [[0u8; SLOT_LEN]; SLOTS];
        for (i, s) in slots.iter_mut().enumerate() {
            s.copy_from_slice(&b[PREFIX_LEN + i * SLOT_LEN..PREFIX_LEN + (i + 1) * SLOT_LEN]);
        }
        Ok(Self { params, salt: b[17..PREFIX_LEN].try_into().expect("32"), slots })
    }

    fn kek(&self, passcode: &str) -> Result<Zeroizing<[u8; 32]>> {
        let mut k = Zeroizing::new([0u8; 32]);
        kdf::argon2id(passcode.as_bytes(), &self.salt, self.params.m_kib, self.params.t, &mut *k)?;
        Ok(k)
    }

    fn wrap(&self, kek: &[u8; 32], role: u8, key: &[u8; 32]) -> [u8; SLOT_LEN] {
        let mut pt = Zeroizing::new([0u8; SLOT_PT]);
        pt[0] = role;
        pt[1..].copy_from_slice(key);
        let sealed = aead::seal_random(kek, &*pt, &self.prefix());
        sealed.try_into().expect("slot length")
    }

    /// Try every slot; returns (slot index, role, key).
    fn open(&self, kek: &[u8; 32]) -> Option<(usize, u8, Zeroizing<[u8; 32]>)> {
        let prefix = self.prefix();
        for (i, s) in self.slots.iter().enumerate() {
            if let Ok(mut pt) = aead::open_random(kek, s, &prefix) {
                let mut key = Zeroizing::new([0u8; 32]);
                key.copy_from_slice(&pt[1..]);
                let role = pt[0];
                pt.zeroize();
                return Some((i, role, key));
            }
        }
        None
    }
}

fn random_slot() -> [u8; SLOT_LEN] {
    sotto_core::rand::bytes()
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    // A unique temporary name per write: concurrent writers of the same record must never share one.
    let tmp = path.with_extension(format!("tmp{}", hex(&sotto_core::rand::bytes::<6>())));
    {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// An open vault. Dropping it forgets the keys.
pub struct Vault {
    dir: PathBuf,
    store: Store,
    passcode_slot: usize,
}

impl Vault {
    pub fn exists(dir: &Path) -> bool {
        dir.join("vault.hdr").exists()
    }

    /// Create a vault. With `duress`, a decoy store is created alongside.
    pub fn create(dir: &Path, passcode: &str, duress: Option<&str>, params: Params) -> Result<Vault> {
        if Self::exists(dir) {
            return Err(VaultError::Exists);
        }
        if duress == Some(passcode) {
            return Err(VaultError::SamePasscode);
        }
        fs::create_dir_all(dir.join("records"))?;
        fs::create_dir_all(dir.join("blobs"))?;
        let mut header = Header { params, salt: sotto_core::rand::bytes(), slots: [[0u8; SLOT_LEN]; SLOTS] };
        for s in header.slots.iter_mut() {
            *s = random_slot();
        }
        let real_key = Zeroizing::new(sotto_core::rand::bytes::<32>());
        let real_slot = sotto_core::rand::below(SLOTS as u32) as usize;
        header.slots[real_slot] = header.wrap(&*header.kek(passcode)?, ROLE_REAL, &real_key);
        if let Some(d) = duress {
            let decoy_key = Zeroizing::new(sotto_core::rand::bytes::<32>());
            let decoy_slot = (real_slot + 1 + sotto_core::rand::below(SLOTS as u32 - 1) as usize) % SLOTS;
            header.slots[decoy_slot] = header.wrap(&*header.kek(d)?, ROLE_DURESS, &decoy_key);
            // Write the decoy index so the decoy opens to a valid, empty store.
            Store::new(dir, *decoy_key).flush_index()?;
        }
        write_atomic(&dir.join("vault.hdr"), &header.encode())?;
        let store = Store::new(dir, *real_key);
        store.flush_index()?;
        Ok(Vault { dir: dir.to_path_buf(), store, passcode_slot: real_slot })
    }

    pub fn unlock(dir: &Path, passcode: &str) -> Result<(Vault, Unlocked)> {
        let bytes = fs::read(dir.join("vault.hdr")).map_err(|_| VaultError::Missing)?;
        let mut header = Header::decode(&bytes)?;
        let kek = header.kek(passcode)?;
        let (slot, role, key) = header.open(&kek).ok_or(VaultError::WrongPasscode)?;
        let store = Store::new(dir, *key);
        let unlocked = if role == ROLE_DURESS {
            // Destroy every other slot first: from this instant the real data is unrecoverable.
            for (i, s) in header.slots.iter_mut().enumerate() {
                if i != slot {
                    *s = random_slot();
                }
            }
            write_atomic(&dir.join("vault.hdr"), &header.encode())?;
            store.remove_foreign_files()?;
            Unlocked::Duress
        } else {
            Unlocked::Real
        };
        store.load_index()?;
        Ok((Vault { dir: dir.to_path_buf(), store, passcode_slot: slot }, unlocked))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// An independent handle to the main store, for sharing across tasks.
    /// Use either this handle or [`Vault::store`] for writes, not both.
    pub fn main_handle(&self) -> Result<Store> {
        let s = Store::new(&self.dir, *self.store.key);
        s.load_index()?;
        Ok(s)
    }

    /// A separate store under its own key, for crypto-shreddable scopes such as cases.
    pub fn scope(&self, key: [u8; 32]) -> Result<Store> {
        let s = Store::new(&self.dir, key);
        s.load_index()?;
        Ok(s)
    }

    pub fn change_passcode(&mut self, old: &str, new: &str) -> Result<()> {
        let bytes = fs::read(self.dir.join("vault.hdr"))?;
        let mut header = Header::decode(&bytes)?;
        let (slot, role, key) = header.open(&*header.kek(old)?).ok_or(VaultError::WrongPasscode)?;
        if slot != self.passcode_slot || role != ROLE_REAL {
            return Err(VaultError::WrongPasscode);
        }
        if header.open(&*header.kek(new)?).is_some() {
            return Err(VaultError::SamePasscode);
        }
        header.slots[slot] = header.wrap(&*header.kek(new)?, ROLE_REAL, &key);
        write_atomic(&self.dir.join("vault.hdr"), &header.encode())
    }

    /// Set, replace or remove the duress passcode. Requires the real passcode.
    pub fn set_duress(&mut self, passcode: &str, duress: Option<&str>) -> Result<()> {
        if duress == Some(passcode) {
            return Err(VaultError::SamePasscode);
        }
        let bytes = fs::read(self.dir.join("vault.hdr"))?;
        let mut header = Header::decode(&bytes)?;
        let (slot, role, _) = header.open(&*header.kek(passcode)?).ok_or(VaultError::WrongPasscode)?;
        if role != ROLE_REAL {
            return Err(VaultError::WrongPasscode);
        }
        for (i, s) in header.slots.iter_mut().enumerate() {
            if i != slot {
                *s = random_slot();
            }
        }
        if let Some(d) = duress {
            let decoy_key = Zeroizing::new(sotto_core::rand::bytes::<32>());
            let decoy_slot = (slot + 1 + sotto_core::rand::below(SLOTS as u32 - 1) as usize) % SLOTS;
            header.slots[decoy_slot] = header.wrap(&*header.kek(d)?, ROLE_DURESS, &decoy_key);
            Store::new(&self.dir, *decoy_key).flush_index()?;
        }
        write_atomic(&self.dir.join("vault.hdr"), &header.encode())
    }

    /// Destroy the vault. The header goes first, which makes everything else unreadable at once;
    /// the remaining files are then removed on a best-effort basis.
    pub fn panic_wipe(dir: &Path) -> Result<()> {
        let hdr = dir.join("vault.hdr");
        if hdr.exists() {
            if let Ok(mut f) = OpenOptions::new().write(true).open(&hdr) {
                let mut noise = vec![0u8; HEADER_LEN];
                sotto_core::rand::fill(&mut noise);
                let _ = f.write_all(&noise);
                let _ = f.sync_all();
            }
            fs::remove_file(&hdr)?;
        }
        for sub in ["records", "blobs"] {
            let _ = fs::remove_dir_all(dir.join(sub));
        }
        Ok(())
    }
}

/// A key-value store of serde values and large blobs under one key.
pub struct Store {
    dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
    index: Mutex<BTreeSet<String>>,
}

impl Store {
    fn new(dir: &Path, key: [u8; 32]) -> Self {
        Self { dir: dir.to_path_buf(), key: Zeroizing::new(key), index: Mutex::new(BTreeSet::new()) }
    }

    fn file_name(&self, kind: &[u8], name: &str) -> String {
        let mut data = kind.to_vec();
        data.extend_from_slice(name.as_bytes());
        hex(&kdf::hmac(&*self.key, &data)[..16])
    }

    fn record_path(&self, name: &str) -> (PathBuf, String) {
        let f = self.file_name(b"rec:", name);
        (self.dir.join("records").join(&f), f)
    }

    fn blob_path(&self, id: &str) -> PathBuf {
        self.dir.join("blobs").join(self.file_name(b"blob:", id))
    }

    fn bucket(len: usize) -> usize {
        let min = 1024;
        if len <= min {
            return min;
        }
        let p = len.next_power_of_two();
        if p <= 256 * 1024 { p } else { len.div_ceil(256 * 1024) * 256 * 1024 }
    }

    fn write_raw(&self, name: &str, bytes: &[u8]) -> Result<()> {
        let (path, fname) = self.record_path(name);
        let mut padded = sotto_core::pad::pad(bytes, Self::bucket(bytes.len() + 4))?;
        let sealed = aead::seal_random(&self.key, &padded, fname.as_bytes());
        padded.zeroize();
        write_atomic(&path, &sealed)
    }

    fn read_raw(&self, name: &str) -> Result<Option<Zeroizing<Vec<u8>>>> {
        let (path, fname) = self.record_path(name);
        let data = match fs::read(&path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let padded = Zeroizing::new(aead::open_random(&self.key, &data, fname.as_bytes()).map_err(|_| VaultError::Corrupt)?);
        Ok(Some(Zeroizing::new(sotto_core::pad::unpad(&padded)?.to_vec())))
    }

    /// Write the index while holding its lock, so a slower flush can never overwrite a newer one.
    fn flush_index(&self) -> Result<()> {
        let index = self.index.lock().expect("index lock");
        let names: Vec<&String> = index.iter().collect();
        let bytes = postcard::to_stdvec(&names).map_err(|_| VaultError::Corrupt)?;
        self.write_raw(INDEX, &bytes)
    }

    fn load_index(&self) -> Result<()> {
        if let Some(raw) = self.read_raw(INDEX)? {
            let names: Vec<String> = postcard::from_bytes(&raw).map_err(|_| VaultError::Corrupt)?;
            *self.index.lock().expect("index lock") = names.into_iter().collect();
        }
        Ok(())
    }

    pub fn put<T: Serialize>(&self, name: &str, value: &T) -> Result<()> {
        let bytes = Zeroizing::new(postcard::to_stdvec(value).map_err(|_| VaultError::Corrupt)?);
        self.write_raw(name, &bytes)?;
        let added = self.index.lock().expect("index lock").insert(name.to_string());
        if added {
            self.flush_index()?;
        }
        Ok(())
    }

    pub fn get<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        match self.read_raw(name)? {
            Some(raw) => Ok(Some(postcard::from_bytes(&raw).map_err(|_| VaultError::Corrupt)?)),
            None => Ok(None),
        }
    }

    pub fn delete(&self, name: &str) -> Result<()> {
        let (path, _) = self.record_path(name);
        let _ = fs::remove_file(path);
        let removed = self.index.lock().expect("index lock").remove(name);
        if removed {
            self.flush_index()?;
        }
        Ok(())
    }

    /// Logical names starting with `prefix`, in sorted order.
    pub fn names(&self, prefix: &str) -> Vec<String> {
        self.index.lock().expect("index lock").iter().filter(|n| n.starts_with(prefix)).cloned().collect()
    }

    /// Keys for one write of a blob. The random per-write salt means rewriting the same id
    /// (for example re-running a review) never reuses a key and nonce sequence.
    fn blob_keys(&self, id: &str, salt: &[u8; 16]) -> (Zeroizing<[u8; 32]>, [u8; 19]) {
        let mut out = Zeroizing::new([0u8; 51]);
        let mut ikm = id.as_bytes().to_vec();
        ikm.extend_from_slice(salt);
        kdf::hkdf(&*self.key, &ikm, b"sotto-vault-blob-v2", &mut *out);
        let mut k = Zeroizing::new([0u8; 32]);
        k.copy_from_slice(&out[..32]);
        let mut p = [0u8; 19];
        p.copy_from_slice(&out[32..]);
        (k, p)
    }

    fn nonce(prefix: &[u8; 19], counter: u32, last: bool) -> [u8; 24] {
        let mut n = [0u8; 24];
        n[..19].copy_from_slice(prefix);
        n[19..23].copy_from_slice(&counter.to_be_bytes());
        n[23] = last as u8;
        n
    }

    /// Store a large blob from a reader. Each id must be written once (ids are random).
    pub fn put_blob(&self, id: &str, input: &mut impl Read) -> Result<u64> {
        let mut w = self.blob_writer(id)?;
        std::io::copy(input, &mut w)?;
        w.finish()
    }

    /// Incremental blob writer, for data that arrives in pieces (downloads).
    /// Nothing is visible under `id` until [`BlobWriter::finish`] succeeds.
    pub fn blob_writer(&self, id: &str) -> Result<BlobWriter> {
        let salt: [u8; 16] = sotto_core::rand::bytes();
        let (key, prefix) = self.blob_keys(id, &salt);
        let path = self.blob_path(id);
        let tmp = path.with_extension(format!("part{}", hex(&sotto_core::rand::bytes::<4>())));
        let mut file = File::create(&tmp)?;
        file.write_all(&salt)?;
        Ok(BlobWriter { file: Some(file), tmp, path, key, prefix, buf: Zeroizing::new(Vec::new()), counter: 0, total: 0 })
    }

    pub fn has_blob(&self, id: &str) -> bool {
        self.blob_path(id).exists()
    }

    /// Decrypt a blob into a writer. Detects tampering and truncation.
    pub fn read_blob(&self, id: &str, output: &mut impl Write) -> Result<u64> {
        let mut input = File::open(self.blob_path(id)).map_err(|_| VaultError::Missing)?;
        let mut salt = [0u8; 16];
        if read_full(&mut input, &mut salt)? != 16 {
            return Err(VaultError::Corrupt);
        }
        let (key, prefix) = self.blob_keys(id, &salt);
        let size = BLOB_CHUNK + aead::TAG_LEN;
        let mut cur = vec![0u8; size];
        let mut next = vec![0u8; size];
        let mut cur_len = read_full(&mut input, &mut cur)?;
        let mut counter = 0u32;
        let mut total = 0u64;
        loop {
            if cur_len < aead::TAG_LEN {
                return Err(VaultError::Corrupt);
            }
            let next_len = if cur_len == size { read_full(&mut input, &mut next)? } else { 0 };
            let last = next_len == 0;
            let pt = Zeroizing::new(
                aead::decrypt(&key, &Self::nonce(&prefix, counter, last), &cur[..cur_len], &[]).map_err(|_| VaultError::Corrupt)?,
            );
            output.write_all(&pt)?;
            total += pt.len() as u64;
            if last {
                break;
            }
            counter += 1;
            std::mem::swap(&mut cur, &mut next);
            cur_len = next_len;
        }
        Ok(total)
    }

    pub fn delete_blob(&self, id: &str) -> Result<()> {
        let _ = fs::remove_file(self.blob_path(id));
        Ok(())
    }

    /// Delete every record this store knows about, and the listed blobs.
    /// Combined with forgetting the store key this is crypto-shredding.
    pub fn destroy(&self, blob_ids: &[String]) -> Result<()> {
        for id in blob_ids {
            self.delete_blob(id)?;
        }
        let names: Vec<String> = self.index.lock().expect("index lock").iter().cloned().collect();
        for n in names {
            let (p, _) = self.record_path(&n);
            let _ = fs::remove_file(p);
        }
        self.index.lock().expect("index lock").clear();
        let (p, _) = self.record_path(INDEX);
        let _ = fs::remove_file(p);
        Ok(())
    }

    /// After a duress unlock: remove record files this key cannot read.
    fn remove_foreign_files(&self) -> Result<()> {
        let dir = self.dir.join("records");
        for entry in fs::read_dir(&dir)?.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            let readable = fs::read(entry.path())
                .ok()
                .map(|d| aead::open_random(&self.key, &d, fname.as_bytes()).is_ok())
                .unwrap_or(false);
            if !readable {
                let _ = fs::remove_file(entry.path());
            }
        }
        // Blobs cannot be attributed without the real key and are already unreadable. Remove them all.
        let _ = fs::remove_dir_all(self.dir.join("blobs"));
        fs::create_dir_all(self.dir.join("blobs"))?;
        Ok(())
    }
}

/// Streams plaintext into an encrypted blob. Dropping it without `finish` discards the data.
pub struct BlobWriter {
    file: Option<File>,
    tmp: PathBuf,
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    prefix: [u8; 19],
    buf: Zeroizing<Vec<u8>>,
    counter: u32,
    total: u64,
}

impl BlobWriter {
    fn flush_chunk(&mut self, last: bool) -> std::io::Result<()> {
        let n = if last { self.buf.len() } else { BLOB_CHUNK };
        let ct = aead::encrypt(&self.key, &Store::nonce(&self.prefix, self.counter, last), &self.buf[..n], &[]);
        self.file.as_mut().expect("open until finish").write_all(&ct)?;
        self.buf.drain(..n);
        self.counter = self.counter.checked_add(1).ok_or_else(|| std::io::Error::other("blob too large"))?;
        Ok(())
    }

    /// Seal the final chunk and make the blob visible. Returns the plaintext size.
    pub fn finish(mut self) -> Result<u64> {
        self.flush_chunk(true)?;
        let file = self.file.take().expect("open until finish");
        file.sync_all()?;
        drop(file);
        fs::rename(&self.tmp, &self.path)?;
        Ok(self.total)
    }
}

impl Write for BlobWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        self.total += data.len() as u64;
        // Keep at least one byte back so the final chunk is always sealed with the "last" flag.
        while self.buf.len() > BLOB_CHUNK {
            self.flush_chunk(false)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for BlobWriter {
    fn drop(&mut self) {
        if self.file.take().is_some() {
            let _ = fs::remove_file(&self.tmp);
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn count_records(dir: &Path) -> usize {
        fs::read_dir(dir.join("records")).unwrap().count()
    }

    #[test]
    fn create_unlock_put_get() {
        let d = tempfile::tempdir().unwrap();
        {
            let v = Vault::create(d.path(), "correct horse", None, Params::testing()).unwrap();
            v.store().put("profile", &("Ada".to_string(), 7u32)).unwrap();
            v.store().put("contact/1", &1u8).unwrap();
            v.store().put("contact/2", &2u8).unwrap();
        }
        assert_eq!(Vault::unlock(d.path(), "wrong").err(), Some(VaultError::WrongPasscode));
        let (v, how) = Vault::unlock(d.path(), "correct horse").unwrap();
        assert_eq!(how, Unlocked::Real);
        assert_eq!(v.store().get::<(String, u32)>("profile").unwrap(), Some(("Ada".into(), 7)));
        assert_eq!(v.store().names("contact/"), vec!["contact/1", "contact/2"]);
        v.store().delete("contact/1").unwrap();
        assert_eq!(v.store().names("contact/"), vec!["contact/2"]);
        assert_eq!(v.store().get::<u8>("contact/1").unwrap(), None);
        assert!(Vault::create(d.path(), "x", None, Params::testing()).is_err());
    }

    #[test]
    fn records_leak_no_names_and_detect_tampering() {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::create(d.path(), "pass", None, Params::testing()).unwrap();
        v.store().put("contact/source-k7", &"secret".to_string()).unwrap();
        for e in fs::read_dir(d.path().join("records")).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            assert!(!name.contains("contact"));
            let data = fs::read(e.path()).unwrap();
            assert!(!data.windows(6).any(|w| w == b"secret"));
            assert_eq!((data.len() - 40) % 1024, 0, "records are padded to buckets");
        }
        let (path, _) = v.store().record_path("contact/source-k7");
        let mut data = fs::read(&path).unwrap();
        data[30] ^= 1;
        fs::write(&path, data).unwrap();
        assert_eq!(v.store().get::<String>("contact/source-k7").err(), Some(VaultError::Corrupt));
    }

    #[test]
    fn duress_opens_decoy_and_destroys_real() {
        let d = tempfile::tempdir().unwrap();
        {
            let v = Vault::create(d.path(), "real-pass", Some("decoy-pass"), Params::testing()).unwrap();
            v.store().put("contact/secret", &"source".to_string()).unwrap();
            v.store().put_blob("doc", &mut &b"leaked document"[..]).unwrap();
        }
        let header_before = fs::read(d.path().join("vault.hdr")).unwrap();
        let (v, how) = Vault::unlock(d.path(), "decoy-pass").unwrap();
        assert_eq!(how, Unlocked::Duress);
        assert!(v.store().names("").is_empty());
        let header_after = fs::read(d.path().join("vault.hdr")).unwrap();
        assert_eq!(header_before.len(), header_after.len(), "header keeps its shape");
        // Only the decoy index remains.
        assert_eq!(count_records(d.path()), 1);
        assert!(!v.store().has_blob("doc"));
        drop(v);
        assert_eq!(Vault::unlock(d.path(), "real-pass").err(), Some(VaultError::WrongPasscode));
        // The decoy keeps working.
        assert_eq!(Vault::unlock(d.path(), "decoy-pass").unwrap().1, Unlocked::Duress);
    }

    #[test]
    fn change_passcode_and_duress_management() {
        let d = tempfile::tempdir().unwrap();
        let mut v = Vault::create(d.path(), "one", None, Params::testing()).unwrap();
        v.store().put("k", &5u8).unwrap();
        assert_eq!(v.change_passcode("bad", "two").err(), Some(VaultError::WrongPasscode));
        v.change_passcode("one", "two").unwrap();
        v.set_duress("two", Some("panic")).unwrap();
        assert_eq!(v.set_duress("two", Some("two")).err(), Some(VaultError::SamePasscode));
        drop(v);
        assert!(Vault::unlock(d.path(), "one").is_err());
        let (v, how) = Vault::unlock(d.path(), "two").unwrap();
        assert_eq!(how, Unlocked::Real);
        assert_eq!(v.store().get::<u8>("k").unwrap(), Some(5));
        drop(v);
        assert_eq!(Vault::unlock(d.path(), "panic").unwrap().1, Unlocked::Duress);
    }

    #[test]
    fn blobs_round_trip_and_scopes_shred() {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::create(d.path(), "p", None, Params::testing()).unwrap();
        let data: Vec<u8> = (0..(BLOB_CHUNK * 2 + 17)).map(|i| (i % 253) as u8).collect();
        assert_eq!(v.store().put_blob("b1", &mut &data[..]).unwrap(), data.len() as u64);
        let mut out = Vec::new();
        v.store().read_blob("b1", &mut out).unwrap();
        assert_eq!(out, data);
        // Incremental writes across chunk boundaries, including exactly one chunk.
        for len in [BLOB_CHUNK, BLOB_CHUNK * 3] {
            let data: Vec<u8> = (0..len).map(|i| (i % 7) as u8).collect();
            let mut w = v.store().blob_writer("inc").unwrap();
            for piece in data.chunks(1000) {
                w.write_all(piece).unwrap();
            }
            assert_eq!(w.finish().unwrap(), len as u64);
            let mut back = Vec::new();
            v.store().read_blob("inc", &mut back).unwrap();
            assert_eq!(back, data);
        }
        // An unfinished writer leaves nothing behind.
        drop(v.store().blob_writer("never").unwrap());
        assert!(!v.store().has_blob("never"));
        let mut empty = Vec::new();
        v.store().put_blob("b0", &mut &b""[..]).unwrap();
        v.store().read_blob("b0", &mut empty).unwrap();
        assert!(empty.is_empty());

        let case_key = sotto_core::rand::bytes();
        let case = v.scope(case_key).unwrap();
        case.put("note/1", &"case note".to_string()).unwrap();
        case.put_blob("evidence", &mut &b"pdf"[..]).unwrap();
        let reopened = v.scope(case_key).unwrap();
        assert_eq!(reopened.names("note/"), vec!["note/1"]);
        let before = count_records(d.path());
        reopened.destroy(&["evidence".to_string()]).unwrap();
        assert_eq!(count_records(d.path()), before - 2);
        assert!(!case.has_blob("evidence"));
        assert_eq!(v.store().get::<u8>("missing").unwrap(), None);
    }

    #[test]
    fn panic_wipe_removes_everything() {
        let d = tempfile::tempdir().unwrap();
        let v = Vault::create(d.path(), "p", None, Params::testing()).unwrap();
        v.store().put("k", &1u8).unwrap();
        drop(v);
        Vault::panic_wipe(d.path()).unwrap();
        assert!(!Vault::exists(d.path()));
        assert!(!d.path().join("records").exists());
        assert_eq!(Vault::unlock(d.path(), "p").err(), Some(VaultError::Missing));
    }
}

