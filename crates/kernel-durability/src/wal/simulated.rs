use kernel_types::RevisionId;

use crate::descriptor::DurableRevisionDescriptor;
use crate::runtime::{
    DurabilityError, DurableCommitReceipt, DurablePrepareToken, RevisionDurability,
};
use crate::wal_frame::{EncodedFrame, RecordKind, encode_frame};
use crate::wal_payload::{CommitRecord, encode_commit_payload, encode_prepare_payload};

#[derive(Debug, Default)]
pub struct SimulatedRevisionWal {
    bytes: Vec<u8>,
    durable_len: usize,
    next_lsn: u64,
}

impl SimulatedRevisionWal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            durable_len: 0,
            next_lsn: 1,
        }
    }

    #[must_use]
    pub fn crash_image(&self) -> &[u8] {
        &self.bytes[..self.durable_len]
    }

    #[must_use]
    pub fn volatile_image(&self) -> &[u8] {
        &self.bytes
    }

    fn append_frame(
        &mut self,
        kind: RecordKind,
        revision: RevisionId,
        payload: &[u8],
    ) -> Result<EncodedFrame, DurabilityError> {
        let lsn = self.next_lsn;
        self.next_lsn = lsn.checked_add(1).ok_or(DurabilityError::LsnExhausted)?;
        let frame = encode_frame(lsn, kind, revision, payload)?;
        self.bytes.extend_from_slice(&frame.bytes);
        Ok(frame)
    }

    fn barrier(&mut self) {
        self.durable_len = self.bytes.len();
    }
}

impl RevisionDurability for SimulatedRevisionWal {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        let payload = encode_prepare_payload(descriptor)?;
        let frame = self.append_frame(
            RecordKind::PrepareRevision,
            descriptor.target_revision,
            &payload,
        )?;
        self.barrier();
        Ok(DurablePrepareToken::from_wal(
            descriptor.transaction_id,
            descriptor.target_revision,
            frame.lsn,
            frame.payload_crc32c,
        ))
    }

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        let record = CommitRecord {
            target_revision: prepared.target_revision(),
            prepare_lsn: prepared.prepare_lsn(),
            prepare_payload_crc32c: prepared.prepare_payload_crc32c(),
        };
        let payload = encode_commit_payload(&record);
        let frame = self.append_frame(
            RecordKind::CommitRevision,
            prepared.target_revision(),
            &payload,
        )?;
        self.barrier();
        Ok(DurableCommitReceipt::from_wal(
            prepared.target_revision(),
            prepared.prepare_lsn(),
            frame.lsn,
        ))
    }
}
