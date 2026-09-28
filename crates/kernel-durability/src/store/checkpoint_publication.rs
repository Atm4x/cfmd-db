use super::checkpoint_storage::write_checkpoint_file;
use super::file_io::sync_directory;
use super::freshness::ExternalFreshnessState;
use super::generation_layout::{checkpoint_path, metadata_path, next_generation, wal_path};
use super::manifest::{ManifestRecord, publish_manifest_with_hook};
use super::metadata_storage::write_metadata_file;
use super::publication_protocol::{
    NoStoreFault, PublicationAttempt, StoreFaultHook, StoreFaultPoint,
};
use super::{DurableGenerationReceipt, DurableRevisionStore};
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    canonical_physical_artifact_specs,
};
use crate::metadata;
use crate::runtime::DurabilityError;
use crate::wal::FileRevisionWal;
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
            return self.rotate_single_file_checkpoint(
                revision,
                materialization_specs,
                physical_artifact_specs,
                artifact_cores,
            );
        }
        let mut publication = PublicationAttempt::default();
        let result = self.rotate_checkpoint_with_hook(
            revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            hook,
            &mut publication,
        );
        if result.is_err() && publication.requires_recovery_after_error() {
            self.poisoned = true;
        }
        result
    }

    fn rotate_checkpoint_with_hook(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
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
        if !self.backend.capabilities().physical_compaction {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durability backend does not support physical compaction",
            });
        }
        self.backend.compact_obsolete_generations(
            &mut self.wal,
            self.generation,
            self.checkpoint.id(),
            self.durable_head,
            hook,
        )
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
}
