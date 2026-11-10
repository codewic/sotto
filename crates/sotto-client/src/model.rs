//! Persisted state and the views handed to the UI.

use serde::{Deserialize, Serialize};
use sotto_core::envelope::FileRef;
use sotto_core::invite::InviteSecret;
use sotto_core::keys::{Identity, PublicIdentity};
use sotto_core::ratchet::Ratchet;
use sotto_core::{Id, QueueAddr, hex};

use crate::conn::Transport;

#[derive(Clone, Serialize, Deserialize)]
pub struct Profile {
    /// The listed identity: the one an outlet can publish in its directory.
    pub identity: Identity,
    pub display_name: String,
    pub default_relay: String,
    pub transport: Transport,
    pub created_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MyQueue {
    pub relay: String,
    pub rcv_id: Id,
    pub snd_id: Id,
    pub seed: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContactState {
    /// We sent an invite and are waiting for the other side to use it.
    Invited,
    Ready,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Contact {
    pub id: Id,
    /// Our private name for them.
    pub nickname: String,
    /// The name they chose to share, if any.
    pub their_name: String,
    pub my_identity: Identity,
    pub listed_identity: bool,
    pub their_identity: Option<PublicIdentity>,
    pub ratchet: Option<Ratchet>,
    pub my_queue: MyQueue,
    pub their_queue: Option<QueueAddr>,
    pub state: ContactState,
    pub verified: bool,
    pub ttl_secs: Option<u32>,
    pub created_at: u64,
    pub last_activity: u64,
    pub unread: u32,
    pub invite_id: Option<Id>,
    pub invite_expires: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PendingInvite {
    pub secret: InviteSecret,
    pub contact_id: Id,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    Sending,
    Sent,
    Delivered,
    Failed,
    Received,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Uploading,
    Sent,
    Downloading,
    Ready,
    Failed,
}

/// Safe-review state of a received file. Stored with postcard, so no internally tagged serde here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Review {
    /// We sent it ourselves.
    Own,
    /// Received and not yet reviewed. Never opened outside the sandbox.
    Quarantined,
    /// A clean copy was produced.
    Clean { clean_blob: String, pages: u32 },
    Failed { reason: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileMeta {
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub sha256: [u8; 32],
    pub blob_id: Option<String>,
    pub status: FileStatus,
    pub review: Review,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Body {
    Text(String),
    File { meta: FileMeta, caption: String },
    /// Notes from Sotto itself, such as timer changes.
    System(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: Id,
    pub contact_id: Id,
    pub outgoing: bool,
    pub body: Body,
    pub sent_at: u64,
    pub received_at: u64,
    pub expires_at: Option<u64>,
    pub delivery: Delivery,
    pub remote_file: Option<FileRef>,
}

impl Message {
    pub fn store_name(&self) -> String {
        message_name(&self.contact_id, self.received_at, &self.id)
    }
}

pub fn message_name(contact: &Id, ts_ms: u64, id: &Id) -> String {
    format!("msg/{}/{:020}-{}", hex(contact), ts_ms, hex(id))
}

pub fn contact_name(id: &Id) -> String {
    format!("contact/{}", hex(id))
}

pub fn invite_name(id: &Id) -> String {
    format!("invite/{}", hex(id))
}

// ---------- Views (JSON for the UI) ----------

#[derive(Clone, Debug, Serialize)]
pub struct ContactView {
    pub id: String,
    pub nickname: String,
    pub their_name: String,
    pub state: &'static str,
    pub verified: bool,
    pub ttl_secs: Option<u32>,
    pub unread: u32,
    pub created_at: u64,
    pub last_activity: u64,
    pub listed_identity: bool,
    pub safety_number: Option<String>,
    pub their_fingerprint: Option<String>,
    pub invite_expires: Option<u64>,
}

impl Contact {
    pub fn view(&self) -> ContactView {
        ContactView {
            id: hex(&self.id),
            nickname: self.nickname.clone(),
            their_name: self.their_name.clone(),
            state: match self.state {
                ContactState::Invited => "invited",
                ContactState::Ready => "ready",
            },
            verified: self.verified,
            ttl_secs: self.ttl_secs,
            unread: self.unread,
            created_at: self.created_at,
            last_activity: self.last_activity,
            listed_identity: self.listed_identity,
            safety_number: self
                .their_identity
                .map(|t| sotto_core::safety::safety_number(&self.my_identity.public().sign, &t.sign)),
            their_fingerprint: self.their_identity.map(|t| t.fingerprint()),
            invite_expires: (self.state == ContactState::Invited).then_some(self.invite_expires),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReviewView {
    /// `own`, `quarantined`, `clean` or `failed`.
    pub state: &'static str,
    pub pages: Option<u32>,
    pub reason: Option<String>,
}

impl Review {
    pub fn view(&self) -> ReviewView {
        match self {
            Review::Own => ReviewView { state: "own", pages: None, reason: None },
            Review::Quarantined => ReviewView { state: "quarantined", pages: None, reason: None },
            Review::Clean { pages, .. } => ReviewView { state: "clean", pages: Some(*pages), reason: None },
            Review::Failed { reason } => ReviewView { state: "failed", pages: None, reason: Some(reason.clone()) },
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FileView {
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub status: FileStatus,
    pub review: ReviewView,
    pub caption: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MessageView {
    pub id: String,
    pub contact_id: String,
    pub outgoing: bool,
    pub kind: &'static str,
    pub text: Option<String>,
    pub file: Option<FileView>,
    pub sent_at: u64,
    pub received_at: u64,
    pub expires_at: Option<u64>,
    pub delivery: Delivery,
}

impl Message {
    pub fn view(&self) -> MessageView {
        let (kind, text, file) = match &self.body {
            Body::Text(t) => ("text", Some(t.clone()), None),
            Body::System(t) => ("system", Some(t.clone()), None),
            Body::File { meta, caption } => (
                "file",
                None,
                Some(FileView {
                    name: meta.name.clone(),
                    mime: meta.mime.clone(),
                    size: meta.size,
                    status: meta.status,
                    review: meta.review.view(),
                    caption: caption.clone(),
                }),
            ),
        };
        MessageView {
            id: hex(&self.id),
            contact_id: hex(&self.contact_id),
            outgoing: self.outgoing,
            kind,
            text,
            file,
            sent_at: self.sent_at,
            received_at: self.received_at,
            expires_at: self.expires_at,
            delivery: self.delivery,
        }
    }
}
