use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;

use crate::replication::authority::ReplicationAuthorityJournal;

use super::DurableRevisionStore;
use super::checkpoint_storage::write_checkpoint_generation;
use super::file_io::sync_directory;
use super::generation_layout::{lock_directory, metadata_path, wal_path};
use super::manifest::{ManifestRecord, highest_manifest_generation, publish_manifest_with_hook};
use super::metadata_storage::write_metadata_file;
use super::publication_protocol::{
    NoStoreFault, PublicationAttempt, StoreFaultHook, StoreFaultPoint,
};
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    canonical_physical_artifact_specs,
};
use crate::domain::IdempotencyEpoch;
use crate::metadata;
use crate::platform_assurance::{
    SupportedDurabilityProfile, VerifiedDestructiveDurabilityCampaignEvidence,
    certify_supported_durability_platform,
};
use crate::runtime::DurabilityError;
use crate::wal::FileRevisionWal;

impl DurableRevisionStore {
    /// Creates a process-local persistence authority with the same canonical
    /// retry/causal/replication reducer as durable stores and no filesystem
    /// realization. The protection floor is still carried by the backend so a
    /// later promotion cannot weaken inherited at-rest policy.
    pub fn create_volatile(
        base_revision: &Revision,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_volatile_with_materializations_physical_artifacts_and_cores(
            base_revision,
            &[],
            &[],
            &[],
            registry,
        )
    }

    pub fn create_volatile_with_materializations_physical_artifacts_and_cores(
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        let _ = registry
            .builtin_modules_for_context(base_revision.semantic_context())
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "base revision requires unavailable semantic implementation",
            })?;
        let physical_artifact_specs = canonical_physical_artifact_specs(physical_artifact_specs);
        let causal_coverage_root = base_revision.id();
        let revision_effects = BTreeMap::new();
        let revision_effect_frontiers = BTreeMap::from([(causal_coverage_root, BTreeSet::new())]);
        let replication = ReplicationAuthorityJournal::open_single_file(
            "volatile-replication-authority",
            &[],
            &[],
        )?;
        Ok(Self {
            backend: super::backend::DurabilityBackend::volatile(
                crate::storage_encryption::StorageProtectionProfile::Unencrypted,
            ),
            generation: 1,
            checkpoint: base_revision.clone(),
            durable_head: base_revision.id(),
            wal: crate::wal::RuntimeRevisionWal::volatile(),
            semantic_registry: registry.clone(),
            materialization_specs: materialization_specs.to_vec(),
            physical_artifact_specs,
            checkpoint_realization: None,
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: Vec::new(),
            historical_epoch_anchors: BTreeMap::new(),
            portable_historical_epochs: BTreeMap::new(),
            migration_complement_index: BTreeMap::new(),
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            committed_transactions: BTreeMap::new(),
            next_revision_effect_id: 1,
            causal_coverage_root,
            revision_effects,
            revision_effect_frontiers,
            replication,
            prepared_transactions: super::prepared_lifecycle::PreparedTransactionLedger::default(),
            streaming_checkpoint: None,
            external_freshness: None,
            poisoned: false,
        })
    }
    /// Creates a store only after the target directory has passed the named
    /// supported-platform durability profile and live fsync/rename probe.
    pub fn create_on_supported_platform(
        directory: impl AsRef<Path>,
        profile: SupportedDurabilityProfile,
        campaign: &VerifiedDestructiveDurabilityCampaignEvidence,
        base_revision: &Revision,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        certify_supported_durability_platform(directory.as_ref(), profile, campaign)?;
        Self::create(directory, base_revision, registry)
    }

    pub fn create(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_materializations_and_physical_artifacts(
            directory,
            base_revision,
            &[],
            &[],
            registry,
        )
    }

    pub fn create_with_materializations(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_materializations_and_physical_artifacts(
            directory,
            base_revision,
            materialization_specs,
            &[],
            registry,
        )
    }

    pub fn create_with_materializations_and_physical_artifacts(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_materializations_physical_artifacts_and_cores(
            directory,
            base_revision,
            materialization_specs,
            physical_artifact_specs,
            &[],
            registry,
        )
    }

    pub fn create_with_materializations_physical_artifacts_and_cores(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_hook(
            directory.as_ref(),
            base_revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            registry,
            &mut NoStoreFault,
        )
    }

    fn create_with_hook(
        directory: &Path,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        registry: &SemanticRegistry,
        hook: &mut impl StoreFaultHook,
    ) -> Result<Self, DurabilityError> {
        let directory = directory.to_path_buf();
        fs::create_dir_all(&directory)?;
        let directory_lock = lock_directory(&directory)?;
        if highest_manifest_generation(&directory)?.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable store already contains a published generation",
            });
        }
        let generation = 1;
        let checkpoint_crc32c = write_checkpoint_generation(&directory, generation, base_revision)?;
        hook.hit(StoreFaultPoint::AfterCheckpointSync)?;
        let wal_path = wal_path(&directory, generation);
        let mut wal = FileRevisionWal::create(&wal_path)?;
        wal.durability_barrier()?;
        hook.hit(StoreFaultPoint::AfterWalSync)?;
        let committed_transactions = BTreeMap::new();
        let semantic_modules = registry
            .builtin_modules_for_context(base_revision.semantic_context())
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "base revision requires unavailable semantic implementation",
            })?;
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
            committed_transactions: committed_transactions.clone(),
            semantic_modules,
            next_revision_effect_id: 1,
            causal_coverage_root: Some(causal_coverage_root),
            revision_effects: revision_effects.clone(),
            revision_effect_frontiers: revision_effect_frontiers.clone(),
            checkpoint_realization: None,
        };
        let metadata_file = metadata_path(&directory, generation);
        let metadata_crc32c = write_metadata_file(&metadata_file, &metadata_record)?;
        hook.hit(StoreFaultPoint::AfterMetadataSync)?;
        sync_directory(&directory)?;
        hook.hit(StoreFaultPoint::AfterPrerequisiteDirectorySync)?;
        let mut publication = PublicationAttempt::default();
        publish_manifest_with_hook(
            &directory,
            ManifestRecord {
                generation,
                base_revision: base_revision.id(),
                published_head: base_revision.id(),
                wal_first_lsn: 1,
                published_tail_lsn: 0,
                checkpoint_crc32c,
                metadata_crc32c,
                prepared_capsule_crc32c: 0,
            },
            hook,
            &mut publication,
        )?;
        let replication =
            ReplicationAuthorityJournal::open_or_create(directory.join("replication.cfre"))?;
        Ok(Self {
            backend: super::backend::DurabilityBackend::directory(directory, directory_lock),
            generation,
            checkpoint: base_revision.clone(),
            durable_head: base_revision.id(),
            wal: wal.into(),
            semantic_registry: registry.clone(),
            materialization_specs: materialization_specs.to_vec(),
            physical_artifact_specs,
            checkpoint_realization: None,
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: Vec::new(),
            historical_epoch_anchors: BTreeMap::new(),
            portable_historical_epochs: BTreeMap::new(),
            migration_complement_index: BTreeMap::new(),
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            committed_transactions,
            next_revision_effect_id: 1,
            causal_coverage_root,
            revision_effects,
            revision_effect_frontiers,
            replication,
            prepared_transactions: super::prepared_lifecycle::PreparedTransactionLedger::default(),
            streaming_checkpoint: None,
            external_freshness: None,
            poisoned: false,
        })
    }
}
