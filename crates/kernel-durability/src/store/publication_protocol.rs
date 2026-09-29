use crate::runtime::DurabilityError;
use crate::single_file::compaction_io::SingleFileCompactionIoStep;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoreFaultPoint {
    AfterCheckpointSync,
    AfterWalSync,
    AfterMetadataSync,
    AfterPrerequisiteDirectorySync,
    AfterPendingManifestSync,
    AfterManifestRename,
    AfterManifestDirectorySync,
    BeforeCompactionRemove,
    AfterCompactionRemove,
    AfterCompactionDirectorySync,
    SingleFileCompaction(SingleFileCompactionIoStep),
}

pub(super) trait StoreFaultHook {
    fn hit(&mut self, point: StoreFaultPoint) -> Result<(), DurabilityError>;
}

pub(super) struct NoStoreFault;

impl StoreFaultHook for NoStoreFault {
    fn hit(&mut self, _point: StoreFaultPoint) -> Result<(), DurabilityError> {
        Ok(())
    }
}

#[derive(Debug, Default)]
pub(super) struct PublicationAttempt {
    manifest_authority_may_have_changed: bool,
}

impl PublicationAttempt {
    pub(super) fn mark_manifest_rename_attempted(&mut self) {
        self.manifest_authority_may_have_changed = true;
    }

    pub(super) fn requires_recovery_after_error(&self) -> bool {
        self.manifest_authority_may_have_changed
    }
}
