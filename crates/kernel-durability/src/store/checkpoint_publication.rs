use super::checkpoint_storage::write_checkpoint_file;
use super::file_io::sync_directory;
use super::freshness::ExternalFreshnessState;
use super::generation_layout::{
    checkpoint_path, metadata_path, next_generation, realization_path, wal_path,
};
use super::manifest::{ManifestRecord, publish_manifest_with_hook};
use super::metadata_storage::write_metadata_file;
use super::publication_protocol::{
    NoStoreFault, PublicationAttempt, StoreFaultHook, StoreFaultPoint,
};
use super::realization_storage::write_factorized_realization_file;
use super::{DurableGenerationReceipt, DurableRevisionStore};
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    canonical_physical_artifact_specs,
};
use crate::metadata;
use crate::realization::DurableFactorizedRealization;
use crate::runtime::DurabilityError;
use crate::single_file::compaction_io::{OsSingleFileCompactionIo, SingleFileCompactionIo};
use crate::wal::FileRevisionWal;
use kernel_realization::{FactorizedRealizationRoot, PhysicalAtomStore};
use kernel_revision::Revision;

impl DurableRevisionStore {
    pub fn rotate_checkpoint(
        &mut self,
        revision: &Revision,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        self.rotate_checkpoint_current_specs_with_hook(revision, &mut NoStoreFault)
    }

    pub(super) fn rotate_checkpoint_current_specs_with_hook(
        &mut self,
        revision: &Revision,
        hook: &mut impl StoreFaultHook,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let materialization_specs = self.materialization_specs.clone();
        let physical_artifact_specs = self.physical_artifact_specs.clone();
        let artifact_cores = self.artifact_cores.clone();
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.streaming_checkpoint.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "synchronous checkpoint rotation is blocked by streaming checkpoint job",
            });
        }
        if revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint revision does not match durable WAL head",
            });
        }
        self.rotate_checkpoint_with_fault_policy(
            revision,
            &materialization_specs,
            &physical_artifact_specs,
            &artifact_cores,
            hook,
        )
    }
}

impl DurableRevisionStore {
    pub fn rotate_checkpoint_with_materializations(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let physical_artifact_specs = self.physical_artifact_specs.clone();
        self.rotate_checkpoint_with_materializations_and_physical_artifacts(
            revision,
            materialization_specs,
            &physical_artifact_specs,
        )
    }

    pub fn rotate_checkpoint_with_materializations_and_physical_artifacts(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        self.rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
            revision,
            materialization_specs,
            physical_artifact_specs,
            &[],
        )
    }

    pub fn rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.streaming_checkpoint.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "synchronous checkpoint rotation is blocked by streaming checkpoint job",
            });
        }
        if revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint revision does not match durable WAL head",
            });
        }
        self.rotate_checkpoint_with_fault_policy(
            revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            &mut NoStoreFault,
        )
    }

    pub fn rotate_checkpoint_with_factorized_realization(
        &mut self,
        revision: &Revision,
        atoms: &PhysicalAtomStore,
        root: &FactorizedRealizationRoot,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.streaming_checkpoint.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "synchronous checkpoint rotation is blocked by streaming checkpoint job",
            });
        }
        if revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "physical checkpoint revision does not match durable WAL head",
            });
        }
        let mut physical =
            DurableFactorizedRealization::new(revision.id(), atoms.clone(), root.clone())?;
        if let Some(previous) = self.checkpoint_realization.as_ref() {
            physical.inherit_retained_historical_roots(
                previous,
                self.checkpoint.semantic_context(),
                self.historical_epoch_anchors
                    .values()
                    .map(|anchor| (anchor.effect_id, anchor.source_revision)),
            )?;
        }
        let materializations = self.materialization_specs.clone();
        let physical_artifacts = self.physical_artifact_specs.clone();
        let artifact_cores = self.artifact_cores.clone();
        if self.backend.is_single_file() {
            self.rotate_single_file_checkpoint(
                revision,
                &materializations,
                &physical_artifacts,
                &artifact_cores,
                Some(&physical),
            )
        } else {
            let mut publication = PublicationAttempt::default();
            let result = self.rotate_checkpoint_with_hook(
                revision,
                &materializations,
                &physical_artifacts,
                &artifact_cores,
                Some(&physical),
                &mut NoStoreFault,
                &mut publication,
            );
            if result.is_err() && publication.requires_recovery_after_error() {
                self.poisoned = true;
            }
            result
        }
    }

    /// Re-encodes the fully recovered in-memory durable authority into the
    /// current writable component formats and publishes it as a fresh immutable
    /// generation. Historical source files are never modified in place.
    pub fn migrate_to_current_format(
        &mut self,
        revision: &Revision,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let materializations = self.materialization_specs.clone();
        let physical_artifacts = self.physical_artifact_specs.clone();
        let artifact_cores = self.artifact_cores.clone();
        self.rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
            revision,
            &materializations,
            &physical_artifacts,
            &artifact_cores,
        )
    }

    fn rotate_checkpoint_with_fault_policy(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        hook: &mut impl StoreFaultHook,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        if self.backend.is_single_file() {
            let physical = self
                .checkpoint_realization
                .as_ref()
                .filter(|physical| physical.revision() == revision.id())
                .cloned();
            return self.rotate_single_file_checkpoint(
                revision,
                materialization_specs,
                physical_artifact_specs,
                artifact_cores,
                physical.as_ref(),
            );
        }
        let physical = self
            .checkpoint_realization
            .as_ref()
            .filter(|physical| physical.revision() == revision.id())
            .cloned();
        let mut publication = PublicationAttempt::default();
        let result = self.rotate_checkpoint_with_hook(
            revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            physical.as_ref(),
            hook,
            &mut publication,
        );
        if result.is_err() && publication.requires_recovery_after_error() {
            self.poisoned = true;
        }
        result
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep the explicit semantic and durability inputs at this boundary."
    )]
    fn rotate_checkpoint_with_hook(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        physical_realization: Option<&DurableFactorizedRealization>,
        hook: &mut impl StoreFaultHook,
        publication: &mut PublicationAttempt,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let directory = self.backend.directory_root()?.to_path_buf();
        let generation = next_generation(&directory)?;
        let checkpoint_file = checkpoint_path(&directory, generation);
        let checkpoint_crc32c = write_checkpoint_file(&checkpoint_file, revision)?;
        hook.hit(StoreFaultPoint::AfterCheckpointSync)?;
        let wal_file = wal_path(&directory, generation);
        let mut wal = FileRevisionWal::create(&wal_file)?;
        wal.durability_barrier()?;
        hook.hit(StoreFaultPoint::AfterWalSync)?;
        let physical_artifact_specs = canonical_physical_artifact_specs(physical_artifact_specs);
        let checkpoint_realization = physical_realization
            .map(|physical| {
                if physical.revision() != revision.id() {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "directory physical realization revision does not match checkpoint cut",
                    });
                }
                write_factorized_realization_file(
                    &realization_path(&directory, generation),
                    physical,
                )
            })
            .transpose()?;
        let metadata_record = metadata::DurableStoreMetadata {
            external_freshness: self
                .external_freshness
                .as_ref()
                .map(ExternalFreshnessState::metadata_binding),
            current_idempotency_epoch: self.current_idempotency_epoch,
            minimum_retry_epoch: self.minimum_retry_epoch,
            materializations: materialization_specs.to_vec(),
            physical_artifacts: physical_artifact_specs.clone(),
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: self.migration_complements.clone(),
            historical_epoch_anchors: self.historical_epoch_anchors.clone(),
            committed_transactions: self.committed_transactions.clone(),
            semantic_modules: self
                .semantic_registry
                .builtin_modules_for_context(revision.semantic_context())
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "checkpoint revision requires unavailable semantic implementation",
                })?,
            causal_coverage_root: Some(self.causal_coverage_root),
            revision_effects: self.revision_effects.clone(),
            revision_effect_frontiers: self.revision_effect_frontiers.clone(),
            checkpoint_realization,
        };
        let metadata_file = metadata_path(&directory, generation);
        let metadata_crc32c = write_metadata_file(&metadata_file, &metadata_record)?;
        hook.hit(StoreFaultPoint::AfterMetadataSync)?;
        sync_directory(&directory)?;
        hook.hit(StoreFaultPoint::AfterPrerequisiteDirectorySync)?;
        publish_manifest_with_hook(
            &directory,
            ManifestRecord {
                generation,
                base_revision: revision.id(),
                published_head: revision.id(),
                wal_first_lsn: 1,
                published_tail_lsn: 0,
                checkpoint_crc32c,
                metadata_crc32c,
                prepared_capsule_crc32c: 0,
            },
            hook,
            publication,
        )?;
        let freshness_digest = wal.freshness_digest();
        self.generation = generation;
        self.checkpoint = revision.clone();
        self.checkpoint_realization = physical_realization.cloned();
        self.wal = wal;
        self.materialization_specs = materialization_specs.to_vec();
        self.physical_artifact_specs = physical_artifact_specs;
        self.artifact_cores = artifact_cores.to_vec();
        self.prepared_transactions.clear();
        self.advance_external_freshness_generation_with_digest(generation, 0, freshness_digest)?;
        Ok(DurableGenerationReceipt {
            generation,
            base_revision: revision.id(),
        })
    }

    pub fn compact_obsolete_generations(&mut self) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.streaming_checkpoint.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "generation compaction is blocked by streaming checkpoint job",
            });
        }
        self.compact_obsolete_generations_with_hook(&mut NoStoreFault)
    }

    fn compact_obsolete_generations_with_hook(
        &mut self,
        hook: &mut impl StoreFaultHook,
    ) -> Result<(), DurabilityError> {
        let mut compaction_io = OsSingleFileCompactionIo;
        self.compact_obsolete_generations_with_hook_and_io(hook, &mut compaction_io)
    }

    fn compact_obsolete_generations_with_hook_and_io(
        &mut self,
        hook: &mut impl StoreFaultHook,
        compaction_io: &mut impl SingleFileCompactionIo,
    ) -> Result<(), DurabilityError> {
        if !self.backend.capabilities().physical_compaction {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durability backend does not support physical compaction",
            });
        }
        let poison_on_error = self.backend.is_single_file();
        let pinned_historical_generations = self.pinned_historical_generations();
        let result = self.backend.compact_obsolete_generations(
            &mut self.wal,
            self.generation,
            self.checkpoint.id(),
            self.durable_head,
            &pinned_historical_generations,
            hook,
            compaction_io,
        );
        if poison_on_error && result.is_err() {
            self.poisoned = true;
        }
        result
    }
}

#[cfg(test)]
impl DurableRevisionStore {
    pub(super) fn test_rotate_checkpoint_with_fault_policy(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        hook: &mut impl StoreFaultHook,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        self.rotate_checkpoint_with_fault_policy(
            revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            hook,
        )
    }

    pub(super) fn test_compact_obsolete_generations_with_hook(
        &mut self,
        hook: &mut impl StoreFaultHook,
    ) -> Result<(), DurabilityError> {
        self.compact_obsolete_generations_with_hook(hook)
    }

    pub(super) fn test_compact_obsolete_generations_with_io(
        &mut self,
        compaction_io: &mut impl SingleFileCompactionIo,
    ) -> Result<(), DurabilityError> {
        self.compact_obsolete_generations_with_hook_and_io(&mut NoStoreFault, compaction_io)
    }
}
