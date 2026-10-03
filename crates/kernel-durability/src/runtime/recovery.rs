use std::collections::BTreeMap;

use kernel_types::{ClientTransactionId, RevisionId};

use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::{DurableCommittedTransaction, DurableTransactionKey, IdempotencyEpoch};

use super::protocol::DurableTransactionOutcome;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRevision {
    pub descriptor: DurableRevisionDescriptor,
    pub prepare_lsn: u64,
    pub prepare_payload_crc32c: u32,
    pub commit_lsn: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailStatus {
    Clean,
    Truncated { offset: usize },
    Garbage { offset: usize },
}

#[derive(Debug)]
pub(crate) struct RecoveredAuthorityState {
    pub committed_transactions: BTreeMap<DurableTransactionKey, DurableCommittedTransaction>,
    pub unresolved_prepares: Vec<(u64, DurableRevisionDescriptor, u32)>,
    pub replication_authority_frames: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryScan {
    base_revision: RevisionId,
    committed: Vec<CommittedRevision>,
    last_good_offset: usize,
    next_lsn: u64,
    tail_status: TailStatus,
    committed_transactions: BTreeMap<DurableTransactionKey, DurableCommittedTransaction>,
    unresolved_prepares: Vec<(u64, DurableRevisionDescriptor, u32)>,
    replication_authority_frames: Vec<Vec<u8>>,
}

impl RecoveryScan {
    pub(crate) fn recovered(
        base_revision: RevisionId,
        committed: Vec<CommittedRevision>,
        last_good_offset: usize,
        next_lsn: u64,
        tail_status: TailStatus,
        authority: RecoveredAuthorityState,
    ) -> Self {
        Self {
            base_revision,
            committed,
            last_good_offset,
            next_lsn,
            tail_status,
            committed_transactions: authority.committed_transactions,
            unresolved_prepares: authority.unresolved_prepares,
            replication_authority_frames: authority.replication_authority_frames,
        }
    }

    #[must_use]
    pub fn durable_revision(&self) -> RevisionId {
        self.committed
            .last()
            .map_or(self.base_revision, |revision| {
                revision.descriptor.target_revision
            })
    }

    #[must_use]
    pub const fn base_revision(&self) -> RevisionId {
        self.base_revision
    }

    #[must_use]
    pub fn committed(&self) -> &[CommittedRevision] {
        &self.committed
    }

    #[must_use]
    pub const fn last_good_offset(&self) -> usize {
        self.last_good_offset
    }

    #[must_use]
    pub const fn next_lsn(&self) -> u64 {
        self.next_lsn
    }

    #[must_use]
    pub const fn tail_status(&self) -> &TailStatus {
        &self.tail_status
    }

    #[must_use]
    pub fn transaction_outcome(
        &self,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        self.transaction_outcome_at(IdempotencyEpoch::ZERO, transaction_id)
    }

    #[must_use]
    pub fn transaction_intent(
        &self,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableCommittedTransaction> {
        self.transaction_intent_at(IdempotencyEpoch::ZERO, transaction_id)
    }

    #[must_use]
    pub fn transaction_outcome_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
            .map_or(DurableTransactionOutcome::Unknown, |intent| {
                DurableTransactionOutcome::Committed {
                    target_revision: intent.target_revision(),
                }
            })
    }

    #[must_use]
    pub const fn committed_transactions(
        &self,
    ) -> &BTreeMap<DurableTransactionKey, DurableCommittedTransaction> {
        &self.committed_transactions
    }

    pub(crate) fn unresolved_prepares(&self) -> &[(u64, DurableRevisionDescriptor, u32)] {
        &self.unresolved_prepares
    }

    pub(crate) fn replication_authority_frames(&self) -> &[Vec<u8>] {
        &self.replication_authority_frames
    }

    #[must_use]
    pub fn transaction_intent_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableCommittedTransaction> {
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
    }
}
