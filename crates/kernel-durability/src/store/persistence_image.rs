use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use kernel_change::RevisionEffectId;
use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;
use kernel_types::RevisionId;

use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
};
use crate::domain::{
    DurableCommittedTransaction, DurableExternalFreshnessBinding, DurableMigrationComplement,
    DurableRevisionEffectRecord, DurableTransactionKey, HistoricalEpochAnchor, IdempotencyEpoch,
};
use crate::runtime::{DurabilityError, RecoveryScan};
use crate::storage_encryption::{StorageEncryption, StorageProtectionProfile};

use super::prepared_capsule::PreparedCutCapsule;
use super::prepared_lifecycle::PreparedTransactionLedger;
use super::{DurableRevisionStore, ExternalFreshnessState};

/// Canonical source-independent persistence bootstrap image.
///
/// This is deliberately a durability-domain object rather than a logical export.
/// Building or publishing the image does not create a new semantic revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalPersistenceImage {
    current_revision: Revision,
    semantic_registry: SemanticRegistry,
    materialization_specs: Vec<DurableMaterializationSpec>,
    physical_artifact_specs: Vec<DurablePhysicalArtifactSpec>,
    artifact_cores: Vec<DurableArtifactCore>,
    physical_realization: Option<crate::DurableFactorizedRealization>,
    migration_complements: Vec<DurableMigrationComplement>,
    current_idempotency_epoch: IdempotencyEpoch,
    minimum_retry_epoch: IdempotencyEpoch,
    committed_transactions: BTreeMap<DurableTransactionKey, DurableCommittedTransaction>,
    prepared_capsule: PreparedCutCapsule,
    replication_snapshot: crate::replication::authority::ReplicationAuthoritySemanticSnapshot,
    historical_epoch_anchors: BTreeMap<RevisionEffectId, HistoricalEpochAnchor>,
    portable_historical_epochs: BTreeMap<RevisionEffectId, Vec<u8>>,
    next_revision_effect_id: u128,
    causal_coverage_root: RevisionId,
    revision_effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    revision_effect_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    external_freshness_handoff: Option<super::ExternalFreshnessHandoff>,
    persistence_protection_floor: StorageProtectionProfile,
}

/// Canonical live-fork image.
///
/// Unlike `CanonicalPersistenceImage`, this object deliberately excludes retry, prepared,
/// replication and external-freshness authority. It carries only the semantic/history closure
/// required for a second independently live database lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkPersistenceImage {
    current_revision: Revision,
    semantic_registry: SemanticRegistry,
    materialization_specs: Vec<DurableMaterializationSpec>,
    physical_artifact_specs: Vec<DurablePhysicalArtifactSpec>,
    artifact_cores: Vec<DurableArtifactCore>,
    physical_realization: Option<crate::DurableFactorizedRealization>,
    migration_complements: Vec<DurableMigrationComplement>,
    historical_epoch_anchors: BTreeMap<RevisionEffectId, HistoricalEpochAnchor>,
    portable_historical_epochs: BTreeMap<RevisionEffectId, Vec<u8>>,
    next_revision_effect_id: u128,
    causal_coverage_root: RevisionId,
    revision_effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    revision_effect_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    source_external_freshness_store_id: Option<[u8; 32]>,
    persistence_protection_floor: StorageProtectionProfile,
}

impl ForkPersistenceImage {
    #[must_use]
    pub const fn current_revision(&self) -> &Revision {
        &self.current_revision
    }
}

impl CanonicalPersistenceImage {
    #[must_use]
    pub const fn current_revision(&self) -> &Revision {
        &self.current_revision
    }

    #[must_use]
    pub const fn causal_coverage_root(&self) -> RevisionId {
        self.causal_coverage_root
    }

    #[must_use]
    pub fn retained_effect_count(&self) -> usize {
        self.revision_effects.len()
    }

    #[must_use]
    pub fn committed_retry_count(&self) -> usize {
        self.committed_transactions.len()
    }
}

fn verify_persistence_protection_floor(
    floor: StorageProtectionProfile,
    encryption: &StorageEncryption,
) -> Result<(), DurabilityError> {
    if floor.accepts_target(encryption.protection_profile()) {
        Ok(())
    } else {
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "persistence target weakens source at-rest protection authority",
        })
    }
}

fn verify_persistence_protection(
    image: &CanonicalPersistenceImage,
    encryption: &StorageEncryption,
) -> Result<(), DurabilityError> {
    verify_persistence_protection_floor(image.persistence_protection_floor, encryption)
}

fn install_persistence_image_authority(
    staged: &mut DurableRevisionStore,
    image: &CanonicalPersistenceImage,
) -> Result<(), DurabilityError> {
    staged
        .migration_complements
        .clone_from(&image.migration_complements);
    staged
        .historical_epoch_anchors
        .clone_from(&image.historical_epoch_anchors);
    let migration_base_schema = staged
        .migration_complements
        .first()
        .map_or(image.current_revision.semantic_revision().schema, |first| {
            first.source_schema
        });
    staged.migration_complement_index = super::migration_history::migration_complement_index(
        &staged.migration_complements,
        migration_base_schema,
    )?;
    staged.current_idempotency_epoch = image.current_idempotency_epoch;
    staged.minimum_retry_epoch = image.minimum_retry_epoch;
    staged
        .committed_transactions
        .clone_from(&image.committed_transactions);
    staged.prepared_transactions =
        PreparedTransactionLedger::from_portable_seeds(&image.prepared_capsule.scan_seeds());
    image
        .replication_snapshot
        .install_without_physical_carrier(&mut staged.replication)?;
    staged.next_revision_effect_id = image.next_revision_effect_id;
    staged.causal_coverage_root = image.causal_coverage_root;
    staged.revision_effects.clone_from(&image.revision_effects);
    staged
        .revision_effect_frontiers
        .clone_from(&image.revision_effect_frontiers);
    Ok(())
}

fn verify_staged_persistence_image(
    reopened: &mut DurableRevisionStore,
    scan: &RecoveryScan,
    image: &CanonicalPersistenceImage,
) -> Result<(), DurabilityError> {
    if reopened.durable_head != image.current_revision.id()
        || reopened.checkpoint.id() != image.current_revision.id()
        || reopened.causal_coverage_root != image.causal_coverage_root
        || reopened.current_idempotency_epoch != image.current_idempotency_epoch
        || reopened.minimum_retry_epoch != image.minimum_retry_epoch
        || reopened.committed_transactions != image.committed_transactions
        || !image.prepared_capsule.matches_recovery_scan(scan)
        || crate::replication::authority::ReplicationAuthoritySemanticSnapshot::capture(
            &reopened.replication,
        ) != image.replication_snapshot
        || reopened.historical_epoch_anchors != image.historical_epoch_anchors
        || reopened.revision_effects != image.revision_effects
        || reopened.revision_effect_frontiers != image.revision_effect_frontiers
        || reopened.checkpoint_factorized_realization() != image.physical_realization.as_ref()
        || scan.durable_revision() != image.current_revision.id()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "staged persistence image failed exact reopen verification",
        });
    }
    for (effect_id, encoded) in &image.portable_historical_epochs {
        let expected =
            super::portable_history::PortableHistoricalEpochClosure::decode(encoded)?.material()?;
        let actual =
            reopened
                .historical_epoch_material(*effect_id)?
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "staged persistence image lost retained historical authority",
                })?;
        if actual != expected {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "staged persistence image changed retained historical authority",
            });
        }
    }
    reopened.prepared_transactions =
        PreparedTransactionLedger::from_portable_seeds(&image.prepared_capsule.scan_seeds());
    Ok(())
}

fn install_fork_image_semantic_history(
    staged: &mut DurableRevisionStore,
    image: &ForkPersistenceImage,
) -> Result<(), DurabilityError> {
    staged
        .migration_complements
        .clone_from(&image.migration_complements);
    staged
        .historical_epoch_anchors
        .clone_from(&image.historical_epoch_anchors);
    let migration_base_schema = staged
        .migration_complements
        .first()
        .map_or(image.current_revision.semantic_revision().schema, |first| {
            first.source_schema
        });
    staged.migration_complement_index = super::migration_history::migration_complement_index(
        &staged.migration_complements,
        migration_base_schema,
    )?;
    staged.next_revision_effect_id = image.next_revision_effect_id;
    staged.causal_coverage_root = image.causal_coverage_root;
    staged.revision_effects.clone_from(&image.revision_effects);
    staged
        .revision_effect_frontiers
        .clone_from(&image.revision_effect_frontiers);
    Ok(())
}

fn verify_staged_fork_image(
    reopened: &mut DurableRevisionStore,
    scan: &RecoveryScan,
    image: &ForkPersistenceImage,
) -> Result<(), DurabilityError> {
    if reopened.durable_head != image.current_revision.id()
        || reopened.checkpoint.id() != image.current_revision.id()
        || reopened.causal_coverage_root != image.causal_coverage_root
        || reopened.migration_complements != image.migration_complements
        || reopened.historical_epoch_anchors != image.historical_epoch_anchors
        || reopened.revision_effects != image.revision_effects
        || reopened.revision_effect_frontiers != image.revision_effect_frontiers
        || reopened.current_idempotency_epoch != IdempotencyEpoch::ZERO
        || reopened.minimum_retry_epoch != IdempotencyEpoch::ZERO
        || !reopened.committed_transactions.is_empty()
        || !reopened.prepared_transactions.is_empty()
        || reopened.current_replication_membership().is_some()
        || scan.durable_revision() != image.current_revision.id()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "staged live fork failed semantic/history preservation or operational re-foundation verification",
        });
    }
    for (effect_id, encoded) in &image.portable_historical_epochs {
        let expected =
            super::portable_history::PortableHistoricalEpochClosure::decode(encoded)?.material()?;
        let actual =
            reopened
                .historical_epoch_material(*effect_id)?
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "staged live fork lost retained historical authority",
                })?;
        if actual != expected {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "staged live fork changed retained historical authority",
            });
        }
    }
    Ok(())
}

impl DurableRevisionStore {
    /// Captures the canonical authority required to bootstrap another durable
    /// store at the *same* semantic revision.
    ///
    /// Portable durability authority is captured exactly. Retained historical
    /// epoch archives and external freshness remain explicit non-portable
    /// blockers; active streaming checkpoint work is not authority and is not
    /// carried.
    pub fn canonical_persistence_image(
        &mut self,
        current_revision: &Revision,
    ) -> Result<CanonicalPersistenceImage, DurabilityError> {
        if current_revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "persistence image revision does not match durable head",
            });
        }
        let prepared_capsule = PreparedCutCapsule::from_prepared_transactions(
            &self.prepared_transactions,
            self.durable_head,
        );
        let replication_snapshot =
            crate::replication::authority::ReplicationAuthoritySemanticSnapshot::capture(
                &self.replication,
            );
        let external_freshness_handoff = self
            .external_freshness
            .as_ref()
            .map(ExternalFreshnessState::handoff)
            .transpose()?;
        let historical_epoch_anchors = self.historical_epoch_anchors.clone();
        let mut portable_historical_epochs = BTreeMap::new();
        for effect_id in historical_epoch_anchors.keys().copied() {
            let material =
                self.historical_epoch_material(effect_id)?
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "retained historical epoch anchor has no materialization authority",
                    })?;
            let closure = super::portable_history::PortableHistoricalEpochClosure::from_material(
                effect_id, &material,
            );
            portable_historical_epochs.insert(effect_id, closure.encode()?);
        }

        Ok(CanonicalPersistenceImage {
            current_revision: current_revision.clone(),
            semantic_registry: self.semantic_registry.clone(),
            materialization_specs: self.materialization_specs.clone(),
            physical_artifact_specs: self.physical_artifact_specs.clone(),
            artifact_cores: self.artifact_cores.clone(),
            physical_realization: self
                .checkpoint_realization
                .as_ref()
                .filter(|realization| realization.revision() == current_revision.id())
                .cloned(),
            migration_complements: self.migration_complements.clone(),
            current_idempotency_epoch: self.current_idempotency_epoch,
            minimum_retry_epoch: self.minimum_retry_epoch,
            committed_transactions: self.committed_transactions.clone(),
            prepared_capsule,
            replication_snapshot,
            historical_epoch_anchors,
            portable_historical_epochs,
            next_revision_effect_id: self.next_revision_effect_id,
            causal_coverage_root: self.causal_coverage_root,
            revision_effects: self.revision_effects.clone(),
            revision_effect_frontiers: self.revision_effect_frontiers.clone(),
            external_freshness_handoff,
            persistence_protection_floor: self.backend.protection_profile(),
        })
    }

    /// Captures the exact semantic/history closure for a concurrently live fork.
    /// Operational authority is intentionally not captured: retry/idempotency, unresolved
    /// prepares, replication authority and external freshness belong to one live database root.
    pub fn fork_persistence_image(
        &mut self,
        current_revision: &Revision,
    ) -> Result<ForkPersistenceImage, DurabilityError> {
        if current_revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "fork image revision does not match durable head",
            });
        }
        let historical_epoch_anchors = self.historical_epoch_anchors.clone();
        let mut portable_historical_epochs = BTreeMap::new();
        for effect_id in historical_epoch_anchors.keys().copied() {
            let material =
                self.historical_epoch_material(effect_id)?
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "retained historical epoch anchor has no materialization authority",
                    })?;
            let closure = super::portable_history::PortableHistoricalEpochClosure::from_material(
                effect_id, &material,
            );
            portable_historical_epochs.insert(effect_id, closure.encode()?);
        }
        Ok(ForkPersistenceImage {
            current_revision: current_revision.clone(),
            semantic_registry: self.semantic_registry.clone(),
            materialization_specs: self.materialization_specs.clone(),
            physical_artifact_specs: self.physical_artifact_specs.clone(),
            artifact_cores: self.artifact_cores.clone(),
            physical_realization: self
                .checkpoint_realization
                .as_ref()
                .filter(|realization| realization.revision() == current_revision.id())
                .cloned(),
            migration_complements: self.migration_complements.clone(),
            historical_epoch_anchors,
            portable_historical_epochs,
            next_revision_effect_id: self.next_revision_effect_id,
            causal_coverage_root: self.causal_coverage_root,
            revision_effects: self.revision_effects.clone(),
            revision_effect_frontiers: self.revision_effect_frontiers.clone(),
            source_external_freshness_store_id: self
                .external_freshness
                .as_ref()
                .map(|freshness| freshness.metadata_binding().store_id),
            persistence_protection_floor: self.backend.protection_profile(),
        })
    }

    /// Stages one independently live single-file fork. Semantic/history state is preserved,
    /// while retry, prepared and replication authority are re-founded by ordinary store bootstrap.
    /// Externally anchored sources remain fail-closed until independent target-freshness bootstrap
    /// has its own theorem; duplicating or silently dropping that trust floor is forbidden.
    pub fn stage_single_file_from_fork_image(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
        image: &ForkPersistenceImage,
    ) -> Result<Self, DurabilityError> {
        if image.source_external_freshness_store_id.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "live fork from externally anchored source requires independent target freshness bootstrap",
            });
        }
        verify_persistence_protection_floor(image.persistence_protection_floor, encryption)?;
        let path = path.as_ref();
        let mut staged = Self::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores(
            path,
            encryption,
            &image.current_revision,
            &image.materialization_specs,
            &image.physical_artifact_specs,
            &image.artifact_cores,
            &image.semantic_registry,
        )?;
        install_fork_image_semantic_history(&mut staged, image)?;
        staged.rotate_single_file_checkpoint_with_portable_history(
            super::single_file_backend::SingleFileCheckpointAuthority {
                revision: &image.current_revision,
                materialization_specs: &image.materialization_specs,
                physical_artifact_specs: &image.physical_artifact_specs,
                artifact_cores: &image.artifact_cores,
                physical_realization: image.physical_realization.as_ref(),
                portable_historical_epochs: &image.portable_historical_epochs,
            },
        )?;
        drop(staged);

        let (mut reopened, scan) = Self::open_single_file_with_encryption(path, encryption)?;
        verify_staged_fork_image(&mut reopened, &scan, image)?;
        Ok(reopened)
    }

    /// Stages a live fork whose target has a newly bootstrapped external-freshness root.
    /// The source anchor is never rebound or copied. The first recoverable target generation is
    /// already freshness-bound, so a crash before the first external cut is published leaves a
    /// fail-closed target rather than an ordinary downgraded database.
    pub fn stage_single_file_from_fork_image_with_external_freshness_bootstrap(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
        image: &ForkPersistenceImage,
        target_config: super::ExternalFreshnessConfig,
        authority: Box<dyn super::ExternalFreshnessAuthority>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        if image
            .source_external_freshness_store_id
            .is_some_and(|source_store_id| source_store_id == target_config.store_id)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "live fork freshness bootstrap requires a distinct target store id",
            });
        }
        verify_persistence_protection_floor(image.persistence_protection_floor, encryption)?;
        let path = path.as_ref();
        if path.exists() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "live fork freshness bootstrap target already exists",
            });
        }

        let freshness = ExternalFreshnessState::prepare_bootstrap_target(target_config, authority)?;
        let binding = freshness.metadata_binding();
        let mut staged = Self::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores_and_freshness_binding(
            path,
            encryption,
            &image.current_revision,
            &image.materialization_specs,
            &image.physical_artifact_specs,
            &image.artifact_cores,
            &image.semantic_registry,
            Some(binding),
        )?;
        install_fork_image_semantic_history(&mut staged, image)?;
        staged.rotate_single_file_checkpoint_with_portable_history_sealed_freshness(
            super::single_file_backend::SingleFileCheckpointAuthority {
                revision: &image.current_revision,
                materialization_specs: &image.materialization_specs,
                physical_artifact_specs: &image.physical_artifact_specs,
                artifact_cores: &image.artifact_cores,
                physical_realization: image.physical_realization.as_ref(),
                portable_historical_epochs: &image.portable_historical_epochs,
            },
            binding,
        )?;
        drop(staged);

        let (mut reopened, scan) =
            Self::open_single_file_sealed_external_freshness_staging(path, encryption)?;
        verify_staged_fork_image(&mut reopened, &scan, image)?;
        reopened.external_freshness = Some(freshness);
        let generation = reopened.generation;
        let wal_lsn = reopened.wal.last_lsn();
        let wal_digest = reopened.wal.freshness_digest();
        reopened
            .advance_external_freshness_generation_with_digest(generation, wal_lsn, wal_digest)?;
        Ok((reopened, scan))
    }

    /// Stages one ordinary single-file store from a canonical persistence
    /// image, reopens it, and verifies the authority cut before returning it.
    /// The semantic revision is unchanged; this is durability realization.
    pub fn stage_single_file_from_persistence_image(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
        image: &CanonicalPersistenceImage,
    ) -> Result<Self, DurabilityError> {
        if image.external_freshness_handoff.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "persistence image with external freshness requires explicit trust-authority rebind",
            });
        }
        verify_persistence_protection(image, encryption)?;
        let path = path.as_ref();
        let mut staged = Self::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores(
            path,
            encryption,
            &image.current_revision,
            &image.materialization_specs,
            &image.physical_artifact_specs,
            &image.artifact_cores,
            &image.semantic_registry,
        )?;
        install_persistence_image_authority(&mut staged, image)?;
        staged.rotate_single_file_checkpoint_with_semantic_base(
            super::single_file_backend::SingleFileCheckpointAuthority {
                revision: &image.current_revision,
                materialization_specs: &image.materialization_specs,
                physical_artifact_specs: &image.physical_artifact_specs,
                artifact_cores: &image.artifact_cores,
                physical_realization: image.physical_realization.as_ref(),
                portable_historical_epochs: &image.portable_historical_epochs,
            },
            &image.replication_snapshot,
        )?;
        drop(staged);

        let (mut reopened, scan) = Self::open_single_file_with_encryption(path, encryption)?;
        verify_staged_persistence_image(&mut reopened, &scan, image)?;
        Ok(reopened)
    }

    /// Re-realizes the current volatile authority as one verified single-file
    /// durable owner. An external `VolatileFence`, when present, is consumed
    /// only after the staged bytes have been reopened and verified.
    pub fn repersist_volatile_to_single_file(
        &mut self,
        current_revision: &Revision,
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
    ) -> Result<Self, DurabilityError> {
        if !self.backend.is_volatile() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "repersistence requires a volatile source authority",
            });
        }
        let image = self.canonical_persistence_image(current_revision)?;
        let Some(handoff) = image.external_freshness_handoff else {
            let target = Self::stage_single_file_from_persistence_image(path, encryption, &image)?;
            self.poisoned = true;
            return Ok(target);
        };
        if handoff.kind != super::freshness::ExternalFreshnessHandoffKind::VolatileFence {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile source carries a non-fenced external freshness authority",
            });
        }

        verify_persistence_protection(&image, encryption)?;
        let path = path.as_ref();
        if path.exists() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "repersistence target already exists",
            });
        }
        let binding = self
            .external_freshness
            .as_ref()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile freshness handoff has no live authority owner",
            })?
            .metadata_binding();
        if binding.store_id != handoff.source_store_id
            || binding.previous_generation_digest != Some(handoff.source_generation_digest)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile freshness handoff does not match live lineage authority",
            });
        }

        let mut staged = Self::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores_and_freshness_binding(
            path,
            encryption,
            &image.current_revision,
            &image.materialization_specs,
            &image.physical_artifact_specs,
            &image.artifact_cores,
            &image.semantic_registry,
            Some(binding),
        )?;
        install_persistence_image_authority(&mut staged, &image)?;
        staged.rotate_single_file_checkpoint_with_semantic_base_sealed_freshness(
            super::single_file_backend::SingleFileCheckpointAuthority {
                revision: &image.current_revision,
                materialization_specs: &image.materialization_specs,
                physical_artifact_specs: &image.physical_artifact_specs,
                artifact_cores: &image.artifact_cores,
                physical_realization: image.physical_realization.as_ref(),
                portable_historical_epochs: &image.portable_historical_epochs,
            },
            binding,
            &image.replication_snapshot,
        )?;
        drop(staged);

        let (mut reopened, scan) =
            Self::open_single_file_sealed_external_freshness_staging(path, encryption)?;
        verify_staged_persistence_image(&mut reopened, &scan, &image)?;
        let freshness = self
            .external_freshness
            .take()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile freshness authority disappeared during repersistence",
            })?;
        reopened.external_freshness = Some(freshness);
        let generation = reopened.generation;
        let wal_lsn = reopened.wal.last_lsn();
        let wal_digest = reopened.wal.freshness_digest();
        if let Err(error) = reopened
            .advance_external_freshness_generation_with_digest(generation, wal_lsn, wal_digest)
        {
            let mut freshness = reopened
                .external_freshness
                .take()
                .expect("repersistence keeps freshness owner until durable-cut publication");
            let source_still_authoritative = freshness.remote_matches_current().unwrap_or(false);
            self.external_freshness = Some(freshness);
            if source_still_authoritative {
                drop(reopened);
                let _ = std::fs::remove_file(path);
            } else {
                self.poisoned = true;
            }
            return Err(error);
        }
        self.poisoned = true;
        Ok(reopened)
    }

    /// Strictly verifies a backup artifact produced from a canonical persistence image.
    ///
    /// Backups are quiescent single-file authorities: their checkpoint is the
    /// current durable head and their WAL tail must be clean. Recoverable live
    ///-database tail truncation is deliberately not accepted as a valid backup.
    pub fn verify_single_file_backup(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
    ) -> Result<RevisionId, DurabilityError> {
        let (store, scan) = Self::open_single_file_with_encryption(path, encryption)?;
        if !matches!(scan.tail_status(), crate::runtime::TailStatus::Clean) {
            return Err(DurabilityError::Protocol {
                offset: scan.last_good_offset(),
                reason: "backup artifact has a truncated or garbage WAL tail",
            });
        }
        if store.checkpoint_revision().id() != store.durable_head()
            || scan.durable_revision() != store.durable_head()
            || !scan.committed().is_empty()
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "backup artifact is not a quiescent canonical authority cut",
            });
        }
        Ok(store.durable_head())
    }

    /// Restores a verified backup into a fresh single-file target.
    ///
    /// Restore re-materializes the same canonical authority cut instead of
    /// copying bytes. This preserves the protection floor and rejects a weaker
    /// target through the same persistence law used by Volatile -> Durable.
    pub fn restore_single_file_backup(
        backup_path: impl AsRef<Path>,
        backup_encryption: &StorageEncryption,
        target_path: impl AsRef<Path>,
        target_encryption: &StorageEncryption,
    ) -> Result<Self, DurabilityError> {
        let backup_path = backup_path.as_ref();
        let target_path = target_path.as_ref();
        if target_path.exists() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "fresh restore target already exists",
            });
        }
        let (mut backup, scan) =
            Self::open_single_file_with_encryption(backup_path, backup_encryption)?;
        if !matches!(scan.tail_status(), crate::runtime::TailStatus::Clean)
            || backup.checkpoint_revision().id() != backup.durable_head()
            || scan.durable_revision() != backup.durable_head()
            || !scan.committed().is_empty()
        {
            return Err(DurabilityError::Protocol {
                offset: scan.last_good_offset(),
                reason: "backup artifact is not a verified quiescent authority cut",
            });
        }
        let revision = backup.checkpoint_revision().clone();
        let image = backup.canonical_persistence_image(&revision)?;
        Self::stage_single_file_from_persistence_image(target_path, target_encryption, &image)
    }

    pub fn stage_single_file_from_persistence_image_with_external_freshness_rebind(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
        image: &CanonicalPersistenceImage,
        target_config: super::ExternalFreshnessConfig,
        authority: Box<dyn super::ExternalFreshnessAuthority>,
    ) -> Result<Self, DurabilityError> {
        let source = image
            .external_freshness_handoff
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "persistence image has no external freshness authority to rebind",
            })?;
        verify_persistence_protection(image, encryption)?;
        let path = path.as_ref();
        if path.exists() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness transfer target already exists",
            });
        }

        let freshness =
            ExternalFreshnessState::prepare_rebind_target(target_config, source, authority)?;
        let binding: DurableExternalFreshnessBinding = freshness.metadata_binding();
        let mut staged = Self::create_single_file_with_encryption_and_materializations_physical_artifacts_and_cores_and_freshness_binding(
            path,
            encryption,
            &image.current_revision,
            &image.materialization_specs,
            &image.physical_artifact_specs,
            &image.artifact_cores,
            &image.semantic_registry,
            Some(binding),
        )?;
        install_persistence_image_authority(&mut staged, image)?;
        staged.rotate_single_file_checkpoint_with_semantic_base_sealed_freshness(
            super::single_file_backend::SingleFileCheckpointAuthority {
                revision: &image.current_revision,
                materialization_specs: &image.materialization_specs,
                physical_artifact_specs: &image.physical_artifact_specs,
                artifact_cores: &image.artifact_cores,
                physical_realization: image.physical_realization.as_ref(),
                portable_historical_epochs: &image.portable_historical_epochs,
            },
            binding,
            &image.replication_snapshot,
        )?;
        drop(staged);

        let (mut reopened, scan) =
            Self::open_single_file_sealed_external_freshness_staging(path, encryption)?;
        verify_staged_persistence_image(&mut reopened, &scan, image)?;
        reopened.external_freshness = Some(freshness);
        let generation = reopened.generation;
        let wal_lsn = reopened.wal.last_lsn();
        let wal_digest = reopened.wal.freshness_digest();
        reopened
            .advance_external_freshness_generation_with_digest(generation, wal_lsn, wal_digest)?;
        Ok(reopened)
    }

    pub fn transfer_external_freshness_to_single_file(
        &mut self,
        current_revision: &Revision,
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
        target_config: super::ExternalFreshnessConfig,
        authority: Box<dyn super::ExternalFreshnessAuthority>,
    ) -> Result<Self, DurabilityError> {
        let image = self.canonical_persistence_image(current_revision)?;
        let target = Self::stage_single_file_from_persistence_image_with_external_freshness_rebind(
            path,
            encryption,
            &image,
            target_config,
            authority,
        )?;
        self.poisoned = true;
        Ok(target)
    }
}
