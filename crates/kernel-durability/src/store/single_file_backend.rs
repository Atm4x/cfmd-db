use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;

use crate::checkpoint;
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    canonical_physical_artifact_specs,
};
use crate::domain::IdempotencyEpoch;
use crate::metadata;
use crate::realization::DurableFactorizedRealization;
use crate::replication::authority::ReplicationAuthorityJournal;
use crate::runtime::{DurabilityError, RecoveryScan};
use crate::single_file::{
    SingleFileContainer, SingleFileSectionInput, SingleFileSectionKind, SingleFileSectionSource,
};
use crate::storage_encryption::StorageEncryption;

use super::freshness::{
    ExternalFreshnessAuthority, ExternalFreshnessConfig, ExternalFreshnessState,
};
use super::prepared_capsule::{
    PreparedCutCapsule, decode_prepared_cut_capsule, encode_prepared_cut_capsule,
};
use super::prepared_lifecycle::PreparedTransactionLedger;
use super::recovery::{rebuild_semantic_registry, recover_canonical_state};
use super::{DurableGenerationReceipt, DurableRevisionStore};

pub(super) struct RevisionSectionSource<'a>(pub(super) &'a Revision);

impl SingleFileSectionSource for RevisionSectionSource<'_> {
    fn plaintext_len(&self) -> Result<u64, DurabilityError> {
        checkpoint::encoded_revision_len(self.0).map_err(Into::into)
    }

    fn write_to(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        checkpoint::stream_revision(self.0, emit)
    }
}

pub(super) struct MetadataSectionSource<'a>(pub(super) &'a metadata::DurableStoreMetadata);

impl SingleFileSectionSource for MetadataSectionSource<'_> {
    fn plaintext_len(&self) -> Result<u64, DurabilityError> {
        metadata::encoded_len(self.0).map_err(Into::into)
    }

    fn write_to(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        metadata::stream(self.0, emit)
    }
}

pub(super) struct FactorizedRealizationSectionSource<'a>(
    pub(super) &'a DurableFactorizedRealization,
);

impl SingleFileSectionSource for FactorizedRealizationSectionSource<'_> {
    fn plaintext_len(&self) -> Result<u64, DurabilityError> {
        self.0.encoded_len()
    }

    fn write_to(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        self.0.stream(emit)
    }
}

impl DurableRevisionStore {
    pub fn rewrap_single_file_database_master_key(
        &mut self,
        next: &StorageEncryption,
    ) -> Result<u64, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.backend
            .single_file_container()?
            .rewrap_database_master_key(next)
            .inspect_err(|_| self.poisoned = true)
    }

    pub fn retire_previous_single_file_wrapped_key(
        &mut self,
        acknowledged_key_epoch: u64,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.backend
            .single_file_container()?
            .retire_previous_wrapped_key_slot(acknowledged_key_epoch)
            .inspect_err(|_| self.poisoned = true)
    }

    pub fn create_single_file(
        path: impl AsRef<Path>,
        base_revision: &Revision,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_single_file_with_materializations_physical_artifacts_and_cores(
            path,
            base_revision,
            &[],
            &[],
            &[],
            registry,
        )
    }

    pub fn create_single_file_with_encryption(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
        base_revision: &Revision,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores(
            path,
            encryption,
            base_revision,
            &[],
            &[],
            &[],
            registry,
        )
    }

    pub fn create_single_file_with_materializations_and_physical_artifacts(
        path: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_single_file_with_materializations_physical_artifacts_and_cores(
            path,
            base_revision,
            materialization_specs,
            physical_artifact_specs,
            &[],
            registry,
        )
    }

    pub fn create_single_file_with_materializations_physical_artifacts_and_cores(
        path: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores(
            path,
            &StorageEncryption::None,
            base_revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            registry,
        )
    }

    pub fn create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let physical_artifact_specs = canonical_physical_artifact_specs(physical_artifact_specs);
        let causal_coverage_root = base_revision.id();
        let revision_effects = BTreeMap::new();
        let revision_effect_frontiers = BTreeMap::from([(causal_coverage_root, BTreeSet::new())]);
        let metadata_record = metadata::DurableStoreMetadata {
            external_freshness: None,
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            materializations: materialization_specs.to_vec(),
            physical_artifacts: physical_artifact_specs.clone(),
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: Vec::new(),
            historical_epoch_anchors: BTreeMap::new(),
            committed_transactions: BTreeMap::new(),
            semantic_modules: registry
                .builtin_modules_for_context(base_revision.semantic_context())
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "base revision requires unavailable semantic implementation",
                })?,
            next_revision_effect_id: 1,
            causal_coverage_root: Some(causal_coverage_root),
            revision_effects: revision_effects.clone(),
            revision_effect_frontiers: revision_effect_frontiers.clone(),
            checkpoint_realization: None,
        };
        let checkpoint_source = RevisionSectionSource(base_revision);
        let metadata_source = MetadataSectionSource(&metadata_record);
        let sections = [
            SingleFileSectionInput::streaming(
                SingleFileSectionKind::Checkpoint,
                0,
                &checkpoint_source,
            ),
            SingleFileSectionInput::streaming(SingleFileSectionKind::Metadata, 0, &metadata_source),
        ];
        let mut container =
            SingleFileContainer::create_with_encryption(&path, &sections, encryption)?;
        let (wal, scan) = container.open_journal_recovered(base_revision.id(), 1, &[])?;
        debug_assert_eq!(scan.durable_revision(), base_revision.id());
        let replication = ReplicationAuthorityJournal::open_single_file(
            &path,
            &[],
            scan.replication_authority_frames(),
        )?;
        Ok(Self {
            backend: super::backend::DurabilityBackend::single_file(container),
            generation: 1,
            checkpoint: base_revision.clone(),
            durable_head: base_revision.id(),
            wal,
            semantic_registry: registry.clone(),
            materialization_specs: materialization_specs.to_vec(),
            physical_artifact_specs,
            checkpoint_realization: None,
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: Vec::new(),
            historical_epoch_anchors: BTreeMap::new(),
            migration_complement_index: BTreeMap::new(),
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            committed_transactions: BTreeMap::new(),
            next_revision_effect_id: 1,
            causal_coverage_root,
            revision_effects,
            revision_effect_frontiers,
            replication,
            prepared_transactions: PreparedTransactionLedger::default(),
            streaming_checkpoint: None,
            external_freshness: None,
            poisoned: false,
        })
    }

    pub fn open_single_file(
        path: impl AsRef<Path>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_single_file_inner(path.as_ref(), false, &StorageEncryption::None)
    }

    pub fn open_single_file_with_encryption(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_single_file_inner(path.as_ref(), false, encryption)
    }

    pub fn open_single_file_with_external_freshness(
        path: impl AsRef<Path>,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_single_file_with_external_freshness_and_encryption(
            path,
            config,
            authority,
            &StorageEncryption::None,
        )
    }

    pub fn open_single_file_with_external_freshness_and_encryption(
        path: impl AsRef<Path>,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
        encryption: &StorageEncryption,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let path = path.as_ref();
        let material =
            super::backend::DurabilityBackend::probe_single_file_freshness_with_encryption(
                path, encryption,
            )?;
        let (mut freshness, pending_advance) =
            ExternalFreshnessState::recover_preflight(&material, config, authority)?;
        let (mut store, scan) = Self::open_single_file_inner(path, true, encryption)?;
        freshness.complete_recovery_advance(pending_advance)?;
        store.external_freshness = Some(freshness);
        Ok((store, scan))
    }

    fn open_single_file_inner(
        path: &Path,
        allow_external_freshness: bool,
        encryption: &StorageEncryption,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let path = path.to_path_buf();
        let mut container = SingleFileContainer::open_with_encryption(&path, encryption)?;
        let metadata = container
            .with_section_reader(SingleFileSectionKind::Metadata, 0, |reader, len| {
                metadata::decode_from_reader(reader, len)
                    .map_err(|reason| DurabilityError::Corruption { offset: 0, reason })
            })?
            .ok_or(DurabilityError::Corruption {
                offset: 0,
                reason: "single-file durable metadata section is missing",
            })?;
        if metadata.external_freshness.is_some() && !allow_external_freshness {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "externally anchored store requires freshness-aware open",
            });
        }
        let registry = rebuild_semantic_registry(&metadata)?;
        let checkpoint = container
            .with_section_reader(SingleFileSectionKind::Checkpoint, 0, |reader, len| {
                checkpoint::decode_revision_from_reader(reader, len, &registry)
            })?
            .ok_or(DurabilityError::Corruption {
                offset: 0,
                reason: "single-file checkpoint section is missing",
            })?;
        let prepared_capsule = container
            .read_section(SingleFileSectionKind::PreparedCapsule, 0)?
            .map_or_else(
                || Ok(PreparedCutCapsule::default()),
                |bytes| decode_prepared_cut_capsule(&bytes),
            )?;
        let checkpoint_realization = container.with_section_reader(
            SingleFileSectionKind::PhysicalArtifact,
            0,
            |reader, len| DurableFactorizedRealization::decode_from_reader(reader, len),
        )?;
        if checkpoint_realization
            .as_ref()
            .is_some_and(|physical| physical.revision() != checkpoint.id())
        {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "durable physical realization revision does not match checkpoint cut",
            });
        }
        let view = container.generation_view()?;
        let seeds = prepared_capsule.scan_seeds();
        let (wal, scan) =
            container.open_journal_recovered(checkpoint.id(), view.journal_first_lsn, &seeds)?;
        let canonical = recover_canonical_state(metadata, checkpoint, &scan, view.generation)?;
        let replication =
            container.recover_replication_authority_journal(scan.replication_authority_frames())?;
        let prepared_transactions = PreparedTransactionLedger::from_recovery_scan(&scan);
        let mut store = canonical.into_store(
            super::backend::DurabilityBackend::single_file(container),
            view.generation,
            wal,
            replication,
        );
        store.prepared_transactions = prepared_transactions;
        store.checkpoint_realization = checkpoint_realization;
        Ok((store, scan))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub(super) fn rotate_single_file_checkpoint(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        physical_realization: Option<&DurableFactorizedRealization>,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
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
            historical_epoch_anchors: self.historical_epoch_anchors.clone(),
            committed_transactions: self.committed_transactions.clone(),
            semantic_modules: self.semantic_registry.builtin_module_specs(),
            next_revision_effect_id: self.next_revision_effect_id,
            causal_coverage_root: Some(self.causal_coverage_root),
            revision_effects: self.revision_effects.clone(),
            revision_effect_frontiers: self.revision_effect_frontiers.clone(),
            checkpoint_realization: None,
        };
        let checkpoint_source = RevisionSectionSource(revision);
        let metadata_source = MetadataSectionSource(&metadata_record);
        let prepared_capsule = PreparedCutCapsule::from_prepared_transactions(
            &self.prepared_transactions,
            self.durable_head,
        );
        let prepared_bytes = encode_prepared_cut_capsule(&prepared_capsule)?;
        let replication_live_count = self.replication.single_file_live_frame_count();
        let replication_frames = self
            .replication
            .single_file_live_frames_prefix(replication_live_count)?;
        let mut sections = vec![
            SingleFileSectionInput::streaming(
                SingleFileSectionKind::Checkpoint,
                0,
                &checkpoint_source,
            ),
            SingleFileSectionInput::streaming(SingleFileSectionKind::Metadata, 0, &metadata_source),
            SingleFileSectionInput::bytes(
                SingleFileSectionKind::PreparedCapsule,
                0,
                &prepared_bytes,
            ),
        ];
        let physical_source = physical_realization.map(FactorizedRealizationSectionSource);
        if let Some(source) = physical_source.as_ref() {
            sections.push(SingleFileSectionInput::streaming(
                SingleFileSectionKind::PhysicalArtifact,
                0,
                source,
            ));
        }
        let retained_historical_generations =
            self.pinned_historical_generations_for(physical_realization);
        let archive_outgoing = retained_historical_generations
            .contains(&self.generation)
            .then_some(crate::single_file::HistoricalGenerationArchive {
                generation: self.generation,
                checkpoint_revision: self.checkpoint.id(),
                durable_head: self.durable_head,
            });
        let container = self.backend.single_file_container()?;
        let view = container
            .publish_generation_after_active_wal(
                &mut self.wal,
                &sections,
                replication_frames,
                archive_outgoing,
                &retained_historical_generations,
            )
            .inspect_err(|_| self.poisoned = true)?;
        let seeds = prepared_capsule.scan_seeds();
        let (wal, scan) = container
            .open_journal_recovered(revision.id(), view.journal_first_lsn, &seeds)
            .inspect_err(|_| self.poisoned = true)?;
        if scan.durable_revision() != revision.id() || !scan.committed().is_empty() {
            self.poisoned = true;
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "fresh single-file generation journal did not reopen at checkpoint cut",
            });
        }
        self.generation = view.generation;
        self.checkpoint = revision.clone();
        self.checkpoint_realization = physical_realization.cloned();
        self.wal = wal;
        self.replication.reset_single_file_generation();
        self.materialization_specs = materialization_specs.to_vec();
        self.physical_artifact_specs = physical_artifact_specs;
        self.artifact_cores = artifact_cores.to_vec();
        self.prepared_transactions
            .retain_published_generation(view.journal_first_lsn, prepared_capsule.prepare_lsns());
        let freshness_digest = self.wal.freshness_digest();
        self.advance_external_freshness_generation_with_digest(
            self.generation,
            self.wal.last_lsn(),
            freshness_digest,
        )?;
        Ok(DurableGenerationReceipt {
            generation: self.generation,
            base_revision: revision.id(),
        })
    }
}
