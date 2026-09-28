use std::collections::{BTreeMap, BTreeSet};

use kernel_auth::{KeyId, Sha256Digest};
use kernel_change::RevisionEffectId;
use kernel_types::RevisionId;

use crate::domain::DurableRevisionEffectRecord;
use crate::runtime::DurabilityError;

fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

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

pub(crate) mod authority;
pub(crate) mod codec;

pub use codec::{
    replicated_effect_id, replicated_origin, replication_membership_digest,
    replication_peer_evidence_signing_message,
};

#[cfg(test)]
mod tests;
