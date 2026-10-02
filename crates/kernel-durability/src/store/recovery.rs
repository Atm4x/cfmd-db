use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use kernel_change::RevisionEffectId;
use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;
use kernel_types::RevisionId;

use crate::checkpoint;
use crate::replication::authority::ReplicationAuthorityJournal;

use super::causal_ledger;
use super::checkpoint_storage::read_checkpoint_generation;
use super::freshness::{
    ExternalFreshnessAuthority, ExternalFreshnessConfig, ExternalFreshnessState,
};
use super::generation_layout::{
    lock_directory, prepared_capsule_path, remove_orphan_checkpoint_stream_spools, wal_path,
};
use super::manifest::{ManifestRecord, read_current_manifest, read_manifest_generation};
use super::metadata_storage::read_published_metadata;
use super::migration_history::{
    MigrationComplementIndex, append_migration_complement, migration_complement_index,
};
use super::prepared_capsule::{decode_prepared_cut_capsule, read_prepared_cut_capsule_file};
use super::realization_storage::read_published_factorized_realization;
use super::semantic_deployment::{
    install_intent_semantic_modules, install_semantic_module_packages,
};
use super::{DurableRevisionStore, PreparedCutCapsule};
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
};
use crate::domain::{
    DurableMigrationComplement, DurableRevisionEffectRecord, DurableTransactionIntent,
    DurableTransactionKey, HistoricalEpochAnchor, IdempotencyEpoch,
};
use crate::metadata;
use crate::platform_assurance::{
    SupportedDurabilityProfile, VerifiedDestructiveDurabilityCampaignEvidence,
    certify_supported_durability_platform,
};
use crate::runtime::{DurabilityError, RecoveryScan};
use crate::wal::FileRevisionWal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalEpochMaterial {
    generation: u64,
    checkpoint: Revision,
    recovery_scan: RecoveryScan,
    semantic_registry: SemanticRegistry,
}

impl HistoricalEpochMaterial {
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn checkpoint(&self) -> &Revision {
        &self.checkpoint
    }

    #[must_use]
    pub const fn recovery_scan(&self) -> &RecoveryScan {
        &self.recovery_scan
    }

    #[must_use]
    pub const fn semantic_registry(&self) -> &SemanticRegistry {
        &self.semantic_registry
    }
}

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
    historical_epoch_anchors: BTreeMap<RevisionEffectId, HistoricalEpochAnchor>,
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
            checkpoint_realization: None,
            artifact_cores: self.artifact_cores,
            migration_complements: self.migration_complements,
            migration_complement_index: self.migration_complement_index,
            historical_epoch_anchors: self.historical_epoch_anchors,
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
    checkpoint_schema: kernel_types::SchemaRevisionId,
    scan: &RecoveryScan,
) -> Result<(Vec<DurableMigrationComplement>, MigrationComplementIndex), DurabilityError> {
    // The complement chain is historical lineage, not a projection of the
    // current checkpoint schema.  Once a checkpoint has crossed A -> B, the
    // durable chain still starts at A even though the checkpoint itself is B.
    let base_schema = complements
        .first()
        .map_or(checkpoint_schema, |first| first.source_schema);
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

fn scan_published_generation_read_only(
    directory: &Path,
    manifest: ManifestRecord,
    registry: &SemanticRegistry,
) -> Result<(Revision, RecoveryScan), DurabilityError> {
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
    let scan = FileRevisionWal::scan_recovered_seeded_read_only(
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
    Ok((checkpoint, scan))
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
    generation: u64,
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
    let mut historical_epoch_anchors = metadata.historical_epoch_anchors;
    for (effect_id, anchor) in &historical_epoch_anchors {
        let Some(event) = revision_effects
            .get(effect_id)
            .and_then(DurableRevisionEffectRecord::semantic_change_event)
        else {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "historical epoch anchor does not reference a semantic migration effect",
            });
        };
        if anchor.effect_id != *effect_id
            || anchor.source_revision != event.source_revision
            || anchor.source_schema != event.source_schema
            || anchor.generation == 0
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "historical epoch anchor does not match its semantic migration boundary",
            });
        }
    }
    for committed in scan.committed() {
        let effect_id = committed
            .descriptor
            .revision_effect_id
            .unwrap_or(RevisionEffectId(committed.descriptor.transaction_id.raw()));
        let Some(event) = revision_effects
            .get(&effect_id)
            .and_then(DurableRevisionEffectRecord::semantic_change_event)
        else {
            continue;
        };
        historical_epoch_anchors
            .entry(effect_id)
            .or_insert(HistoricalEpochAnchor {
                effect_id,
                source_revision: event.source_revision,
                source_schema: event.source_schema,
                generation,
            });
    }
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
        historical_epoch_anchors,
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
    pub fn historical_epoch_material(
        &mut self,
        effect_id: RevisionEffectId,
    ) -> Result<Option<HistoricalEpochMaterial>, DurabilityError> {
        let Some(anchor) = self.historical_epoch_anchors.get(&effect_id).copied() else {
            return Ok(None);
        };

        let (generation, metadata, checkpoint, scan) = if self.backend.is_single_file() {
            let active_generation = self.generation;
            let container = self.backend.single_file_container()?;
            if anchor.generation == active_generation {
                let view = container.generation_view()?;
                if view.generation != anchor.generation {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "single-file historical epoch anchor generation is not active",
                    });
                }
                let metadata = container
                    .with_section_reader(
                        crate::single_file::SingleFileSectionKind::Metadata,
                        0,
                        |reader, len| {
                            metadata::decode_from_reader(reader, len)
                                .map_err(|reason| DurabilityError::Corruption { offset: 0, reason })
                        },
                    )?
                    .ok_or(DurabilityError::Corruption {
                        offset: 0,
                        reason: "single-file historical metadata section is missing",
                    })?;
                let initial_registry = rebuild_semantic_registry(&metadata, None)?;
                let checkpoint = container
                    .with_section_reader(
                        crate::single_file::SingleFileSectionKind::Checkpoint,
                        0,
                        |reader, len| {
                            checkpoint::decode_revision_from_reader(reader, len, &initial_registry)
                        },
                    )?
                    .ok_or(DurabilityError::Corruption {
                        offset: 0,
                        reason: "single-file historical checkpoint section is missing",
                    })?;
                let prepared = container
                    .read_section(
                        crate::single_file::SingleFileSectionKind::PreparedCapsule,
                        0,
                    )?
                    .map_or_else(
                        || Ok(PreparedCutCapsule::default()),
                        |bytes| decode_prepared_cut_capsule(&bytes),
                    )?;
                let scan = container
                    .scan_active_journal_read_only(checkpoint.id(), &prepared.scan_seeds())?;
                (view.generation, metadata, checkpoint, scan)
            } else {
                let metadata = container
                    .with_historical_epoch_section_reader(
                        anchor.generation,
                        crate::single_file::SingleFileSectionKind::HistoricalMetadata,
                        |reader, len| {
                            metadata::decode_from_reader(reader, len)
                                .map_err(|reason| DurabilityError::Corruption { offset: 0, reason })
                        },
                    )?
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "single-file historical epoch archive is missing metadata authority",
                    })?;
                let initial_registry = rebuild_semantic_registry(&metadata, None)?;
                let checkpoint = container
                    .with_historical_epoch_section_reader(
                        anchor.generation,
                        crate::single_file::SingleFileSectionKind::HistoricalCheckpoint,
                        |reader, len| {
                            checkpoint::decode_revision_from_reader(reader, len, &initial_registry)
                        },
                    )?
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "single-file historical epoch archive is missing checkpoint authority",
                    })?;
                let prepared = container
                    .read_historical_epoch_section(
                        anchor.generation,
                        crate::single_file::SingleFileSectionKind::HistoricalPreparedCapsule,
                    )?
                    .map_or_else(
                        || Ok(PreparedCutCapsule::default()),
                        |bytes| decode_prepared_cut_capsule(&bytes),
                    )?;
                let scan = container
                    .scan_historical_epoch_journal(anchor.generation, &prepared.scan_seeds())?
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "single-file historical epoch archive is missing WAL authority",
                    })?;
                (anchor.generation, metadata, checkpoint, scan)
            }
        } else {
            let directory = self.backend.historical_directory_root()?;
            let manifest = read_manifest_generation(directory, anchor.generation)?;
            let metadata = read_published_metadata(directory, manifest)?;
            let initial_registry = rebuild_semantic_registry(&metadata, None)?;
            let (checkpoint, scan) =
                scan_published_generation_read_only(directory, manifest, &initial_registry)?;
            (manifest.generation, metadata, checkpoint, scan)
        };

        let initial_registry = rebuild_semantic_registry(&metadata, None)?;
        let _canonical =
            recover_canonical_state(metadata, checkpoint.clone(), &scan, generation, None)?;
        let mut historical_registry = initial_registry;
        let mut source_present = anchor.source_revision == checkpoint.id();
        if !source_present {
            for committed in scan.committed() {
                install_intent_semantic_modules(
                    &mut historical_registry,
                    &committed.descriptor.intent,
                )?;
                if committed.descriptor.target_revision == anchor.source_revision {
                    source_present = true;
                    break;
                }
            }
        }
        if !source_present {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "historical epoch anchor source revision is absent from its durable generation",
            });
        }
        Ok(Some(HistoricalEpochMaterial {
            generation,
            checkpoint,
            recovery_scan: scan,
            semantic_registry: historical_registry,
        }))
    }

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
        remove_orphan_checkpoint_stream_spools(&directory)?;
        let manifest = read_current_manifest(&directory)?;
        let metadata = read_published_metadata(&directory, manifest)?;
        let checkpoint_realization_binding = metadata.checkpoint_realization;
        if metadata.external_freshness.is_some() && !allow_external_freshness {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "externally anchored store requires freshness-aware open",
            });
        }
        let registry = rebuild_semantic_registry(&metadata, legacy_registry)?;
        let (checkpoint, wal, scan) = open_published_generation(&directory, manifest, &registry)?;
        let mut canonical = recover_canonical_state(
            metadata,
            checkpoint,
            &scan,
            manifest.generation,
            legacy_registry,
        )?;
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
        store.checkpoint_realization = checkpoint_realization_binding
            .map(|binding| {
                if binding.revision != store.checkpoint.id() {
                    return Err(DurabilityError::Corruption {
                        offset: 0,
                        reason: "published durable realization is bound to a different checkpoint revision",
                    });
                }
                read_published_factorized_realization(
                    store.backend.directory_root()?,
                    manifest.generation,
                    binding,
                )
            })
            .transpose()?;
        store.prepared_transactions = prepared_transactions;
        Ok((store, scan))
    }
}
