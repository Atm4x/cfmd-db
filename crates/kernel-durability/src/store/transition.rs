use std::collections::BTreeSet;

use kernel_semantics::SemanticRegistry;
use kernel_types::{RevisionId, SchemaRevisionId};

use super::DurableRevisionStore;
use super::causal_ledger::allocate_local_revision_effect_id;
use super::commit_authority::PreparedCommitAuthority;
use super::prepare_validation::validate_bound_prepare_intent;
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::DurableTransactionKey;
use crate::runtime::{
    DurabilityError, DurableCommitReceipt, DurablePrepareToken, RevisionDurability,
};

pub(super) enum DurableGroupCommitExecution {
    Rejected(DurabilityError),
    RecoveryRequired(DurabilityError),
    Committed(Vec<DurableCommitReceipt>),
    CommittedFreshnessUnconfirmed {
        receipts: Vec<DurableCommitReceipt>,
        error: DurabilityError,
    },
}

impl DurableGroupCommitExecution {
    fn into_public_result(self) -> Result<Vec<DurableCommitReceipt>, DurabilityError> {
        match self {
            Self::Committed(receipts) => Ok(receipts),
            Self::Rejected(error)
            | Self::RecoveryRequired(error)
            | Self::CommittedFreshnessUnconfirmed { error, .. } => Err(error),
        }
    }
}

fn validate_prepare_identity(
    store: &DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
    expected_source: RevisionId,
) -> Result<(), DurabilityError> {
    if store.poisoned {
        return Err(DurabilityError::Poisoned);
    }
    let key = DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
    if descriptor.idempotency_epoch < store.minimum_retry_epoch {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction retry epoch has expired",
        });
    }
    if descriptor.idempotency_epoch != store.current_idempotency_epoch {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction retry epoch does not match durable store epoch",
        });
    }
    if let Some(existing_intent) = store.committed_transactions.get(&key) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: if existing_intent == &descriptor.intent {
                "transaction is already committed"
            } else {
                "transaction retry key already committed to another exact intent"
            },
        });
    }
    if store
        .prepared_transactions
        .descriptor_by_retry_key(key)
        .is_some()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction retry key already belongs to prepared descriptor",
        });
    }
    if store
        .prepared_transactions
        .descriptor_by_target_revision(descriptor.target_revision)
        .is_some()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "target revision already belongs to prepared descriptor",
        });
    }
    if descriptor.source_revision != expected_source {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "prepare source revision does not match durable store head",
        });
    }
    if store
        .revision_effect_frontiers
        .contains_key(&descriptor.target_revision)
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local target revision already belongs to local causal authority",
        });
    }
    if store
        .replication
        .revision_frontier(descriptor.target_revision)
        .is_some()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local target revision already belongs to replicated causal authority",
        });
    }
    Ok(())
}

fn bind_prepare_descriptor_at(
    store: &DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
    expected_source: RevisionId,
    next_effect_id: &mut u128,
) -> Result<DurableRevisionDescriptor, DurabilityError> {
    let mut descriptor = descriptor.clone();
    descriptor.idempotency_epoch = store.current_idempotency_epoch;
    validate_prepare_identity(store, &descriptor, expected_source)?;
    descriptor.revision_effect_id = Some(allocate_local_revision_effect_id(next_effect_id)?);
    Ok(descriptor)
}

fn bind_group_prepare_descriptor(
    store: &DurableRevisionStore,
    registry: &mut SemanticRegistry,
    descriptor: &DurableRevisionDescriptor,
    expected_source: RevisionId,
    expected_migration_source_schema: SchemaRevisionId,
    next_effect_id: &mut u128,
) -> Result<DurableRevisionDescriptor, DurabilityError> {
    let descriptor =
        bind_prepare_descriptor_at(store, descriptor, expected_source, next_effect_id)?;
    validate_bound_prepare_intent(
        store,
        registry,
        &descriptor,
        Some(expected_migration_source_schema),
    )?;
    Ok(descriptor)
}

impl RevisionDurability for DurableRevisionStore {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        let mut next_effect_id = self.next_revision_effect_id;
        let descriptor =
            bind_prepare_descriptor_at(self, descriptor, self.durable_head, &mut next_effect_id)?;
        let mut semantic_registry = self.semantic_registry.clone();
        validate_bound_prepare_intent(self, &mut semantic_registry, &descriptor, None)?;
        let (token, frame) = self
            .wal
            .append_prepare_unflushed_with_frame(&descriptor)
            .inspect_err(|_| self.poisoned = true)?;
        self.mirror_streaming_frame(&frame);
        self.wal
            .durability_barrier()
            .inspect_err(|_| self.poisoned = true)?;
        self.barrier_streaming_shadow();
        self.prepared_transactions
            .publish_prepare(token, descriptor.clone());
        self.next_revision_effect_id = next_effect_id;
        self.semantic_registry = semantic_registry;
        self.advance_external_freshness_wal()?;
        Ok(token)
    }

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let transaction_id = prepared.transaction_id();
        let descriptor = self
            .prepared_transactions
            .commit_descriptor_by_lsn(prepared.prepare_lsn())
            .cloned()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "commit token has no prepared descriptor in this store",
            })?;
        if descriptor.transaction_id != transaction_id
            || descriptor.target_revision != prepared.target_revision()
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "commit token identity does not match prepared descriptor",
            });
        }
        if descriptor.source_revision != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "commit prepared source no longer matches durable store head",
            });
        }
        let authority = PreparedCommitAuthority::prepare(self, std::slice::from_ref(&descriptor))?;
        let (receipt, frame) = self
            .wal
            .append_commit_unflushed_with_frame(prepared)
            .inspect_err(|_| self.poisoned = true)?;
        self.mirror_streaming_frame(&frame);
        self.wal
            .durability_barrier()
            .inspect_err(|_| self.poisoned = true)?;
        self.barrier_streaming_shadow();
        authority.publish(self);
        self.prepared_transactions
            .retire_committed(prepared.prepare_lsn());
        self.advance_external_freshness_wal()?;
        Ok(receipt)
    }
}

impl DurableRevisionStore {
    fn bind_group_descriptors(
        &self,
        descriptors: &[DurableRevisionDescriptor],
    ) -> Result<(Vec<DurableRevisionDescriptor>, SemanticRegistry, u128), DurabilityError> {
        let mut expected_source = self.durable_head;
        let mut expected_migration_source_schema = self
            .migration_complements
            .last()
            .map_or(self.checkpoint.semantic_revision().schema, |previous| {
                previous.target_schema
            });
        let mut transaction_ids = BTreeSet::new();
        let mut provisional_local_targets = self
            .revision_effect_frontiers
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        let mut next_effect_id = self.next_revision_effect_id;
        let mut semantic_registry = self.semantic_registry.clone();
        let mut bound = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors {
            if descriptor.source_revision != expected_source {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "group commit descriptors are not a contiguous revision chain",
                });
            }
            if !transaction_ids.insert(descriptor.transaction_id) {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "group commit repeats a transaction id",
                });
            }
            let descriptor = bind_group_prepare_descriptor(
                self,
                &mut semantic_registry,
                descriptor,
                expected_source,
                expected_migration_source_schema,
                &mut next_effect_id,
            )?;
            if !provisional_local_targets.insert(descriptor.target_revision) {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "local target revision already belongs to local causal authority",
                });
            }
            if let super::DurableTransactionIntent::SchemaMigrationExact {
                migration_complement,
                ..
            } = &descriptor.intent
            {
                expected_migration_source_schema = migration_complement.target_schema;
            }
            expected_source = descriptor.target_revision;
            bound.push(descriptor);
        }
        Ok((bound, semantic_registry, next_effect_id))
    }

    /// Commits an ordered contiguous descriptor chain with one PREPARE
    /// durability barrier and one COMMIT durability barrier for the group.
    /// No receipt escapes before the final barrier succeeds.
    pub fn durably_commit_group(
        &mut self,
        descriptors: &[DurableRevisionDescriptor],
    ) -> Result<Vec<DurableCommitReceipt>, DurabilityError> {
        self.execute_durable_commit_group(descriptors)
            .into_public_result()
    }

    pub(super) fn execute_durable_commit_group(
        &mut self,
        descriptors: &[DurableRevisionDescriptor],
    ) -> DurableGroupCommitExecution {
        if self.poisoned {
            return DurableGroupCommitExecution::Rejected(DurabilityError::Poisoned);
        }
        if descriptors.is_empty() {
            return DurableGroupCommitExecution::Committed(Vec::new());
        }
        let (bound, semantic_registry, next_effect_id) =
            match self.bind_group_descriptors(descriptors) {
                Ok(bound) => bound,
                Err(error) => return DurableGroupCommitExecution::Rejected(error),
            };
        let authority = match PreparedCommitAuthority::prepare(self, &bound) {
            Ok(authority) => authority,
            Err(error) => return DurableGroupCommitExecution::Rejected(error),
        };
        let mut prepared = Vec::with_capacity(bound.len());
        for descriptor in &bound {
            let (token, frame) = match self.wal.append_prepare_unflushed_with_frame(descriptor) {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.poisoned = true;
                    return DurableGroupCommitExecution::RecoveryRequired(error);
                }
            };
            self.mirror_streaming_frame(&frame);
            prepared.push(token);
        }
        if let Err(error) = self.wal.durability_barrier() {
            self.poisoned = true;
            return DurableGroupCommitExecution::RecoveryRequired(error);
        }
        self.barrier_streaming_shadow();

        let mut receipts = Vec::with_capacity(prepared.len());
        for token in prepared {
            let (receipt, frame) = match self.wal.append_commit_unflushed_with_frame(token) {
                Ok(committed) => committed,
                Err(error) => {
                    self.poisoned = true;
                    return DurableGroupCommitExecution::RecoveryRequired(error);
                }
            };
            self.mirror_streaming_frame(&frame);
            receipts.push(receipt);
        }
        if let Err(error) = self.wal.durability_barrier() {
            self.poisoned = true;
            return DurableGroupCommitExecution::RecoveryRequired(error);
        }
        self.barrier_streaming_shadow();
        authority.publish(self);
        self.next_revision_effect_id = next_effect_id;
        self.semantic_registry = semantic_registry;
        match self.advance_external_freshness_wal() {
            Ok(()) => DurableGroupCommitExecution::Committed(receipts),
            Err(error) => {
                DurableGroupCommitExecution::CommittedFreshnessUnconfirmed { receipts, error }
            }
        }
    }
}
