use kernel_model::Value;
use kernel_types::SemanticId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LensSpecId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticManifestId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArchiveProofId(pub SemanticId);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComplementCapsule {
    pub source_schema: kernel_types::SchemaRevisionId,
    pub target_schema: kernel_types::SchemaRevisionId,
    pub lens_spec: LensSpecId,
    pub semantic_pins: SemanticManifestId,
    pub encoding_version: u32,
    pub complement: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplementRetention {
    Forever,
    UntilRevision(kernel_types::RevisionId),
    UntilEpoch(u64),
    ExternalArchive(ArchiveProofId),
    Forget,
}

impl ComplementRetention {
    #[must_use]
    pub const fn requires_local_storage(self) -> bool {
        matches!(
            self,
            Self::Forever | Self::UntilRevision(_) | Self::UntilEpoch(_)
        )
    }

    #[must_use]
    pub const fn is_explicit_forget(self) -> bool {
        matches!(self, Self::Forget)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MigrationComplementChain {
    steps: Vec<(ComplementCapsule, ComplementRetention)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationChainError {
    SchemaDiscontinuity,
}

impl MigrationComplementChain {
    pub fn push(
        &mut self,
        capsule: ComplementCapsule,
        retention: ComplementRetention,
    ) -> Result<(), MigrationChainError> {
        if let Some((previous, _)) = self.steps.last()
            && previous.target_schema != capsule.source_schema
        {
            return Err(MigrationChainError::SchemaDiscontinuity);
        }
        self.steps.push((capsule, retention));
        Ok(())
    }

    #[must_use]
    pub fn steps(&self) -> &[(ComplementCapsule, ComplementRetention)] {
        &self.steps
    }
}

#[cfg(test)]
mod migration_complement_tests {
    use super::*;
    use kernel_types::{RevisionId, SchemaRevisionId};

    fn capsule(source: u64, target: u64, payload: i64) -> ComplementCapsule {
        ComplementCapsule {
            source_schema: SchemaRevisionId(source),
            target_schema: SchemaRevisionId(target),
            lens_spec: LensSpecId(SemanticId(300)),
            semantic_pins: SemanticManifestId(SemanticId(301)),
            encoding_version: 1,
            complement: Value::I64(payload),
        }
    }

    #[test]
    fn migration_chain_requires_schema_continuity() {
        let mut chain = MigrationComplementChain::default();
        chain
            .push(capsule(1, 2, 10), ComplementRetention::Forever)
            .unwrap();
        assert_eq!(
            chain.push(capsule(3, 4, 20), ComplementRetention::Forever),
            Err(MigrationChainError::SchemaDiscontinuity)
        );
        chain
            .push(
                capsule(2, 3, 30),
                ComplementRetention::UntilRevision(RevisionId(9)),
            )
            .unwrap();
        assert_eq!(chain.steps().len(), 2);
    }

    #[test]
    fn forget_and_external_archive_are_explicit_nonlocal_retention() {
        assert!(ComplementRetention::Forget.is_explicit_forget());
        assert!(!ComplementRetention::Forget.requires_local_storage());
        assert!(
            !ComplementRetention::ExternalArchive(ArchiveProofId(SemanticId(1)))
                .requires_local_storage()
        );
        assert!(ComplementRetention::Forever.requires_local_storage());
    }
}
