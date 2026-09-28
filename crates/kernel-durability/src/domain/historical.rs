use std::collections::BTreeMap;

use kernel_lens::{
    ArchiveProofId, ComplementCapsule, ComplementRetention, LensSpecId, SemanticManifestId,
};
use kernel_model::Value;
use kernel_types::{RevisionId, SemanticId};

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
