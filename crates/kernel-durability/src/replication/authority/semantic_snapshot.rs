use std::collections::{BTreeMap, BTreeSet};

use kernel_auth::{KeyId, Sha256Digest};
use kernel_change::RevisionEffectId;
use kernel_types::RevisionId;

use super::{AuthenticatedRecoveryAckIndex, MembershipVoteIndex, ReplicationAuthorityJournal};
use crate::replication::{
    DurableSequencerOrder, ReplicaId, ReplicatedEffectEnvelope, ReplicationAuthenticationReceipt,
    ReplicationBranchHead, ReplicationBranchId, ReplicationDecisionLock, ReplicationDecisionVote,
    ReplicationJointMembershipAck, ReplicationJointMembershipCertificate,
    ReplicationLeaderCertificate, ReplicationLockSummary, ReplicationMembership,
    ReplicationPeerAuthPolicy, ReplicationQuorumAvailability, ReplicationQuorumCertificate,
};

/// Exact semantic state produced by replaying replication authority.
///
/// Physical journal ownership, pending/live frame buffers and poison state are deliberately
/// excluded: they are persistence mechanics, not authority semantics.  Keeping this projection
/// explicit is the first half of semantic compaction: any future compact wire image must restore
/// exactly this state, rather than retaining historical frames merely because they happened to be
/// the path by which the state was reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReplicationAuthoritySemanticSnapshot {
    effects: BTreeMap<RevisionEffectId, ReplicatedEffectEnvelope>,
    branches: BTreeMap<ReplicationBranchId, ReplicationBranchHead>,
    revision_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    ordered_slots: BTreeMap<DurableSequencerOrder, RevisionEffectId>,
    sequencer_epochs: BTreeMap<ReplicaId, u64>,
    memberships: BTreeMap<u64, ReplicationMembership>,
    current_membership_epoch: Option<u64>,
    quorum_certificates: BTreeMap<RevisionEffectId, ReplicationQuorumCertificate>,
    effect_votes: BTreeMap<(u64, u64, ReplicaId), RevisionEffectId>,
    membership_votes: BTreeMap<(u64, ReplicaId), MembershipVoteIndex>,
    membership_vote_successors: BTreeMap<Sha256Digest, ReplicationMembership>,
    promised_terms: BTreeMap<(u64, ReplicaId), u64>,
    highest_promised_terms: BTreeMap<u64, u64>,
    leader_votes: BTreeMap<(u64, u64, ReplicaId), ReplicaId>,
    leader_certificates: BTreeMap<(u64, u64), ReplicationLeaderCertificate>,
    decision_votes: BTreeMap<(u64, u64, u64, ReplicaId), ReplicationDecisionVote>,
    decision_locks: BTreeMap<u64, ReplicationDecisionLock>,
    highest_decision_lock_term: u64,
    joint_membership_certificates: BTreeMap<u64, ReplicationJointMembershipCertificate>,
    peer_auth_policy: Option<ReplicationPeerAuthPolicy>,
    authenticated_evidence: BTreeMap<Sha256Digest, ReplicationAuthenticationReceipt>,
    authenticated_evidence_index: BTreeMap<(u64, ReplicaId, Sha256Digest), KeyId>,
    joint_membership_acks: BTreeMap<(u64, u64, ReplicaId), ReplicationJointMembershipAck>,
    recovery_acks: BTreeMap<(u64, u64, ReplicaId), AuthenticatedRecoveryAckIndex>,
    recovery_lock_frontiers: BTreeMap<Sha256Digest, Vec<ReplicationLockSummary>>,
    quorum_availability: Option<ReplicationQuorumAvailability>,
    published_effects: BTreeSet<RevisionEffectId>,
    published_branches: BTreeMap<ReplicationBranchId, ReplicationBranchHead>,
}

impl ReplicationAuthoritySemanticSnapshot {
    pub(super) fn capture(journal: &ReplicationAuthorityJournal) -> Self {
        // Deliberately exhaustive: adding authority state to the journal must make this model stop
        // compiling until the semantic projection is updated. Physical-only fields are named and
        // discarded explicitly rather than hidden behind `..`.
        let ReplicationAuthorityJournal {
            path: _,
            file: _,
            single_file_capture: _,
            pending_single_file_frames: _,
            live_single_file_frames: _,
            effects,
            branches,
            revision_frontiers,
            ordered_slots,
            sequencer_epochs,
            memberships,
            current_membership_epoch,
            quorum_certificates,
            effect_votes,
            membership_votes,
            membership_vote_successors,
            promised_terms,
            highest_promised_terms,
            leader_votes,
            leader_certificates,
            decision_votes,
            decision_locks,
            highest_decision_lock_term,
            joint_membership_certificates,
            peer_auth_policy,
            authenticated_evidence,
            authenticated_evidence_index,
            joint_membership_acks,
            recovery_acks,
            recovery_lock_frontiers,
            quorum_availability,
            published_effects,
            published_branches,
            poisoned: _,
        } = journal;
        Self {
            effects: effects.clone(),
            branches: branches.clone(),
            revision_frontiers: revision_frontiers.clone(),
            ordered_slots: ordered_slots.clone(),
            sequencer_epochs: sequencer_epochs.clone(),
            memberships: memberships.clone(),
            current_membership_epoch: *current_membership_epoch,
            quorum_certificates: quorum_certificates.clone(),
            effect_votes: effect_votes.clone(),
            membership_votes: membership_votes.clone(),
            membership_vote_successors: membership_vote_successors.clone(),
            promised_terms: promised_terms.clone(),
            highest_promised_terms: highest_promised_terms.clone(),
            leader_votes: leader_votes.clone(),
            leader_certificates: leader_certificates.clone(),
            decision_votes: decision_votes.clone(),
            decision_locks: decision_locks.clone(),
            highest_decision_lock_term: *highest_decision_lock_term,
            joint_membership_certificates: joint_membership_certificates.clone(),
            peer_auth_policy: peer_auth_policy.clone(),
            authenticated_evidence: authenticated_evidence.clone(),
            authenticated_evidence_index: authenticated_evidence_index.clone(),
            joint_membership_acks: joint_membership_acks.clone(),
            recovery_acks: recovery_acks.clone(),
            recovery_lock_frontiers: recovery_lock_frontiers.clone(),
            quorum_availability: *quorum_availability,
            published_effects: published_effects.clone(),
            published_branches: published_branches.clone(),
        }
    }

    pub(super) fn restore(self, journal: &mut ReplicationAuthorityJournal) {
        journal.effects = self.effects;
        journal.branches = self.branches;
        journal.revision_frontiers = self.revision_frontiers;
        journal.ordered_slots = self.ordered_slots;
        journal.sequencer_epochs = self.sequencer_epochs;
        journal.memberships = self.memberships;
        journal.current_membership_epoch = self.current_membership_epoch;
        journal.quorum_certificates = self.quorum_certificates;
        journal.effect_votes = self.effect_votes;
        journal.membership_votes = self.membership_votes;
        journal.membership_vote_successors = self.membership_vote_successors;
        journal.promised_terms = self.promised_terms;
        journal.highest_promised_terms = self.highest_promised_terms;
        journal.leader_votes = self.leader_votes;
        journal.leader_certificates = self.leader_certificates;
        journal.decision_votes = self.decision_votes;
        journal.decision_locks = self.decision_locks;
        journal.highest_decision_lock_term = self.highest_decision_lock_term;
        journal.joint_membership_certificates = self.joint_membership_certificates;
        journal.peer_auth_policy = self.peer_auth_policy;
        journal.authenticated_evidence = self.authenticated_evidence;
        journal.authenticated_evidence_index = self.authenticated_evidence_index;
        journal.joint_membership_acks = self.joint_membership_acks;
        journal.recovery_acks = self.recovery_acks;
        journal.recovery_lock_frontiers = self.recovery_lock_frontiers;
        journal.quorum_availability = self.quorum_availability;
        journal.published_effects = self.published_effects;
        journal.published_branches = self.published_branches;
    }

    /// Returns history-bearing semantic cardinalities which cannot be assumed bounded merely
    /// because the append journal is compacted.  In particular, replicated effects remain part of
    /// the causal API and therefore establish a lower bound for any exact semantic snapshot.
    pub(super) fn retained_history_shape(&self) -> (usize, usize, usize, usize) {
        (
            self.effects.len(),
            self.revision_frontiers.len(),
            self.decision_locks.len(),
            self.memberships.len(),
        )
    }
}

impl ReplicationAuthorityJournal {
    pub(crate) fn test_semantic_snapshot_roundtrip(
        &self,
    ) -> Result<(), crate::runtime::DurabilityError> {
        let snapshot = ReplicationAuthoritySemanticSnapshot::capture(self);
        let expected = snapshot.clone();
        let mut restored = Self::open_single_file(self.path(), &[], &[])?;
        snapshot.restore(&mut restored);
        assert_eq!(
            expected,
            ReplicationAuthoritySemanticSnapshot::capture(&restored)
        );
        Ok(())
    }

    pub(crate) fn test_semantic_snapshot_retained_history_shape(
        &self,
    ) -> (usize, usize, usize, usize) {
        ReplicationAuthoritySemanticSnapshot::capture(self).retained_history_shape()
    }
}
