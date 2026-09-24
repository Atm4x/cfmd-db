use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kernel_auth::{KeyId, Sha256Digest, TrustRootSet, sha256};
use kernel_change::RevisionEffectId;
use kernel_types::RevisionId;

use super::{
    CodecError, Cursor, DurabilityError, DurableRevisionEffectRecord, crc32c, metadata, push_len,
    push_u64, push_u128, read_u16, read_u32,
};

const REPLICATION_MAGIC: [u8; 4] = *b"CFRP";
const REPLICATION_VERSION: u16 = 1;
const FRAME_HEADER_LEN: usize = 16;
const KIND_INGEST: u8 = 1;
const KIND_RETIRE: u8 = 2;
const KIND_MEMBERSHIP: u8 = 3;
const KIND_QUORUM: u8 = 4;
const KIND_PUBLISH: u8 = 5;
const KIND_EFFECT_VOTE: u8 = 6;
const KIND_MEMBERSHIP_VOTE: u8 = 7;
const KIND_TERM_PROMISE: u8 = 8;
const KIND_LEADER_VOTE: u8 = 9;
const KIND_LEADER_CERTIFICATE: u8 = 10;
const KIND_DECISION_VOTE: u8 = 11;
const KIND_DECISION_LOCK: u8 = 12;
const KIND_JOINT_MEMBERSHIP_CERTIFICATE: u8 = 13;
const KIND_PEER_AUTH_POLICY: u8 = 14;
const KIND_AUTHENTICATED_PEER_EVIDENCE: u8 = 15;
const KIND_QUORUM_LOSS: u8 = 16;
const KIND_QUORUM_RECOVERY: u8 = 17;

const PEER_EVIDENCE_DOMAIN: &[u8] = b"CFMD-REPLICATION-PEER-EVIDENCE-v1\0";
const EVIDENCE_TERM_PROMISE: u8 = 1;
const EVIDENCE_LEADER_VOTE: u8 = 2;
const EVIDENCE_EFFECT_VOTE: u8 = 3;
const EVIDENCE_MEMBERSHIP_VOTE: u8 = 4;
const EVIDENCE_DECISION_VOTE: u8 = 5;
const EVIDENCE_JOINT_MEMBERSHIP_ACK: u8 = 6;
const EVIDENCE_RECOVERY_ACK: u8 = 7;

/// Stable origin namespace for replicated causal event identities.
/// Origin zero is reserved for the local store's existing effect allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaId(u64);

impl ReplicaId {
    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Stable logical branch identity. Branch identity is not a `RevisionId`: a
/// branch advances through revisions while retaining one durable lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicationBranchId(u128);

impl ReplicationBranchId {
    #[must_use]
    pub const fn new(raw: u128) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u128 {
        self.0
    }
}

/// Durable ordering witness produced by an external sequencer/consensus path.
/// This crate validates uniqueness and replay stability of the ordered slot; it
/// deliberately does not pretend to implement membership, signatures, quorum
/// transport, leader election, or failure detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurableSequencerOrder {
    pub sequencer: ReplicaId,
    pub epoch: u64,
    pub position: u64,
}

/// Remote effect envelope admitted into the durable branch journal.
/// The effect itself remains the same REIC causal record used by local commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicatedEffectEnvelope {
    pub origin: ReplicaId,
    pub origin_sequence: u64,
    pub branch: ReplicationBranchId,
    pub effect: DurableRevisionEffectRecord,
    pub ordered_by: DurableSequencerOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationBranchHead {
    pub branch: ReplicationBranchId,
    pub head_revision: RevisionId,
    pub head_effect: RevisionEffectId,
    pub retired: bool,
}

/// Durable voting configuration for replication admission. Authentication of
/// individual acknowledgements belongs to the transport/security adapter; this
/// type is the authority-side membership and threshold contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationMembership {
    pub epoch: u64,
    pub members: BTreeSet<ReplicaId>,
    pub quorum_size: usize,
}

impl ReplicationMembership {
    fn validate(&self) -> Result<(), DurabilityError> {
        if self.epoch == 0 || self.members.is_empty() {
            return Err(protocol(
                "replication membership is empty or has zero epoch",
            ));
        }
        if self.members.iter().any(|member| member.raw() == 0) {
            return Err(protocol("replication membership contains replica zero"));
        }
        if self.quorum_size == 0
            || self.quorum_size > self.members.len()
            || self.quorum_size <= self.members.len() / 2
        {
            return Err(protocol(
                "replication membership quorum is not a strict majority",
            ));
        }
        Ok(())
    }
}

/// Proof that a membership replacement was accepted by the prior membership.
/// Bootstrap uses an empty acknowledgement set because no prior membership
/// exists; every later epoch requires the old epoch's configured quorum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationMembershipChange {
    pub next: ReplicationMembership,
    pub acknowledged_by_previous: BTreeSet<ReplicaId>,
}

/// Structural quorum evidence for one already-local-durable replicated effect.
/// Signatures/MACs are intentionally outside this value; callers must only
/// construct it from authenticated peer acknowledgements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationQuorumCertificate {
    pub effect: RevisionEffectId,
    pub membership_epoch: u64,
    pub acknowledged_by: BTreeSet<ReplicaId>,
}

/// One authenticated peer vote for an already-local-durable replicated
/// effect. Authentication is performed by the transport/security adapter;
/// this journal makes the vote durable and enforces vote-once for the global
/// decision position in one membership epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationEffectVote {
    pub voter: ReplicaId,
    pub effect: RevisionEffectId,
    pub membership_epoch: u64,
}

/// One durable vote for the successor of a membership epoch. A voter may bind
/// itself to only one successor configuration for a given previous epoch.
/// This intentionally favours safety over liveness until a full election/
/// locking protocol is introduced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationMembershipVote {
    pub voter: ReplicaId,
    pub previous_membership_epoch: u64,
    pub term: u64,
    pub next: ReplicationMembership,
}

/// Durable promise that one membership voter will not vote in a lower term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationTermPromise {
    pub voter: ReplicaId,
    pub membership_epoch: u64,
    pub term: u64,
}

/// Durable vote-for-leader. One voter may vote for only one candidate in one term.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationLeaderVote {
    pub voter: ReplicaId,
    pub membership_epoch: u64,
    pub term: u64,
    pub candidate: ReplicaId,
}

/// Quorum-backed leader authority for one membership epoch and term.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationLeaderCertificate {
    pub membership_epoch: u64,
    pub term: u64,
    pub leader: ReplicaId,
    pub acknowledged_by: BTreeSet<ReplicaId>,
}

/// Term-bound vote for one globally ordered decision position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationDecisionVote {
    pub voter: ReplicaId,
    pub membership_epoch: u64,
    pub term: u64,
    pub leader: ReplicaId,
    pub position: u64,
    pub effect: RevisionEffectId,
    pub carried_from_term: Option<u64>,
}

/// Durable quorum lock. Once a position is locked, later terms may only carry
/// the same value forward and must name the prior lock term explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationDecisionLock {
    pub membership_epoch: u64,
    pub term: u64,
    pub leader: ReplicaId,
    pub position: u64,
    pub effect: RevisionEffectId,
    pub acknowledged_by: BTreeSet<ReplicaId>,
    pub carried_from_term: Option<u64>,
}

/// Joint-consensus evidence for a membership transition. The same successor
/// configuration is acknowledged by quorums of both the old and new sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationJointMembershipCertificate {
    pub previous_membership_epoch: u64,
    pub term: u64,
    pub leader: ReplicaId,
    pub next: ReplicationMembership,
    pub acknowledged_by_previous: BTreeSet<ReplicaId>,
    pub acknowledged_by_next: BTreeSet<ReplicaId>,
}

/// Stable cluster namespace included in every peer-authentication signature.
/// Cross-cluster replay therefore cannot turn a valid peer signature into
/// authority for another CFMD deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicationClusterId(pub [u8; 32]);

/// Durable mapping from replication identities to the currently authorized
/// trust-root keys. The verifying key bytes themselves remain owned by
/// `kernel-auth`; this journal persists only key identities and the trust epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationPeerAuthPolicy {
    pub cluster: ReplicationClusterId,
    pub trust_epoch: u64,
    pub peer_keys: BTreeMap<ReplicaId, KeyId>,
}

/// Signed acknowledgement by either side of a joint-membership transition.
/// The full successor membership is represented by its canonical digest so a
/// signature cannot be replayed onto a different configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationJointMembershipAck {
    pub voter: ReplicaId,
    pub previous_membership_epoch: u64,
    pub next_membership_epoch: u64,
    pub term: u64,
    pub leader: ReplicaId,
    pub next_membership_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReplicationLockSummary {
    pub position: u64,
    pub term: u64,
    pub effect: RevisionEffectId,
}

/// Peer recovery statement used after a durable quorum-loss fence. Every
/// recovery voter reports the lock frontier it knows; recovery is allowed only
/// when the local journal has reconciled the union of those durable locks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationRecoveryAck {
    pub voter: ReplicaId,
    pub membership_epoch: u64,
    pub recovery_term: u64,
    pub leader: ReplicaId,
    pub locks: Vec<ReplicationLockSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationPeerEvidence {
    TermPromise(ReplicationTermPromise),
    LeaderVote(ReplicationLeaderVote),
    EffectVote(ReplicationEffectVote),
    MembershipVote(ReplicationMembershipVote),
    DecisionVote(ReplicationDecisionVote),
    JointMembershipAck(ReplicationJointMembershipAck),
    RecoveryAck(ReplicationRecoveryAck),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedReplicationPeerEvidence {
    pub trust_epoch: u64,
    pub signer: KeyId,
    pub evidence: ReplicationPeerEvidence,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationAuthenticationReceipt {
    pub proof_digest: Sha256Digest,
    pub payload_digest: Sha256Digest,
    pub trust_epoch: u64,
    pub voter: ReplicaId,
    pub signer: KeyId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationQuorumLoss {
    pub membership_epoch: u64,
    pub observed_term: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationRecoveryCertificate {
    pub membership_epoch: u64,
    pub recovery_term: u64,
    pub leader: ReplicaId,
    pub acknowledged_by: BTreeSet<ReplicaId>,
    pub reconciled_locks: Vec<ReplicationLockSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationQuorumAvailability {
    Available { membership_epoch: u64, term: u64 },
    Lost(ReplicationQuorumLoss),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReplicationEffectStage {
    Received,
    LocalDurable,
    QuorumDurable,
    Published,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationIngestOutcome {
    Inserted,
    AlreadyPresent,
}

#[derive(Debug)]
pub(crate) struct ReplicationAuthorityJournal {
    path: PathBuf,
    file: File,
    effects: BTreeMap<RevisionEffectId, ReplicatedEffectEnvelope>,
    branches: BTreeMap<ReplicationBranchId, ReplicationBranchHead>,
    revision_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    ordered_slots: BTreeMap<DurableSequencerOrder, RevisionEffectId>,
    sequencer_epochs: BTreeMap<ReplicaId, u64>,
    memberships: BTreeMap<u64, ReplicationMembership>,
    current_membership_epoch: Option<u64>,
    quorum_certificates: BTreeMap<RevisionEffectId, ReplicationQuorumCertificate>,
    effect_votes: BTreeMap<(u64, u64, ReplicaId), RevisionEffectId>,
    membership_votes: BTreeMap<(u64, ReplicaId), ReplicationMembershipVote>,
    promised_terms: BTreeMap<(u64, ReplicaId), u64>,
    leader_votes: BTreeMap<(u64, u64, ReplicaId), ReplicaId>,
    leader_certificates: BTreeMap<(u64, u64), ReplicationLeaderCertificate>,
    decision_votes: BTreeMap<(u64, u64, u64, ReplicaId), ReplicationDecisionVote>,
    decision_locks: BTreeMap<u64, ReplicationDecisionLock>,
    joint_membership_certificates: BTreeMap<u64, ReplicationJointMembershipCertificate>,
    peer_auth_policy: Option<ReplicationPeerAuthPolicy>,
    authenticated_evidence: BTreeMap<Sha256Digest, ReplicationAuthenticationReceipt>,
    joint_membership_acks: BTreeMap<(u64, u64, ReplicaId), ReplicationJointMembershipAck>,
    recovery_acks: BTreeMap<(u64, u64, ReplicaId), ReplicationRecoveryAck>,
    quorum_availability: Option<ReplicationQuorumAvailability>,
    published_effects: BTreeSet<RevisionEffectId>,
    published_branches: BTreeMap<ReplicationBranchId, ReplicationBranchHead>,
    poisoned: bool,
}

impl ReplicationAuthorityJournal {
    pub(crate) fn open_or_create(path: impl AsRef<Path>) -> Result<Self, DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        let mut journal = Self {
            path,
            file,
            effects: BTreeMap::new(),
            branches: BTreeMap::new(),
            revision_frontiers: BTreeMap::new(),
            ordered_slots: BTreeMap::new(),
            sequencer_epochs: BTreeMap::new(),
            memberships: BTreeMap::new(),
            current_membership_epoch: None,
            quorum_certificates: BTreeMap::new(),
            effect_votes: BTreeMap::new(),
            membership_votes: BTreeMap::new(),
            promised_terms: BTreeMap::new(),
            leader_votes: BTreeMap::new(),
            leader_certificates: BTreeMap::new(),
            decision_votes: BTreeMap::new(),
            decision_locks: BTreeMap::new(),
            joint_membership_certificates: BTreeMap::new(),
            peer_auth_policy: None,
            authenticated_evidence: BTreeMap::new(),
            joint_membership_acks: BTreeMap::new(),
            recovery_acks: BTreeMap::new(),
            quorum_availability: None,
            published_effects: BTreeSet::new(),
            published_branches: BTreeMap::new(),
            poisoned: false,
        };
        let last_good = journal.replay(&bytes)?;
        if last_good < bytes.len() {
            journal
                .file
                .set_len(u64::try_from(last_good).map_err(|_| CodecError::LengthOverflow)?)?;
            journal.file.sync_all()?;
        }
        journal.file.seek(SeekFrom::End(0))?;
        Ok(journal)
    }

    #[must_use]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub(crate) fn effect(&self, id: RevisionEffectId) -> Option<&DurableRevisionEffectRecord> {
        self.effects.get(&id).map(|envelope| &envelope.effect)
    }

    #[must_use]
    pub(crate) fn branch_head(&self, branch: ReplicationBranchId) -> Option<ReplicationBranchHead> {
        self.branches.get(&branch).copied()
    }

    #[must_use]
    pub(crate) fn revision_frontier(
        &self,
        revision: RevisionId,
    ) -> Option<&BTreeSet<RevisionEffectId>> {
        self.revision_frontiers.get(&revision)
    }

    #[must_use]
    pub(crate) fn current_membership(&self) -> Option<&ReplicationMembership> {
        self.current_membership_epoch
            .and_then(|epoch| self.memberships.get(&epoch))
    }

    #[must_use]
    pub(crate) fn promised_term(&self, membership_epoch: u64, voter: ReplicaId) -> Option<u64> {
        self.promised_terms.get(&(membership_epoch, voter)).copied()
    }

    #[must_use]
    pub(crate) fn leader_certificate(
        &self,
        membership_epoch: u64,
        term: u64,
    ) -> Option<&ReplicationLeaderCertificate> {
        self.leader_certificates.get(&(membership_epoch, term))
    }

    #[must_use]
    pub(crate) fn decision_lock(&self, position: u64) -> Option<&ReplicationDecisionLock> {
        self.decision_locks.get(&position)
    }

    #[must_use]
    pub(crate) fn decision_lock_summaries(&self) -> Vec<ReplicationLockSummary> {
        self.decision_locks
            .values()
            .map(|lock| ReplicationLockSummary {
                position: lock.position,
                term: lock.term,
                effect: lock.effect,
            })
            .collect()
    }

    #[must_use]
    pub(crate) fn current_consensus_term(&self) -> u64 {
        let availability_term = match self.quorum_availability {
            Some(ReplicationQuorumAvailability::Available { term, .. }) => term,
            Some(ReplicationQuorumAvailability::Lost(loss)) => loss.observed_term,
            None => 0,
        };
        let promised_term = self.promised_terms.values().copied().max().unwrap_or(0);
        let leader_term = self
            .leader_certificates
            .keys()
            .map(|(_, term)| *term)
            .max()
            .unwrap_or(0);
        availability_term.max(promised_term).max(leader_term)
    }

    #[must_use]
    pub(crate) fn peer_auth_policy(&self) -> Option<&ReplicationPeerAuthPolicy> {
        self.peer_auth_policy.as_ref()
    }

    #[must_use]
    pub(crate) const fn quorum_availability(&self) -> Option<ReplicationQuorumAvailability> {
        self.quorum_availability
    }

    #[must_use]
    pub(crate) fn authentication_receipt(
        &self,
        proof_digest: Sha256Digest,
    ) -> Option<ReplicationAuthenticationReceipt> {
        self.authenticated_evidence.get(&proof_digest).copied()
    }

    #[must_use]
    pub(crate) fn effect_stage(&self, id: RevisionEffectId) -> Option<ReplicationEffectStage> {
        if self.published_effects.contains(&id) {
            Some(ReplicationEffectStage::Published)
        } else if self.quorum_certificates.contains_key(&id) {
            Some(ReplicationEffectStage::QuorumDurable)
        } else if self.effects.contains_key(&id) {
            Some(ReplicationEffectStage::LocalDurable)
        } else {
            None
        }
    }

    #[must_use]
    pub(crate) fn published_branch_head(
        &self,
        branch: ReplicationBranchId,
    ) -> Option<ReplicationBranchHead> {
        self.published_branches.get(&branch).copied()
    }

    pub(crate) fn effects_iter(
        &self,
    ) -> impl Iterator<Item = (&RevisionEffectId, &ReplicatedEffectEnvelope)> {
        self.effects.iter()
    }

    pub(crate) fn install_peer_auth_policy(
        &mut self,
        policy: ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.validate_peer_auth_policy(&policy, trust)?;
        if self.peer_auth_policy.as_ref() == Some(&policy) {
            return Ok(());
        }
        let payload = encode_peer_auth_policy(&policy)?;
        self.append_frame(KIND_PEER_AUTH_POLICY, &payload)?;
        self.peer_auth_policy = Some(policy);
        Ok(())
    }

    pub(crate) fn record_authenticated_peer_evidence(
        &mut self,
        trust: &TrustRootSet,
        signed: SignedReplicationPeerEvidence,
    ) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let receipt = self.verify_peer_evidence(trust, &signed)?;
        if let Some(existing) = self.authenticated_evidence.get(&receipt.proof_digest) {
            if *existing == receipt {
                self.commit_authenticated_peer_semantics(signed.evidence)?;
                return Ok(receipt);
            }
            return Err(protocol(
                "replication authentication proof digest collision",
            ));
        }
        let payload = encode_signed_peer_evidence(&signed)?;
        self.append_frame(KIND_AUTHENTICATED_PEER_EVIDENCE, &payload)?;
        self.apply_authenticated_peer_evidence(&signed, receipt);
        self.commit_authenticated_peer_semantics(signed.evidence)?;
        Ok(receipt)
    }

    pub(crate) fn mark_quorum_lost(
        &mut self,
        loss: ReplicationQuorumLoss,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.validate_quorum_loss(loss)?;
        if self.quorum_availability == Some(ReplicationQuorumAvailability::Lost(loss)) {
            return Ok(());
        }
        self.append_frame(KIND_QUORUM_LOSS, &encode_quorum_loss(loss))?;
        self.quorum_availability = Some(ReplicationQuorumAvailability::Lost(loss));
        Ok(())
    }

    pub(crate) fn recover_quorum(
        &mut self,
        certificate: &ReplicationRecoveryCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.validate_recovery_certificate(certificate)?;
        let payload = encode_recovery_certificate(certificate)?;
        self.append_frame(KIND_QUORUM_RECOVERY, &payload)?;
        self.apply_recovery_certificate(certificate);
        Ok(())
    }

    pub(crate) fn ingest(
        &mut self,
        envelope: ReplicatedEffectEnvelope,
    ) -> Result<ReplicationIngestOutcome, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if let Some(existing) = self.effects.get(&envelope.effect.id) {
            return if existing == &envelope {
                Ok(ReplicationIngestOutcome::AlreadyPresent)
            } else {
                Err(protocol(
                    "replicated effect identity conflicts with durable branch journal",
                ))
            };
        }
        self.validate_envelope(&envelope)?;
        let payload = encode_ingest(&envelope)?;
        self.append_frame(KIND_INGEST, &payload)?;
        self.apply_ingest(envelope)?;
        Ok(ReplicationIngestOutcome::Inserted)
    }

    pub(crate) fn retire_branch(
        &mut self,
        branch: ReplicationBranchId,
        expected_head: RevisionEffectId,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let head = self
            .branches
            .get(&branch)
            .copied()
            .ok_or_else(|| protocol("replication branch does not exist"))?;
        if head.retired {
            return Err(protocol("replication branch is already retired"));
        }
        if head.head_effect != expected_head {
            return Err(protocol("replication branch retirement head changed"));
        }
        let mut payload = Vec::new();
        push_u128(&mut payload, branch.raw());
        push_u128(&mut payload, expected_head.0);
        self.append_frame(KIND_RETIRE, &payload)?;
        self.branches
            .get_mut(&branch)
            .expect("validated branch exists")
            .retired = true;
        if let Some(published) = self.published_branches.get_mut(&branch) {
            published.retired = true;
        }
        Ok(())
    }

    pub(crate) fn install_membership(
        &mut self,
        change: ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.memberships.get(&change.next.epoch) == Some(&change.next) {
            return Ok(());
        }
        if self.current_membership().is_some() {
            self.require_quorum_available()?;
        }
        self.validate_membership_change(&change)?;
        let payload = encode_membership_change(&change)?;
        self.append_frame(KIND_MEMBERSHIP, &payload)?;
        self.apply_membership_change(change)
    }

    pub(crate) fn record_effect_vote(
        &mut self,
        vote: ReplicationEffectVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::EffectVote(vote))?;
        self.validate_effect_vote(&vote)?;
        let envelope = self
            .effects
            .get(&vote.effect)
            .expect("validated vote references local-durable effect");
        let key = (
            vote.membership_epoch,
            envelope.ordered_by.position,
            vote.voter,
        );
        if let Some(existing) = self.effect_votes.get(&key) {
            return if *existing == vote.effect {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already voted for another effect in this decision slot",
                ))
            };
        }
        let payload = encode_effect_vote(&vote);
        self.append_frame(KIND_EFFECT_VOTE, &payload)?;
        self.effect_votes.insert(key, vote.effect);
        Ok(())
    }

    pub(crate) fn record_membership_vote(
        &mut self,
        vote: ReplicationMembershipVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::MembershipVote(
            vote.clone(),
        ))?;
        self.validate_membership_vote(&vote)?;
        let key = (vote.previous_membership_epoch, vote.voter);
        if let Some(existing) = self.membership_votes.get(&key) {
            return if existing == &vote {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already voted for another successor membership",
                ))
            };
        }
        let payload = encode_membership_vote(&vote)?;
        self.append_frame(KIND_MEMBERSHIP_VOTE, &payload)?;
        self.membership_votes.insert(key, vote);
        Ok(())
    }

    pub(crate) fn record_term_promise(
        &mut self,
        promise: ReplicationTermPromise,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::TermPromise(promise))?;
        self.validate_term_member(promise.membership_epoch, promise.voter, promise.term)?;
        let key = (promise.membership_epoch, promise.voter);
        if let Some(existing) = self.promised_terms.get(&key) {
            if promise.term < *existing {
                return Err(protocol("replication term promise regressed"));
            }
            if promise.term == *existing {
                return Ok(());
            }
        }
        self.append_frame(KIND_TERM_PROMISE, &encode_term_promise(&promise))?;
        self.promised_terms.insert(key, promise.term);
        Ok(())
    }

    pub(crate) fn record_leader_vote(
        &mut self,
        vote: ReplicationLeaderVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::LeaderVote(vote))?;
        self.validate_leader_vote(&vote)?;
        let key = (vote.membership_epoch, vote.term, vote.voter);
        if let Some(existing) = self.leader_votes.get(&key) {
            return if *existing == vote.candidate {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already voted for another leader in this term",
                ))
            };
        }
        self.append_frame(KIND_LEADER_VOTE, &encode_leader_vote(&vote))?;
        self.promised_terms
            .entry((vote.membership_epoch, vote.voter))
            .and_modify(|term| *term = (*term).max(vote.term))
            .or_insert(vote.term);
        self.leader_votes.insert(key, vote.candidate);
        Ok(())
    }

    pub(crate) fn certify_leader(
        &mut self,
        certificate: ReplicationLeaderCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self
            .leader_certificates
            .get(&(certificate.membership_epoch, certificate.term))
        {
            return if existing == &certificate {
                Ok(())
            } else {
                Err(protocol(
                    "replication term already has a different leader certificate",
                ))
            };
        }
        self.validate_leader_certificate(&certificate)?;
        let payload = encode_leader_certificate(&certificate)?;
        self.append_frame(KIND_LEADER_CERTIFICATE, &payload)?;
        let membership_epoch = certificate.membership_epoch;
        let term = certificate.term;
        self.leader_certificates
            .insert((membership_epoch, term), certificate);
        self.note_available_term(membership_epoch, term);
        Ok(())
    }

    pub(crate) fn record_decision_vote(
        &mut self,
        vote: ReplicationDecisionVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::DecisionVote(vote))?;
        self.validate_decision_vote(&vote)?;
        let key = (vote.membership_epoch, vote.term, vote.position, vote.voter);
        if let Some(existing) = self.decision_votes.get(&key) {
            return if existing == &vote {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already accepted another value in this term/position",
                ))
            };
        }
        self.append_frame(KIND_DECISION_VOTE, &encode_decision_vote(&vote))?;
        self.decision_votes.insert(key, vote);
        Ok(())
    }

    pub(crate) fn lock_decision(
        &mut self,
        lock: ReplicationDecisionLock,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self.decision_locks.get(&lock.position) {
            if existing == &lock {
                return Ok(());
            }
            if lock.term <= existing.term {
                return Err(protocol("replication decision lock did not advance term"));
            }
        }
        self.validate_decision_lock(&lock)?;
        let payload = encode_decision_lock(&lock)?;
        self.append_frame(KIND_DECISION_LOCK, &payload)?;
        self.decision_locks.insert(lock.position, lock);
        Ok(())
    }

    pub(crate) fn certify_joint_membership(
        &mut self,
        certificate: ReplicationJointMembershipCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self
            .joint_membership_certificates
            .get(&certificate.next.epoch)
        {
            return if existing == &certificate {
                Ok(())
            } else {
                Err(protocol(
                    "replication successor epoch already has a different joint certificate",
                ))
            };
        }
        self.validate_joint_membership_certificate(&certificate)?;
        let payload = encode_joint_membership_certificate(&certificate)?;
        self.append_frame(KIND_JOINT_MEMBERSHIP_CERTIFICATE, &payload)?;
        self.joint_membership_certificates
            .insert(certificate.next.epoch, certificate);
        Ok(())
    }

    pub(crate) fn certify_quorum(
        &mut self,
        certificate: ReplicationQuorumCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self.quorum_certificates.get(&certificate.effect) {
            return if existing == &certificate {
                Ok(())
            } else {
                Err(protocol(
                    "replicated effect already has a different durable quorum certificate",
                ))
            };
        }
        self.validate_quorum_certificate(&certificate)?;
        let payload = encode_quorum_certificate(&certificate)?;
        self.append_frame(KIND_QUORUM, &payload)?;
        self.quorum_certificates
            .insert(certificate.effect, certificate);
        Ok(())
    }

    pub(crate) fn publish(&mut self, effect: RevisionEffectId) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if self.published_effects.contains(&effect) {
            return Ok(());
        }
        self.validate_publish(effect)?;
        let mut payload = Vec::new();
        push_u128(&mut payload, effect.0);
        self.append_frame(KIND_PUBLISH, &payload)?;
        self.apply_publish(effect)
    }

    fn validate_peer_auth_policy(
        &self,
        policy: &ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
    ) -> Result<(), DurabilityError> {
        if policy.cluster.0 == [0; 32] {
            return Err(protocol("replication peer-auth cluster identity is zero"));
        }
        if policy.trust_epoch == 0 || trust.epoch() != policy.trust_epoch {
            return Err(protocol(
                "replication peer-auth trust epoch does not match trust roots",
            ));
        }
        if policy.peer_keys.is_empty() {
            return Err(protocol("replication peer-auth policy has no peer keys"));
        }
        if policy.peer_keys.keys().any(|replica| replica.raw() == 0) {
            return Err(protocol(
                "replication peer-auth policy contains replica zero",
            ));
        }
        if policy.peer_keys.values().any(|key| !trust.contains(*key)) {
            return Err(protocol(
                "replication peer-auth policy references an untrusted key",
            ));
        }
        if let Some(current) = self.current_membership()
            && current
                .members
                .iter()
                .any(|member| !policy.peer_keys.contains_key(member))
        {
            return Err(protocol(
                "replication peer-auth policy does not cover current membership",
            ));
        }
        if let Some(previous) = &self.peer_auth_policy {
            if previous.cluster != policy.cluster {
                return Err(protocol("replication peer-auth cluster identity changed"));
            }
            if policy.trust_epoch != previous.trust_epoch.saturating_add(1) {
                return Err(protocol(
                    "replication peer-auth trust epoch did not advance by one",
                ));
            }
        }
        Ok(())
    }

    fn validate_replayed_peer_auth_policy(
        &self,
        policy: &ReplicationPeerAuthPolicy,
    ) -> Result<(), DurabilityError> {
        if policy.cluster.0 == [0; 32]
            || policy.trust_epoch == 0
            || policy.peer_keys.is_empty()
            || policy.peer_keys.keys().any(|replica| replica.raw() == 0)
        {
            return Err(protocol(
                "replication peer-auth policy is malformed during replay",
            ));
        }
        if let Some(current) = self.current_membership()
            && current
                .members
                .iter()
                .any(|member| !policy.peer_keys.contains_key(member))
        {
            return Err(protocol(
                "replication peer-auth policy does not cover current membership during replay",
            ));
        }
        if let Some(previous) = &self.peer_auth_policy
            && (previous.cluster != policy.cluster
                || policy.trust_epoch != previous.trust_epoch.saturating_add(1))
        {
            return Err(protocol(
                "replication peer-auth policy replay violates monotone trust epoch",
            ));
        }
        Ok(())
    }

    fn validate_replayed_peer_evidence(
        &self,
        signed: &SignedReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        let policy = self
            .peer_auth_policy
            .as_ref()
            .ok_or_else(|| protocol("replication peer evidence replay has no auth policy"))?;
        if signed.trust_epoch != policy.trust_epoch {
            return Err(protocol(
                "replication peer evidence replay uses stale trust epoch",
            ));
        }
        let voter = evidence_voter(&signed.evidence);
        if policy.peer_keys.get(&voter) != Some(&signed.signer) {
            return Err(protocol(
                "replication peer evidence replay signer does not match durable policy",
            ));
        }
        self.validate_peer_evidence_semantics(&signed.evidence)
    }

    fn verify_peer_evidence(
        &self,
        trust: &TrustRootSet,
        signed: &SignedReplicationPeerEvidence,
    ) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
        let policy = self
            .peer_auth_policy
            .as_ref()
            .ok_or_else(|| protocol("replication peer authentication is not activated"))?;
        if signed.trust_epoch != policy.trust_epoch || trust.epoch() != policy.trust_epoch {
            return Err(protocol(
                "replication peer evidence uses a stale trust epoch",
            ));
        }
        let voter = evidence_voter(&signed.evidence);
        let expected = policy
            .peer_keys
            .get(&voter)
            .ok_or_else(|| protocol("replication peer evidence voter has no authorized key"))?;
        if *expected != signed.signer || !trust.contains(signed.signer) {
            return Err(protocol(
                "replication peer evidence signer is not authorized for voter",
            ));
        }
        self.validate_peer_evidence_semantics(&signed.evidence)?;
        let message = replication_peer_evidence_signing_message(
            policy.cluster,
            signed.trust_epoch,
            signed.signer,
            &signed.evidence,
        )?;
        trust
            .verify_message(signed.signer, &message, &signed.signature)
            .map_err(|_| protocol("replication peer evidence signature verification failed"))?;
        authentication_receipt(signed)
    }

    fn validate_peer_evidence_semantics(
        &self,
        evidence: &ReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        match evidence {
            ReplicationPeerEvidence::TermPromise(value) => {
                self.validate_term_member(value.membership_epoch, value.voter, value.term)
            }
            ReplicationPeerEvidence::LeaderVote(value) => self.validate_leader_vote(value),
            ReplicationPeerEvidence::EffectVote(value) => self.validate_effect_vote(value),
            ReplicationPeerEvidence::MembershipVote(value) => self.validate_membership_vote(value),
            ReplicationPeerEvidence::DecisionVote(value) => self.validate_decision_vote(value),
            ReplicationPeerEvidence::JointMembershipAck(value) => {
                self.validate_joint_membership_ack(value)
            }
            ReplicationPeerEvidence::RecoveryAck(value) => self.validate_recovery_ack(value),
        }
    }

    fn apply_authenticated_peer_evidence(
        &mut self,
        signed: &SignedReplicationPeerEvidence,
        receipt: ReplicationAuthenticationReceipt,
    ) {
        self.authenticated_evidence
            .insert(receipt.proof_digest, receipt);
        match &signed.evidence {
            ReplicationPeerEvidence::JointMembershipAck(value) => {
                self.joint_membership_acks.insert(
                    (
                        value.previous_membership_epoch,
                        value.next_membership_epoch,
                        value.voter,
                    ),
                    *value,
                );
            }
            ReplicationPeerEvidence::RecoveryAck(value) => {
                self.recovery_acks.insert(
                    (value.membership_epoch, value.recovery_term, value.voter),
                    value.clone(),
                );
            }
            _ => {}
        }
    }

    fn commit_authenticated_peer_semantics(
        &mut self,
        evidence: ReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        match evidence {
            ReplicationPeerEvidence::TermPromise(value) => self.record_term_promise(value),
            ReplicationPeerEvidence::LeaderVote(value) => self.record_leader_vote(value),
            ReplicationPeerEvidence::EffectVote(value) => self.record_effect_vote(value),
            ReplicationPeerEvidence::MembershipVote(value) => self.record_membership_vote(value),
            ReplicationPeerEvidence::DecisionVote(value) => self.record_decision_vote(value),
            ReplicationPeerEvidence::JointMembershipAck(_)
            | ReplicationPeerEvidence::RecoveryAck(_) => Ok(()),
        }
    }

    fn require_authenticated_peer_evidence(
        &self,
        evidence: &ReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        let Some(policy) = &self.peer_auth_policy else {
            return Ok(());
        };
        let voter = evidence_voter(evidence);
        let (_, payload) = encode_peer_evidence(evidence)?;
        let payload_digest = sha256(&payload);
        if self.authenticated_evidence.values().any(|receipt| {
            receipt.trust_epoch == policy.trust_epoch
                && receipt.voter == voter
                && receipt.payload_digest == payload_digest
                && policy.peer_keys.get(&voter) == Some(&receipt.signer)
        }) {
            Ok(())
        } else {
            Err(protocol(
                "replication peer evidence is not authenticated in current trust epoch",
            ))
        }
    }

    fn validate_joint_membership_ack(
        &self,
        ack: &ReplicationJointMembershipAck,
    ) -> Result<(), DurabilityError> {
        if ack.term == 0 || ack.next_membership_epoch <= ack.previous_membership_epoch {
            return Err(protocol(
                "joint membership acknowledgement has invalid epoch/term",
            ));
        }
        let current = self.current_membership().ok_or_else(|| {
            protocol("joint membership acknowledgement has no current membership")
        })?;
        if current.epoch != ack.previous_membership_epoch {
            return Err(protocol(
                "joint membership acknowledgement uses a stale previous epoch",
            ));
        }
        if !current.members.contains(&ack.leader) {
            return Err(protocol(
                "joint membership acknowledgement names a non-member leader",
            ));
        }
        Ok(())
    }

    fn validate_recovery_ack(&self, ack: &ReplicationRecoveryAck) -> Result<(), DurabilityError> {
        validate_lock_summaries(&ack.locks)?;
        let ReplicationQuorumAvailability::Lost(loss) =
            self.quorum_availability.ok_or_else(|| {
                protocol("replication recovery acknowledgement has no quorum-loss fence")
            })?
        else {
            return Err(protocol(
                "replication recovery acknowledgement requires quorum loss",
            ));
        };
        if ack.membership_epoch != loss.membership_epoch {
            return Err(protocol(
                "replication recovery acknowledgement uses a stale membership",
            ));
        }
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication recovery acknowledgement has no membership"))?;
        if !current.members.contains(&ack.voter) || !current.members.contains(&ack.leader) {
            return Err(protocol(
                "replication recovery acknowledgement references a non-member",
            ));
        }
        let floor = loss
            .observed_term
            .max(self.highest_promised_term(loss.membership_epoch))
            .max(
                self.decision_locks
                    .values()
                    .map(|lock| lock.term)
                    .max()
                    .unwrap_or(0),
            );
        if ack.recovery_term <= floor {
            return Err(protocol(
                "replication recovery term does not advance the durable safety floor",
            ));
        }
        Ok(())
    }

    fn validate_quorum_loss(&self, loss: ReplicationQuorumLoss) -> Result<(), DurabilityError> {
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication quorum loss has no durable membership"))?;
        if loss.membership_epoch != current.epoch {
            return Err(protocol(
                "replication quorum loss uses a stale membership epoch",
            ));
        }
        if loss.observed_term < self.highest_promised_term(current.epoch) {
            return Err(protocol(
                "replication quorum loss observed term is below durable promises",
            ));
        }
        if let Some(ReplicationQuorumAvailability::Lost(existing)) = self.quorum_availability
            && loss.observed_term < existing.observed_term
        {
            return Err(protocol("replication quorum loss observed term regressed"));
        }
        Ok(())
    }

    fn validate_recovery_certificate(
        &self,
        certificate: &ReplicationRecoveryCertificate,
    ) -> Result<(), DurabilityError> {
        validate_lock_summaries(&certificate.reconciled_locks)?;
        let ReplicationQuorumAvailability::Lost(loss) = self
            .quorum_availability
            .ok_or_else(|| protocol("replication recovery has no durable quorum-loss fence"))?
        else {
            return Err(protocol("replication recovery requires quorum loss"));
        };
        if certificate.membership_epoch != loss.membership_epoch {
            return Err(protocol(
                "replication recovery uses a stale membership epoch",
            ));
        }
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication recovery has no durable membership"))?;
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by,
            "replication recovery lacks configured quorum",
        )?;
        let safety_floor = loss
            .observed_term
            .max(self.highest_promised_term(loss.membership_epoch))
            .max(
                self.decision_locks
                    .values()
                    .map(|lock| lock.term)
                    .max()
                    .unwrap_or(0),
            );
        if certificate.recovery_term <= safety_floor {
            return Err(protocol(
                "replication recovery term does not advance the durable safety floor",
            ));
        }
        if !current.members.contains(&certificate.leader) {
            return Err(protocol(
                "replication recovery leader is not a membership voter",
            ));
        }
        let local_locks: Vec<_> = self
            .decision_locks
            .values()
            .map(|lock| ReplicationLockSummary {
                position: lock.position,
                term: lock.term,
                effect: lock.effect,
            })
            .collect();
        if certificate.reconciled_locks != local_locks {
            return Err(protocol(
                "replication recovery did not reconcile the local lock frontier",
            ));
        }
        for voter in &certificate.acknowledged_by {
            let ack = self
                .recovery_acks
                .get(&(
                    certificate.membership_epoch,
                    certificate.recovery_term,
                    *voter,
                ))
                .ok_or_else(|| {
                    protocol("replication recovery lacks authenticated peer evidence")
                })?;
            if ack.leader != certificate.leader {
                return Err(protocol(
                    "replication recovery acknowledgements disagree on leader",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::RecoveryAck(
                ack.clone(),
            ))?;
            for remote in &ack.locks {
                let Some(local) = local_locks
                    .iter()
                    .find(|lock| lock.position == remote.position)
                else {
                    return Err(protocol(
                        "replication recovery requires anti-entropy for missing lock",
                    ));
                };
                if local.effect != remote.effect || local.term < remote.term {
                    return Err(protocol(
                        "replication recovery lock frontier conflicts with peer evidence",
                    ));
                }
            }
        }
        Ok(())
    }

    fn apply_recovery_certificate(&mut self, certificate: &ReplicationRecoveryCertificate) {
        self.leader_certificates.insert(
            (certificate.membership_epoch, certificate.recovery_term),
            ReplicationLeaderCertificate {
                membership_epoch: certificate.membership_epoch,
                term: certificate.recovery_term,
                leader: certificate.leader,
                acknowledged_by: certificate.acknowledged_by.clone(),
            },
        );
        for voter in &certificate.acknowledged_by {
            self.promised_terms
                .entry((certificate.membership_epoch, *voter))
                .and_modify(|term| *term = (*term).max(certificate.recovery_term))
                .or_insert(certificate.recovery_term);
        }
        self.quorum_availability = Some(ReplicationQuorumAvailability::Available {
            membership_epoch: certificate.membership_epoch,
            term: certificate.recovery_term,
        });
    }

    fn require_quorum_available(&self) -> Result<(), DurabilityError> {
        if matches!(
            self.quorum_availability,
            Some(ReplicationQuorumAvailability::Lost(_))
        ) {
            Err(protocol(
                "replication consensus authority is fenced by quorum loss",
            ))
        } else {
            Ok(())
        }
    }

    fn note_available_term(&mut self, membership_epoch: u64, term: u64) {
        if let Some(ReplicationQuorumAvailability::Available {
            membership_epoch: current_epoch,
            term: current_term,
        }) = &mut self.quorum_availability
            && *current_epoch == membership_epoch
        {
            *current_term = (*current_term).max(term);
        }
    }

    fn validate_membership_change(
        &self,
        change: &ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        change.next.validate()?;
        match self.current_membership() {
            None => {
                if !change.acknowledged_by_previous.is_empty() {
                    return Err(protocol(
                        "replication membership bootstrap cannot claim prior acknowledgements",
                    ));
                }
            }
            Some(current) => {
                self.require_quorum_available()?;
                if change.next.epoch <= current.epoch {
                    return Err(protocol("replication membership epoch did not advance"));
                }
                validate_acknowledgements(
                    current,
                    &change.acknowledged_by_previous,
                    "replication membership change lacks previous-epoch quorum",
                )?;
                let mut decision_term = None;
                for voter in &change.acknowledged_by_previous {
                    let vote = self
                        .membership_votes
                        .get(&(current.epoch, *voter))
                        .ok_or_else(|| {
                            protocol("replication membership change lacks durable vote evidence")
                        })?;
                    if vote.next != change.next {
                        return Err(protocol(
                            "replication membership vote targets another successor",
                        ));
                    }
                    match decision_term {
                        None => decision_term = Some(vote.term),
                        Some(term) if term == vote.term => {}
                        Some(_) => {
                            return Err(protocol(
                                "replication membership quorum mixes election terms",
                            ));
                        }
                    }
                }
                if self
                    .leader_certificates
                    .keys()
                    .any(|(epoch, _)| *epoch == current.epoch)
                {
                    let term = decision_term.expect("non-empty old quorum has a decision term");
                    if !self
                        .leader_certificates
                        .contains_key(&(current.epoch, term))
                    {
                        return Err(protocol(
                            "replication membership change is not bound to a certified leader term",
                        ));
                    }
                    if term < self.highest_promised_term(current.epoch) {
                        return Err(protocol(
                            "replication membership change uses a stale consensus term",
                        ));
                    }
                    let joint = self
                        .joint_membership_certificates
                        .get(&change.next.epoch)
                        .ok_or_else(|| {
                            protocol("replication membership change lacks joint quorum certificate")
                        })?;
                    if joint.previous_membership_epoch != current.epoch
                        || joint.term != term
                        || joint.next != change.next
                        || joint.acknowledged_by_previous != change.acknowledged_by_previous
                    {
                        return Err(protocol(
                            "replication membership change does not match joint quorum certificate",
                        ));
                    }
                }
            }
        }
        if self.memberships.contains_key(&change.next.epoch) {
            return Err(protocol(
                "replication membership epoch is already bound to another configuration",
            ));
        }
        Ok(())
    }

    fn apply_membership_change(
        &mut self,
        change: ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        self.validate_membership_change(&change)?;
        let epoch = change.next.epoch;
        self.memberships.insert(epoch, change.next);
        self.current_membership_epoch = Some(epoch);
        self.quorum_availability = Some(ReplicationQuorumAvailability::Available {
            membership_epoch: epoch,
            term: 0,
        });
        Ok(())
    }

    fn validate_term_member(
        &self,
        membership_epoch: u64,
        voter: ReplicaId,
        term: u64,
    ) -> Result<(), DurabilityError> {
        if term == 0 {
            return Err(protocol("replication consensus term is zero"));
        }
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication consensus has no durable membership"))?;
        if current.epoch != membership_epoch {
            return Err(protocol(
                "replication consensus uses a stale membership epoch",
            ));
        }
        if !current.members.contains(&voter) {
            return Err(protocol(
                "replication consensus references a non-member replica",
            ));
        }
        Ok(())
    }

    fn highest_promised_term(&self, membership_epoch: u64) -> u64 {
        self.promised_terms
            .iter()
            .filter_map(|((epoch, _), term)| (*epoch == membership_epoch).then_some(*term))
            .max()
            .unwrap_or(0)
    }

    fn validate_leader_vote(&self, vote: &ReplicationLeaderVote) -> Result<(), DurabilityError> {
        self.validate_term_member(vote.membership_epoch, vote.voter, vote.term)?;
        let current = self
            .current_membership()
            .expect("validated membership exists");
        if !current.members.contains(&vote.candidate) {
            return Err(protocol(
                "replication leader candidate is not a membership voter",
            ));
        }
        if self
            .promised_terms
            .get(&(vote.membership_epoch, vote.voter))
            .is_some_and(|promised| *promised > vote.term)
        {
            return Err(protocol("replication leader vote uses a stale term"));
        }
        if self
            .leader_votes
            .get(&(vote.membership_epoch, vote.term, vote.voter))
            .is_some_and(|candidate| *candidate != vote.candidate)
        {
            return Err(protocol(
                "replication voter already voted for another leader in this term",
            ));
        }
        Ok(())
    }

    fn validate_leader_certificate(
        &self,
        certificate: &ReplicationLeaderCertificate,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        self.validate_term_member(
            certificate.membership_epoch,
            certificate.leader,
            certificate.term,
        )?;
        let current = self
            .current_membership()
            .expect("validated membership exists");
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by,
            "replication leader certificate lacks configured quorum",
        )?;
        if certificate.term < self.highest_promised_term(certificate.membership_epoch) {
            return Err(protocol(
                "replication leader certificate is fenced by a higher promised term",
            ));
        }
        for voter in &certificate.acknowledged_by {
            if self
                .leader_votes
                .get(&(certificate.membership_epoch, certificate.term, *voter))
                != Some(&certificate.leader)
            {
                return Err(protocol(
                    "replication leader certificate lacks durable vote evidence",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::LeaderVote(
                ReplicationLeaderVote {
                    voter: *voter,
                    membership_epoch: certificate.membership_epoch,
                    term: certificate.term,
                    candidate: certificate.leader,
                },
            ))?;
        }
        Ok(())
    }

    fn validate_decision_vote(
        &self,
        vote: &ReplicationDecisionVote,
    ) -> Result<(), DurabilityError> {
        self.validate_term_member(vote.membership_epoch, vote.voter, vote.term)?;
        let certificate = self
            .leader_certificates
            .get(&(vote.membership_epoch, vote.term))
            .ok_or_else(|| {
                protocol("replication decision vote has no durable leader certificate")
            })?;
        if certificate.leader != vote.leader {
            return Err(protocol(
                "replication decision vote names a different leader",
            ));
        }
        if vote.term < self.highest_promised_term(vote.membership_epoch) {
            return Err(protocol("replication decision vote uses a stale term"));
        }
        let envelope = self.effects.get(&vote.effect).ok_or_else(|| {
            protocol("replication decision vote references a non-local-durable effect")
        })?;
        if envelope.ordered_by.position != vote.position {
            return Err(protocol(
                "replication decision vote position does not match effect ordering",
            ));
        }
        if let Some(lock) = self.decision_locks.get(&vote.position) {
            if lock.effect != vote.effect {
                return Err(protocol(
                    "replication decision conflicts with a durable locked value",
                ));
            }
            if vote.term > lock.term && vote.carried_from_term != Some(lock.term) {
                return Err(protocol(
                    "replication later-term decision did not carry forward the durable lock",
                ));
            }
        } else if vote.carried_from_term.is_some() {
            return Err(protocol(
                "replication decision claims a carry-forward without a prior lock",
            ));
        }
        if self
            .decision_votes
            .get(&(vote.membership_epoch, vote.term, vote.position, vote.voter))
            .is_some_and(|existing| existing != vote)
        {
            return Err(protocol(
                "replication voter already accepted another value in this term/position",
            ));
        }
        Ok(())
    }

    fn validate_decision_lock(
        &self,
        lock: &ReplicationDecisionLock,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        self.validate_term_member(lock.membership_epoch, lock.leader, lock.term)?;
        let current = self
            .current_membership()
            .expect("validated membership exists");
        validate_acknowledgements(
            current,
            &lock.acknowledged_by,
            "replication decision lock lacks configured quorum",
        )?;
        let certificate = self
            .leader_certificates
            .get(&(lock.membership_epoch, lock.term))
            .ok_or_else(|| {
                protocol("replication decision lock has no durable leader certificate")
            })?;
        if certificate.leader != lock.leader {
            return Err(protocol(
                "replication decision lock names a different leader",
            ));
        }
        if lock.term < self.highest_promised_term(lock.membership_epoch) {
            return Err(protocol("replication decision lock uses a stale term"));
        }
        for voter in &lock.acknowledged_by {
            let vote = self
                .decision_votes
                .get(&(lock.membership_epoch, lock.term, lock.position, *voter))
                .ok_or_else(|| protocol("replication decision lock lacks durable vote evidence"))?;
            if vote.effect != lock.effect
                || vote.leader != lock.leader
                || vote.carried_from_term != lock.carried_from_term
            {
                return Err(protocol(
                    "replication decision lock vote evidence does not match",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::DecisionVote(
                *vote,
            ))?;
        }
        if let Some(existing) = self.decision_locks.get(&lock.position) {
            if existing.effect != lock.effect {
                return Err(protocol(
                    "replication decision lock conflicts with prior locked value",
                ));
            }
            if lock.term <= existing.term {
                return Err(protocol("replication decision lock did not advance term"));
            }
            if lock.carried_from_term != Some(existing.term) {
                return Err(protocol(
                    "replication later-term lock did not carry forward prior lock term",
                ));
            }
        } else if lock.carried_from_term.is_some() {
            return Err(protocol(
                "replication decision lock claims carry-forward without prior lock",
            ));
        }
        Ok(())
    }

    fn validate_joint_membership_certificate(
        &self,
        certificate: &ReplicationJointMembershipCertificate,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        certificate.next.validate()?;
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("joint membership certificate has no previous membership"))?;
        if current.epoch != certificate.previous_membership_epoch {
            return Err(protocol(
                "joint membership certificate uses a stale previous epoch",
            ));
        }
        if certificate.next.epoch <= current.epoch {
            return Err(protocol(
                "joint membership certificate successor epoch did not advance",
            ));
        }
        let leader = self
            .leader_certificates
            .get(&(current.epoch, certificate.term))
            .ok_or_else(|| {
                protocol("joint membership certificate has no durable leader certificate")
            })?;
        if leader.leader != certificate.leader {
            return Err(protocol(
                "joint membership certificate names a different leader",
            ));
        }
        if certificate.term < self.highest_promised_term(current.epoch) {
            return Err(protocol(
                "joint membership certificate is fenced by a higher promised term",
            ));
        }
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by_previous,
            "joint membership certificate lacks previous-membership quorum",
        )?;
        validate_acknowledgements(
            &certificate.next,
            &certificate.acknowledged_by_next,
            "joint membership certificate lacks successor-membership quorum",
        )?;
        if let Some(policy) = &self.peer_auth_policy
            && certificate
                .next
                .members
                .iter()
                .any(|member| !policy.peer_keys.contains_key(member))
        {
            return Err(protocol(
                "joint membership successor is not covered by peer-auth policy",
            ));
        }
        for voter in &certificate.acknowledged_by_previous {
            let vote = self
                .membership_votes
                .get(&(current.epoch, *voter))
                .ok_or_else(|| {
                    protocol(
                        "joint membership certificate lacks durable old-membership vote evidence",
                    )
                })?;
            if vote.term != certificate.term || vote.next != certificate.next {
                return Err(protocol(
                    "joint membership certificate vote evidence does not match",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::MembershipVote(
                vote.clone(),
            ))?;
        }
        if self.peer_auth_policy.is_some() {
            let next_digest = replication_membership_digest(&certificate.next)?;
            for voter in &certificate.acknowledged_by_next {
                let ack = self
                    .joint_membership_acks
                    .get(&(current.epoch, certificate.next.epoch, *voter))
                    .ok_or_else(|| {
                        protocol(
                            "joint membership certificate lacks authenticated successor evidence",
                        )
                    })?;
                if ack.term != certificate.term
                    || ack.leader != certificate.leader
                    || ack.next_membership_digest != next_digest
                {
                    return Err(protocol(
                        "joint membership successor evidence does not match certificate",
                    ));
                }
                self.require_authenticated_peer_evidence(
                    &ReplicationPeerEvidence::JointMembershipAck(*ack),
                )?;
            }
        }
        Ok(())
    }

    fn validate_quorum_certificate(
        &self,
        certificate: &ReplicationQuorumCertificate,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        if !self.effects.contains_key(&certificate.effect) {
            return Err(protocol(
                "quorum certificate references a non-local-durable replicated effect",
            ));
        }
        let Some(current) = self.current_membership() else {
            return Err(protocol("replication quorum has no durable membership"));
        };
        if certificate.membership_epoch != current.epoch {
            return Err(protocol(
                "replication quorum certificate uses a stale membership epoch",
            ));
        }
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by,
            "replication quorum certificate lacks configured quorum",
        )?;
        let position = self
            .effects
            .get(&certificate.effect)
            .expect("validated certificate references local-durable effect")
            .ordered_by
            .position;
        if self
            .leader_certificates
            .keys()
            .any(|(epoch, _)| *epoch == current.epoch)
        {
            let lock = self
                .decision_locks
                .get(&position)
                .ok_or_else(|| protocol("replication quorum lacks consensus decision lock"))?;
            if lock.effect != certificate.effect || lock.membership_epoch != current.epoch {
                return Err(protocol(
                    "replication quorum does not match consensus decision lock",
                ));
            }
        }
        let position = self
            .effects
            .get(&certificate.effect)
            .expect("validated certificate references local-durable effect")
            .ordered_by
            .position;
        for voter in &certificate.acknowledged_by {
            if self
                .effect_votes
                .get(&(certificate.membership_epoch, position, *voter))
                != Some(&certificate.effect)
            {
                return Err(protocol(
                    "replication quorum certificate lacks durable vote evidence",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::EffectVote(
                ReplicationEffectVote {
                    voter: *voter,
                    effect: certificate.effect,
                    membership_epoch: certificate.membership_epoch,
                },
            ))?;
        }
        Ok(())
    }

    fn validate_effect_vote(&self, vote: &ReplicationEffectVote) -> Result<(), DurabilityError> {
        let Some(current) = self.current_membership() else {
            return Err(protocol("replication vote has no durable membership"));
        };
        if vote.membership_epoch != current.epoch {
            return Err(protocol("replication vote uses a stale membership epoch"));
        }
        if !current.members.contains(&vote.voter) {
            return Err(protocol("replication vote references a non-member replica"));
        }
        let envelope = self
            .effects
            .get(&vote.effect)
            .ok_or_else(|| protocol("replication vote references a non-local-durable effect"))?;
        let key = (
            vote.membership_epoch,
            envelope.ordered_by.position,
            vote.voter,
        );
        if self
            .effect_votes
            .get(&key)
            .is_some_and(|existing| *existing != vote.effect)
        {
            return Err(protocol(
                "replication voter already voted for another effect in this decision slot",
            ));
        }
        Ok(())
    }

    fn validate_membership_vote(
        &self,
        vote: &ReplicationMembershipVote,
    ) -> Result<(), DurabilityError> {
        vote.next.validate()?;
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("membership vote has no previous durable membership"))?;
        if current.epoch != vote.previous_membership_epoch {
            return Err(protocol("membership vote uses a stale previous epoch"));
        }
        if vote.term == 0 || vote.next.epoch <= current.epoch {
            return Err(protocol(
                "membership vote has invalid term or successor epoch",
            ));
        }
        if !current.members.contains(&vote.voter) {
            return Err(protocol("membership vote references a non-member replica"));
        }
        if self
            .membership_votes
            .get(&(current.epoch, vote.voter))
            .is_some_and(|existing| existing != vote)
        {
            return Err(protocol(
                "replication voter already voted for another successor membership",
            ));
        }
        Ok(())
    }

    fn validate_publish(&self, effect: RevisionEffectId) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        if !self.quorum_certificates.contains_key(&effect) {
            return Err(protocol(
                "replicated effect cannot publish before quorum durability",
            ));
        }
        let envelope = self
            .effects
            .get(&effect)
            .ok_or_else(|| protocol("published replicated effect is missing"))?;
        if envelope.effect.prerequisites.iter().any(|prerequisite| {
            replicated_origin(*prerequisite).is_some()
                && !self.quorum_certificates.contains_key(prerequisite)
        }) {
            return Err(protocol(
                "replicated effect cannot publish before replicated prerequisites are quorum durable",
            ));
        }
        if self
            .branches
            .get(&envelope.branch)
            .is_some_and(|branch| branch.retired)
        {
            return Err(protocol("retired replication branch cannot publish"));
        }
        if let Some(published) = self.published_branches.get(&envelope.branch) {
            if envelope.effect.source_revision != published.head_revision {
                return Err(protocol("replicated branch publication is not contiguous"));
            }
        } else if self.effects.values().any(|candidate| {
            candidate.branch == envelope.branch
                && candidate.effect.target_revision == envelope.effect.source_revision
        }) {
            return Err(protocol(
                "replicated branch publication skipped an earlier local-durable effect",
            ));
        }
        Ok(())
    }

    fn apply_publish(&mut self, effect: RevisionEffectId) -> Result<(), DurabilityError> {
        self.validate_publish(effect)?;
        let envelope = self
            .effects
            .get(&effect)
            .expect("validated publication references durable effect");
        self.published_effects.insert(effect);
        self.published_branches.insert(
            envelope.branch,
            ReplicationBranchHead {
                branch: envelope.branch,
                head_revision: envelope.effect.target_revision,
                head_effect: effect,
                retired: false,
            },
        );
        Ok(())
    }

    fn validate_envelope(
        &self,
        envelope: &ReplicatedEffectEnvelope,
    ) -> Result<(), DurabilityError> {
        if envelope.origin.raw() == 0 || envelope.origin_sequence == 0 {
            return Err(protocol("replicated effect origin namespace is invalid"));
        }
        if envelope.branch.raw() == 0 {
            return Err(protocol("replication branch identity is zero"));
        }
        if envelope.ordered_by.sequencer.raw() == 0 || envelope.ordered_by.position == 0 {
            return Err(protocol(
                "replicated effect lacks a valid ordered admission witness",
            ));
        }
        if self
            .sequencer_epochs
            .get(&envelope.ordered_by.sequencer)
            .is_some_and(|current| envelope.ordered_by.epoch < *current)
        {
            return Err(protocol("replicated effect uses a stale sequencer epoch"));
        }
        let expected = replicated_effect_id(envelope.origin, envelope.origin_sequence);
        if envelope.effect.id != expected {
            return Err(protocol(
                "replicated effect id does not match origin sequence namespace",
            ));
        }
        envelope.effect.validate_identity().map_err(protocol)?;
        if let Some(existing) = self.ordered_slots.get(&envelope.ordered_by)
            && *existing != envelope.effect.id
        {
            return Err(protocol(
                "sequencer order slot is already bound to another effect",
            ));
        }
        if envelope
            .effect
            .prerequisites
            .iter()
            .any(|id| replicated_origin(*id).is_some() && !self.effects.contains_key(id))
        {
            return Err(protocol(
                "replicated effect is not down-closed over remote dependencies",
            ));
        }
        if let Some(head) = self.branches.get(&envelope.branch) {
            if head.retired {
                return Err(protocol("retired replication branch cannot be advanced"));
            }
            if envelope.effect.source_revision != head.head_revision {
                return Err(protocol(
                    "replicated effect does not advance the durable branch head",
                ));
            }
        }
        if self
            .revision_frontiers
            .contains_key(&envelope.effect.target_revision)
        {
            return Err(protocol(
                "replicated target revision already has a durable branch frontier",
            ));
        }
        Ok(())
    }

    fn apply_ingest(&mut self, envelope: ReplicatedEffectEnvelope) -> Result<(), DurabilityError> {
        self.validate_envelope(&envelope)?;
        let id = envelope.effect.id;
        let target = envelope.effect.target_revision;
        self.ordered_slots.insert(envelope.ordered_by, id);
        self.sequencer_epochs
            .entry(envelope.ordered_by.sequencer)
            .and_modify(|epoch| *epoch = (*epoch).max(envelope.ordered_by.epoch))
            .or_insert(envelope.ordered_by.epoch);
        self.revision_frontiers.insert(target, BTreeSet::from([id]));
        self.branches.insert(
            envelope.branch,
            ReplicationBranchHead {
                branch: envelope.branch,
                head_revision: target,
                head_effect: id,
                retired: false,
            },
        );
        self.effects.insert(id, envelope);
        Ok(())
    }

    fn append_frame(&mut self, kind: u8, payload: &[u8]) -> Result<(), DurabilityError> {
        let len = u32::try_from(payload.len()).map_err(|_| CodecError::LengthOverflow)?;
        let mut header = [0_u8; FRAME_HEADER_LEN];
        header[..4].copy_from_slice(&REPLICATION_MAGIC);
        header[4..6].copy_from_slice(&REPLICATION_VERSION.to_le_bytes());
        header[6] = kind;
        header[8..12].copy_from_slice(&len.to_le_bytes());
        header[12..16].copy_from_slice(&crc32c(payload).to_le_bytes());
        if let Err(error) = self
            .file
            .write_all(&header)
            .and_then(|()| self.file.write_all(payload))
        {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        if let Err(error) = self.file.sync_data() {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        Ok(())
    }

    fn replay_consensus_frame(
        &mut self,
        kind: u8,
        payload: &[u8],
        offset: usize,
    ) -> Result<bool, DurabilityError> {
        match kind {
            KIND_TERM_PROMISE => {
                let promise = decode_term_promise(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::TermPromise(
                    promise,
                ))?;
                self.validate_term_member(promise.membership_epoch, promise.voter, promise.term)?;
                let key = (promise.membership_epoch, promise.voter);
                if self
                    .promised_terms
                    .get(&key)
                    .is_some_and(|term| *term > promise.term)
                {
                    return Err(corruption(
                        offset,
                        "replication term promise regressed during replay",
                    ));
                }
                self.promised_terms.insert(key, promise.term);
            }
            KIND_LEADER_VOTE => {
                let vote = decode_leader_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::LeaderVote(
                    vote,
                ))?;
                self.validate_leader_vote(&vote)?;
                self.promised_terms
                    .entry((vote.membership_epoch, vote.voter))
                    .and_modify(|term| *term = (*term).max(vote.term))
                    .or_insert(vote.term);
                self.leader_votes.insert(
                    (vote.membership_epoch, vote.term, vote.voter),
                    vote.candidate,
                );
            }
            KIND_LEADER_CERTIFICATE => {
                let certificate = decode_leader_certificate(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_leader_certificate(&certificate)?;
                let membership_epoch = certificate.membership_epoch;
                let term = certificate.term;
                self.leader_certificates
                    .insert((membership_epoch, term), certificate);
                self.note_available_term(membership_epoch, term);
            }
            KIND_DECISION_VOTE => {
                let vote = decode_decision_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::DecisionVote(
                    vote,
                ))?;
                self.validate_decision_vote(&vote)?;
                self.decision_votes.insert(
                    (vote.membership_epoch, vote.term, vote.position, vote.voter),
                    vote,
                );
            }
            KIND_DECISION_LOCK => {
                let lock = decode_decision_lock(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_decision_lock(&lock)?;
                self.decision_locks.insert(lock.position, lock);
            }
            KIND_JOINT_MEMBERSHIP_CERTIFICATE => {
                let certificate = decode_joint_membership_certificate(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_joint_membership_certificate(&certificate)?;
                self.joint_membership_certificates
                    .insert(certificate.next.epoch, certificate);
            }
            KIND_PEER_AUTH_POLICY => {
                let policy = decode_peer_auth_policy(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_replayed_peer_auth_policy(&policy)?;
                self.peer_auth_policy = Some(policy);
            }
            KIND_AUTHENTICATED_PEER_EVIDENCE => {
                let signed = decode_signed_peer_evidence(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_replayed_peer_evidence(&signed)?;
                let receipt = authentication_receipt(&signed)?;
                self.apply_authenticated_peer_evidence(&signed, receipt);
            }
            KIND_QUORUM_LOSS => {
                let loss = decode_quorum_loss(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_quorum_loss(loss)?;
                self.quorum_availability = Some(ReplicationQuorumAvailability::Lost(loss));
            }
            KIND_QUORUM_RECOVERY => {
                let certificate = decode_recovery_certificate(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_recovery_certificate(&certificate)?;
                self.apply_recovery_certificate(&certificate);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn replay_vote_frame(
        &mut self,
        kind: u8,
        payload: &[u8],
        offset: usize,
    ) -> Result<bool, DurabilityError> {
        match kind {
            KIND_EFFECT_VOTE => {
                let vote = decode_effect_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::EffectVote(
                    vote,
                ))?;
                self.validate_effect_vote(&vote)?;
                let envelope = self
                    .effects
                    .get(&vote.effect)
                    .expect("validated replay vote references effect");
                self.effect_votes.insert(
                    (
                        vote.membership_epoch,
                        envelope.ordered_by.position,
                        vote.voter,
                    ),
                    vote.effect,
                );
            }
            KIND_MEMBERSHIP_VOTE => {
                let vote = decode_membership_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(
                    &ReplicationPeerEvidence::MembershipVote(vote.clone()),
                )?;
                self.validate_membership_vote(&vote)?;
                self.membership_votes
                    .insert((vote.previous_membership_epoch, vote.voter), vote);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn replay(&mut self, bytes: &[u8]) -> Result<usize, DurabilityError> {
        let mut offset = 0;
        while bytes.len().saturating_sub(offset) >= FRAME_HEADER_LEN {
            let header = &bytes[offset..offset + FRAME_HEADER_LEN];
            if header[..4] != REPLICATION_MAGIC {
                return Err(corruption(offset, "replication journal magic mismatch"));
            }
            if read_u16(&header[4..6]) != REPLICATION_VERSION {
                return Err(corruption(
                    offset,
                    "unsupported replication journal version",
                ));
            }
            let kind = header[6];
            let len = usize::try_from(read_u32(&header[8..12]))
                .map_err(|_| CodecError::LengthOverflow)?;
            let end = offset
                .checked_add(FRAME_HEADER_LEN)
                .and_then(|value| value.checked_add(len))
                .ok_or(CodecError::LengthOverflow)?;
            if end > bytes.len() {
                break;
            }
            let payload = &bytes[offset + FRAME_HEADER_LEN..end];
            if crc32c(payload) != read_u32(&header[12..16]) {
                return Err(corruption(
                    offset,
                    "replication journal payload checksum mismatch",
                ));
            }
            match kind {
                KIND_INGEST => self.apply_ingest(
                    decode_ingest(payload)
                        .map_err(|reason| DurabilityError::Corruption { offset, reason })?,
                )?,
                KIND_RETIRE => {
                    let (branch, expected_head) = decode_retire(payload)
                        .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                    let head = self.branches.get_mut(&branch).ok_or_else(|| {
                        corruption(offset, "retirement references unknown branch")
                    })?;
                    if head.retired || head.head_effect != expected_head {
                        return Err(corruption(
                            offset,
                            "replication branch retirement conflicts with journal state",
                        ));
                    }
                    head.retired = true;
                    if let Some(published) = self.published_branches.get_mut(&branch) {
                        published.retired = true;
                    }
                }
                KIND_MEMBERSHIP => self.apply_membership_change(
                    decode_membership_change(payload)
                        .map_err(|reason| DurabilityError::Corruption { offset, reason })?,
                )?,
                KIND_QUORUM => {
                    let certificate = decode_quorum_certificate(payload)
                        .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                    self.validate_quorum_certificate(&certificate)?;
                    self.quorum_certificates
                        .insert(certificate.effect, certificate);
                }
                KIND_PUBLISH => {
                    let effect = decode_publish(payload)
                        .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                    self.apply_publish(effect)?;
                }
                _ if self.replay_vote_frame(kind, payload, offset)? => {}
                _ if self.replay_consensus_frame(kind, payload, offset)? => {}
                _ => return Err(corruption(offset, "unknown replication journal frame kind")),
            }
            offset = end;
        }
        Ok(offset)
    }
}

fn encode_effect_vote(vote: &ReplicationEffectVote) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u128(&mut out, vote.effect.0);
    push_u64(&mut out, vote.membership_epoch);
    out
}

fn decode_effect_vote(bytes: &[u8]) -> Result<ReplicationEffectVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let vote = ReplicationEffectVote {
        voter: ReplicaId::new(cursor.u64()?),
        effect: RevisionEffectId(cursor.u128()?),
        membership_epoch: cursor.u64()?,
    };
    cursor.finish()?;
    Ok(vote)
}

fn encode_membership_vote(vote: &ReplicationMembershipVote) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.previous_membership_epoch);
    push_u64(&mut out, vote.term);
    push_u64(&mut out, vote.next.epoch);
    push_len(&mut out, vote.next.members.len())?;
    for member in &vote.next.members {
        push_u64(&mut out, member.raw());
    }
    push_len(&mut out, vote.next.quorum_size)?;
    Ok(out)
}

fn decode_membership_vote(bytes: &[u8]) -> Result<ReplicationMembershipVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let voter = ReplicaId::new(cursor.u64()?);
    let previous_membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let epoch = cursor.u64()?;
    let members = decode_replica_set(
        &mut cursor,
        "membership vote members are not sorted and unique",
    )?;
    let quorum_size = cursor.len()?;
    cursor.finish()?;
    Ok(ReplicationMembershipVote {
        voter,
        previous_membership_epoch,
        term,
        next: ReplicationMembership {
            epoch,
            members,
            quorum_size,
        },
    })
}

fn encode_term_promise(promise: &ReplicationTermPromise) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, promise.voter.raw());
    push_u64(&mut out, promise.membership_epoch);
    push_u64(&mut out, promise.term);
    out
}

fn decode_term_promise(bytes: &[u8]) -> Result<ReplicationTermPromise, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let promise = ReplicationTermPromise {
        voter: ReplicaId::new(cursor.u64()?),
        membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
    };
    cursor.finish()?;
    Ok(promise)
}

fn encode_leader_vote(vote: &ReplicationLeaderVote) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.membership_epoch);
    push_u64(&mut out, vote.term);
    push_u64(&mut out, vote.candidate.raw());
    out
}

fn decode_leader_vote(bytes: &[u8]) -> Result<ReplicationLeaderVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let vote = ReplicationLeaderVote {
        voter: ReplicaId::new(cursor.u64()?),
        membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
        candidate: ReplicaId::new(cursor.u64()?),
    };
    cursor.finish()?;
    Ok(vote)
}

fn encode_leader_certificate(
    certificate: &ReplicationLeaderCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, certificate.membership_epoch);
    push_u64(&mut out, certificate.term);
    push_u64(&mut out, certificate.leader.raw());
    push_len(&mut out, certificate.acknowledged_by.len())?;
    for voter in &certificate.acknowledged_by {
        push_u64(&mut out, voter.raw());
    }
    Ok(out)
}

fn decode_leader_certificate(bytes: &[u8]) -> Result<ReplicationLeaderCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication leader certificate acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationLeaderCertificate {
        membership_epoch,
        term,
        leader,
        acknowledged_by,
    })
}

fn encode_optional_term(out: &mut Vec<u8>, term: Option<u64>) {
    match term {
        None => push_u64(out, 0),
        Some(term) => push_u64(out, term),
    }
}

fn decode_optional_term(cursor: &mut Cursor<'_>) -> Result<Option<u64>, &'static str> {
    match cursor.u64()? {
        0 => Ok(None),
        term => Ok(Some(term)),
    }
}

fn encode_decision_vote(vote: &ReplicationDecisionVote) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.membership_epoch);
    push_u64(&mut out, vote.term);
    push_u64(&mut out, vote.leader.raw());
    push_u64(&mut out, vote.position);
    push_u128(&mut out, vote.effect.0);
    encode_optional_term(&mut out, vote.carried_from_term);
    out
}

fn decode_decision_vote(bytes: &[u8]) -> Result<ReplicationDecisionVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let vote = ReplicationDecisionVote {
        voter: ReplicaId::new(cursor.u64()?),
        membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
        leader: ReplicaId::new(cursor.u64()?),
        position: cursor.u64()?,
        effect: RevisionEffectId(cursor.u128()?),
        carried_from_term: decode_optional_term(&mut cursor)?,
    };
    cursor.finish()?;
    Ok(vote)
}

fn encode_decision_lock(lock: &ReplicationDecisionLock) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, lock.membership_epoch);
    push_u64(&mut out, lock.term);
    push_u64(&mut out, lock.leader.raw());
    push_u64(&mut out, lock.position);
    push_u128(&mut out, lock.effect.0);
    encode_optional_term(&mut out, lock.carried_from_term);
    push_len(&mut out, lock.acknowledged_by.len())?;
    for voter in &lock.acknowledged_by {
        push_u64(&mut out, voter.raw());
    }
    Ok(out)
}

fn decode_decision_lock(bytes: &[u8]) -> Result<ReplicationDecisionLock, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let position = cursor.u64()?;
    let effect = RevisionEffectId(cursor.u128()?);
    let carried_from_term = decode_optional_term(&mut cursor)?;
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication decision lock acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationDecisionLock {
        membership_epoch,
        term,
        leader,
        position,
        effect,
        acknowledged_by,
        carried_from_term,
    })
}

fn encode_membership_value(
    out: &mut Vec<u8>,
    membership: &ReplicationMembership,
) -> Result<(), CodecError> {
    push_u64(out, membership.epoch);
    push_len(out, membership.members.len())?;
    for member in &membership.members {
        push_u64(out, member.raw());
    }
    push_len(out, membership.quorum_size)?;
    Ok(())
}

fn decode_membership_value(cursor: &mut Cursor<'_>) -> Result<ReplicationMembership, &'static str> {
    let epoch = cursor.u64()?;
    let members = decode_replica_set(
        cursor,
        "joint membership certificate members are not sorted and unique",
    )?;
    let quorum_size = cursor.len()?;
    Ok(ReplicationMembership {
        epoch,
        members,
        quorum_size,
    })
}

fn encode_joint_membership_certificate(
    certificate: &ReplicationJointMembershipCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, certificate.previous_membership_epoch);
    push_u64(&mut out, certificate.term);
    push_u64(&mut out, certificate.leader.raw());
    encode_membership_value(&mut out, &certificate.next)?;
    push_len(&mut out, certificate.acknowledged_by_previous.len())?;
    for voter in &certificate.acknowledged_by_previous {
        push_u64(&mut out, voter.raw());
    }
    push_len(&mut out, certificate.acknowledged_by_next.len())?;
    for voter in &certificate.acknowledged_by_next {
        push_u64(&mut out, voter.raw());
    }
    Ok(out)
}

fn decode_joint_membership_certificate(
    bytes: &[u8],
) -> Result<ReplicationJointMembershipCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let previous_membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let next = decode_membership_value(&mut cursor)?;
    let acknowledged_by_previous = decode_replica_set(
        &mut cursor,
        "joint membership previous acknowledgements are not sorted and unique",
    )?;
    let acknowledged_by_next = decode_replica_set(
        &mut cursor,
        "joint membership next acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationJointMembershipCertificate {
        previous_membership_epoch,
        term,
        leader,
        next,
        acknowledged_by_previous,
        acknowledged_by_next,
    })
}

fn encode_lock_summaries(
    out: &mut Vec<u8>,
    locks: &[ReplicationLockSummary],
) -> Result<(), CodecError> {
    push_len(out, locks.len())?;
    for lock in locks {
        push_u64(out, lock.position);
        push_u64(out, lock.term);
        push_u128(out, lock.effect.0);
    }
    Ok(())
}

fn decode_lock_summaries(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<ReplicationLockSummary>, &'static str> {
    let count = cursor.len()?;
    let mut locks = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let lock = ReplicationLockSummary {
            position: cursor.u64()?,
            term: cursor.u64()?,
            effect: RevisionEffectId(cursor.u128()?),
        };
        if previous.is_some_and(|position| position >= lock.position) {
            return Err("replication lock summaries are not sorted and unique");
        }
        previous = Some(lock.position);
        locks.push(lock);
    }
    Ok(locks)
}

fn validate_lock_summaries(locks: &[ReplicationLockSummary]) -> Result<(), DurabilityError> {
    if locks
        .windows(2)
        .any(|pair| pair[0].position >= pair[1].position)
    {
        return Err(protocol(
            "replication lock summaries are not sorted and unique",
        ));
    }
    if locks.iter().any(|lock| lock.term == 0) {
        return Err(protocol("replication lock summary has zero term"));
    }
    Ok(())
}

pub fn replication_membership_digest(
    membership: &ReplicationMembership,
) -> Result<Sha256Digest, DurabilityError> {
    membership.validate()?;
    let mut encoded = Vec::new();
    encode_membership_value(&mut encoded, membership)?;
    Ok(sha256(&encoded))
}

pub(crate) fn evidence_voter(evidence: &ReplicationPeerEvidence) -> ReplicaId {
    match evidence {
        ReplicationPeerEvidence::TermPromise(value) => value.voter,
        ReplicationPeerEvidence::LeaderVote(value) => value.voter,
        ReplicationPeerEvidence::EffectVote(value) => value.voter,
        ReplicationPeerEvidence::MembershipVote(value) => value.voter,
        ReplicationPeerEvidence::DecisionVote(value) => value.voter,
        ReplicationPeerEvidence::JointMembershipAck(value) => value.voter,
        ReplicationPeerEvidence::RecoveryAck(value) => value.voter,
    }
}

fn encode_peer_evidence(
    evidence: &ReplicationPeerEvidence,
) -> Result<(u8, Vec<u8>), DurabilityError> {
    let encoded = match evidence {
        ReplicationPeerEvidence::TermPromise(value) => {
            (EVIDENCE_TERM_PROMISE, encode_term_promise(value))
        }
        ReplicationPeerEvidence::LeaderVote(value) => {
            (EVIDENCE_LEADER_VOTE, encode_leader_vote(value))
        }
        ReplicationPeerEvidence::EffectVote(value) => {
            (EVIDENCE_EFFECT_VOTE, encode_effect_vote(value))
        }
        ReplicationPeerEvidence::MembershipVote(value) => {
            (EVIDENCE_MEMBERSHIP_VOTE, encode_membership_vote(value)?)
        }
        ReplicationPeerEvidence::DecisionVote(value) => {
            (EVIDENCE_DECISION_VOTE, encode_decision_vote(value))
        }
        ReplicationPeerEvidence::JointMembershipAck(value) => {
            let mut out = Vec::new();
            push_u64(&mut out, value.voter.raw());
            push_u64(&mut out, value.previous_membership_epoch);
            push_u64(&mut out, value.next_membership_epoch);
            push_u64(&mut out, value.term);
            push_u64(&mut out, value.leader.raw());
            out.extend_from_slice(&value.next_membership_digest.0);
            (EVIDENCE_JOINT_MEMBERSHIP_ACK, out)
        }
        ReplicationPeerEvidence::RecoveryAck(value) => {
            validate_lock_summaries(&value.locks)?;
            let mut out = Vec::new();
            push_u64(&mut out, value.voter.raw());
            push_u64(&mut out, value.membership_epoch);
            push_u64(&mut out, value.recovery_term);
            push_u64(&mut out, value.leader.raw());
            encode_lock_summaries(&mut out, &value.locks)?;
            (EVIDENCE_RECOVERY_ACK, out)
        }
    };
    Ok(encoded)
}

fn decode_peer_evidence(tag: u8, bytes: &[u8]) -> Result<ReplicationPeerEvidence, &'static str> {
    match tag {
        EVIDENCE_TERM_PROMISE => {
            decode_term_promise(bytes).map(ReplicationPeerEvidence::TermPromise)
        }
        EVIDENCE_LEADER_VOTE => decode_leader_vote(bytes).map(ReplicationPeerEvidence::LeaderVote),
        EVIDENCE_EFFECT_VOTE => decode_effect_vote(bytes).map(ReplicationPeerEvidence::EffectVote),
        EVIDENCE_MEMBERSHIP_VOTE => {
            decode_membership_vote(bytes).map(ReplicationPeerEvidence::MembershipVote)
        }
        EVIDENCE_DECISION_VOTE => {
            decode_decision_vote(bytes).map(ReplicationPeerEvidence::DecisionVote)
        }
        EVIDENCE_JOINT_MEMBERSHIP_ACK => {
            let mut cursor = Cursor::new(bytes);
            let voter = ReplicaId::new(cursor.u64()?);
            let previous_membership_epoch = cursor.u64()?;
            let next_membership_epoch = cursor.u64()?;
            let term = cursor.u64()?;
            let leader = ReplicaId::new(cursor.u64()?);
            let next_membership_digest = Sha256Digest(
                cursor
                    .take(32)?
                    .try_into()
                    .map_err(|_| "joint membership digest decode")?,
            );
            cursor.finish()?;
            Ok(ReplicationPeerEvidence::JointMembershipAck(
                ReplicationJointMembershipAck {
                    voter,
                    previous_membership_epoch,
                    next_membership_epoch,
                    term,
                    leader,
                    next_membership_digest,
                },
            ))
        }
        EVIDENCE_RECOVERY_ACK => {
            let mut cursor = Cursor::new(bytes);
            let value = ReplicationRecoveryAck {
                voter: ReplicaId::new(cursor.u64()?),
                membership_epoch: cursor.u64()?,
                recovery_term: cursor.u64()?,
                leader: ReplicaId::new(cursor.u64()?),
                locks: decode_lock_summaries(&mut cursor)?,
            };
            cursor.finish()?;
            Ok(ReplicationPeerEvidence::RecoveryAck(value))
        }
        _ => Err("unknown replication peer evidence kind"),
    }
}

pub fn replication_peer_evidence_signing_message(
    cluster: ReplicationClusterId,
    trust_epoch: u64,
    signer: KeyId,
    evidence: &ReplicationPeerEvidence,
) -> Result<Vec<u8>, DurabilityError> {
    if trust_epoch == 0 {
        return Err(protocol("replication peer evidence trust epoch is zero"));
    }
    let voter = evidence_voter(evidence);
    let (kind, payload) = encode_peer_evidence(evidence)?;
    let payload_digest = sha256(&payload);
    let mut message = Vec::with_capacity(PEER_EVIDENCE_DOMAIN.len() + 32 + 8 + 32 + 8 + 1 + 32);
    message.extend_from_slice(PEER_EVIDENCE_DOMAIN);
    message.extend_from_slice(&cluster.0);
    message.extend_from_slice(&trust_epoch.to_le_bytes());
    message.extend_from_slice(&signer.0);
    message.extend_from_slice(&voter.raw().to_le_bytes());
    message.push(kind);
    message.extend_from_slice(&payload_digest.0);
    Ok(message)
}

pub(crate) fn encode_signed_peer_evidence(
    signed: &SignedReplicationPeerEvidence,
) -> Result<Vec<u8>, DurabilityError> {
    let (kind, payload) = encode_peer_evidence(&signed.evidence)?;
    let mut out = Vec::new();
    push_u64(&mut out, signed.trust_epoch);
    out.extend_from_slice(&signed.signer.0);
    out.extend_from_slice(&signed.signature);
    out.push(kind);
    push_len(&mut out, payload.len())?;
    out.extend_from_slice(&payload);
    Ok(out)
}

pub(crate) fn decode_signed_peer_evidence(
    bytes: &[u8],
) -> Result<SignedReplicationPeerEvidence, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let trust_epoch = cursor.u64()?;
    let signer = KeyId(
        cursor
            .take(32)?
            .try_into()
            .map_err(|_| "peer evidence signer decode")?,
    );
    let signature = cursor
        .take(64)?
        .try_into()
        .map_err(|_| "peer evidence signature decode")?;
    let kind = cursor.u8()?;
    let payload_len = cursor.len()?;
    let payload = cursor.take(payload_len)?;
    let evidence = decode_peer_evidence(kind, payload)?;
    cursor.finish()?;
    Ok(SignedReplicationPeerEvidence {
        trust_epoch,
        signer,
        evidence,
        signature,
    })
}

fn authentication_receipt(
    signed: &SignedReplicationPeerEvidence,
) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
    let (_, payload) = encode_peer_evidence(&signed.evidence)?;
    let encoded = encode_signed_peer_evidence(signed)?;
    Ok(ReplicationAuthenticationReceipt {
        proof_digest: sha256(&encoded),
        payload_digest: sha256(&payload),
        trust_epoch: signed.trust_epoch,
        voter: evidence_voter(&signed.evidence),
        signer: signed.signer,
    })
}

fn encode_peer_auth_policy(policy: &ReplicationPeerAuthPolicy) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.extend_from_slice(&policy.cluster.0);
    push_u64(&mut out, policy.trust_epoch);
    push_len(&mut out, policy.peer_keys.len())?;
    for (replica, key) in &policy.peer_keys {
        push_u64(&mut out, replica.raw());
        out.extend_from_slice(&key.0);
    }
    Ok(out)
}

fn decode_peer_auth_policy(bytes: &[u8]) -> Result<ReplicationPeerAuthPolicy, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let cluster = ReplicationClusterId(
        cursor
            .take(32)?
            .try_into()
            .map_err(|_| "replication cluster id decode")?,
    );
    let trust_epoch = cursor.u64()?;
    let count = cursor.len()?;
    let mut peer_keys = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        let replica = ReplicaId::new(cursor.u64()?);
        if previous.is_some_and(|prior| prior >= replica) {
            return Err("replication peer auth policy is not sorted and unique");
        }
        previous = Some(replica);
        let key = KeyId(
            cursor
                .take(32)?
                .try_into()
                .map_err(|_| "replication peer key id decode")?,
        );
        peer_keys.insert(replica, key);
    }
    cursor.finish()?;
    Ok(ReplicationPeerAuthPolicy {
        cluster,
        trust_epoch,
        peer_keys,
    })
}

fn encode_quorum_loss(loss: ReplicationQuorumLoss) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, loss.membership_epoch);
    push_u64(&mut out, loss.observed_term);
    out
}

fn decode_quorum_loss(bytes: &[u8]) -> Result<ReplicationQuorumLoss, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let loss = ReplicationQuorumLoss {
        membership_epoch: cursor.u64()?,
        observed_term: cursor.u64()?,
    };
    cursor.finish()?;
    Ok(loss)
}

fn encode_recovery_certificate(
    certificate: &ReplicationRecoveryCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, certificate.membership_epoch);
    push_u64(&mut out, certificate.recovery_term);
    push_u64(&mut out, certificate.leader.raw());
    push_len(&mut out, certificate.acknowledged_by.len())?;
    for voter in &certificate.acknowledged_by {
        push_u64(&mut out, voter.raw());
    }
    encode_lock_summaries(&mut out, &certificate.reconciled_locks)?;
    Ok(out)
}

fn decode_recovery_certificate(
    bytes: &[u8],
) -> Result<ReplicationRecoveryCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let membership_epoch = cursor.u64()?;
    let recovery_term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication recovery acknowledgements are not sorted and unique",
    )?;
    let reconciled_locks = decode_lock_summaries(&mut cursor)?;
    cursor.finish()?;
    Ok(ReplicationRecoveryCertificate {
        membership_epoch,
        recovery_term,
        leader,
        acknowledged_by,
        reconciled_locks,
    })
}

fn validate_acknowledgements(
    membership: &ReplicationMembership,
    acknowledged_by: &BTreeSet<ReplicaId>,
    insufficient_reason: &'static str,
) -> Result<(), DurabilityError> {
    if !acknowledged_by.is_subset(&membership.members) {
        return Err(protocol(
            "replication acknowledgement references a non-member replica",
        ));
    }
    if acknowledged_by.len() < membership.quorum_size {
        return Err(protocol(insufficient_reason));
    }
    Ok(())
}

#[must_use]
pub const fn replicated_effect_id(origin: ReplicaId, sequence: u64) -> RevisionEffectId {
    RevisionEffectId(((origin.raw() as u128) << 64) | sequence as u128)
}

#[must_use]
pub const fn replicated_origin(id: RevisionEffectId) -> Option<ReplicaId> {
    let raw = (id.0 >> 64) as u64;
    if raw == 0 {
        None
    } else {
        Some(ReplicaId::new(raw))
    }
}

fn encode_ingest(envelope: &ReplicatedEffectEnvelope) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, envelope.origin.raw());
    push_u64(&mut out, envelope.origin_sequence);
    push_u128(&mut out, envelope.branch.raw());
    push_u64(&mut out, envelope.ordered_by.sequencer.raw());
    push_u64(&mut out, envelope.ordered_by.epoch);
    push_u64(&mut out, envelope.ordered_by.position);
    encode_effect(&mut out, &envelope.effect)?;
    Ok(out)
}

fn decode_ingest(bytes: &[u8]) -> Result<ReplicatedEffectEnvelope, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let origin = ReplicaId::new(cursor.u64()?);
    let origin_sequence = cursor.u64()?;
    let branch = ReplicationBranchId::new(cursor.u128()?);
    let ordered_by = DurableSequencerOrder {
        sequencer: ReplicaId::new(cursor.u64()?),
        epoch: cursor.u64()?,
        position: cursor.u64()?,
    };
    let effect = decode_effect(&mut cursor)?;
    cursor.finish()?;
    Ok(ReplicatedEffectEnvelope {
        origin,
        origin_sequence,
        branch,
        effect,
        ordered_by,
    })
}

fn encode_effect(
    out: &mut Vec<u8>,
    effect: &DurableRevisionEffectRecord,
) -> Result<(), CodecError> {
    push_u128(out, effect.id.0);
    push_len(out, effect.prerequisites.len())?;
    for prerequisite in &effect.prerequisites {
        push_u128(out, prerequisite.0);
    }
    push_u64(out, effect.transaction_epoch.raw());
    push_u128(out, effect.transaction_id.raw());
    metadata::encode_transaction_intent(out, &effect.intent)?;
    push_u64(out, effect.source_revision.raw());
    push_u64(out, effect.target_revision.raw());
    Ok(())
}

fn decode_effect(cursor: &mut Cursor<'_>) -> Result<DurableRevisionEffectRecord, &'static str> {
    let id = RevisionEffectId(cursor.u128()?);
    let count = cursor.len()?;
    let mut prerequisites = BTreeSet::new();
    let mut previous = None;
    for _ in 0..count {
        let prerequisite = RevisionEffectId(cursor.u128()?);
        if previous.is_some_and(|prior| prior >= prerequisite) {
            return Err("replicated prerequisites are not strictly sorted and unique");
        }
        previous = Some(prerequisite);
        prerequisites.insert(prerequisite);
    }
    let transaction_epoch = super::IdempotencyEpoch::new(cursor.u64()?);
    let transaction_id = kernel_types::ClientTransactionId::new(cursor.u128()?);
    let intent = metadata::decode_transaction_intent(cursor)?;
    let source_revision = RevisionId::new(cursor.u64()?);
    let target_revision = RevisionId::new(cursor.u64()?);
    let effect = DurableRevisionEffectRecord {
        id,
        prerequisites,
        transaction_epoch,
        transaction_id,
        intent,
        source_revision,
        target_revision,
    };
    effect.validate_identity()?;
    Ok(effect)
}

fn decode_retire(bytes: &[u8]) -> Result<(ReplicationBranchId, RevisionEffectId), &'static str> {
    let mut cursor = Cursor::new(bytes);
    let branch = ReplicationBranchId::new(cursor.u128()?);
    let effect = RevisionEffectId(cursor.u128()?);
    cursor.finish()?;
    Ok((branch, effect))
}

fn encode_membership_change(change: &ReplicationMembershipChange) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, change.next.epoch);
    push_len(&mut out, change.next.members.len())?;
    for member in &change.next.members {
        push_u64(&mut out, member.raw());
    }
    push_len(&mut out, change.next.quorum_size)?;
    push_len(&mut out, change.acknowledged_by_previous.len())?;
    for member in &change.acknowledged_by_previous {
        push_u64(&mut out, member.raw());
    }
    Ok(out)
}

fn decode_membership_change(bytes: &[u8]) -> Result<ReplicationMembershipChange, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let epoch = cursor.u64()?;
    let members = decode_replica_set(
        &mut cursor,
        "replication membership is not sorted and unique",
    )?;
    let quorum_size = cursor.len()?;
    let acknowledged_by_previous = decode_replica_set(
        &mut cursor,
        "replication membership acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationMembershipChange {
        next: ReplicationMembership {
            epoch,
            members,
            quorum_size,
        },
        acknowledged_by_previous,
    })
}

fn encode_quorum_certificate(
    certificate: &ReplicationQuorumCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u128(&mut out, certificate.effect.0);
    push_u64(&mut out, certificate.membership_epoch);
    push_len(&mut out, certificate.acknowledged_by.len())?;
    for member in &certificate.acknowledged_by {
        push_u64(&mut out, member.raw());
    }
    Ok(out)
}

fn decode_quorum_certificate(bytes: &[u8]) -> Result<ReplicationQuorumCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let effect = RevisionEffectId(cursor.u128()?);
    let membership_epoch = cursor.u64()?;
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication quorum acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationQuorumCertificate {
        effect,
        membership_epoch,
        acknowledged_by,
    })
}

fn decode_publish(bytes: &[u8]) -> Result<RevisionEffectId, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let effect = RevisionEffectId(cursor.u128()?);
    cursor.finish()?;
    Ok(effect)
}

fn decode_replica_set(
    cursor: &mut Cursor<'_>,
    unsorted_reason: &'static str,
) -> Result<BTreeSet<ReplicaId>, &'static str> {
    let count = cursor.len()?;
    let mut replicas = BTreeSet::new();
    let mut previous = None;
    for _ in 0..count {
        let replica = ReplicaId::new(cursor.u64()?);
        if previous.is_some_and(|prior| prior >= replica) {
            return Err(unsorted_reason);
        }
        previous = Some(replica);
        replicas.insert(replica);
    }
    Ok(replicas)
}

fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

fn corruption(offset: usize, reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset, reason }
}
