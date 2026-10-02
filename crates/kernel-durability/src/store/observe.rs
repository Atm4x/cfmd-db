use std::path::Path;

use kernel_change::RevisionEffectId;
use kernel_realization::{FactorizedRealizationRoot, PhysicalAtomStore};
use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;
use kernel_types::RevisionId;

use super::DurableRevisionStore;
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
};

impl DurableRevisionStore {
    #[must_use]
    pub fn directory(&self) -> &Path {
        self.backend.path()
    }

    #[must_use]
    pub fn wal_path(&self) -> &Path {
        self.wal.path()
    }

    #[must_use]
    pub fn materialization_specs(&self) -> &[DurableMaterializationSpec] {
        &self.materialization_specs
    }

    #[must_use]
    pub fn physical_artifact_specs(&self) -> &[DurablePhysicalArtifactSpec] {
        &self.physical_artifact_specs
    }

    #[must_use]
    pub const fn checkpoint_factorized_realization(
        &self,
    ) -> Option<&crate::realization::DurableFactorizedRealization> {
        self.checkpoint_realization.as_ref()
    }

    #[must_use]
    pub fn historical_factorized_realization_root(
        &self,
        effect_id: RevisionEffectId,
    ) -> Option<(RevisionId, &PhysicalAtomStore, &FactorizedRealizationRoot)> {
        let physical = self.checkpoint_realization.as_ref()?;
        let historical = physical.historical_root(effect_id)?;
        Some((historical.revision(), physical.atoms(), historical.root()))
    }

    #[must_use]
    pub fn historical_factorized_read_snapshot(
        &self,
        revision: RevisionId,
    ) -> Option<crate::DurableFactorizedReadSnapshot> {
        self.checkpoint_realization
            .as_ref()?
            .historical_read_snapshot(revision)
    }

    pub fn historical_revision_from_realization(
        &self,
        effect_id: RevisionEffectId,
    ) -> Result<Option<kernel_revision::Revision>, crate::runtime::DurabilityError> {
        let Some(anchor) = self.historical_epoch_anchors.get(&effect_id) else {
            return Ok(None);
        };
        let Some(physical) = self.checkpoint_realization.as_ref() else {
            return Ok(None);
        };
        let Some(revision) = physical.historical_revision(effect_id, &self.semantic_registry)?
        else {
            return Ok(None);
        };
        if revision.id() != anchor.source_revision
            || revision.semantic_revision().schema != anchor.source_schema
        {
            return Err(crate::runtime::DurabilityError::Protocol {
                offset: 0,
                reason: "historical realization root does not match its causal anchor",
            });
        }
        Ok(Some(revision))
    }

    #[must_use]
    pub fn artifact_cores(&self) -> &[DurableArtifactCore] {
        &self.artifact_cores
    }

    #[must_use]
    pub const fn semantic_registry(&self) -> &SemanticRegistry {
        &self.semantic_registry
    }

    /// Returns whether the in-process store can no longer determine the
    /// authoritative durable generation without reopen/recovery.
    ///
    /// Failures that happen before manifest publication leave this false: the
    /// previous generation remains authoritative and serving may continue.
    #[must_use]
    pub const fn requires_recovery(&self) -> bool {
        self.poisoned
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn checkpoint_revision(&self) -> &Revision {
        &self.checkpoint
    }

    #[must_use]
    pub const fn durable_head(&self) -> RevisionId {
        self.durable_head
    }

    #[must_use]
    pub const fn causal_coverage_root(&self) -> RevisionId {
        self.causal_coverage_root
    }
}
