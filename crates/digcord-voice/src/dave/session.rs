//! DAVE'in MLS kısmı: grup oturumu (openmls üzerinde).
//!
//! Ses gateway'inin ikili opcode'larının (25-30) yüklerini işler; sıra ve geçiş
//! (transition) mantığı çağıran tarafta, burada sadece MLS durumu ve doğrulamalar var.
//! Hiçbir sır (anahtar, exporter çıktısı) hata mesajlarına yazılmaz.

use std::collections::HashSet;
use std::fmt::Display;

use openmls::ciphersuite::hash_ref::{HashReference, ProposalRef};
use openmls::credentials::{BasicCredential, Credential, CredentialType, CredentialWithKey};
use openmls::extensions::{Extension, ExtensionType, Extensions, ExternalSender};
use openmls::group::{
    GroupId, MlsGroup, MlsGroupCreateConfig, MlsGroupJoinConfig, PURE_PLAINTEXT_WIRE_FORMAT_POLICY,
    StagedWelcome,
};
use openmls::key_packages::{KeyPackage, Lifetime};
use openmls::prelude::{
    Capabilities, Ciphersuite, MlsMessageIn, ProcessedMessageContent, ProposalOrRefType,
    ProtocolMessage, ProtocolVersion, Sender, Welcome,
};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::OpenMlsRustCrypto;
use openmls_traits::OpenMlsProvider;
use openmls_traits::storage::StorageProvider;
use tls_codec::{DeserializeBytes, Serialize};

const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMP256_AES128GCM_SHA256_P256;
const EXPORTER_LABEL: &str = "Discord Secure Frames v0";
const SECRET_LEN: usize = 16;

/// `Session` hataları. Mesajlar sır içermez.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("malformed {0}")]
    Malformed(&'static str),
    #[error("external sender not set")]
    NoExternalSender,
    #[error("external sender already used by an established group")]
    AlreadyEstablished,
    #[error("no group")]
    NoGroup,
    #[error("no matching key package (a fresh one must be sent first)")]
    NoKeyPackage,
    #[error("group is not established yet")]
    NotEstablished,
    #[error("unexpected commit before welcome")]
    UnexpectedCommit,
    #[error("invalid message: {0}")]
    Invalid(&'static str),
    #[error("unrecognized user in group or proposal")]
    UnrecognizedUser,
    #[error("duplicate or malformed identity in roster")]
    BadRoster,
    #[error("unknown proposal reference")]
    UnknownProposalRef,
    #[error("user is not a group member")]
    UnknownMember,
    #[error("removed from group")]
    Removed,
    #[error("mls {op}: {detail}")]
    Mls { op: &'static str, detail: String },
}

fn mls<E: Display>(op: &'static str) -> impl FnOnce(E) -> SessionError {
    move |e| SessionError::Mls {
        op,
        detail: e.to_string(),
    }
}

/// Tek bir kullanıcının DAVE-MLS oturumu.
pub struct Session {
    provider: OpenMlsRustCrypto,
    signer: SignatureKeyPair,
    user_id: u64,
    channel_id: u64,
    external_sender: Option<ExternalSender>,
    /// Son üretilen key package'in hash referansı (sadece o geçerli).
    key_package_ref: Option<HashReference>,
    group: Option<MlsGroup>,
    /// false: yerel epoch-0 grup (bekleyen); true: Welcome/commit ile kurulmuş grup.
    established: bool,
    /// Kendi yolladığımız commit'in baytları (op 29'da aynısı dönerse kendi commit'imizdir).
    outbound_commit: Option<Vec<u8>>,
}

impl Session {
    pub fn new(user_id: u64, channel_id: u64) -> Result<Self, SessionError> {
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm())
            .map_err(mls("signature key"))?;
        let provider = OpenMlsRustCrypto::default();
        signer.store(provider.storage()).map_err(mls("store key"))?;
        Ok(Self {
            provider,
            signer,
            user_id,
            channel_id,
            external_sender: None,
            key_package_ref: None,
            group: None,
            established: false,
            outbound_commit: None,
        })
    }

    pub fn user_id(&self) -> u64 {
        self.user_id
    }

    /// Grup ve bekleyen durumu atar; imza anahtarı ve external sender kalır.
    pub fn reset(&mut self) {
        self.drop_group();
        self.drop_key_package();
        self.established = false;
        self.outbound_commit = None;
    }

    fn drop_group(&mut self) {
        if let Some(mut g) = self.group.take() {
            // Depolama hatası bellek-içi sağlayıcıda gerçekçi değil; yutmak güvenli.
            let _ = g.delete(self.provider.storage());
        }
    }

    fn drop_key_package(&mut self) {
        if let Some(r) = self.key_package_ref.take() {
            let _ = self.provider.storage().delete_key_package(&r);
        }
    }

    /// op 25.
    pub fn set_external_sender(&mut self, payload: &[u8]) -> Result<(), SessionError> {
        if self.established {
            return Err(SessionError::AlreadyEstablished);
        }
        let sender = ExternalSender::tls_deserialize_exact_bytes(payload)
            .map_err(|_| SessionError::Malformed("external sender"))?;
        self.external_sender = Some(sender);
        // Yeni gönderici: eski bekleyen grup geçersiz.
        self.drop_group();
        self.ensure_pending_group()
    }

    /// Epoch-0 yerel grubu: sadece biz, group id = kanal id (BE).
    fn ensure_pending_group(&mut self) -> Result<(), SessionError> {
        if self.group.is_some() {
            return Ok(());
        }
        let Some(sender) = self.external_sender.clone() else {
            return Ok(());
        };
        let ext = Extensions::single(Extension::ExternalSenders(vec![sender]))
            .map_err(mls("group extensions"))?;
        let config = MlsGroupCreateConfig::builder()
            .ciphersuite(CIPHERSUITE)
            .wire_format_policy(PURE_PLAINTEXT_WIRE_FORMAT_POLICY)
            .use_ratchet_tree_extension(true)
            .capabilities(capabilities())
            .lifetime(lifetime())
            .with_group_context_extensions(ext)
            .build();
        let group = MlsGroup::new_with_group_id(
            &self.provider,
            &self.signer,
            &config,
            GroupId::from_slice(&self.channel_id.to_be_bytes()),
            self.credential_with_key(),
        )
        .map_err(mls("create group"))?;
        self.group = Some(group);
        self.established = false;
        Ok(())
    }

    fn credential_with_key(&self) -> CredentialWithKey {
        CredentialWithKey {
            credential: BasicCredential::new(self.user_id.to_be_bytes().to_vec()).into(),
            signature_key: self.signer.public().into(),
        }
    }

    /// op 26: her çağrıda yeni key package (yalnızca sonuncusu Welcome için geçerli).
    pub fn key_package(&mut self) -> Result<Vec<u8>, SessionError> {
        self.drop_key_package();
        let bundle = KeyPackage::builder()
            .key_package_lifetime(lifetime())
            .leaf_node_capabilities(capabilities())
            .build(
                CIPHERSUITE,
                &self.provider,
                &self.signer,
                self.credential_with_key(),
            )
            .map_err(mls("key package"))?;
        let kp = bundle.key_package();
        self.key_package_ref = Some(
            kp.hash_ref(self.provider.crypto())
                .map_err(mls("key package ref"))?,
        );
        let bytes = kp
            .tls_serialize_detached()
            .map_err(mls("serialize key package"))?;
        self.ensure_pending_group()?;
        Ok(bytes)
    }

    /// op 27 -> op 28 yükü (varsa).
    pub fn process_proposals(
        &mut self,
        payload: &[u8],
        recognized: &HashSet<u64>,
    ) -> Result<Option<Vec<u8>>, SessionError> {
        let (&op, rest) = payload
            .split_first()
            .ok_or(SessionError::Malformed("proposals"))?;
        let body = read_vl_exact(rest).ok_or(SessionError::Malformed("proposals"))?;
        let Some(group) = self.group.as_mut() else {
            return Ok(None);
        };
        match op {
            0 => {
                let mut queued = Vec::new();
                let mut buf = body;
                while !buf.is_empty() {
                    let (msg, next) = MlsMessageIn::tls_deserialize_bytes(buf)
                        .map_err(|_| SessionError::Malformed("proposal message"))?;
                    buf = next;
                    let ProtocolMessage::PublicMessage(pm) = msg
                        .try_into_protocol_message()
                        .map_err(|_| SessionError::Invalid("not a protocol message"))?
                    else {
                        return Err(SessionError::Invalid("proposal is not plaintext"));
                    };
                    let processed = group
                        .process_message(&self.provider, ProtocolMessage::PublicMessage(pm))
                        .map_err(mls("process proposal"))?;
                    if !matches!(processed.sender(), Sender::External(_)) {
                        return Err(SessionError::Invalid("proposal sender not external"));
                    }
                    let ProcessedMessageContent::ProposalMessage(qp) = processed.into_content()
                    else {
                        return Err(SessionError::Invalid("not a proposal"));
                    };
                    match qp.proposal() {
                        openmls::prelude::Proposal::Add(add) => {
                            let uid = user_id_of(add.key_package().leaf_node().credential())?;
                            if !recognized.contains(&uid) {
                                return Err(SessionError::UnrecognizedUser);
                            }
                        }
                        openmls::prelude::Proposal::Remove(_) => {}
                        _ => return Err(SessionError::Invalid("proposal type")),
                    }
                    queued.push(*qp);
                }
                // Tüm yük doğrulandıktan sonra duruma yaz.
                let _ = group.clear_pending_commit(self.provider.storage());
                for qp in queued {
                    let known = group
                        .pending_proposals()
                        .any(|p| p.proposal_reference_ref() == qp.proposal_reference_ref());
                    if !known {
                        group
                            .store_pending_proposal(self.provider.storage(), qp)
                            .map_err(mls("store proposal"))?;
                    }
                }
            }
            1 => {
                let mut refs: Vec<ProposalRef> = Vec::new();
                let mut buf = body;
                while !buf.is_empty() {
                    let (r, next) = HashReference::tls_deserialize_bytes(buf)
                        .map_err(|_| SessionError::Malformed("proposal ref"))?;
                    buf = next;
                    refs.push(r);
                }
                for r in &refs {
                    if !group
                        .pending_proposals()
                        .any(|p| p.proposal_reference_ref() == r)
                    {
                        return Err(SessionError::UnknownProposalRef);
                    }
                }
                let _ = group.clear_pending_commit(self.provider.storage());
                for r in &refs {
                    group
                        .remove_pending_proposal(self.provider.storage(), r)
                        .map_err(mls("remove proposal"))?;
                }
            }
            _ => return Err(SessionError::Malformed("proposals operation")),
        }

        if !group.has_pending_proposals() {
            self.outbound_commit = None;
            return Ok(None);
        }
        let (commit, welcome, _) = group
            .commit_builder()
            .consume_proposal_store(true)
            .load_psks(self.provider.storage())
            .map_err(mls("commit psks"))?
            .build(
                self.provider.rand(),
                self.provider.crypto(),
                &self.signer,
                |_| true,
            )
            .map_err(mls("commit"))?
            .stage_commit(&self.provider)
            .map_err(mls("stage commit"))?
            .into_contents();
        let mut out = commit
            .tls_serialize_detached()
            .map_err(mls("serialize commit"))?;
        self.outbound_commit = Some(out.clone());
        if let Some(w) = welcome {
            out.extend(
                w.tls_serialize_detached()
                    .map_err(mls("serialize welcome"))?,
            );
        }
        Ok(Some(out))
    }

    /// op 29 (transition id çıkarılmış).
    pub fn process_commit(&mut self, payload: &[u8]) -> Result<(), SessionError> {
        let (msg, rest) = MlsMessageIn::tls_deserialize_bytes(payload)
            .map_err(|_| SessionError::Malformed("commit"))?;
        if !rest.is_empty() {
            return Err(SessionError::Malformed("commit trailing bytes"));
        }
        let group = self.group.as_mut().ok_or(SessionError::NoGroup)?;
        if !self.established && !group.has_pending_proposals() {
            return Ok(()); // bekleyen önerimiz yok: yoksay
        }
        if self.outbound_commit.as_deref() == Some(payload) && group.pending_commit().is_some() {
            group
                .merge_pending_commit(&self.provider)
                .map_err(mls("merge own commit"))?;
        } else {
            if !self.established {
                return Err(SessionError::UnexpectedCommit);
            }
            let ProtocolMessage::PublicMessage(pm) = msg
                .try_into_protocol_message()
                .map_err(|_| SessionError::Invalid("not a protocol message"))?
            else {
                return Err(SessionError::Invalid("commit is not plaintext"));
            };
            if pm.group_id().as_slice() != self.channel_id.to_be_bytes() {
                return Err(SessionError::Invalid("group id"));
            }
            // Kendi bekleyen commit'imiz yarışı kaybetti.
            let _ = group.clear_pending_commit(self.provider.storage());
            let processed = group
                .process_message(&self.provider, ProtocolMessage::PublicMessage(pm))
                .map_err(mls("process commit"))?;
            if !matches!(processed.sender(), Sender::Member(_)) {
                return Err(SessionError::Invalid("commit sender not a member"));
            }
            let ProcessedMessageContent::StagedCommitMessage(staged) = processed.into_content()
            else {
                return Err(SessionError::Invalid("not a commit"));
            };
            if staged
                .queued_proposals()
                .any(|p| p.proposal_or_ref_type() != ProposalOrRefType::Reference)
            {
                return Err(SessionError::Invalid("inline proposal in commit"));
            }
            group
                .merge_staged_commit(&self.provider, *staged)
                .map_err(mls("merge commit"))?;
            if !group.is_active() {
                self.reset();
                return Err(SessionError::Removed);
            }
        }
        self.established = true;
        self.outbound_commit = None;
        if let Err(e) = self.check_roster(None) {
            self.reset();
            return Err(e);
        }
        Ok(())
    }

    /// op 30 (transition id çıkarılmış).
    pub fn process_welcome(
        &mut self,
        payload: &[u8],
        recognized: &HashSet<u64>,
    ) -> Result<(), SessionError> {
        let welcome = Welcome::tls_deserialize_exact_bytes(payload)
            .map_err(|_| SessionError::Malformed("welcome"))?;
        if self.established {
            return Err(SessionError::AlreadyEstablished);
        }
        if self.key_package_ref.is_none() {
            return Err(SessionError::NoKeyPackage);
        }
        let sender = self
            .external_sender
            .clone()
            .ok_or(SessionError::NoExternalSender)?;
        let join_config = MlsGroupJoinConfig::builder()
            .wire_format_policy(PURE_PLAINTEXT_WIRE_FORMAT_POLICY)
            .use_ratchet_tree_extension(true)
            .build();
        let result = StagedWelcome::build_from_welcome(&self.provider, &join_config, welcome)
            .and_then(|b| b.replace_old_group().build());
        // Key package her durumda tüketildi.
        self.key_package_ref = None;
        let staged = result.map_err(mls("welcome"))?;
        let ctx = staged.group_context();
        if ctx.group_id().as_slice() != self.channel_id.to_be_bytes() {
            return Err(SessionError::Invalid("group id"));
        }
        let exts = ctx.extensions();
        if exts.iter().count() != 1 || exts.external_senders() != Some(&vec![sender]) {
            return Err(SessionError::Invalid("external senders extension"));
        }
        let group = staged
            .into_group(&self.provider)
            .map_err(mls("join group"))?;
        // Eski bekleyen grup replace_old_group ile değişti.
        self.group = Some(group);
        self.established = true;
        self.outbound_commit = None;
        if let Err(e) = self.check_roster(Some(recognized)) {
            self.reset();
            return Err(e);
        }
        Ok(())
    }

    /// Kimlikler: sadece basic, 8 bayt, tekrarsız (ve verilirse tanınan kullanıcılar).
    fn check_roster(&self, recognized: Option<&HashSet<u64>>) -> Result<(), SessionError> {
        let group = self.group.as_ref().ok_or(SessionError::NoGroup)?;
        let mut seen = HashSet::new();
        for m in group.members() {
            let uid = user_id_of(&m.credential).map_err(|_| SessionError::BadRoster)?;
            if !seen.insert(uid) {
                return Err(SessionError::BadRoster);
            }
            if let Some(r) = recognized
                && !r.contains(&uid)
            {
                return Err(SessionError::UnrecognizedUser);
            }
        }
        Ok(())
    }

    /// Kurulu epoch'taki `user_id` için 16 baytlık taban sır.
    pub fn sender_secret(&self, user_id: u64) -> Result<[u8; SECRET_LEN], SessionError> {
        let group = self.established_group()?;
        if !self.members().contains(&user_id) {
            return Err(SessionError::UnknownMember);
        }
        let v = group
            .export_secret(
                self.provider.crypto(),
                EXPORTER_LABEL,
                &user_id.to_le_bytes(),
                SECRET_LEN,
            )
            .map_err(mls("export secret"))?;
        v.try_into()
            .map_err(|_| SessionError::Invalid("exporter length"))
    }

    fn established_group(&self) -> Result<&MlsGroup, SessionError> {
        match &self.group {
            Some(g) if self.established => Ok(g),
            _ => Err(SessionError::NotEstablished),
        }
    }

    pub fn members(&self) -> Vec<u64> {
        let Ok(group) = self.established_group() else {
            return Vec::new();
        };
        group
            .members()
            .filter_map(|m| user_id_of(&m.credential).ok())
            .collect()
    }

    pub fn epoch(&self) -> Option<u64> {
        self.established_group().ok().map(|g| g.epoch().as_u64())
    }
}

/// op 28 yükünü (commit || welcome?) ayırır: (commit, welcome).
pub fn split_commit_welcome(payload: &[u8]) -> Result<(&[u8], &[u8]), SessionError> {
    let (_, rest) = MlsMessageIn::tls_deserialize_bytes(payload)
        .map_err(|_| SessionError::Malformed("commit"))?;
    Ok(payload.split_at(payload.len() - rest.len()))
}

fn capabilities() -> Capabilities {
    Capabilities::builder()
        .versions(vec![ProtocolVersion::Mls10])
        .ciphersuites(vec![CIPHERSUITE])
        .extensions(Vec::<ExtensionType>::new())
        .proposals(Vec::new())
        .credentials(vec![CredentialType::Basic])
        .build()
}

fn lifetime() -> Lifetime {
    Lifetime::init(0, u64::MAX)
}

fn user_id_of(credential: &Credential) -> Result<u64, SessionError> {
    let basic = BasicCredential::try_from(credential.clone())
        .map_err(|_| SessionError::Invalid("credential type"))?;
    let id: [u8; 8] = basic
        .identity()
        .try_into()
        .map_err(|_| SessionError::Invalid("identity length"))?;
    Ok(u64::from_be_bytes(id))
}

/// RFC 9420 §2.1.2 değişken uzunluk önekli vektör; `buf` tam olarak onu içermeli.
fn read_vl_exact(buf: &[u8]) -> Option<&[u8]> {
    let first = *buf.first()?;
    let (hdr, len, min) = match first >> 6 {
        0 => (1, usize::from(first & 0x3f), 0),
        1 => (
            2,
            usize::from(first & 0x3f) << 8 | usize::from(*buf.get(1)?),
            64,
        ),
        2 => (
            4,
            usize::from(first & 0x3f) << 24
                | usize::from(*buf.get(1)?) << 16
                | usize::from(*buf.get(2)?) << 8
                | usize::from(*buf.get(3)?),
            16384,
        ),
        _ => return None,
    };
    if len < min || buf.len() != hdr + len {
        return None;
    }
    buf.get(hdr..)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use openmls::extensions::SenderExtensionIndex;
    use openmls::group::GroupEpoch;
    use openmls::key_packages::KeyPackageIn;
    use openmls::prelude::{ExternalProposal, LeafNodeIndex};
    use openmls_traits::types::SignatureScheme;

    pub(crate) const CHANNEL: u64 = 4242;

    /// Sahte ses sunucusu (dış gönderici); diğer modüllerin testleri de kullanır.
    pub(crate) struct Server {
        signer: SignatureKeyPair,
        pub(crate) sender_bytes: Vec<u8>,
        provider: OpenMlsRustCrypto,
    }

    impl Server {
        pub(crate) fn new() -> Self {
            let signer = SignatureKeyPair::new(SignatureScheme::ECDSA_SECP256R1_SHA256).unwrap();
            let cred: Credential = BasicCredential::new(b"server".to_vec()).into();
            let sender = ExternalSender::new(signer.to_public_vec().into(), cred);
            Self {
                signer,
                sender_bytes: sender.tls_serialize_detached().unwrap(),
                provider: OpenMlsRustCrypto::default(),
            }
        }

        fn gid() -> GroupId {
            GroupId::from_slice(&CHANNEL.to_be_bytes())
        }

        /// op 27 append yükü: her anahtar paketi için Add.
        pub(crate) fn append_add(&self, epoch: u64, kps: &[Vec<u8>]) -> Vec<u8> {
            let mut body = Vec::new();
            for kp in kps {
                let kp = KeyPackageIn::tls_deserialize_exact_bytes(kp)
                    .unwrap()
                    .validate(self.provider.crypto(), ProtocolVersion::Mls10)
                    .unwrap();
                let m = ExternalProposal::new_add::<OpenMlsRustCrypto>(
                    kp,
                    Self::gid(),
                    GroupEpoch::from(epoch),
                    &self.signer,
                    SenderExtensionIndex::new(0),
                )
                .unwrap();
                body.extend(m.tls_serialize_detached().unwrap());
            }
            Self::wrap(0, &body)
        }

        pub(crate) fn append_remove(&self, epoch: u64, leaf: u32) -> Vec<u8> {
            let m = ExternalProposal::new_remove::<OpenMlsRustCrypto>(
                LeafNodeIndex::new(leaf),
                Self::gid(),
                GroupEpoch::from(epoch),
                &self.signer,
                SenderExtensionIndex::new(0),
            )
            .unwrap();
            Self::wrap(0, &m.tls_serialize_detached().unwrap())
        }

        fn wrap(op: u8, body: &[u8]) -> Vec<u8> {
            let mut out = vec![op];
            let n = body.len();
            if n < 64 {
                out.push(n as u8);
            } else if n < 16384 {
                out.extend(((n as u16) | 0x4000).to_be_bytes());
            } else {
                out.extend(((n as u32) | 0x8000_0000).to_be_bytes());
            }
            out.extend(body);
            out
        }
    }

    fn set(ids: &[u64]) -> HashSet<u64> {
        ids.iter().copied().collect()
    }

    fn sorted(mut v: Vec<u64>) -> Vec<u64> {
        v.sort_unstable();
        v
    }

    fn client(srv: &Server, uid: u64) -> (Session, Vec<u8>) {
        let mut s = Session::new(uid, CHANNEL).unwrap();
        s.set_external_sender(&srv.sender_bytes).unwrap();
        let kp = s.key_package().unwrap();
        (s, kp)
    }

    /// A (1) ve B (2) epoch 1'de.
    fn two(srv: &Server) -> (Session, Session) {
        let (mut a, _) = client(srv, 1);
        let (mut b, kp_b) = client(srv, 2);
        let out = a
            .process_proposals(&srv.append_add(0, &[kp_b]), &set(&[1, 2]))
            .unwrap()
            .unwrap();
        let (commit, welcome) = split_commit_welcome(&out).unwrap();
        a.process_commit(commit).unwrap();
        b.process_welcome(welcome, &set(&[1, 2])).unwrap();
        (a, b)
    }

    #[test]
    fn two_party_handshake() {
        let srv = Server::new();
        let (a, b) = two(&srv);
        assert_eq!(a.epoch(), Some(1));
        assert_eq!(b.epoch(), Some(1));
        assert_eq!(sorted(a.members()), vec![1, 2]);
        assert_eq!(sorted(b.members()), vec![1, 2]);
        assert_eq!(a.sender_secret(1).unwrap(), b.sender_secret(1).unwrap());
        assert_eq!(a.sender_secret(2).unwrap(), b.sender_secret(2).unwrap());
        assert_ne!(a.sender_secret(1).unwrap(), a.sender_secret(2).unwrap());
        assert!(a.sender_secret(9).is_err());
    }

    #[test]
    fn validation_and_garbage() {
        let srv = Server::new();
        let (mut a, _) = client(&srv, 1);
        let (mut b, kp_b) = client(&srv, 2);
        let add = srv.append_add(0, std::slice::from_ref(&kp_b));

        // tanınmayan eklenen kullanıcı
        assert!(a.process_proposals(&add, &set(&[1])).is_err());
        // yanlış epoch
        assert!(
            a.process_proposals(&srv.append_add(5, &[kp_b]), &set(&[1, 2]))
                .is_err()
        );
        let out = a.process_proposals(&add, &set(&[1, 2])).unwrap().unwrap();
        let (commit, welcome) = split_commit_welcome(&out).unwrap();

        // Welcome: B'nin tanınanlarında 1 yok
        assert!(b.process_welcome(welcome, &set(&[2])).is_err());
        // key package tüketildi: yeniden denemek hata verir
        assert!(b.process_welcome(welcome, &set(&[1, 2])).is_err());

        // çöp girdiler
        let junk: [&[u8]; 6] = [&[], &[0], &[1, 2, 3], &[0xff; 40], &[0, 0x40, 1], &[7, 0]];
        for j in junk {
            assert!(a.set_external_sender(j).is_err());
            assert!(a.process_proposals(j, &set(&[1, 2])).is_err());
            assert!(a.process_commit(j).is_err());
            assert!(a.process_welcome(j, &set(&[1, 2])).is_err());
        }
        // rastgele-benzeri uzun çöp
        let long: Vec<u8> = (0..3000u32).map(|i| (i * 31 % 251) as u8).collect();
        assert!(a.process_proposals(&long, &set(&[1])).is_err());
        assert!(a.process_commit(&long).is_err());
        assert!(a.process_welcome(&long, &set(&[1])).is_err());

        // A hâlâ kendi commit'ini kabul edebilir
        a.process_commit(commit).unwrap();
        assert_eq!(a.epoch(), Some(1));
    }

    #[test]
    fn revoke_all_yields_none() {
        let srv = Server::new();
        let (mut a, _) = client(&srv, 1);
        let (_b, kp_b) = client(&srv, 2);
        let rec = set(&[1, 2]);
        a.process_proposals(&srv.append_add(0, &[kp_b]), &rec)
            .unwrap()
            .unwrap();
        let refs: Vec<ProposalRef> = a
            .group
            .as_ref()
            .unwrap()
            .pending_proposals()
            .map(|p| p.proposal_reference_ref().clone())
            .collect();
        assert_eq!(refs.len(), 1);
        let mut body = Vec::new();
        for r in &refs {
            body.extend(r.tls_serialize_detached().unwrap());
        }
        let revoke = Server::wrap(1, &body);
        assert!(a.process_proposals(&revoke, &rec).unwrap().is_none());
        // bilinmeyen ref artık hata
        assert!(a.process_proposals(&revoke, &rec).is_err());
    }

    #[test]
    fn third_member_via_other_members_commit() {
        let srv = Server::new();
        let (mut a, mut b) = two(&srv);
        let (mut c, kp_c) = client(&srv, 3);
        let rec = set(&[1, 2, 3]);
        let add = srv.append_add(1, &[kp_c]);
        let out_a = a.process_proposals(&add, &rec).unwrap().unwrap();
        // B de aynı öneriyi alır ve kendi commit'ini üretir (kaybedecek).
        assert!(b.process_proposals(&add, &rec).unwrap().is_some());
        let (commit, welcome) = split_commit_welcome(&out_a).unwrap();
        a.process_commit(commit).unwrap();
        b.process_commit(commit).unwrap();
        c.process_welcome(welcome, &rec).unwrap();
        for s in [&a, &b, &c] {
            assert_eq!(s.epoch(), Some(2));
            assert_eq!(sorted(s.members()), vec![1, 2, 3]);
        }
        for u in 1..=3 {
            let x = a.sender_secret(u).unwrap();
            assert_eq!(x, b.sender_secret(u).unwrap());
            assert_eq!(x, c.sender_secret(u).unwrap());
        }
    }

    #[test]
    fn removal() {
        let srv = Server::new();
        let (mut a, mut b) = two(&srv);
        let (mut c, kp_c) = client(&srv, 3);
        let rec = set(&[1, 2, 3]);
        let add = srv.append_add(1, &[kp_c]);
        let out = a.process_proposals(&add, &rec).unwrap().unwrap();
        b.process_proposals(&add, &rec).unwrap();
        let (commit, welcome) = split_commit_welcome(&out).unwrap();
        a.process_commit(commit).unwrap();
        b.process_commit(commit).unwrap();
        c.process_welcome(welcome, &rec).unwrap();

        // B (yaprak 1) çıkarılır
        let rm = srv.append_remove(2, 1);
        let out = a.process_proposals(&rm, &rec).unwrap().unwrap();
        c.process_proposals(&rm, &rec).unwrap();
        let (commit, welcome) = split_commit_welcome(&out).unwrap();
        assert!(welcome.is_empty());
        a.process_commit(commit).unwrap();
        c.process_commit(commit).unwrap();
        assert_eq!(a.epoch(), Some(3));
        assert_eq!(sorted(a.members()), vec![1, 3]);
        assert_eq!(sorted(c.members()), vec![1, 3]);
        assert_eq!(a.sender_secret(3).unwrap(), c.sender_secret(3).unwrap());
        // B kendisinin çıkarıldığını öğrenir
        assert!(b.process_commit(commit).is_err());
    }
}
