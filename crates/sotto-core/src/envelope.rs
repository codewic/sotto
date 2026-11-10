//! Queue envelopes and the application payloads inside ratchet messages.

use serde::{Deserialize, Serialize};

use crate::pqxdh::JoinHeader;
use crate::ratchet::RatchetMessage;
use crate::wire::BLOB_SIZE;
use crate::{Id, QueueAddr, Result, pad};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Envelope {
    Join { header: JoinHeader, msg: RatchetMessage },
    Msg(RatchetMessage),
}

impl Envelope {
    /// Serialize and pad to exactly `BLOB_SIZE` bytes.
    pub fn to_blob(&self) -> Result<Vec<u8>> {
        pad::pad(&postcard::to_stdvec(self)?, BLOB_SIZE)
    }

    pub fn from_blob(blob: &[u8]) -> Result<Self> {
        if blob.len() != BLOB_SIZE {
            return Err(crate::Error::Malformed("blob size"));
        }
        Ok(postcard::from_bytes(pad::unpad(blob)?)?)
    }
}

/// Largest serialized payload that is guaranteed to fit in a join envelope.
pub const MAX_PAYLOAD: usize = BLOB_SIZE - 2048;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Profile {
    /// Display name or pseudonym. Empty means "no name shared".
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkRef {
    pub fid: Id,
    /// Ed25519 seed that authorises `FGet` for this chunk.
    pub read_key: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChunkList {
    Direct(Vec<ChunkRef>),
    /// The chunk list was too long for one envelope and is stored as its own file.
    Manifest { chunks: Vec<ChunkRef>, manifest_len: u64, manifest_key: [u8; 32] },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    pub id: Id,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub sha256: [u8; 32],
    pub key: [u8; 32],
    pub relay: String,
    pub chunks: ChunkList,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Payload {
    /// First message from a joiner: where to reply, and an optional name.
    Hello { reply_queue: QueueAddr, profile: Profile },
    Text { id: Id, body: String, sent_at: u64, ttl_secs: Option<u32> },
    File { id: Id, file: FileRef, caption: String, sent_at: u64, ttl_secs: Option<u32> },
    /// Delivery receipts for message ids.
    Receipt { ids: Vec<Id> },
    /// Ask the other side to delete messages (honoured by well-behaved clients).
    Delete { ids: Vec<Id> },
    /// Switch future messages to a new queue.
    QueueRotate { queue: QueueAddr },
    Profile(Profile),
    /// Disappearing-message timer change for the conversation.
    Timer { ttl_secs: Option<u32> },
    /// Cases: ask a colleague's client for an MLS key package.
    KeyPackageRequest { case_id: Id },
    KeyPackage { case_id: Id, data: Vec<u8> },
    /// Cases: MLS Welcome plus the group queue capabilities.
    CaseInvite { case_id: Id, title: String, welcome: Vec<u8>, relay: String, read_id: Id, write_id: Id, owner_key: Vec<u8> },
    /// Insurance file: capability to fetch a time-locked package after its deadline, and the key to open it.
    /// The relay holds the ciphertext and refuses it until the deadline; the colleague holds only the key.
    InsuranceGrant { case_title: String, relay: String, read_id: Id, deadline: u64, key: [u8; 32], chunks: u32 },
    /// The owner checked in: the package stays locked until the new deadline.
    InsuranceUpdate { read_id: Id, deadline: u64 },
}

impl Payload {
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let b = postcard::to_stdvec(self)?;
        if b.len() > MAX_PAYLOAD {
            return Err(crate::Error::TooLarge);
        }
        Ok(b)
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self> {
        Ok(postcard::from_bytes(b)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invite::Invite;
    use crate::keys::Identity;

    #[test]
    fn join_envelope_with_max_payload_fits() {
        let a = Identity::generate();
        let b = Identity::generate();
        let q = QueueAddr { relay: "ws://".to_string() + &"x".repeat(62) + ".onion", snd_id: [1; 16] };
        let (inv, _) = Invite::create(&a, q.clone(), "label", 60).unwrap();
        let (header, mut r) = crate::pqxdh::join(&b, &inv).unwrap();
        let body = "x".repeat(MAX_PAYLOAD - 64);
        let p = Payload::Text { id: [0; 16], body, sent_at: 0, ttl_secs: Some(1) }.to_bytes().unwrap();
        let env = Envelope::Join { header, msg: r.encrypt(&p).unwrap() };
        let blob = env.to_blob().unwrap();
        assert_eq!(blob.len(), BLOB_SIZE);
        assert_eq!(Envelope::from_blob(&blob).unwrap(), env);
        assert!(Payload::Text { id: [0; 16], body: "x".repeat(MAX_PAYLOAD), sent_at: 0, ttl_secs: None }.to_bytes().is_err());
    }
}
