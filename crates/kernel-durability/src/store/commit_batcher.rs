use std::collections::BTreeSet;

use kernel_types::ClientTransactionId;

use super::DurableRevisionStore;
use super::transition::DurableGroupCommitExecution;
use crate::descriptor::DurableRevisionDescriptor;
use crate::runtime::{DurabilityError, DurableCommitReceipt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableCommitBatchPolicy {
    max_descriptors: usize,
}

impl DurableCommitBatchPolicy {
    pub fn new(max_descriptors: usize) -> Result<Self, DurabilityError> {
        if max_descriptors == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durability batch size must be nonzero",
            });
        }
        Ok(Self { max_descriptors })
    }

    #[must_use]
    pub const fn max_descriptors(self) -> usize {
        self.max_descriptors
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableBatchEnqueueOutcome {
    Queued,
    FlushRequired,
}

/// Non-authoritative scheduler state. Enqueue never acknowledges durability;
/// receipts only exist after `flush` crosses the shared COMMIT barrier.
#[derive(Debug)]
pub struct DurableCommitBatcher {
    policy: DurableCommitBatchPolicy,
    pending: Vec<DurableRevisionDescriptor>,
    pending_transactions: BTreeSet<ClientTransactionId>,
}

impl DurableCommitBatcher {
    #[must_use]
    pub const fn new(policy: DurableCommitBatchPolicy) -> Self {
        Self {
            policy,
            pending: Vec::new(),
            pending_transactions: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn enqueue(
        &mut self,
        store: &DurableRevisionStore,
        descriptor: DurableRevisionDescriptor,
    ) -> Result<DurableBatchEnqueueOutcome, DurabilityError> {
        let expected_source = self
            .pending
            .last()
            .map_or(store.durable_head(), |tail| tail.target_revision);
        if descriptor.source_revision != expected_source {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "async durability batch is not a contiguous revision chain",
            });
        }
        if !self.pending_transactions.insert(descriptor.transaction_id) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "async durability batch repeats a transaction id",
            });
        }
        self.pending.push(descriptor);
        Ok(if self.pending.len() >= self.policy.max_descriptors {
            DurableBatchEnqueueOutcome::FlushRequired
        } else {
            DurableBatchEnqueueOutcome::Queued
        })
    }

    pub fn flush(
        &mut self,
        store: &mut DurableRevisionStore,
    ) -> Result<Vec<DurableCommitReceipt>, DurabilityError> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        match store.execute_durable_commit_group(&self.pending) {
            DurableGroupCommitExecution::Rejected(error) => Err(error),
            DurableGroupCommitExecution::RecoveryRequired(error) => {
                self.clear_pending();
                Err(error)
            }
            DurableGroupCommitExecution::Committed(receipts) => {
                self.clear_pending();
                Ok(receipts)
            }
            DurableGroupCommitExecution::CommittedFreshnessUnconfirmed { receipts, error } => {
                debug_assert_eq!(receipts.len(), self.pending.len());
                self.clear_pending();
                Err(error)
            }
        }
    }

    fn clear_pending(&mut self) {
        self.pending.clear();
        self.pending_transactions.clear();
    }
}
