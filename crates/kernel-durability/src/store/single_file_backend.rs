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
use crate::replication::authority::ReplicationAuthorityJournal;
use crate::runtime::{DurabilityError, RecoveryScan};
use crate::single_file::{SingleFileContainer, SingleFileSectionInput, SingleFileSectionKind};

use super::freshness::{
    ExternalFreshnessAuthority, ExternalFreshnessConfig, ExternalFreshnessState,
};
use super::prepared_capsule::{
    PreparedCutCapsule, decode_prepared_cut_capsule, encode_prepared_cut_capsule,
};
use super::prepared_lifecycle::PreparedTransactionLedger;
use super::recovery::{rebuild_semantic_registry, recover_canonical_state};
use super::{DurableGenerationReceipt, DurableRevisionStore};

impl DurableRevisionStore {
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
            committed_transactions: BTreeMap::new(),
            semantic_modules: registry
                .builtin_modules_for_context(base_revision.semantic_context())
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "base revision requires unavailable semantic implementation",
                })?,
            causal_coverage_root: Some(causal_coverage_root),
            revision_effects: revision_effects.clone(),
            revision_effect_frontiers: revision_effect_frontiers.clone(),
        };
        let checkpoint_bytes = checkpoint::encode_revision(base_revision)?;
        let metadata_bytes = metadata::encode(&metadata_record)?;
        let sections = [
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Checkpoint,
                ordinal: 0,
                bytes: &checkpoint_bytes,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Metadata,
                ordinal: 0,
                bytes: &metadata_bytes,
            },
        ];
        let mut container = SingleFileContainer::create(&path, &sections)?;
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
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: Vec::new(),
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
        Self::open_single_file_inner(path.as_ref(), None, false)
    }

    pub fn open_single_file_with_legacy_registry(
        path: impl AsRef<Path>,
        legacy_registry: &SemanticRegistry,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_single_file_inner(path.as_ref(), Some(legacy_registry), false)
    }

    pub fn open_single_file_with_external_freshness(
        path: impl AsRef<Path>,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let path = path.as_ref();
        let material = super::backend::DurabilityBackend::probe_single_file_freshness(path)?;
        let (mut freshness, pending_advance) =
            ExternalFreshnessState::recover_preflight(&material, config, authority)?;
        let (mut store, scan) = Self::open_single_file_inner(path, None, true)?;
        freshness.complete_recovery_advance(pending_advance)?;
        store.external_freshness = Some(freshness);
        Ok((store, scan))
    }

    fn open_single_file_inner(
        path: &Path,
        legacy_registry: Option<&SemanticRegistry>,
        allow_external_freshness: bool,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let path = path.to_path_buf();
        let mut container = SingleFileContainer::open(&path)?;
        let metadata_bytes = container
            .read_section(SingleFileSectionKind::Metadata, 0)?
            .ok_or(DurabilityError::Corruption {
                offset: 0,
                reason: "single-file durable metadata section is missing",
            })?;
        let metadata = metadata::decode(&metadata_bytes)
            .map_err(|reason| DurabilityError::Corruption { offset: 0, reason })?;
        if metadata.external_freshness.is_some() && !allow_external_freshness {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "externally anchored store requires freshness-aware open",
            });
        }
        let registry = rebuild_semantic_registry(&metadata, legacy_registry)?;
        let checkpoint_bytes = container
            .read_section(SingleFileSectionKind::Checkpoint, 0)?
            .ok_or(DurabilityError::Corruption {
                offset: 0,
                reason: "single-file checkpoint section is missing",
            })?;
        let checkpoint = checkpoint::decode_revision(&checkpoint_bytes, &registry)?;
        let prepared_capsule = container
            .read_section(SingleFileSectionKind::PreparedCapsule, 0)?
            .map_or_else(
                || Ok(PreparedCutCapsule::default()),
                |bytes| decode_prepared_cut_capsule(&bytes),
            )?;
        let view = container.generation_view()?;
        let seeds = prepared_capsule.scan_seeds();
        let (wal, scan) =
            container.open_journal_recovered(checkpoint.id(), view.journal_first_lsn, &seeds)?;
        let canonical = recover_canonical_state(metadata, checkpoint, &scan, legacy_registry)?;
        let replication_archive = container
            .read_section(SingleFileSectionKind::ReplicationAuthority, 0)?
            .unwrap_or_default();
        let replication = ReplicationAuthorityJournal::open_single_file(
            &path,
            &replication_archive,
            scan.replication_authority_frames(),
        )?;
        let prepared_transactions = PreparedTransactionLedger::from_recovery_scan(&scan);
        let mut store = canonical.into_store(
            super::backend::DurabilityBackend::single_file(container),
            view.generation,
            wal,
            replication,
        );
        store.prepared_transactions = prepared_transactions;
        Ok((store, scan))
    }

    pub(super) fn single_file_replication_archive(&mut self) -> Result<Vec<u8>, DurabilityError> {
        let mut archive = self
            .backend
            .single_file_container()?
            .read_section(SingleFileSectionKind::ReplicationAuthority, 0)?
            .unwrap_or_default();
        let live = self.replication.single_file_live_archive_bytes()?;
        archive
            .try_reserve(live.len())
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        archive.extend_from_slice(&live);
        Ok(archive)
    }

    pub(super) fn single_file_replication_archive_prefix(
        &mut self,
        live_count: usize,
    ) -> Result<Vec<u8>, DurabilityError> {
        let mut archive = self
            .backend
            .single_file_container()?
            .read_section(SingleFileSectionKind::ReplicationAuthority, 0)?
            .unwrap_or_default();
        let live = self
            .replication
            .single_file_live_archive_prefix(live_count)?;
        archive
            .try_reserve(live.len())
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        archive.extend_from_slice(&live);
        Ok(archive)
    }

    pub(super) fn rotate_single_file_checkpoint(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
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
        let checkpoint_bytes = checkpoint::encode_revision(revision)?;
        let metadata_bytes = metadata::encode(&metadata_record)?;
        let prepared_capsule = PreparedCutCapsule::from_prepared_transactions(
            &self.prepared_transactions,
            self.durable_head,
        );
        let prepared_bytes = encode_prepared_cut_capsule(&prepared_capsule)?;
        let replication_archive = self.single_file_replication_archive()?;
        let sections = [
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Checkpoint,
                ordinal: 0,
                bytes: &checkpoint_bytes,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Metadata,
                ordinal: 0,
                bytes: &metadata_bytes,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::PreparedCapsule,
                ordinal: 0,
                bytes: &prepared_bytes,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::ReplicationAuthority,
                ordinal: 0,
                bytes: &replication_archive,
            },
        ];
        let container = self.backend.single_file_container()?;
        let view = container
            .publish_generation_after_active_wal(&mut self.wal, &sections)
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
