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

/// Text that must never reach a log: its `Debug` prints nothing of it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Hidden(pub String);

impl std::fmt::Debug for Hidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Hidden(..)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Payload {
    /// First message from a joiner: where to reply, and an optional name.
    Hello { reply_queue: QueueAddr, profile: Profile },
    Text { id: Id, body: String, sent_at: u64, ttl_secs: Option<u32>, reply_to: Option<Id> },
    /// Add or remove one reaction to a message.
    Reaction { target: Id, emoji: String, on: bool },
    /// Replace the text of a message the sender wrote earlier.
    Edit { target: Id, body: String },
    /// Read receipts. Sent only when the reader turned them on for this conversation.
    Read { ids: Vec<Id> },
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
    /// Call signalling. Media never travels here: it goes to the call server, end-to-end
    /// encrypted with the key carried in the offer. Added last so older clients still decode
    /// every earlier variant.
    Call(CallSignal),
    /// Groups: MLS Welcome plus the group queue capabilities (like `CaseInvite`, for a group chat).
    GroupInvite { group_id: Id, title: String, welcome: Vec<u8>, relay: String, read_id: Id, write_id: Id, owner_key: Vec<u8> },
    /// Channels: one-way posts from a desk, sent separately over each subscriber's own
    /// conversation so subscribers never learn about each other. See docs/groups.md.
    Channel(ChannelSignal),
    /// A one-time secret (a password, a key): shown once on the receiver's device, then deleted.
    /// The sender keeps only the label. `expires_at` is when an unopened secret is destroyed.
    Secret { id: Id, label: String, body: Hidden, sent_at: u64, expires_at: u64 },
    /// The receiver opened a secret (so the sender can see it was used, and by when).
    SecretOpened { id: Id },
    /// The avatar the sender picked from Sotto's built-in collection (`v1:shape:hue:turn`), or
    /// `None` for their default. Never an image: only the choice travels.
    Avatar { id: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelSignal {
    /// Owner to a contact: would you like to follow this channel?
    Invite { channel_id: Id, title: String, about: String },
    /// Subscriber to owner: yes, follow it.
    Join { channel_id: Id },
    /// Subscriber to owner: stop following (or turn down the invitation).
    Leave { channel_id: Id },
    /// Owner to subscriber: you no longer follow this channel (`removed`), or it closed.
    Closed { channel_id: Id, removed: bool },
    /// Owner to subscriber: a new name or description.
    Meta { channel_id: Id, title: String, about: String },
    Post { channel_id: Id, post_id: Id, body: String, sent_at: u64 },
    Edit { channel_id: Id, post_id: Id, body: String },
    Delete { channel_id: Id, post_id: Id },
    /// Subscriber to owner only: a private reply to one post.
    Reply { channel_id: Id, post_id: Id, reply_id: Id, body: String, sent_at: u64 },
    /// Owner to subscriber: a post carrying a file (`caption` is its text).
    File { channel_id: Id, post_id: Id, file: Box<FileRef>, caption: String, sent_at: u64 },
    /// Subscriber to owner, only if the subscriber turned read receipts on for this channel.
    Seen { channel_id: Id, post_ids: Vec<Id> },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallSignal {
    /// "Please join this call": where, with which join token, and the media key.
    /// `group`: you're being added to a call that's already going.
    /// `room`: the room's name at the call broker, so anyone in the call can add people.
    Offer { call_id: Id, url: String, room: String, token: String, key: [u8; 32], video: bool, expires_at: u64, group: bool },
    /// The other side answered and is joining.
    Answer { call_id: Id },
    /// An explicit "can't talk now". Declining is silent unless the person chooses this.
    Decline { call_id: Id },
    /// The caller gave up before an answer.
    Cancel { call_id: Id },
    /// Either side hung up.
    End { call_id: Id },
}

impl CallSignal {
    pub fn call_id(&self) -> Id {
        match self {
            CallSignal::Offer { call_id, .. }
            | CallSignal::Answer { call_id }
            | CallSignal::Decline { call_id }
            | CallSignal::Cancel { call_id }
            | CallSignal::End { call_id } => *call_id,
        }
    }
}

// Offers carry a join token and the media key: keep both out of debug output.
impl std::fmt::Debug for CallSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallSignal::Offer { call_id, url, video, expires_at, group, .. } => f
                .debug_struct("Offer")
                .field("call_id", &crate::hex(call_id))
                .field("url", url)
                .field("token", &"[hidden]")
                .field("key", &"[hidden]")
                .field("video", video)
                .field("expires_at", expires_at)
                .field("group", group)
                .finish(),
            other => write!(f, "{}({})", match other {
                CallSignal::Answer { .. } => "Answer",
                CallSignal::Decline { .. } => "Decline",
                CallSignal::Cancel { .. } => "Cancel",
                _ => "End",
            }, crate::hex(&other.call_id())),
        }
    }
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
        let p = Payload::Text { id: [0; 16], body, sent_at: 0, ttl_secs: Some(1), reply_to: None }.to_bytes().unwrap();
        let env = Envelope::Join { header, msg: r.encrypt(&p).unwrap() };
        let blob = env.to_blob().unwrap();
        assert_eq!(blob.len(), BLOB_SIZE);
        assert_eq!(Envelope::from_blob(&blob).unwrap(), env);
        assert!(Payload::Text { id: [0; 16], body: "x".repeat(MAX_PAYLOAD), sent_at: 0, ttl_secs: None, reply_to: None }.to_bytes().is_err());
    }

    #[test]
    fn call_offer_fits_and_hides_its_secrets() {
        // A real LiveKit join token with room settings is about 600 bytes; allow far more.
        let offer = Payload::Call(CallSignal::Offer {
            call_id: [7; 16],
            url: "wss://rtc.example.org".into(),
            room: "r".repeat(26),
            token: "t".repeat(4096),
            key: [9; 32],
            video: false,
            expires_at: 1,
            group: true,
        });
        let b = offer.to_bytes().unwrap();
        assert_eq!(Payload::from_bytes(&b).unwrap(), offer);
        let shown = format!("{offer:?}");
        assert!(!shown.contains("tttt") && !shown.contains("[9, 9"), "{shown}");
        assert!(shown.contains("[hidden]"));
    }
}
