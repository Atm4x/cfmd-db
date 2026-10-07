use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kernel_auth::AuthorityDigest;
use kernel_types::RevisionId;
use sha2::{Digest, Sha256};

use crate::descriptor::DurableRevisionDescriptor;
use crate::runtime::{
    CodecError, DurabilityError, DurableCommitReceipt, DurablePrepareToken, RecoveryScan,
    RevisionDurability,
};
use crate::storage_encryption::{StorageAeadCodec, StorageEncryptionDomain, StorageNonceSequence};
use crate::wal_frame::{EncodedFrame, RecordKind, encode_frame};
use crate::wal_payload::{
    CommitRecord, IntentSealRecord, encode_commit_payload, encode_intent_seal_payload,
    encode_prepare_payload,
};

use super::recovery::{
    scan_wal_file_region_seeded, scan_wal_file_seeded, scan_wal_reader_region_seeded,
};
use super::{WAL_FRESHNESS_PREFIX_DOMAIN, WalRegionScanSpec, wal_aad_context};

#[derive(Debug)]
pub struct FileRevisionWal {
    path: PathBuf,
    file: File,
    start_offset: u64,
    next_lsn: u64,
    freshness_hasher: Sha256,
    poisoned: bool,
    crypto: Option<StorageAeadCodec>,
    nonce_sequence: Option<StorageNonceSequence>,
}

pub(crate) struct WalRegionRecovery<'a> {
    pub path: PathBuf,
    pub file: File,
    pub start_offset: u64,
    pub end_offset: u64,
    pub base_revision: RevisionId,
    pub first_lsn: u64,
    pub seeded_prepares: &'a [(u64, DurableRevisionDescriptor, u32)],
    pub writable: bool,
    pub crypto: Option<StorageAeadCodec>,
}

pub(crate) struct WalRegionScan {
    scan: RecoveryScan,
    freshness_hasher: Sha256,
    good_end: u64,
}

impl WalRegionScan {
    #[must_use]
    pub(crate) const fn good_end(&self) -> u64 {
        self.good_end
    }
}

impl FileRevisionWal {
    pub(crate) fn scan_recovered_seeded_read_only(
        path: impl AsRef<Path>,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<RecoveryScan, DurabilityError> {
        let path = path.as_ref();
        let mut file = OpenOptions::new().read(true).open(path)?;
        let (scan, _, _) =
            scan_wal_file_seeded(&mut file, base_revision, first_lsn, seeded_prepares, None)?;
        Ok(scan)
    }

    pub(crate) fn scan_region_seeded_read_only(
        path: impl AsRef<Path>,
        start_offset: u64,
        end_offset: u64,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
        crypto: Option<&StorageAeadCodec>,
    ) -> Result<RecoveryScan, DurabilityError> {
        let mut file = OpenOptions::new().read(true).open(path)?;
        let (scan, _) = scan_wal_file_region_seeded(
            &mut file,
            start_offset,
            end_offset,
            base_revision,
            first_lsn,
            seeded_prepares,
            crypto,
        )?;
        Ok(scan)
    }

    pub fn create(path: impl AsRef<Path>) -> Result<Self, DurabilityError> {
        Self::create_at_lsn(path, 1)
    }

    pub(crate) fn create_at_lsn(
        path: impl AsRef<Path>,
        next_lsn: u64,
    ) -> Result<Self, DurabilityError> {
        if next_lsn == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "WAL next LSN must be nonzero",
            });
        }
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
        file.lock()?;
        let mut freshness_hasher = Sha256::new();
        freshness_hasher.update(WAL_FRESHNESS_PREFIX_DOMAIN);
        Ok(Self {
            path,
            file,
            start_offset: 0,
            next_lsn,
            freshness_hasher,
            poisoned: false,
            crypto: None,
            nonce_sequence: None,
        })
    }

    pub fn open_recovered(
        path: impl AsRef<Path>,
        base_revision: RevisionId,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_recovered_seeded(path, base_revision, 1, &[])
    }

    pub(crate) fn open_recovered_seeded(
        path: impl AsRef<Path>,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        file.lock()?;
        let (scan, freshness_hasher, original_len) =
            scan_wal_file_seeded(&mut file, base_revision, first_lsn, seeded_prepares, None)?;
        if u64::try_from(scan.last_good_offset()).map_err(|_| CodecError::LengthOverflow)?
            < original_len
        {
            file.set_len(
                u64::try_from(scan.last_good_offset()).map_err(|_| CodecError::LengthOverflow)?,
            )?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;
        Ok((
            Self {
                path,
                file,
                start_offset: 0,
                next_lsn: scan.next_lsn(),
                freshness_hasher,
                poisoned: false,
                crypto: None,
                nonce_sequence: None,
            },
            scan,
        ))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub(crate) const fn start_offset(&self) -> u64 {
        self.start_offset
    }

    #[must_use]
    pub(crate) const fn next_lsn(&self) -> u64 {
        self.next_lsn
    }

    pub(crate) fn current_end_offset(&mut self) -> Result<u64, DurabilityError> {
        self.file.stream_position().map_err(DurabilityError::Io)
    }

    pub(crate) fn scan_subregion_seeded(
        &self,
        start_offset: u64,
        end_offset: u64,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<RecoveryScan, DurabilityError> {
        let mut file = self.file.try_clone()?;
        let (scan, _) = scan_wal_file_region_seeded(
            &mut file,
            start_offset,
            end_offset,
            base_revision,
            first_lsn,
            seeded_prepares,
            self.crypto.as_ref(),
        )?;
        Ok(scan)
    }

    pub(crate) fn seal_for_generation_rotation(&mut self) -> Result<u64, DurabilityError> {
        self.seal_for_generation_rotation_with(File::sync_data)
    }

    pub(crate) fn seal_for_generation_rotation_with(
        &mut self,
        sync: impl FnOnce(&File) -> std::io::Result<()>,
    ) -> Result<u64, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if let Err(error) = sync(&self.file) {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        self.poisoned = true;
        Ok(self.next_lsn)
    }

    #[must_use]
    pub(crate) const fn last_lsn(&self) -> u64 {
        self.next_lsn - 1
    }

    #[must_use]
    pub(crate) fn freshness_digest(&self) -> AuthorityDigest {
        AuthorityDigest(self.freshness_hasher.clone().finalize().into())
    }

    pub(crate) fn scan_recovery_seeded(
        &mut self,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<(RecoveryScan, u64), DurabilityError> {
        let file_len = self.file.metadata()?.len();
        let (scan, _) = scan_wal_file_region_seeded(
            &mut self.file,
            self.start_offset,
            file_len,
            base_revision,
            first_lsn,
            seeded_prepares,
            self.crypto.as_ref(),
        )?;
        Ok((scan, file_len - self.start_offset))
    }

    pub(crate) fn open_region_recovered(
        mut region: WalRegionRecovery<'_>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let backing_len = region.file.metadata()?.len();
        let spec = WalRegionScanSpec {
            start_offset: region.start_offset,
            end_offset: region.end_offset,
            base_revision: region.base_revision,
            first_lsn: region.first_lsn,
            seeded_prepares: region.seeded_prepares,
            crypto: region.crypto.as_ref(),
        };
        let scanned = Self::scan_region_recovery(&spec, &mut region.file, backing_len)?;
        if region.writable && scanned.good_end < region.end_offset {
            region.file.set_len(scanned.good_end)?;
            region.file.sync_all()?;
        }
        region.file.seek(SeekFrom::Start(scanned.good_end))?;
        Self::from_region_scan(region, scanned)
    }

    pub(crate) fn scan_region_recovery(
        spec: &WalRegionScanSpec<'_>,
        reader: &mut (impl Read + Seek),
        backing_len: u64,
    ) -> Result<WalRegionScan, DurabilityError> {
        let (scan, freshness_hasher) = scan_wal_reader_region_seeded(reader, backing_len, spec)?;
        let logical_good =
            u64::try_from(scan.last_good_offset()).map_err(|_| CodecError::LengthOverflow)?;
        let good_end = spec
            .start_offset
            .checked_add(logical_good)
            .ok_or(CodecError::LengthOverflow)?;
        Ok(WalRegionScan {
            scan,
            freshness_hasher,
            good_end,
        })
    }

    pub(crate) fn from_region_scan(
        region: WalRegionRecovery<'_>,
        scanned: WalRegionScan,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let nonce_sequence = region
            .crypto
            .as_ref()
            .map(|_| StorageNonceSequence::random())
            .transpose()?;
        Ok((
            Self {
                path: region.path,
                file: region.file,
                start_offset: region.start_offset,
                next_lsn: scanned.scan.next_lsn(),
                freshness_hasher: scanned.freshness_hasher,
                poisoned: false,
                crypto: region.crypto,
                nonce_sequence,
            },
            scanned.scan,
        ))
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
        let stored_payload = if let Some(crypto) = &self.crypto {
            let nonce = self
                .nonce_sequence
                .as_mut()
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "encrypted WAL writer is missing its nonce sequence",
                })?
                .next_nonce()?;
            crypto.seal(
                StorageEncryptionDomain::Wal,
                nonce,
                &wal_aad_context(kind, lsn, revision),
                payload,
            )?
        } else {
            payload.to_vec()
        };
        let frame = encode_frame(lsn, kind, revision, &stored_payload)?;
        if let Err(error) = self.file.write_all(&frame.bytes) {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        self.freshness_hasher.update(&frame.bytes);
        self.next_lsn = next_lsn;
        Ok(frame)
    }

    pub(crate) fn append_exact_frame(
        &mut self,
        frame: &EncodedFrame,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if frame.lsn != self.next_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "shadow WAL exact frame LSN is not contiguous",
            });
        }
        if let Err(error) = self.file.write_all(&frame.bytes) {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        self.freshness_hasher.update(&frame.bytes);
        self.next_lsn = self
            .next_lsn
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?;
        Ok(())
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
    fn barrier(&mut self) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if let Err(error) = self.file.sync_data() {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        Ok(())
    }

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
        let receipt = DurableCommitReceipt::from_wal(
            prepared.target_revision(),
            prepared.prepare_lsn(),
            frame.lsn,
        );
        Ok((receipt, frame))
    }

    pub(crate) fn durability_barrier(&mut self) -> Result<(), DurabilityError> {
        self.barrier()
    }
}

impl RevisionDurability for FileRevisionWal {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        let prepared = self.append_prepare_unflushed(descriptor)?;
        self.barrier()?;
        Ok(prepared)
    }

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        let receipt = self.append_commit_unflushed(prepared)?;
        self.barrier()?;
        Ok(receipt)
    }
}
