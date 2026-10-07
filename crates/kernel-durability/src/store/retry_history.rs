use kernel_types::ClientTransactionId;

use super::DurableRevisionStore;
use crate::domain::{
    DurableClientIntent, DurableCommittedTransaction, DurableTransactionKey, IdempotencyEpoch,
};
use crate::runtime::{DurabilityError, DurableTransactionOutcome};
use crate::wal_payload::IntentSealRecord;

impl DurableRevisionStore {
    /// Durably records that an exact client intent is already satisfied by the
    /// current semantic head without creating a synthetic revision.
    ///
    /// The WAL seal is retry authority only: it does not advance `durable_head`,
    /// allocate a revision effect, or enter semantic history.
    pub fn durably_seal_satisfied_client_intent(
        &mut self,
        transaction_id: ClientTransactionId,
        intent: DurableClientIntent,
    ) -> Result<super::DurableSatisfiedIntentSealOutcome, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let key = DurableTransactionKey::new(self.current_idempotency_epoch, transaction_id);
        if let Some(existing) = self.committed_transactions.get(&key) {
            if existing.intent == intent {
                return Ok(super::DurableSatisfiedIntentSealOutcome::AlreadySealed {
                    revision: existing.target_revision(),
                });
            }
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "transaction retry key already committed to another exact intent",
            });
        }
        if self
            .prepared_transactions
            .descriptor_by_retry_key(key)
            .is_some()
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "transaction retry key already belongs to prepared descriptor",
            });
        }

        let committed = DurableCommittedTransaction {
            target_revision: self.durable_head,
            intent,
        };
        let frame = self
            .wal
            .append_intent_seal_unflushed(&IntentSealRecord {
                key,
                committed: committed.clone(),
            })
            .inspect_err(|_| self.poisoned = true)?;
        self.mirror_streaming_frame(&frame);
        self.wal
            .durability_barrier()
            .inspect_err(|_| self.poisoned = true)?;
        self.barrier_streaming_shadow();
        self.committed_transactions.insert(key, committed);
        self.advance_external_freshness_wal()?;
        Ok(super::DurableSatisfiedIntentSealOutcome::Sealed {
            revision: self.durable_head,
        })
    }

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
