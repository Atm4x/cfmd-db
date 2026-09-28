use kernel_types::{ClientTransactionId, RevisionId};

use crate::descriptor::DurableRevisionDescriptor;

use super::error::DurabilityError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableTransactionOutcome {
    Unknown,
    RetryHistoryExpired,
    Committed { target_revision: RevisionId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurablePrepareToken {
    transaction_id: ClientTransactionId,
    target_revision: RevisionId,
    prepare_lsn: u64,
    prepare_payload_crc32c: u32,
}

impl DurablePrepareToken {
    pub(crate) const fn from_wal(
        transaction_id: ClientTransactionId,
        target_revision: RevisionId,
        prepare_lsn: u64,
        prepare_payload_crc32c: u32,
    ) -> Self {
        Self {
            transaction_id,
            target_revision,
            prepare_lsn,
            prepare_payload_crc32c,
        }
    }

    #[must_use]
    pub const fn transaction_id(self) -> ClientTransactionId {
        self.transaction_id
    }

    #[must_use]
    pub const fn target_revision(self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub const fn prepare_lsn(self) -> u64 {
        self.prepare_lsn
    }

    pub(crate) const fn prepare_payload_crc32c(self) -> u32 {
        self.prepare_payload_crc32c
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableCommitReceipt {
    target_revision: RevisionId,
    prepare_lsn: u64,
    commit_lsn: u64,
}

impl DurableCommitReceipt {
    pub(crate) const fn from_wal(
        target_revision: RevisionId,
        prepare_lsn: u64,
        commit_lsn: u64,
    ) -> Self {
        Self {
            target_revision,
            prepare_lsn,
            commit_lsn,
        }
    }

    #[must_use]
    pub const fn target_revision(self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub const fn prepare_lsn(self) -> u64 {
        self.prepare_lsn
    }

    #[must_use]
    pub const fn commit_lsn(self) -> u64 {
        self.commit_lsn
    }
}

pub trait RevisionDurability {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError>;

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError>;
}
