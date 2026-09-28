use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use kernel_change::RevisionEffectId;
use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;
use kernel_types::RevisionId;

use crate::replication::authority::ReplicationAuthorityJournal;

use super::causal_ledger;
use super::checkpoint_storage::read_checkpoint_generation;
use super::freshness::{
    ExternalFreshnessAuthority, ExternalFreshnessConfig, ExternalFreshnessState,
};
use super::generation_layout::{lock_directory, prepared_capsule_path, wal_path};
use super::manifest::{ManifestRecord, read_current_manifest};
use super::metadata_storage::read_published_metadata;
use super::migration_history::{
    MigrationComplementIndex, append_migration_complement, migration_complement_index,
};
use super::prepared_capsule::read_prepared_cut_capsule_file;
use super::semantic_deployment::{
    install_intent_semantic_modules, install_semantic_module_packages,
};
use super::{DurableRevisionStore, PreparedCutCapsule};
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
};
use crate::domain::{
    DurableMigrationComplement, DurableRevisionEffectRecord, DurableTransactionIntent,
    DurableTransactionKey, IdempotencyEpoch,
};
use crate::metadata;
use crate::platform_assurance::{
    SupportedDurabilityProfile, VerifiedDestructiveDurabilityCampaignEvidence,
    certify_supported_durability_platform,
};
use crate::runtime::{DurabilityError, RecoveryScan};
use crate::wal::FileRevisionWal;

#[derive(Debug)]
pub(super) struct CanonicalDurableState {
    checkpoint: Revision,
    durable_head: RevisionId,
    semantic_registry: SemanticRegistry,
    materialization_specs: Vec<DurableMaterializationSpec>,
    physical_artifact_specs: Vec<DurablePhysicalArtifactSpec>,
    artifact_cores: Vec<DurableArtifactCore>,
    migration_complements: Vec<DurableMigrationComplement>,
    migration_complement_index: MigrationComplementIndex,
    current_idempotency_epoch: IdempotencyEpoch,
    minimum_retry_epoch: IdempotencyEpoch,
    committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    next_revision_effect_id: u128,
    causal_coverage_root: RevisionId,
    revision_effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    revision_effect_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
}

impl CanonicalDurableState {
    pub(super) fn into_store(
        self,
        backend: super::backend::DurabilityBackend,
        generation: u64,
        wal: FileRevisionWal,
        replication: ReplicationAuthorityJournal,
    ) -> DurableRevisionStore {
        DurableRevisionStore {
            backend,
            generation,
            checkpoint: self.checkpoint,
            durable_head: self.durable_head,
            wal,
            semantic_registry: self.semantic_registry,
            materialization_specs: self.materialization_specs,
            physical_artifact_specs: self.physical_artifact_specs,
            artifact_cores: self.artifact_cores,
            migration_complements: self.migration_complements,
            migration_complement_index: self.migration_complement_index,
            current_idempotency_epoch: self.current_idempotency_epoch,
            minimum_retry_epoch: self.minimum_retry_epoch,
            committed_transactions: self.committed_transactions,
            next_revision_effect_id: self.next_revision_effect_id,
            causal_coverage_root: self.causal_coverage_root,
            revision_effects: self.revision_effects,
            revision_effect_frontiers: self.revision_effect_frontiers,
            replication,
            prepared_transactions: super::prepared_lifecycle::PreparedTransactionLedger::default(),
            streaming_checkpoint: None,
            external_freshness: None,
            poisoned: false,
        }
    }
}

fn merge_wal_migration_complements(
    mut complements: Vec<DurableMigrationComplement>,
    base_schema: kernel_types::SchemaRevisionId,
    scan: &RecoveryScan,
) -> Result<(Vec<DurableMigrationComplement>, MigrationComplementIndex), DurabilityError> {
    let mut index = migration_complement_index(&complements, base_schema)?;
    for committed in scan.committed() {
        if let DurableTransactionIntent::SchemaMigrationExact {
            migration_complement,
            ..
        } = &committed.descriptor.intent
        {
            append_migration_complement(
                &mut complements,
                &mut index,
                base_schema,
                migration_complement.clone(),
            )?;
        }
    }
    Ok((complements, index))
}

pub(super) fn rebuild_semantic_registry(
    metadata: &metadata::DurableStoreMetadata,
    legacy_registry: Option<&SemanticRegistry>,
) -> Result<SemanticRegistry, DurabilityError> {
    let mut registry = if metadata.semantic_modules.is_empty() {
        legacy_registry.cloned().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "durable semantic deployment manifest is missing",
        })?
    } else {
        let mut registry = SemanticRegistry::default();
        install_semantic_module_packages(&mut registry, &metadata.semantic_modules)?;
        registry
    };
    for intent in metadata.committed_transactions.values() {
        install_intent_semantic_modules(&mut registry, intent)?;
    }
    Ok(registry)
}

fn merge_committed_transaction_intents(
    mut committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    scan: &RecoveryScan,
    registry: &mut SemanticRegistry,
) -> Result<BTreeMap<DurableTransactionKey, DurableTransactionIntent>, DurabilityError> {
    for (&transaction_id, intent) in scan.committed_transactions() {
        install_intent_semantic_modules(registry, intent)?;
        if let Some(existing) = committed_transactions.insert(transaction_id, intent.clone())
            && existing != *intent
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint transaction intent ledger conflicts with WAL tail",
            });
        }
    }
    Ok(committed_transactions)
}

fn open_published_generation(
    directory: &Path,
    manifest: ManifestRecord,
    registry: &SemanticRegistry,
) -> Result<(Revision, FileRevisionWal, RecoveryScan), DurabilityError> {
    let checkpoint = read_checkpoint_generation(directory, manifest, registry)?;
    if checkpoint.id() != manifest.base_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "manifest base revision does not match checkpoint",
        });
    }
    let prepared_capsule = if manifest.prepared_capsule_crc32c == 0 {
        PreparedCutCapsule::default()
    } else {
        read_prepared_cut_capsule_file(
            &prepared_capsule_path(directory, manifest.generation),
            manifest.prepared_capsule_crc32c,
        )?
    };
    let wal_file = wal_path(directory, manifest.generation);
    if !wal_file.is_file() {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published WAL segment is missing",
        });
    }
    let seeds = prepared_capsule.scan_seeds();
    let (wal, scan) = FileRevisionWal::open_recovered_seeded(
        &wal_file,
        checkpoint.id(),
        manifest.wal_first_lsn,
        &seeds,
    )?;
    if scan.next_lsn() <= manifest.published_tail_lsn {
        return Err(DurabilityError::Corruption {
            offset: scan.last_good_offset(),
            reason: "published shadow WAL tail is shorter than manifest certificate",
        });
    }
    let certified_head = scan
        .committed()
        .iter()
        .take_while(|committed| committed.commit_lsn <= manifest.published_tail_lsn)
        .last()
        .map_or(checkpoint.id(), |committed| {
            committed.descriptor.target_revision
        });
    if certified_head != manifest.published_head {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "published WAL certificate does not reach manifest head exactly",
        });
    }
    Ok((checkpoint, wal, scan))
}

fn recover_retry_ledger(
    committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    current: IdempotencyEpoch,
    minimum: IdempotencyEpoch,
    scan: &RecoveryScan,
    registry: &mut SemanticRegistry,
) -> Result<
    (
        BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
        IdempotencyEpoch,
    ),
    DurabilityError,
> {
    let committed_transactions =
        merge_committed_transaction_intents(committed_transactions, scan, registry)?;
    let current = committed_transactions
        .keys()
        .map(|key| key.epoch)
        .max()
        .map_or(current, |epoch| epoch.max(current));
    if committed_transactions.keys().any(|key| key.epoch < minimum) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "WAL tail contains a transaction below the retry-history watermark",
        });
    }
    Ok((committed_transactions, current))
}

pub(super) fn recover_canonical_state(
    metadata: metadata::DurableStoreMetadata,
    checkpoint: Revision,
    scan: &RecoveryScan,
    legacy_registry: Option<&SemanticRegistry>,
) -> Result<CanonicalDurableState, DurabilityError> {
    let mut registry = rebuild_semantic_registry(&metadata, legacy_registry)?;
    let minimum_retry_epoch = metadata.minimum_retry_epoch;
    let (committed_transactions, current_idempotency_epoch) = recover_retry_ledger(
        metadata.committed_transactions,
        metadata.current_idempotency_epoch,
        minimum_retry_epoch,
        scan,
        &mut registry,
    )?;
    let (migration_complements, migration_complement_index) = merge_wal_migration_complements(
        metadata.migration_complements,
        checkpoint.semantic_revision().schema,
        scan,
    )?;
    let (causal_coverage_root, revision_effects, revision_effect_frontiers) =
        causal_ledger::recover_revision_effect_state(
            metadata.causal_coverage_root,
            metadata.revision_effects,
            metadata.revision_effect_frontiers,
            scan,
        )?;
    causal_ledger::validate_revision_effect_state(
        causal_coverage_root,
        &revision_effects,
        &revision_effect_frontiers,
    )?;
    let next_revision_effect_id = revision_effects
        .keys()
        .map(|id| id.0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "revision effect identity space is exhausted",
        })?;
    Ok(CanonicalDurableState {
        checkpoint,
        durable_head: scan.durable_revision(),
        semantic_registry: registry,
        materialization_specs: metadata.materializations,
        physical_artifact_specs: metadata.physical_artifacts,
        artifact_cores: metadata.artifact_cores,
        migration_complements,
        migration_complement_index,
        current_idempotency_epoch,
        minimum_retry_epoch,
        committed_transactions,
        next_revision_effect_id,
        causal_coverage_root,
        revision_effects,
        revision_effect_frontiers,
    })
}

fn validate_replicated_authority(
    canonical: &mut CanonicalDurableState,
    replication: &ReplicationAuthorityJournal,
) -> Result<(), DurabilityError> {
    for envelope in replication.effects_iter().map(|(_, envelope)| envelope) {
        if canonical.revision_effects.contains_key(&envelope.effect.id)
            || canonical
                .revision_effect_frontiers
                .contains_key(&envelope.effect.target_revision)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated causal authority collides with local authority",
            });
        }
        let expected = causal_ledger::causal_prerequisites_for_replicated_effect(
            &canonical.revision_effect_frontiers,
            replication,
            &envelope.effect,
        )?;
        if expected != envelope.effect.prerequisites {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replayed replicated effect has a stale causal cut",
            });
        }
        install_intent_semantic_modules(&mut canonical.semantic_registry, &envelope.effect.intent)?;
    }
    Ok(())
}

impl DurableRevisionStore {
    /// Opens a store only after the target directory has passed the named
    /// supported-platform durability profile and live fsync/rename probe.
    pub fn open_on_supported_platform(
        directory: impl AsRef<Path>,
        profile: SupportedDurabilityProfile,
        campaign: &VerifiedDestructiveDurabilityCampaignEvidence,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        certify_supported_durability_platform(directory.as_ref(), profile, campaign)?;
        Self::open(directory)
    }

    pub fn open(directory: impl AsRef<Path>) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_inner(directory.as_ref(), None, false)
    }

    pub fn open_with_legacy_registry(
        directory: impl AsRef<Path>,
        legacy_registry: &SemanticRegistry,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_inner(directory.as_ref(), Some(legacy_registry), false)
    }

    pub fn open_with_external_freshness_on_supported_platform(
        directory: impl AsRef<Path>,
        profile: SupportedDurabilityProfile,
        campaign: &VerifiedDestructiveDurabilityCampaignEvidence,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        certify_supported_durability_platform(directory.as_ref(), profile, campaign)?;
        Self::open_with_external_freshness(directory, config, authority)
    }

    pub fn open_with_external_freshness(
        directory: impl AsRef<Path>,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let directory = directory.as_ref();
        let material = super::backend::DurabilityBackend::probe_directory_freshness(directory)?;
        let (mut freshness, pending_advance) =
            ExternalFreshnessState::recover_preflight(&material, config, authority)?;
        let (mut store, scan) = Self::open_inner(directory, None, true)?;
        freshness.complete_recovery_advance(pending_advance)?;
        store.external_freshness = Some(freshness);
        Ok((store, scan))
    }

    fn open_inner(
        directory: &Path,
        legacy_registry: Option<&SemanticRegistry>,
        allow_external_freshness: bool,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let directory = directory.to_path_buf();
        let directory_lock = lock_directory(&directory)?;
        let manifest = read_current_manifest(&directory)?;
        let metadata = read_published_metadata(&directory, manifest)?;
        if metadata.external_freshness.is_some() && !allow_external_freshness {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "externally anchored store requires freshness-aware open",
            });
        }
        let registry = rebuild_semantic_registry(&metadata, legacy_registry)?;
        let (checkpoint, wal, scan) = open_published_generation(&directory, manifest, &registry)?;
        let mut canonical = recover_canonical_state(metadata, checkpoint, &scan, legacy_registry)?;
        let replication =
            ReplicationAuthorityJournal::open_or_create(directory.join("replication.cfre"))?;
        validate_replicated_authority(&mut canonical, &replication)?;
        let prepared_transactions =
            super::prepared_lifecycle::PreparedTransactionLedger::from_recovery_scan(&scan);
        let mut store = canonical.into_store(
            super::backend::DurabilityBackend::directory(directory, directory_lock),
            manifest.generation,
            wal,
            replication,
        );
        store.prepared_transactions = prepared_transactions;
        Ok((store, scan))
    }
}
