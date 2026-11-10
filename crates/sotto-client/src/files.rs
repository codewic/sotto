//! File transfer: fixed-size encrypted chunks on a relay (`docs/protocol.md` section 8).

use std::io::{Read, Write};

use sha2::{Digest, Sha256};
use sotto_core::envelope::{ChunkList, ChunkRef};
use sotto_core::keys::{sign, signing_keypair};
use sotto_core::wire::{Cmd, FILE_CHUNK, Resp, auth_message};
use sotto_core::{aead, kdf, pad};
use zeroize::Zeroizing;

use crate::conn::RelayConn;
use crate::{Error, Result};

/// Chunk lists longer than this go into a manifest so the payload fits one envelope.
pub const DIRECT_LIMIT: usize = 200;
pub const FILE_TTL_DAYS: u8 = 7;

fn chunk_key(file_key: &[u8; 32], index: u32) -> Zeroizing<[u8; 32]> {
    let mut info = b"SottoFileChunk".to_vec();
    info.extend_from_slice(&index.to_be_bytes());
    Zeroizing::new(kdf::hkdf32(&[0u8; 32], file_key, &info))
}

pub fn encrypt_chunk(file_key: &[u8; 32], index: u32, plain: &[u8]) -> Result<Vec<u8>> {
    let padded = Zeroizing::new(pad::pad(plain, FILE_CHUNK + 4)?);
    Ok(aead::encrypt(&chunk_key(file_key, index), &[0u8; 24], &padded, &index.to_be_bytes()))
}

pub fn decrypt_chunk(file_key: &[u8; 32], index: u32, data: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    let padded = Zeroizing::new(aead::decrypt(&chunk_key(file_key, index), &[0u8; 24], data, &index.to_be_bytes())?);
    Ok(Zeroizing::new(pad::unpad(&padded)?.to_vec()))
}

fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(sotto_core::Error::from(e).into()),
        }
    }
    Ok(filled)
}

pub struct Uploaded {
    pub chunks: Vec<ChunkRef>,
    pub size: u64,
    pub sha256: [u8; 32],
}

async fn put_chunk(conn: &RelayConn, data: Vec<u8>, max_reads: u8) -> Result<ChunkRef> {
    let (seed, public) = signing_keypair();
    match conn.request(move |_| Cmd::FPut { read_auth: public, max_reads, ttl_days: FILE_TTL_DAYS, data }).await? {
        Resp::FId { fid } => Ok(ChunkRef { fid, read_key: seed }),
        _ => Err(Error::BadResponse),
    }
}

/// Encrypt and upload everything from `input`. If `copy` is given, the plaintext is also written there.
pub async fn upload(
    conn: &RelayConn,
    file_key: &[u8; 32],
    input: &mut (impl Read + Send),
    mut copy: Option<&mut (dyn Write + Send)>,
    max_reads: u8,
) -> Result<Uploaded> {
    let mut hasher = Sha256::new();
    let mut chunks = Vec::new();
    let mut size = 0u64;
    let mut buf = Zeroizing::new(vec![0u8; FILE_CHUNK]);
    let mut index = 0u32;
    loop {
        let n = read_full(input, &mut buf)?;
        if n == 0 && index > 0 {
            break;
        }
        hasher.update(&buf[..n]);
        if let Some(w) = copy.as_mut() {
            w.write_all(&buf[..n]).map_err(sotto_core::Error::from)?;
        }
        size += n as u64;
        chunks.push(put_chunk(conn, encrypt_chunk(file_key, index, &buf[..n])?, max_reads).await?);
        index = index.checked_add(1).ok_or(Error::Invalid("file too large"))?;
        if n < FILE_CHUNK {
            break;
        }
    }
    Ok(Uploaded { chunks, size, sha256: hasher.finalize().into() })
}

/// Turn an uploaded chunk list into what goes in the `File` payload, adding a manifest if needed.
pub async fn chunk_list(conn: &RelayConn, chunks: Vec<ChunkRef>, max_reads: u8) -> Result<ChunkList> {
    if chunks.len() <= DIRECT_LIMIT {
        return Ok(ChunkList::Direct(chunks));
    }
    let manifest = postcard::to_stdvec(&chunks).map_err(sotto_core::Error::from)?;
    let manifest_key: [u8; 32] = sotto_core::rand::bytes();
    let up = upload(conn, &manifest_key, &mut &manifest[..], None, max_reads).await?;
    if up.chunks.len() > DIRECT_LIMIT {
        return Err(Error::Invalid("file too large"));
    }
    Ok(ChunkList::Manifest { chunks: up.chunks, manifest_len: manifest.len() as u64, manifest_key })
}

async fn get_chunk(conn: &RelayConn, c: &ChunkRef) -> Result<Vec<u8>> {
    let (fid, seed) = (c.fid, c.read_key);
    match conn.request(move |n| Cmd::FGet { fid, sig: sign(&seed, &auth_message(n, "fget", &[&fid])) }).await? {
        Resp::FData { data } => Ok(data),
        _ => Err(Error::BadResponse),
    }
}

/// Download, decrypt and verify a file into `out`. Returns the size.
pub async fn download(
    conn: &RelayConn,
    file_key: &[u8; 32],
    list: &ChunkList,
    expected_size: u64,
    expected_sha: &[u8; 32],
    out: &mut (dyn Write + Send),
) -> Result<u64> {
    let chunks = match list {
        ChunkList::Direct(c) => c.clone(),
        ChunkList::Manifest { chunks, manifest_len, manifest_key } => {
            if *manifest_len > (DIRECT_LIMIT * FILE_CHUNK) as u64 {
                return Err(Error::Integrity);
            }
            let mut manifest = Vec::new();
            for (i, c) in chunks.iter().enumerate() {
                manifest.extend_from_slice(&decrypt_chunk(manifest_key, i as u32, &get_chunk(conn, c).await?)?);
            }
            if manifest.len() as u64 != *manifest_len {
                return Err(Error::Integrity);
            }
            postcard::from_bytes(&manifest).map_err(|_| Error::Integrity)?
        }
    };
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    for (i, c) in chunks.iter().enumerate() {
        let plain = decrypt_chunk(file_key, i as u32, &get_chunk(conn, c).await?).map_err(|_| Error::Integrity)?;
        size += plain.len() as u64;
        if size > expected_size {
            return Err(Error::Integrity);
        }
        hasher.update(&plain[..]);
        out.write_all(&plain).map_err(sotto_core::Error::from)?;
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if size != expected_size || &digest != expected_sha {
        return Err(Error::Integrity);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_are_fixed_size_and_bound_to_index() {
        let k = sotto_core::rand::bytes();
        let a = encrypt_chunk(&k, 0, b"short").unwrap();
        let b = encrypt_chunk(&k, 1, &vec![1u8; FILE_CHUNK]).unwrap();
        assert_eq!(a.len(), sotto_core::wire::FILE_BLOB_SIZE);
        assert_eq!(b.len(), sotto_core::wire::FILE_BLOB_SIZE);
        assert_eq!(&decrypt_chunk(&k, 0, &a).unwrap()[..], b"short");
        // A chunk moved to another position does not decrypt.
        assert!(decrypt_chunk(&k, 1, &a).is_err());
    }
}
