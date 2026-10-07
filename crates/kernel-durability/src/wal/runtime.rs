use std::path::Path;

use kernel_auth::AuthorityDigest;
use kernel_types::RevisionId;

use crate::descriptor::DurableRevisionDescriptor;
use crate::runtime::{DurabilityError, DurableCommitReceipt, DurablePrepareToken, RecoveryScan};
use crate::wal_frame::EncodedFrame;
use crate::wal_payload::IntentSealRecord;

use super::{FileRevisionWal, VolatileRevisionWal};

/// Storage-neutral active WAL owner. Semantic store code talks only to this
/// type; filesystem WAL and process-local WAL are physical realizations of the
/// same framed transition protocol.
#[derive(Debug)]
pub(crate) enum RuntimeRevisionWal {
    File(FileRevisionWal),
    Volatile(VolatileRevisionWal),
}

impl From<FileRevisionWal> for RuntimeRevisionWal {
    fn from(value: FileRevisionWal) -> Self {
        Self::File(value)
    }
}

impl RuntimeRevisionWal {
    pub(crate) fn volatile() -> Self {
        Self::Volatile(VolatileRevisionWal::new())
    }

    pub(crate) fn volatile_at_lsn(next_lsn: u64) -> Self {
        Self::Volatile(VolatileRevisionWal::at_lsn(next_lsn))
    }

    pub(crate) fn file_mut(&mut self) -> Result<&mut FileRevisionWal, DurabilityError> {
        match self {
            Self::File(wal) => Ok(wal),
            Self::Volatile(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile WAL cannot satisfy a file-only physical backend operation",
            }),
        }
    }

    pub(crate) fn path(&self) -> Option<&Path> {
        match self {
            Self::File(wal) => Some(wal.path()),
            Self::Volatile(_) => None,
        }
    }

    pub(crate) fn next_lsn(&self) -> u64 {
        match self {
            Self::File(wal) => wal.next_lsn(),
            Self::Volatile(wal) => wal.next_lsn(),
        }
    }

    pub(crate) fn last_lsn(&self) -> u64 {
        match self {
            Self::File(wal) => wal.last_lsn(),
            Self::Volatile(wal) => wal.last_lsn(),
        }
    }

    pub(crate) fn current_end_offset(&mut self) -> Result<u64, DurabilityError> {
        match self {
            Self::File(wal) => wal.current_end_offset(),
            Self::Volatile(wal) => wal.current_end_offset(),
        }
    }

    pub(crate) fn freshness_digest(&self) -> AuthorityDigest {
        match self {
            Self::File(wal) => wal.freshness_digest(),
            Self::Volatile(wal) => wal.freshness_digest(),
        }
    }

    pub(crate) fn volatile_recovery_scan(
        &self,
        base_revision: RevisionId,
    ) -> Result<Option<RecoveryScan>, DurabilityError> {
        match self {
            Self::File(_) => Ok(None),
            Self::Volatile(wal) => wal.recovery_scan(base_revision).map(Some),
        }
    }

    pub(crate) fn scan_subregion_seeded(
        &self,
        start_offset: u64,
        end_offset: u64,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<RecoveryScan, DurabilityError> {
        match self {
            Self::File(wal) => wal.scan_subregion_seeded(
                start_offset,
                end_offset,
                base_revision,
                first_lsn,
                seeded_prepares,
            ),
            Self::Volatile(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile WAL has no physical subregion scan",
            }),
        }
    }

    pub(crate) fn append_replication_authority_frame(
        &mut self,
        frame: &[u8],
    ) -> Result<(), DurabilityError> {
        match self {
            Self::File(wal) => wal.append_replication_authority_frame(frame),
            Self::Volatile(wal) => wal.append_replication_authority_frame(frame),
        }
    }

    pub(crate) fn append_intent_seal_unflushed(
        &mut self,
        record: &IntentSealRecord,
    ) -> Result<EncodedFrame, DurabilityError> {
        match self {
            Self::File(wal) => wal.append_intent_seal_unflushed(record),
            Self::Volatile(wal) => wal.append_intent_seal_unflushed(record),
        }
    }

    #[cfg(test)]
    pub(crate) fn append_prepare_unflushed(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        self.append_prepare_unflushed_with_frame(descriptor)
            .map(|(token, _)| token)
    }

    pub(crate) fn append_prepare_unflushed_with_frame(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<(DurablePrepareToken, EncodedFrame), DurabilityError> {
        match self {
            Self::File(wal) => wal.append_prepare_unflushed_with_frame(descriptor),
            Self::Volatile(wal) => wal.append_prepare_unflushed_with_frame(descriptor),
        }
    }

    #[cfg(test)]
    pub(crate) fn append_commit_unflushed(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        self.append_commit_unflushed_with_frame(prepared)
            .map(|(receipt, _)| receipt)
    }

    pub(crate) fn append_commit_unflushed_with_frame(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<(DurableCommitReceipt, EncodedFrame), DurabilityError> {
        match self {
            Self::File(wal) => wal.append_commit_unflushed_with_frame(prepared),
            Self::Volatile(wal) => wal.append_commit_unflushed_with_frame(prepared),
        }
    }

    pub(crate) fn durability_barrier(&mut self) -> Result<(), DurabilityError> {
        match self {
            Self::File(wal) => wal.durability_barrier(),
            Self::Volatile(wal) => wal.durability_barrier(),
        }
    }
}
