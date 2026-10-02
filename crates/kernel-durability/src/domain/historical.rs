use std::collections::BTreeMap;

use kernel_change::RevisionEffectId;
use kernel_lens::{
    ArchiveProofId, ComplementCapsule, ComplementRetention, LensSpecId, SemanticManifestId,
};
use kernel_model::Value;
use kernel_types::{ClientTransactionId, RevisionId, SchemaRevisionId, SemanticId};

use super::transaction::IdempotencyEpoch;

/// Authority retained at one semantic schema boundary.
///
/// This is deliberately separate from "undo with a Plan" and from mathematical
/// invertibility of the forward migration transform.  It records where the
/// historical side of the boundary is authoritative.  Full historical world
/// materialization may additionally require an epoch anchor/checkpoint; this
/// enum must therefore never be interpreted as a complete snapshot by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalBoundaryAuthority {
    LocalComplement,
    ExternalArchive(ArchiveProofId),
    ExplicitlyForgotten,
    LocalPayloadReleased,
}

impl HistoricalBoundaryAuthority {
    #[must_use]
    pub const fn retains_history_authority(self) -> bool {
        matches!(self, Self::LocalComplement | Self::ExternalArchive(_))
    }
}

/// First-class semantic history projection for one committed schema migration.
///
/// The event is projected from the existing durable causal effect record; it is
/// not a second history log.  Physical backfill/checkpoint work is intentionally
/// absent because it is storage maintenance, not another semantic database
/// revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticChangeEvent {
    pub effect_id: RevisionEffectId,
    pub transaction_epoch: IdempotencyEpoch,
    pub transaction_id: ClientTransactionId,
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
    pub source_schema: SchemaRevisionId,
    pub target_schema: SchemaRevisionId,
    pub lens_spec: LensSpecId,
    pub semantic_pins: SemanticManifestId,
    pub encoding_version: u32,
    pub historical_authority: HistoricalBoundaryAuthority,
}

/// Physical authority from which the source world of one semantic migration
/// boundary can still be reconstructed.
///
/// The anchor is intentionally keyed by the causal effect rather than by a
/// frontend migration name.  `generation` identifies the durable generation
/// whose checkpoint + WAL prefix contains `source_revision` under
/// `source_schema`.  Compaction may only discard that generation after this
/// authority has been replaced by another certified historical authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoricalEpochAnchor {
    pub effect_id: RevisionEffectId,
    pub source_revision: RevisionId,
    pub source_schema: SchemaRevisionId,
    pub generation: u64,
}

/// Derived physical progress for one already-authoritative schema migration.
///
/// This is intentionally not another persisted state machine.  The state is
/// reconstructed from the causal migration effect, the current checkpoint
/// frontier and the retained source-epoch anchor.  Physical materialization
/// therefore cannot create a second semantic history event or disagree with
/// the durable revision lineage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationPhysicalAuthority {
    /// The semantic target is authoritative in the committed WAL tail while
    /// the active checkpoint still represents a revision before the cutover.
    WalForwardCutover {
        source_generation: u64,
        checkpoint_revision: RevisionId,
    },
    /// The active checkpoint has crossed the migration target, so the current
    /// durable base is natively represented under the target semantics.
    NativeCheckpoint {
        generation: u64,
        checkpoint_revision: RevisionId,
    },
}

impl MigrationPhysicalAuthority {
    #[must_use]
    pub const fn is_pending(self) -> bool {
        matches!(self, Self::WalForwardCutover { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaMigrationPhysicalState {
    pub effect_id: RevisionEffectId,
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
    pub source_schema: SchemaRevisionId,
    pub target_schema: SchemaRevisionId,
    pub authority: MigrationPhysicalAuthority,
}

/// Durable migration-complement step. The chain metadata is retained even
/// after local payload release so historical reversibility loss is explicit.
/// `released=true` is irreversible: local complement bytes can never become
/// authority again merely because a later process happens to have a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableMigrationComplement {
    pub source_schema: kernel_types::SchemaRevisionId,
    pub target_schema: kernel_types::SchemaRevisionId,
    pub lens_spec: LensSpecId,
    pub semantic_pins: SemanticManifestId,
    pub encoding_version: u32,
    pub retention: ComplementRetention,
    pub local_complement: Option<Value>,
    pub released: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalComplementError {
    PathNotFound {
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    },
    ExternalArchiveRequired(ArchiveProofId),
    ExplicitlyForgotten {
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    },
    LocalPayloadReleased {
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalHistoricalComplementChain {
    steps: Vec<DurableMigrationComplement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HistoricalLensImplementationKey {
    pub lens_spec: LensSpecId,
    pub semantic_pins: SemanticManifestId,
    pub encoding_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoricalLensImplementation {
    Identity,
    ProductField { field: SemanticId },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoricalLensRegistry {
    implementations: BTreeMap<HistoricalLensImplementationKey, HistoricalLensImplementation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoricalRestoreError {
    MissingImplementation(HistoricalLensImplementationKey),
    DuplicateImplementation(HistoricalLensImplementationKey),
    ComplementShapeMismatch,
}

impl HistoricalLensRegistry {
    pub fn register(
        &mut self,
        key: HistoricalLensImplementationKey,
        implementation: HistoricalLensImplementation,
    ) -> Result<(), HistoricalRestoreError> {
        if self.implementations.insert(key, implementation).is_some() {
            return Err(HistoricalRestoreError::DuplicateImplementation(key));
        }
        Ok(())
    }

    fn restore_step(
        &self,
        step: &DurableMigrationComplement,
        target: &Value,
    ) -> Result<Value, HistoricalRestoreError> {
        let key = HistoricalLensImplementationKey {
            lens_spec: step.lens_spec,
            semantic_pins: step.semantic_pins,
            encoding_version: step.encoding_version,
        };
        let implementation = self
            .implementations
            .get(&key)
            .ok_or(HistoricalRestoreError::MissingImplementation(key))?;
        let complement = step
            .local_complement
            .as_ref()
            .ok_or(HistoricalRestoreError::ComplementShapeMismatch)?;
        match implementation {
            HistoricalLensImplementation::Identity => {
                if complement == &Value::Unit {
                    Ok(target.clone())
                } else {
                    Err(HistoricalRestoreError::ComplementShapeMismatch)
                }
            }
            HistoricalLensImplementation::ProductField { field } => {
                let Value::Product(remainder) = complement else {
                    return Err(HistoricalRestoreError::ComplementShapeMismatch);
                };
                if remainder.contains_key(field) {
                    return Err(HistoricalRestoreError::ComplementShapeMismatch);
                }
                let mut source = remainder.clone();
                source.insert(*field, target.clone());
                Ok(Value::Product(source))
            }
        }
    }
}

impl LocalHistoricalComplementChain {
    #[must_use]
    pub fn steps(&self) -> &[DurableMigrationComplement] {
        &self.steps
    }

    pub(crate) fn push(&mut self, step: DurableMigrationComplement) {
        self.steps.push(step);
    }

    pub fn restore_value(
        &self,
        target: &Value,
        registry: &HistoricalLensRegistry,
    ) -> Result<Value, HistoricalRestoreError> {
        let mut current = target.clone();
        for step in self.steps.iter().rev() {
            current = registry.restore_step(step, &current)?;
        }
        Ok(current)
    }
}

impl DurableMigrationComplement {
    #[must_use]
    pub fn from_capsule(capsule: ComplementCapsule, retention: ComplementRetention) -> Self {
        let local_required = retention.requires_local_storage();
        Self {
            source_schema: capsule.source_schema,
            target_schema: capsule.target_schema,
            lens_spec: capsule.lens_spec,
            semantic_pins: capsule.semantic_pins,
            encoding_version: capsule.encoding_version,
            retention,
            local_complement: local_required.then_some(capsule.complement),
            released: !local_required,
        }
    }

    #[must_use]
    pub fn local_capsule(&self) -> Option<ComplementCapsule> {
        let complement = (!self.released)
            .then(|| self.local_complement.clone())
            .flatten()?;
        Some(ComplementCapsule {
            source_schema: self.source_schema,
            target_schema: self.target_schema,
            lens_spec: self.lens_spec,
            semantic_pins: self.semantic_pins,
            encoding_version: self.encoding_version,
            complement,
        })
    }

    #[must_use]
    pub const fn archive_proof(&self) -> Option<ArchiveProofId> {
        match self.retention {
            ComplementRetention::ExternalArchive(proof) => Some(proof),
            ComplementRetention::Forever
            | ComplementRetention::UntilRevision(_)
            | ComplementRetention::UntilEpoch(_)
            | ComplementRetention::Forget => None,
        }
    }

    #[must_use]
    pub const fn historical_boundary_authority(&self) -> HistoricalBoundaryAuthority {
        match self.retention {
            ComplementRetention::ExternalArchive(proof) => {
                HistoricalBoundaryAuthority::ExternalArchive(proof)
            }
            ComplementRetention::Forget => HistoricalBoundaryAuthority::ExplicitlyForgotten,
            ComplementRetention::Forever
            | ComplementRetention::UntilRevision(_)
            | ComplementRetention::UntilEpoch(_)
                if self.released =>
            {
                HistoricalBoundaryAuthority::LocalPayloadReleased
            }
            ComplementRetention::Forever
            | ComplementRetention::UntilRevision(_)
            | ComplementRetention::UntilEpoch(_) => HistoricalBoundaryAuthority::LocalComplement,
        }
    }

    pub(crate) fn release_is_due(&self, revision: RevisionId, epoch: u64) -> bool {
        if self.released {
            return false;
        }
        match self.retention {
            ComplementRetention::Forever => false,
            ComplementRetention::UntilRevision(deadline) => deadline == revision,
            ComplementRetention::UntilEpoch(deadline) => epoch >= deadline,
            ComplementRetention::ExternalArchive(_) | ComplementRetention::Forget => true,
        }
    }

    /// Releases local authority only when the declared retention boundary is
    /// explicitly reached. Revision deadlines are matched nominally rather
    /// than ordered numerically; revision IDs are identities, not timestamps.
    pub fn release_if_due(&mut self, revision: RevisionId, epoch: u64) -> bool {
        if !self.release_is_due(revision, epoch) {
            return false;
        }
        self.local_complement = None;
        self.released = true;
        true
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        match self.retention {
            ComplementRetention::Forever if self.released || self.local_complement.is_none() => {
                Err("forever-retained complement lost local payload")
            }
            ComplementRetention::ExternalArchive(_) | ComplementRetention::Forget
                if !self.released || self.local_complement.is_some() =>
            {
                Err("nonlocal complement retention still carries local authority")
            }
            _ if self.released && self.local_complement.is_some() => {
                Err("released complement still carries local payload")
            }
            _ if !self.released && self.local_complement.is_none() => {
                Err("unreleased complement is missing local payload")
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod semantic_change_tests {
    use super::*;
    use kernel_types::{RevisionId, SchemaRevisionId};

    fn capsule() -> ComplementCapsule {
        ComplementCapsule {
            source_schema: SchemaRevisionId::new(380),
            target_schema: SchemaRevisionId::new(381),
            lens_spec: LensSpecId(SemanticId::new(380_381)),
            semantic_pins: SemanticManifestId(SemanticId::new(381_380)),
            encoding_version: 1,
            complement: Value::Unit,
        }
    }

    #[test]
    fn historical_boundary_authority_is_independent_from_transform_invertibility() {
        let forever =
            DurableMigrationComplement::from_capsule(capsule(), ComplementRetention::Forever);
        assert_eq!(
            forever.historical_boundary_authority(),
            HistoricalBoundaryAuthority::LocalComplement
        );
        assert!(
            forever
                .historical_boundary_authority()
                .retains_history_authority()
        );

        let archive = DurableMigrationComplement::from_capsule(
            capsule(),
            ComplementRetention::ExternalArchive(ArchiveProofId(SemanticId::new(9))),
        );
        assert_eq!(
            archive.historical_boundary_authority(),
            HistoricalBoundaryAuthority::ExternalArchive(ArchiveProofId(SemanticId::new(9)))
        );
        assert!(
            archive
                .historical_boundary_authority()
                .retains_history_authority()
        );

        let forgotten =
            DurableMigrationComplement::from_capsule(capsule(), ComplementRetention::Forget);
        assert_eq!(
            forgotten.historical_boundary_authority(),
            HistoricalBoundaryAuthority::ExplicitlyForgotten
        );
        assert!(
            !forgotten
                .historical_boundary_authority()
                .retains_history_authority()
        );

        let mut expiring =
            DurableMigrationComplement::from_capsule(capsule(), ComplementRetention::UntilEpoch(7));
        assert!(expiring.release_if_due(RevisionId::new(0), 7));
        assert_eq!(
            expiring.historical_boundary_authority(),
            HistoricalBoundaryAuthority::LocalPayloadReleased
        );
    }
}
