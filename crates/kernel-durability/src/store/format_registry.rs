use crate::FORMAT_VERSION;
use crate::runtime::{DurabilityError, DurableFormatComponent};

pub(super) const MANIFEST_FORMAT_TAG: u16 = FORMAT_VERSION;
pub(super) const CHECKPOINT_FORMAT_TAG: u16 = FORMAT_VERSION;
pub(super) const METADATA_FILE_TAG: u16 = FORMAT_VERSION;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct DurableFormatRegistry;

impl DurableFormatRegistry {
    pub(super) fn require_manifest(version: u16) -> Result<(), DurabilityError> {
        Self::require(DurableFormatComponent::Manifest, version)
    }

    pub(super) fn require_checkpoint_current(version: u16) -> Result<(), DurabilityError> {
        Self::require(DurableFormatComponent::CheckpointFile, version)
    }

    pub(super) fn require_metadata(version: u16) -> Result<(), DurabilityError> {
        Self::require(DurableFormatComponent::MetadataFile, version)
    }

    fn require(component: DurableFormatComponent, version: u16) -> Result<(), DurabilityError> {
        if version == FORMAT_VERSION {
            Ok(())
        } else {
            Err(DurabilityError::UnsupportedDurableFormat { component, version })
        }
    }
}
