use kernel_auth::AuthorityDigest;
use kernel_types::RevisionId;
use sha2::{Digest, Sha256};

use crate::descriptor::DurableRevisionDescriptor;
use crate::runtime::{DurabilityError, DurableCommitReceipt, DurablePrepareToken, RecoveryScan};
use crate::wal_frame::{EncodedFrame, RecordKind, encode_frame};
use crate::wal_payload::{
    CommitRecord, IntentSealRecord, encode_commit_payload, encode_intent_seal_payload,
    encode_prepare_payload,
};

use super::WAL_FRESHNESS_PREFIX_DOMAIN;

/// Process-local WAL authority used by the volatile persistence owner.
///
/// This is not a simulated crash store and not a temporary file. It retains
/// exactly the same framed prepare/commit/intent-seal vocabulary and LSN
/// ordering as the file WAL, while its publication barrier is intentionally a
/// no-I/O boundary because `Volatile` promises process lifetime only.
#[derive(Debug)]
pub(crate) struct VolatileRevisionWal {
    bytes: Vec<u8>,
    next_lsn: u64,
    freshness_hasher: Sha256,
    poisoned: bool,
}

impl VolatileRevisionWal {
    pub(crate) fn new() -> Self {
        Self::at_lsn(1)
    }

    pub(crate) fn at_lsn(next_lsn: u64) -> Self {
        debug_assert!(next_lsn != 0);
        let mut freshness_hasher = Sha256::new();
        freshness_hasher.update(WAL_FRESHNESS_PREFIX_DOMAIN);
        Self {
            bytes: Vec::new(),
            next_lsn,
            freshness_hasher,
            poisoned: false,
        }
    }

    pub(crate) const fn next_lsn(&self) -> u64 {
        self.next_lsn
    }

    pub(crate) const fn last_lsn(&self) -> u64 {
        self.next_lsn - 1
    }

    pub(crate) fn current_end_offset(&self) -> Result<u64, DurabilityError> {
        u64::try_from(self.bytes.len())
            .map_err(|_| crate::runtime::CodecError::LengthOverflow.into())
    }

    pub(crate) fn freshness_digest(&self) -> AuthorityDigest {
        AuthorityDigest(self.freshness_hasher.clone().finalize().into())
    }

    pub(crate) fn recovery_scan(
        &self,
        base_revision: RevisionId,
    ) -> Result<RecoveryScan, DurabilityError> {
        crate::wal::scan_wal(&self.bytes, base_revision)
    }

    fn append_frame(
        &mut self,
        kind: RecordKind,
        revision: RevisionId,
        payload: &[u8],
    ) -> Result<EncodedFrame, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let lsn = self.next_lsn;
        let next_lsn = lsn.checked_add(1).ok_or(DurabilityError::LsnExhausted)?;
        let frame = encode_frame(lsn, kind, revision, payload)?;
        self.bytes.extend_from_slice(&frame.bytes);
        self.freshness_hasher.update(&frame.bytes);
        self.next_lsn = next_lsn;
        Ok(frame)
    }

    pub(crate) fn append_replication_authority_frame(
        &mut self,
        encoded_replication_frame: &[u8],
    ) -> Result<(), DurabilityError> {
        self.append_frame(
            RecordKind::ReplicationAuthority,
            RevisionId::new(0),
            encoded_replication_frame,
        )?;
        Ok(())
    }

    pub(crate) fn append_intent_seal_unflushed(
        &mut self,
        record: &IntentSealRecord,
    ) -> Result<EncodedFrame, DurabilityError> {
        let payload = encode_intent_seal_payload(record)?;
        self.append_frame(
            RecordKind::SealClientIntent,
            record.committed.target_revision(),
            &payload,
        )
    }

    pub(crate) fn append_prepare_unflushed_with_frame(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<(DurablePrepareToken, EncodedFrame), DurabilityError> {
        let payload = encode_prepare_payload(descriptor)?;
        let frame = self.append_frame(
            RecordKind::PrepareRevision,
            descriptor.target_revision,
            &payload,
        )?;
        let token = DurablePrepareToken::from_wal(
            descriptor.transaction_id,
            descriptor.target_revision,
            descriptor
                .revision_effect_id
                .unwrap_or(kernel_change::RevisionEffectId(
                    descriptor.transaction_id.raw(),
                )),
            frame.lsn,
            frame.payload_crc32c,
        );
        Ok((token, frame))
    }

    pub(crate) fn append_commit_unflushed_with_frame(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<(DurableCommitReceipt, EncodedFrame), DurabilityError> {
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
        Ok((
            DurableCommitReceipt::from_wal(
                prepared.target_revision(),
                prepared.prepare_lsn(),
                frame.lsn,
            ),
            frame,
        ))
    }

    pub(crate) fn durability_barrier(&mut self) -> Result<(), DurabilityError> {
        if self.poisoned {
            Err(DurabilityError::Poisoned)
        } else {
            Ok(())
        }
    }
}
