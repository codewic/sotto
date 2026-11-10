//! Relay wire protocol: fixed-size frames over WebSocket (`docs/protocol.md` section 7).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::keys::Sig;
use crate::{Error, Id, Result, pad};

pub const PROTOCOL_VERSION: u16 = 1;
/// Every ordinary frame is exactly this many bytes.
pub const FRAME_SIZE: usize = 16 * 1024;
/// Frames that carry a file chunk are exactly this many bytes.
pub const FILE_FRAME_SIZE: usize = 16 * 1024 + 256 * 1024;
/// Every queue blob is exactly this many bytes, so it always fits in a frame.
pub const BLOB_SIZE: usize = FRAME_SIZE - 128;
/// Plaintext bytes per file chunk.
pub const FILE_CHUNK: usize = 256 * 1024;
/// Stored bytes per encrypted file chunk (length prefix + chunk + tag).
pub const FILE_BLOB_SIZE: usize = FILE_CHUNK + 4 + crate::aead::TAG_LEN;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cmd {
    New { rcv_auth: [u8; 32], ttl_days: u8 },
    Send { snd_id: Id, blob: Vec<u8> },
    Sub { rcv_id: Id, sig: Sig },
    Ack { rcv_id: Id, msg_id: Id, sig: Sig },
    Del { rcv_id: Id, sig: Sig },
    GNew { ttl_days: u8 },
    GSend { write_id: Id, blob: Vec<u8> },
    GSub { read_id: Id, from: u64 },
    FPut { read_auth: [u8; 32], max_reads: u8, ttl_days: u8, data: Vec<u8> },
    FGet { fid: Id, sig: Sig },
    FDel { fid: Id, sig: Sig },
    /// Create a time-locked package owned by `owner` (Ed25519 public key).
    TNew { owner: [u8; 32], deadline: u64 },
    /// Store chunk `index` of a package. Signed over `tl_id || index || sha256(blob)`.
    TPut { tl_id: Id, index: u32, blob: Vec<u8>, sig: Sig },
    TCheckIn { tl_id: Id, deadline: u64, sig: Sig },
    /// Fetch chunk `index`; refused with `Locked` until the deadline has passed.
    TGet { read_id: Id, index: u32 },
    TDel { tl_id: Id, sig: Sig },
    Ping,
    Cover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrCode {
    Auth,
    NotFound,
    Full,
    TooLarge,
    BadRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resp {
    Ok,
    Ids { rcv_id: Id, snd_id: Id },
    GIds { read_id: Id, write_id: Id },
    GSeq { seq: u64 },
    FId { fid: Id },
    FData { data: Vec<u8> },
    TIds { tl_id: Id, read_id: Id },
    TData { blob: Vec<u8>, count: u32 },
    Locked { deadline: u64 },
    Pong,
    Err(ErrCode),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientFrame {
    pub corr: u32,
    pub cmd: Cmd,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServerMsg {
    Hello { version: u16, nonce: [u8; 32] },
    Resp { corr: u32, resp: Resp },
    Msg { rcv_id: Id, msg_id: Id, blob: Vec<u8> },
    GMsg { read_id: Id, seq: u64, blob: Vec<u8> },
    Cover,
}

/// Encode into a frame of exactly `FRAME_SIZE` or `FILE_FRAME_SIZE` bytes.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = postcard::to_stdvec(value)?;
    let size = if bytes.len() + 4 <= FRAME_SIZE { FRAME_SIZE } else { FILE_FRAME_SIZE };
    pad::pad(&bytes, size)
}

/// Decode a frame, refusing anything that is not one of the two fixed sizes.
pub fn decode_frame<T: DeserializeOwned>(frame: &[u8]) -> Result<T> {
    if frame.len() != FRAME_SIZE && frame.len() != FILE_FRAME_SIZE {
        return Err(Error::Malformed("frame size"));
    }
    Ok(postcard::from_bytes(pad::unpad(frame)?)?)
}

/// Bytes signed for authenticated relay commands, bound to the connection nonce.
pub fn auth_message(nonce: &[u8; 32], tag: &str, fields: &[&[u8]]) -> Vec<u8> {
    let mut m = b"sotto-relay-v1".to_vec();
    m.extend_from_slice(nonce);
    m.extend_from_slice(tag.as_bytes());
    for f in fields {
        m.extend_from_slice(&(f.len() as u32).to_le_bytes());
        m.extend_from_slice(f);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_have_fixed_sizes() {
        let small = encode_frame(&ClientFrame { corr: 1, cmd: Cmd::Ping }).unwrap();
        assert_eq!(small.len(), FRAME_SIZE);
        let send = encode_frame(&ClientFrame { corr: 2, cmd: Cmd::Send { snd_id: [0; 16], blob: vec![1; BLOB_SIZE] } }).unwrap();
        assert_eq!(send.len(), FRAME_SIZE);
        let file = encode_frame(&ClientFrame {
            corr: 3,
            cmd: Cmd::FPut { read_auth: [0; 32], max_reads: 1, ttl_days: 7, data: vec![1; FILE_BLOB_SIZE] },
        })
        .unwrap();
        assert_eq!(file.len(), FILE_FRAME_SIZE);
        let back: ClientFrame = decode_frame(&file).unwrap();
        assert_eq!(back.corr, 3);
        assert!(decode_frame::<ClientFrame>(&small[..100]).is_err());
    }

    #[test]
    fn auth_messages_are_unambiguous() {
        let n = [1u8; 32];
        assert_ne!(auth_message(&n, "sub", &[b"ab", b"c"]), auth_message(&n, "sub", &[b"a", b"bc"]));
    }
}
