use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom};
use std::path::Path;

use kernel_auth::Sha256Digest;
use kernel_change::RevisionEffectId;
use kernel_types::RevisionId;

use super::ReplicationAuthorityJournal;
use crate::domain::DurableRevisionEffectRecord;
use crate::replication::{
    ReplicaId, ReplicatedEffectEnvelope, ReplicationAuthenticationReceipt, ReplicationBranchHead,
    ReplicationBranchId, ReplicationDecisionLock, ReplicationEffectStage,
    ReplicationLeaderCertificate, ReplicationLockSummary, ReplicationMembership,
    ReplicationPeerAuthPolicy, ReplicationQuorumAvailability,
};
use crate::runtime::DurabilityError;

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
            membership_vote_successors: BTreeMap::new(),
            promised_terms: BTreeMap::new(),
            highest_promised_terms: BTreeMap::new(),
            leader_votes: BTreeMap::new(),
            leader_certificates: BTreeMap::new(),
            decision_votes: BTreeMap::new(),
            decision_locks: BTreeMap::new(),
            highest_decision_lock_term: 0,
            joint_membership_certificates: BTreeMap::new(),
            peer_auth_policy: None,
            authenticated_evidence: BTreeMap::new(),
            authenticated_evidence_index: BTreeMap::new(),
            joint_membership_acks: BTreeMap::new(),
            recovery_acks: BTreeMap::new(),
            recovery_lock_frontiers: BTreeMap::new(),
            quorum_availability: None,
            published_effects: BTreeSet::new(),
            published_branches: BTreeMap::new(),
            poisoned: false,
        };
        let (last_good, original_len) = journal.replay_file()?;
        if last_good < original_len {
            journal.file.set_len(last_good)?;
            journal.file.sync_all()?;
        }
        journal.file.seek(SeekFrom::End(0))?;
        Ok(journal)
    }

    #[must_use]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn recovery_ack_storage_shape(&self) -> (usize, usize) {
        (self.recovery_acks.len(), self.recovery_lock_frontiers.len())
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn membership_vote_storage_shape(&self) -> (usize, usize) {
        (
            self.membership_votes.len(),
            self.membership_vote_successors.len(),
        )
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

    pub(crate) fn decision_lock_summary_chunk(
        &self,
        from_position: u64,
        limit: usize,
    ) -> (Vec<ReplicationLockSummary>, bool) {
        let mut matching =
            self.decision_locks
                .range(from_position..)
                .map(|(_, lock)| ReplicationLockSummary {
                    position: lock.position,
                    term: lock.term,
                    effect: lock.effect,
                });
        let locks = matching.by_ref().take(limit).collect();
        let complete = matching.next().is_none();
        (locks, complete)
    }

    #[must_use]
    pub(crate) fn current_consensus_term(&self) -> u64 {
        let availability_term = match self.quorum_availability {
            Some(ReplicationQuorumAvailability::Available { term, .. }) => term,
            Some(ReplicationQuorumAvailability::Lost(loss)) => loss.observed_term,
            None => 0,
        };
        let promised_term = self
            .highest_promised_terms
            .values()
            .copied()
            .max()
            .unwrap_or(0);
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
}
