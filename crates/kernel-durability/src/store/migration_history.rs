use std::collections::{BTreeMap, BTreeSet};

use kernel_revision::Revision;
use kernel_types::SchemaRevisionId;

use super::publication_protocol::{NoStoreFault, StoreFaultHook};
use super::{DurableGenerationReceipt, DurableRevisionStore};
use crate::domain::{
    DurableMigrationComplement, HistoricalComplementError, LocalHistoricalComplementChain,
};
use crate::runtime::DurabilityError;

type MigrationComplementKey = (SchemaRevisionId, SchemaRevisionId);
pub(super) type MigrationComplementIndex = BTreeMap<MigrationComplementKey, usize>;

pub(super) fn migration_step_replay_compatible(
    durable: &DurableMigrationComplement,
    incoming: &DurableMigrationComplement,
) -> bool {
    durable.source_schema == incoming.source_schema
        && durable.target_schema == incoming.target_schema
        && durable.lens_spec == incoming.lens_spec
        && durable.semantic_pins == incoming.semantic_pins
        && durable.encoding_version == incoming.encoding_version
        && durable.retention == incoming.retention
        && (durable.released
            || (durable.released == incoming.released
                && durable.local_complement == incoming.local_complement))
}

pub(super) fn migration_complement_key(
    complement: &DurableMigrationComplement,
) -> MigrationComplementKey {
    (complement.source_schema, complement.target_schema)
}

pub(super) fn validate_migration_complement_append(
    complements: &[DurableMigrationComplement],
    index: &MigrationComplementIndex,
    base_schema: SchemaRevisionId,
    complement: &DurableMigrationComplement,
) -> Result<(), DurabilityError> {
    let key = migration_complement_key(complement);
    if let Some(&position) = index.get(&key) {
        if migration_step_replay_compatible(&complements[position], complement) {
            return Ok(());
        }
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement step identity conflicts with durable history",
        });
    }
    let expected_source = complements
        .last()
        .map_or(base_schema, |previous| previous.target_schema);
    if complement.source_schema != expected_source {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement chain is discontinuous",
        });
    }
    if index
        .range(
            (complement.source_schema, SchemaRevisionId::new(0))
                ..=(complement.source_schema, SchemaRevisionId::new(u64::MAX)),
        )
        .next()
        .is_some()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement chain revisits a schema revision",
        });
    }
    Ok(())
}

pub(super) fn append_migration_complement(
    complements: &mut Vec<DurableMigrationComplement>,
    index: &mut MigrationComplementIndex,
    base_schema: SchemaRevisionId,
    complement: DurableMigrationComplement,
) -> Result<(), DurabilityError> {
    validate_migration_complement_append(complements, index, base_schema, &complement)?;
    let key = migration_complement_key(&complement);
    if index.contains_key(&key) {
        return Ok(());
    }
    let position = complements.len();
    complements.push(complement);
    index.insert(key, position);
    Ok(())
}

pub(super) struct MigrationCommitOverlay<'a> {
    complements: &'a [DurableMigrationComplement],
    index: &'a MigrationComplementIndex,
    base_schema: SchemaRevisionId,
    staged: Vec<DurableMigrationComplement>,
    staged_by_key: BTreeMap<MigrationComplementKey, usize>,
    staged_sources: BTreeSet<SchemaRevisionId>,
}

impl<'a> MigrationCommitOverlay<'a> {
    pub(super) fn new(
        complements: &'a [DurableMigrationComplement],
        index: &'a MigrationComplementIndex,
        base_schema: SchemaRevisionId,
    ) -> Self {
        Self {
            complements,
            index,
            base_schema,
            staged: Vec::new(),
            staged_by_key: BTreeMap::new(),
            staged_sources: BTreeSet::new(),
        }
    }

    pub(super) fn prepare_append(
        &mut self,
        complement: &DurableMigrationComplement,
    ) -> Result<(), DurabilityError> {
        let key = migration_complement_key(complement);
        if let Some(&position) = self.index.get(&key) {
            if migration_step_replay_compatible(&self.complements[position], complement) {
                return Ok(());
            }
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "migration complement step identity conflicts with durable history",
            });
        }
        if let Some(&position) = self.staged_by_key.get(&key) {
            if migration_step_replay_compatible(&self.staged[position], complement) {
                return Ok(());
            }
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "migration complement step identity conflicts with durable history",
            });
        }
        let expected_source = self.staged.last().map_or_else(
            || {
                self.complements
                    .last()
                    .map_or(self.base_schema, |previous| previous.target_schema)
            },
            |previous| previous.target_schema,
        );
        if complement.source_schema != expected_source {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "migration complement chain is discontinuous",
            });
        }
        let source_seen = self
            .index
            .range(
                (complement.source_schema, SchemaRevisionId::new(0))
                    ..=(complement.source_schema, SchemaRevisionId::new(u64::MAX)),
            )
            .next()
            .is_some()
            || self.staged_sources.contains(&complement.source_schema);
        if source_seen {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "migration complement chain revisits a schema revision",
            });
        }
        let position = self.staged.len();
        self.staged.push(complement.clone());
        self.staged_by_key.insert(key, position);
        self.staged_sources.insert(complement.source_schema);
        Ok(())
    }

    pub(super) fn into_appends(self) -> Vec<DurableMigrationComplement> {
        self.staged
    }
}

pub(super) fn publish_prepared_migration_complements(
    complements: &mut Vec<DurableMigrationComplement>,
    index: &mut MigrationComplementIndex,
    prepared: Vec<DurableMigrationComplement>,
) {
    for complement in prepared {
        let key = migration_complement_key(&complement);
        debug_assert!(!index.contains_key(&key));
        let position = complements.len();
        complements.push(complement);
        index.insert(key, position);
    }
}

pub(super) fn migration_complement_index(
    complements: &[DurableMigrationComplement],
    base_schema: SchemaRevisionId,
) -> Result<MigrationComplementIndex, DurabilityError> {
    let mut validated = Vec::with_capacity(complements.len());
    let mut index = BTreeMap::new();
    for complement in complements {
        append_migration_complement(&mut validated, &mut index, base_schema, complement.clone())?;
    }
    Ok(index)
}

impl DurableRevisionStore {
    #[must_use]
    pub fn migration_complements(&self) -> &[DurableMigrationComplement] {
        &self.migration_complements
    }

    /// Resolves the exact locally-restorable complement chain between schema
    /// revisions. Retention policy is enforced here: released local payload,
    /// explicit Forget, and external-archive authority are never silently
    /// treated as locally reversible history.
    pub fn local_historical_complement_chain(
        &self,
        source: SchemaRevisionId,
        target: SchemaRevisionId,
    ) -> Result<LocalHistoricalComplementChain, HistoricalComplementError> {
        if source == target {
            return Ok(LocalHistoricalComplementChain::default());
        }
        let Some(start_position) = self
            .migration_complement_index
            .range((source, SchemaRevisionId::new(0))..=(source, SchemaRevisionId::new(u64::MAX)))
            .next()
            .map(|(_, position)| *position)
        else {
            return Err(HistoricalComplementError::PathNotFound { source, target });
        };
        let mut cursor = source;
        let mut chain = LocalHistoricalComplementChain::default();
        for durable in &self.migration_complements[start_position..] {
            if durable.source_schema != cursor {
                return Err(HistoricalComplementError::PathNotFound { source, target });
            }
            let Some(capsule) = durable.local_capsule() else {
                match durable.retention {
                    kernel_lens::ComplementRetention::ExternalArchive(proof) => {
                        return Err(HistoricalComplementError::ExternalArchiveRequired(proof));
                    }
                    kernel_lens::ComplementRetention::Forget => {
                        return Err(HistoricalComplementError::ExplicitlyForgotten {
                            source: durable.source_schema,
                            target: durable.target_schema,
                        });
                    }
                    kernel_lens::ComplementRetention::Forever
                    | kernel_lens::ComplementRetention::UntilRevision(_)
                    | kernel_lens::ComplementRetention::UntilEpoch(_) => {
                        return Err(HistoricalComplementError::LocalPayloadReleased {
                            source: durable.source_schema,
                            target: durable.target_schema,
                        });
                    }
                }
            };
            debug_assert_eq!(capsule.source_schema, durable.source_schema);
            debug_assert_eq!(capsule.target_schema, durable.target_schema);
            chain.push(durable.clone());
            cursor = durable.target_schema;
            if cursor == target {
                return Ok(chain);
            }
        }
        Err(HistoricalComplementError::PathNotFound { source, target })
    }
}

impl DurableRevisionStore {
    /// Publishes one logical migration-complement step in a fresh checkpoint
    /// generation before the corresponding schema transition is committed.
    /// An interrupted later transition may leave an orphan step, but can never
    /// leave a committed migration without its required complement authority.
    pub fn stage_migration_complement(
        &mut self,
        revision: &Revision,
        complement: DurableMigrationComplement,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        self.stage_migration_complement_with_hook(revision, complement, &mut NoStoreFault)
    }

    fn stage_migration_complement_with_hook(
        &mut self,
        revision: &Revision,
        complement: DurableMigrationComplement,
        hook: &mut impl StoreFaultHook,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        complement
            .validate()
            .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
        if complement.source_schema != revision.semantic_revision().schema {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "migration complement source schema does not match durable head schema",
            });
        }
        let key = migration_complement_key(&complement);
        let previous_len = self.migration_complements.len();
        append_migration_complement(
            &mut self.migration_complements,
            &mut self.migration_complement_index,
            self.checkpoint.semantic_revision().schema,
            complement,
        )?;
        let appended = self.migration_complements.len() != previous_len;
        let result = self.rotate_checkpoint_current_specs_with_hook(revision, hook);
        if result.is_err() && !self.poisoned && appended {
            let removed = self.migration_complements.pop();
            debug_assert!(removed.is_some());
            self.migration_complement_index.remove(&key);
        }
        result
    }

    /// Irreversibly releases local complement payloads whose declared boundary
    /// is explicitly satisfied, then persists the tombstone-only chain in a
    /// new checkpoint generation. No-op releases do not rotate the generation.
    pub fn release_due_migration_complements(
        &mut self,
        revision: &Revision,
        epoch: u64,
    ) -> Result<Option<DurableGenerationReceipt>, DurabilityError> {
        self.release_due_migration_complements_with_hook(revision, epoch, &mut NoStoreFault)
    }

    fn release_due_migration_complements_with_hook(
        &mut self,
        revision: &Revision,
        epoch: u64,
        hook: &mut impl StoreFaultHook,
    ) -> Result<Option<DurableGenerationReceipt>, DurabilityError> {
        if revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "complement release revision does not match durable WAL head",
            });
        }
        let mut rollback = Vec::new();
        for (index, complement) in self.migration_complements.iter_mut().enumerate() {
            if complement.release_is_due(revision.id(), epoch) {
                rollback.push((index, complement.clone()));
                let released = complement.release_if_due(revision.id(), epoch);
                debug_assert!(released);
            }
        }
        if rollback.is_empty() {
            return Ok(None);
        }
        let result = self.rotate_checkpoint_current_specs_with_hook(revision, hook);
        if result.is_err() && !self.poisoned {
            for (index, previous) in rollback {
                self.migration_complements[index] = previous;
            }
        }
        result.map(Some)
    }
}

#[cfg(test)]
impl DurableRevisionStore {
    pub(super) fn test_stage_migration_complement_with_hook(
        &mut self,
        revision: &Revision,
        complement: DurableMigrationComplement,
        hook: &mut impl StoreFaultHook,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        self.stage_migration_complement_with_hook(revision, complement, hook)
    }

    pub(super) fn test_release_due_migration_complements_with_hook(
        &mut self,
        revision: &Revision,
        epoch: u64,
        hook: &mut impl StoreFaultHook,
    ) -> Result<Option<DurableGenerationReceipt>, DurabilityError> {
        self.release_due_migration_complements_with_hook(revision, epoch, hook)
    }
}
