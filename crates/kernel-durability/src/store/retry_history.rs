use kernel_types::ClientTransactionId;

use super::DurableRevisionStore;
use crate::domain::{DurableCommittedTransaction, DurableTransactionKey, IdempotencyEpoch};
use crate::runtime::{DurabilityError, DurableTransactionOutcome};

impl DurableRevisionStore {
    #[must_use]
    pub fn transaction_outcome(
        &self,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        self.transaction_outcome_at(self.current_idempotency_epoch, transaction_id)
    }

    #[must_use]
    pub fn transaction_outcome_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        if epoch < self.minimum_retry_epoch {
            return DurableTransactionOutcome::RetryHistoryExpired;
        }
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
            .map_or(DurableTransactionOutcome::Unknown, |intent| {
                DurableTransactionOutcome::Committed {
                    target_revision: intent.target_revision(),
                }
            })
    }

    #[must_use]
    pub fn transaction_intent(
        &self,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableCommittedTransaction> {
        self.transaction_intent_at(self.current_idempotency_epoch, transaction_id)
    }

    #[must_use]
    pub fn transaction_intent_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableCommittedTransaction> {
        if epoch < self.minimum_retry_epoch {
            return None;
        }
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
    }

    #[must_use]
    pub const fn current_idempotency_epoch(&self) -> IdempotencyEpoch {
        self.current_idempotency_epoch
    }

    #[must_use]
    pub const fn minimum_retry_epoch(&self) -> IdempotencyEpoch {
        self.minimum_retry_epoch
    }

    pub fn advance_idempotency_epoch(
        &mut self,
        next: IdempotencyEpoch,
    ) -> Result<(), DurabilityError> {
        if next <= self.current_idempotency_epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "idempotency epoch must advance monotonically",
            });
        }
        self.current_idempotency_epoch = next;
        Ok(())
    }

    pub fn expire_retry_history_before(
        &mut self,
        minimum: IdempotencyEpoch,
    ) -> Result<usize, DurabilityError> {
        if minimum < self.minimum_retry_epoch || minimum > self.current_idempotency_epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "retry-history watermark is outside the active epoch range",
            });
        }
        let before = self.committed_transactions.len();
        self.committed_transactions
            .retain(|key, _| key.epoch >= minimum);
        self.minimum_retry_epoch = minimum;
        Ok(before - self.committed_transactions.len())
    }
}
