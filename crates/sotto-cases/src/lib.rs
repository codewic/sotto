//! MLS (RFC 9420) for cases, via OpenMLS.
//!
//! One `Mls` client per desk holds every case group. Its whole state lives in an
//! in-memory store that the agent snapshots into the encrypted vault after each
//! change. Handshake messages use the pure-ciphertext wire format, so a relay
//! cannot even see who joined or left. Only the case owner commits; members send
//! application messages only, which avoids competing commits on the ordered
//! relay queue that acts as the delivery service.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::RwLock;

use openmls::prelude::tls_codec::{Deserialize as TlsDeserialize, Serialize as TlsSerialize};
use openmls::prelude::*;
use openmls_basic_credential::SignatureKeyPair;
use openmls_memory_storage::MemoryStorage;
use openmls_rust_crypto::RustCrypto;
use serde::{Deserialize, Serialize};

pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;

#[derive(Debug, thiserror::Error)]
pub enum CaseError {
    #[error("case cryptography failed: {0}")]
    Mls(String),
    #[error("unknown case")]
    UnknownGroup,
    #[error("malformed case message")]
    Malformed,
}

fn mls<E: std::fmt::Debug>(e: E) -> CaseError {
    CaseError::Mls(format!("{e:?}"))
}

pub type Result<T> = core::result::Result<T, CaseError>;

#[derive(Default)]
struct Provider {
    crypto: RustCrypto,
    storage: MemoryStorage,
}

impl OpenMlsProvider for Provider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MemoryStorage;
    fn storage(&self) -> &MemoryStorage {
        &self.storage
    }
    fn crypto(&self) -> &RustCrypto {
        &self.crypto
    }
    fn rand(&self) -> &RustCrypto {
        &self.crypto
    }
}

/// Everything needed to rebuild the client. Stored encrypted in the vault.
#[derive(Clone, Serialize, Deserialize)]
pub struct MlsState {
    storage: Vec<(Vec<u8>, Vec<u8>)>,
    sig_public: Vec<u8>,
    identity: Vec<u8>,
}

/// What processing a group message produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Processed {
    /// `sender_key` is the sender's MLS signature key: use it, not the display name, for authorisation.
    Application { sender: Vec<u8>, sender_key: Vec<u8>, data: Vec<u8> },
    Commit { added: Vec<Vec<u8>>, removed: Vec<Vec<u8>> },
    /// A proposal or other message with nothing for the application to do.
    Other,
}

pub struct Mls {
    provider: Provider,
    signer: SignatureKeyPair,
    credential: CredentialWithKey,
    identity: Vec<u8>,
}

fn create_config() -> MlsGroupCreateConfig {
    MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .max_past_epochs(8)
        .build()
}

fn join_config() -> MlsGroupJoinConfig {
    MlsGroupJoinConfig::builder()
        .wire_format_policy(PURE_CIPHERTEXT_WIRE_FORMAT_POLICY)
        .use_ratchet_tree_extension(true)
        .max_past_epochs(8)
        .build()
}

fn identity_of(c: &Credential) -> Vec<u8> {
    BasicCredential::try_from(c.clone()).map(|b| b.identity().to_vec()).unwrap_or_default()
}

impl Mls {
    /// A new client. `identity` is what other members see (a display name, never a key).
    pub fn new(identity: &[u8]) -> Result<Self> {
        let provider = Provider::default();
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).map_err(mls)?;
        signer.store(provider.storage()).map_err(mls)?;
        let credential = CredentialWithKey { credential: BasicCredential::new(identity.to_vec()).into(), signature_key: signer.public().into() };
        Ok(Self { provider, signer, credential, identity: identity.to_vec() })
    }

    pub fn restore(state: &MlsState) -> Result<Self> {
        let provider =
            Provider { crypto: RustCrypto::default(), storage: MemoryStorage { values: RwLock::new(state.storage.iter().cloned().collect::<HashMap<_, _>>()) } };
        let signer = SignatureKeyPair::read(provider.storage(), &state.sig_public, CIPHERSUITE.signature_algorithm()).ok_or(CaseError::Malformed)?;
        let credential = CredentialWithKey { credential: BasicCredential::new(state.identity.clone()).into(), signature_key: signer.public().into() };
        Ok(Self { provider, signer, credential, identity: state.identity.clone() })
    }

    pub fn snapshot(&self) -> MlsState {
        let values = self.provider.storage.values.read().unwrap_or_else(|p| p.into_inner());
        MlsState { storage: values.iter().map(|(k, v)| (k.clone(), v.clone())).collect(), sig_public: self.signer.to_public_vec(), identity: self.identity.clone() }
    }

    pub fn identity(&self) -> &[u8] {
        &self.identity
    }

    /// This desk's MLS signature public key (pinned by members as the case owner's key).
    pub fn signature_key(&self) -> Vec<u8> {
        self.signer.to_public_vec()
    }

    /// A one-time key package so a case owner can add this desk.
    pub fn key_package(&self) -> Result<Vec<u8>> {
        let bundle = KeyPackage::builder().build(CIPHERSUITE, &self.provider, &self.signer, self.credential.clone()).map_err(mls)?;
        MlsMessageOut::from(bundle.key_package().clone()).tls_serialize_detached().map_err(mls)
    }

    fn load(&self, group_id: &[u8]) -> Result<MlsGroup> {
        MlsGroup::load(self.provider.storage(), &GroupId::from_slice(group_id)).map_err(mls)?.ok_or(CaseError::UnknownGroup)
    }

    pub fn create_group(&self) -> Result<Vec<u8>> {
        let group = MlsGroup::new(&self.provider, &self.signer, &create_config(), self.credential.clone()).map_err(mls)?;
        Ok(group.group_id().as_slice().to_vec())
    }

    /// Add a member from their key package. Returns `(commit, welcome)`; the commit is already applied locally.
    pub fn add_member(&self, group_id: &[u8], key_package: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let msg = MlsMessageIn::tls_deserialize_exact(key_package).map_err(|_| CaseError::Malformed)?;
        let kp = match msg.extract() {
            MlsMessageBodyIn::KeyPackage(kp) => kp.validate(self.provider.crypto(), ProtocolVersion::Mls10).map_err(mls)?,
            _ => return Err(CaseError::Malformed),
        };
        let mut group = self.load(group_id)?;
        let (commit, welcome, _) = group.add_members(&self.provider, &self.signer, &[kp]).map_err(mls)?;
        group.merge_pending_commit(&self.provider).map_err(mls)?;
        Ok((commit.tls_serialize_detached().map_err(mls)?, welcome.tls_serialize_detached().map_err(mls)?))
    }

    /// Remove members by identity. Returns the commit, already applied locally.
    pub fn remove_members(&self, group_id: &[u8], identities: &[Vec<u8>]) -> Result<Vec<u8>> {
        let mut group = self.load(group_id)?;
        let leaves: Vec<LeafNodeIndex> = group.members().filter(|m| identities.contains(&identity_of(&m.credential))).map(|m| m.index).collect();
        if leaves.is_empty() {
            return Err(CaseError::Malformed);
        }
        let (commit, _, _) = group.remove_members(&self.provider, &self.signer, &leaves).map_err(mls)?;
        group.merge_pending_commit(&self.provider).map_err(mls)?;
        commit.tls_serialize_detached().map_err(mls)
    }

    /// Join from a Welcome. Returns the group id.
    pub fn join(&self, welcome: &[u8]) -> Result<Vec<u8>> {
        let msg = MlsMessageIn::tls_deserialize_exact(welcome).map_err(|_| CaseError::Malformed)?;
        let welcome = match msg.extract() {
            MlsMessageBodyIn::Welcome(w) => w,
            _ => return Err(CaseError::Malformed),
        };
        let group = StagedWelcome::new_from_welcome(&self.provider, &join_config(), welcome, None).map_err(mls)?.into_group(&self.provider).map_err(mls)?;
        Ok(group.group_id().as_slice().to_vec())
    }

    pub fn encrypt(&self, group_id: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut group = self.load(group_id)?;
        let out = group.create_message(&self.provider, &self.signer, plaintext).map_err(mls)?;
        out.tls_serialize_detached().map_err(mls)
    }

    /// Process a group message. Commits are merged only if `committer` is `None` or matches the
    /// committing member's signature key; anything else is rejected without changing state.
    pub fn process(&self, group_id: &[u8], message: &[u8], committer: Option<&[u8]>) -> Result<Processed> {
        let msg = MlsMessageIn::tls_deserialize_exact(message).map_err(|_| CaseError::Malformed)?;
        let protocol: ProtocolMessage = match msg.extract() {
            MlsMessageBodyIn::PrivateMessage(m) => m.into(),
            MlsMessageBodyIn::PublicMessage(m) => m.into(),
            _ => return Err(CaseError::Malformed),
        };
        let mut group = self.load(group_id)?;
        let processed = group.process_message(&self.provider, protocol).map_err(mls)?;
        let sender = identity_of(processed.credential());
        let sender_key = match processed.sender() {
            Sender::Member(idx) => group.members().find(|m| m.index == *idx).map(|m| m.signature_key.clone()).unwrap_or_default(),
            _ => Vec::new(),
        };
        match processed.into_content() {
            ProcessedMessageContent::ApplicationMessage(app) => Ok(Processed::Application { sender, sender_key, data: app.into_bytes() }),
            ProcessedMessageContent::StagedCommitMessage(staged) => {
                if let Some(allowed) = committer {
                    if sender_key != allowed {
                        return Err(CaseError::Mls("commit from someone other than the case owner".into()));
                    }
                }
                let before: Vec<Vec<u8>> = group.members().map(|m| identity_of(&m.credential)).collect();
                group.merge_staged_commit(&self.provider, *staged).map_err(mls)?;
                let after: Vec<Vec<u8>> = group.members().map(|m| identity_of(&m.credential)).collect();
                Ok(Processed::Commit {
                    added: after.iter().filter(|i| !before.contains(i)).cloned().collect(),
                    removed: before.iter().filter(|i| !after.contains(i)).cloned().collect(),
                })
            }
            _ => Ok(Processed::Other),
        }
    }

    pub fn members(&self, group_id: &[u8]) -> Result<Vec<Vec<u8>>> {
        Ok(self.load(group_id)?.members().map(|m| identity_of(&m.credential)).collect())
    }

    /// Whether this client is still in the group (false after being removed).
    pub fn is_active(&self, group_id: &[u8]) -> bool {
        self.load(group_id).map(|g| g.is_active()).unwrap_or(false)
    }

    pub fn forget(&self, group_id: &[u8]) -> Result<()> {
        let mut group = self.load(group_id)?;
        group.delete(self.provider.storage()).map_err(mls)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_adds_member_exchange_remove_and_restore() {
        let owner = Mls::new(b"Ada").unwrap();
        let editor = Mls::new(b"Ben").unwrap();
        let lawyer = Mls::new(b"Cleo").unwrap();
        let gid = owner.create_group().unwrap();

        let (_c1, welcome) = owner.add_member(&gid, &editor.key_package().unwrap()).unwrap();
        assert_eq!(editor.join(&welcome).unwrap(), gid);

        let m = owner.encrypt(&gid, b"Note: the contract is dated March").unwrap();
        assert_eq!(editor.process(&gid, &m, None).unwrap(), Processed::Application { sender: b"Ada".to_vec(), sender_key: owner.signature_key(), data: b"Note: the contract is dated March".to_vec() });
        let r = editor.encrypt(&gid, b"Got it").unwrap();
        assert!(matches!(owner.process(&gid, &r, Some(&owner.signature_key())).unwrap(), Processed::Application { .. }));

        let (c2, w2) = owner.add_member(&gid, &lawyer.key_package().unwrap()).unwrap();
        assert_eq!(editor.process(&gid, &c2, Some(&owner.signature_key())).unwrap(), Processed::Commit { added: vec![b"Cleo".to_vec()], removed: vec![] });
        lawyer.join(&w2).unwrap();
        assert_eq!(owner.members(&gid).unwrap().len(), 3);

        let editor = Mls::restore(&editor.snapshot()).unwrap();
        let m = owner.encrypt(&gid, b"after restore").unwrap();
        assert!(matches!(editor.process(&gid, &m, Some(&owner.signature_key())).unwrap(), Processed::Application { .. }));
        assert!(matches!(lawyer.process(&gid, &m, Some(&owner.signature_key())).unwrap(), Processed::Application { .. }));

        let c3 = owner.remove_members(&gid, &[b"Cleo".to_vec()]).unwrap();
        assert_eq!(editor.process(&gid, &c3, Some(&owner.signature_key())).unwrap(), Processed::Commit { added: vec![], removed: vec![b"Cleo".to_vec()] });
        let _ = lawyer.process(&gid, &c3, Some(&owner.signature_key()));
        let secret = owner.encrypt(&gid, b"not for Cleo").unwrap();
        assert!(lawyer.process(&gid, &secret, Some(&owner.signature_key())).is_err());
        assert!(matches!(editor.process(&gid, &secret, Some(&owner.signature_key())).unwrap(), Processed::Application { .. }));
    }

    #[test]
    fn commits_from_non_owners_are_rejected() {
        let owner = Mls::new(b"Ada").unwrap();
        let editor = Mls::new(b"Ben").unwrap();
        let rogue = Mls::new(b"Cleo").unwrap();
        let gid = owner.create_group().unwrap();
        let (_, w) = owner.add_member(&gid, &editor.key_package().unwrap()).unwrap();
        editor.join(&w).unwrap();
        // The editor (not the owner) tries to add someone: other members refuse the commit.
        let (rogue_commit, _) = editor.add_member(&gid, &rogue.key_package().unwrap()).unwrap();
        assert!(owner.process(&gid, &rogue_commit, Some(&owner.signature_key())).is_err());
        assert_eq!(owner.members(&gid).unwrap().len(), 2);
    }

    #[test]
    fn garbage_is_rejected() {
        let a = Mls::new(b"A").unwrap();
        let gid = a.create_group().unwrap();
        assert!(a.process(&gid, b"nonsense", None).is_err());
        assert!(a.join(b"nonsense").is_err());
        assert!(a.add_member(&gid, b"nonsense").is_err());
        assert!(matches!(a.encrypt(b"no-such-group", b"x"), Err(CaseError::UnknownGroup)));
    }
}
