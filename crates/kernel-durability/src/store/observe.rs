use std::path::Path;

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
        &self.directory
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
