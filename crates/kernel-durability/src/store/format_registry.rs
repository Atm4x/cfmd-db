use crate::runtime::{DurabilityError, DurableFormatComponent};

pub(super) const MANIFEST_FORMAT_TAG: u16 = 3;
pub(super) const CHECKPOINT_FORMAT_TAG: u16 = 2;
pub(super) const METADATA_FILE_TAG: u16 = 1;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct DurableFormatRegistry;

impl DurableFormatRegistry {
    pub(super) fn require_manifest(version: u16) -> Result<(), DurabilityError> {
        Self::require(
            DurableFormatComponent::Manifest,
            version,
            &[MANIFEST_FORMAT_TAG],
        )
    }

    pub(super) fn require_checkpoint_current(version: u16) -> Result<(), DurabilityError> {
        Self::require(
            DurableFormatComponent::CheckpointFile,
            version,
            &[CHECKPOINT_FORMAT_TAG],
        )
    }

    pub(super) fn require_metadata(version: u16) -> Result<(), DurabilityError> {
        Self::require(
            DurableFormatComponent::MetadataFile,
            version,
            &[METADATA_FILE_TAG],
        )
    }

    fn require(
        component: DurableFormatComponent,
        version: u16,
        supported: &[u16],
    ) -> Result<(), DurabilityError> {
        if supported.contains(&version) {
            Ok(())
        } else {
            Err(DurabilityError::UnsupportedDurableFormat { component, version })
        }
    }
}
