use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Read, Seek, SeekFrom};

use kernel_types::RevisionId;
use sha2::{Digest, Sha256};

use crate::binary_codec::{crc32c, read_u32, read_u64};
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::DurableTransactionKey;
use crate::runtime::{
    CodecError, CommittedRevision, DurabilityError, RecoveredAuthorityState, RecoveryScan,
    TailStatus,
};
use crate::storage_encryption::{StorageAeadCodec, StorageEncryptionDomain};
use crate::wal_frame::{
    DecodedFrame, FrameRead, HEADER_LEN, MAGIC, MAX_PAYLOAD_LEN, RecordKind, read_frame,
    validate_frame_header,
};
use crate::wal_payload::{CommitRecord, decode_commit_payload, decode_prepare_payload};

use super::{WAL_FRESHNESS_PREFIX_DOMAIN, wal_aad_context};

pub fn scan_wal(bytes: &[u8], base_revision: RevisionId) -> Result<RecoveryScan, DurabilityError> {
    scan_wal_seeded(bytes, base_revision, 1, &[])
}

struct OwnedWalFrame {
    header: [u8; HEADER_LEN],
    kind: RecordKind,
    lsn: u64,
    revision: RevisionId,
    payload_crc: u32,
    payload: Vec<u8>,
    stored_payload: Vec<u8>,
    frame_len: usize,
}

enum ReaderFrameRead {
    Complete(OwnedWalFrame),
    Tail(TailStatus),
}

fn read_declared_region_exact(
    reader: &mut impl Read,
    bytes: &mut [u8],
    logical_offset: usize,
) -> Result<(), DurabilityError> {
    match reader.read_exact(bytes) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Err(DurabilityError::Corruption {
                offset: logical_offset,
                reason: "WAL backing data ends before its declared region boundary",
            })
        }
        Err(error) => Err(DurabilityError::Io(error)),
    }
}

pub(crate) struct WalRegionScanSpec<'a> {
    pub(crate) start_offset: u64,
    pub(crate) end_offset: u64,
    pub(crate) base_revision: RevisionId,
    pub(crate) first_lsn: u64,
    pub(crate) seeded_prepares: &'a [(u64, DurableRevisionDescriptor, u32)],
    pub(crate) crypto: Option<&'a StorageAeadCodec>,
}

fn read_wal_reader_frame(
    reader: &mut impl Read,
    region_end: u64,
    absolute_offset: u64,
    logical_offset: u64,
    expected_lsn: u64,
    crypto: Option<&StorageAeadCodec>,
) -> Result<ReaderFrameRead, DurabilityError> {
    let remaining = region_end.saturating_sub(absolute_offset);
    let offset_usize = usize::try_from(logical_offset).map_err(|_| CodecError::LengthOverflow)?;
    if remaining < u64::try_from(HEADER_LEN).expect("WAL header length fits u64") {
        let tail_len = usize::try_from(remaining).map_err(|_| CodecError::LengthOverflow)?;
        let mut tail = vec![0_u8; tail_len];
        read_declared_region_exact(reader, &mut tail, offset_usize)?;
        let prefix_len = tail.len().min(MAGIC.len());
        let looks_torn = tail[..prefix_len] == MAGIC[..prefix_len];
        return Ok(ReaderFrameRead::Tail(if looks_torn {
            TailStatus::Truncated {
                offset: offset_usize,
            }
        } else {
            TailStatus::Garbage {
                offset: offset_usize,
            }
        }));
    }

    let mut header = [0_u8; HEADER_LEN];
    read_declared_region_exact(reader, &mut header, offset_usize)?;
    if header[..4] != MAGIC {
        return Err(DurabilityError::Corruption {
            offset: offset_usize,
            reason: "non-frame bytes large enough to hide a complete frame",
        });
    }
    validate_frame_header(&header, offset_usize, expected_lsn)?;
    let payload_len =
        usize::try_from(read_u32(&header[8..12])).map_err(|_| DurabilityError::Corruption {
            offset: offset_usize,
            reason: "payload length overflow",
        })?;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::Corruption {
            offset: offset_usize,
            reason: "payload length exceeds hard limit",
        });
    }
    let frame_len = HEADER_LEN
        .checked_add(payload_len)
        .ok_or(DurabilityError::Corruption {
            offset: offset_usize,
            reason: "frame length overflow",
        })?;
    if remaining < u64::try_from(frame_len).map_err(|_| CodecError::LengthOverflow)? {
        return Ok(ReaderFrameRead::Tail(TailStatus::Truncated {
            offset: offset_usize,
        }));
    }
    let mut stored_payload = Vec::new();
    stored_payload
        .try_reserve_exact(payload_len)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    stored_payload.resize(payload_len, 0);
    read_declared_region_exact(reader, &mut stored_payload, offset_usize)?;
    let payload_crc = read_u32(&header[28..32]);
    if crc32c(&stored_payload) != payload_crc {
        return Err(DurabilityError::Corruption {
            offset: offset_usize,
            reason: "payload checksum mismatch",
        });
    }
    let kind = RecordKind::try_from(header[6]).map_err(|()| DurabilityError::Corruption {
        offset: offset_usize,
        reason: "unknown frame kind",
    })?;
    let lsn = read_u64(&header[12..20]);
    let revision = RevisionId::new(read_u64(&header[20..28]));
    let payload = if let Some(crypto) = crypto {
        crypto.open(
            StorageEncryptionDomain::Wal,
            &wal_aad_context(kind, lsn, revision),
            &stored_payload,
        )?
    } else {
        if stored_payload.starts_with(b"CFAE") {
            return Err(DurabilityError::Protocol {
                offset: offset_usize,
                reason: "encrypted WAL frame requires a database key",
            });
        }
        stored_payload.clone()
    };
    Ok(ReaderFrameRead::Complete(OwnedWalFrame {
        header,
        kind,
        lsn,
        revision,
        payload_crc,
        payload,
        stored_payload,
        frame_len,
    }))
}

pub(super) fn scan_wal_file_seeded(
    file: &mut std::fs::File,
    base_revision: RevisionId,
    first_lsn: u64,
    seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    crypto: Option<&StorageAeadCodec>,
) -> Result<(RecoveryScan, Sha256, u64), DurabilityError> {
    let file_len = file.metadata()?.len();
    let (scan, hasher) = scan_wal_file_region_seeded(
        file,
        0,
        file_len,
        base_revision,
        first_lsn,
        seeded_prepares,
        crypto,
    )?;
    Ok((scan, hasher, file_len))
}

pub(super) fn scan_wal_file_region_seeded(
    file: &mut std::fs::File,
    start_offset: u64,
    end_offset: u64,
    base_revision: RevisionId,
    first_lsn: u64,
    seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    crypto: Option<&StorageAeadCodec>,
) -> Result<(RecoveryScan, Sha256), DurabilityError> {
    let backing_len = file.metadata()?.len();
    let spec = WalRegionScanSpec {
        start_offset,
        end_offset,
        base_revision,
        first_lsn,
        seeded_prepares,
        crypto,
    };
    scan_wal_reader_region_seeded(file, backing_len, &spec)
}

pub(super) fn scan_wal_reader_region_seeded(
    reader: &mut (impl Read + Seek),
    backing_len: u64,
    spec: &WalRegionScanSpec<'_>,
) -> Result<(RecoveryScan, Sha256), DurabilityError> {
    if spec.first_lsn == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "WAL first LSN must be nonzero",
        });
    }
    if spec.start_offset > spec.end_offset || spec.end_offset > backing_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "WAL region range is outside the backing file",
        });
    }
    reader.seek(SeekFrom::Start(spec.start_offset))?;
    let mut state = ScanState::new(spec.base_revision);
    for (lsn, descriptor, payload_crc) in spec.seeded_prepares {
        state.seed_prepare(*lsn, descriptor.clone(), *payload_crc)?;
    }
    let mut freshness_hasher = Sha256::new();
    freshness_hasher.update(WAL_FRESHNESS_PREFIX_DOMAIN);
    let mut offset = 0_u64;
    let region_len = spec.end_offset - spec.start_offset;
    let mut expected_lsn = spec.first_lsn;

    while offset < region_len {
        let absolute_offset = spec
            .start_offset
            .checked_add(offset)
            .ok_or(CodecError::LengthOverflow)?;
        reader.seek(SeekFrom::Start(absolute_offset))?;
        match read_wal_reader_frame(
            reader,
            spec.end_offset,
            absolute_offset,
            offset,
            expected_lsn,
            spec.crypto,
        )? {
            ReaderFrameRead::Tail(tail_status) => {
                return Ok((
                    state.finish(
                        usize::try_from(offset).map_err(|_| CodecError::LengthOverflow)?,
                        expected_lsn,
                        tail_status,
                    ),
                    freshness_hasher,
                ));
            }
            ReaderFrameRead::Complete(frame) => {
                state.accept(&DecodedFrame {
                    offset: usize::try_from(offset).map_err(|_| CodecError::LengthOverflow)?,
                    kind: frame.kind,
                    lsn: frame.lsn,
                    revision: frame.revision,
                    payload_crc: frame.payload_crc,
                    payload: &frame.payload,
                    frame_len: frame.frame_len,
                })?;
                freshness_hasher.update(frame.header);
                freshness_hasher.update(&frame.stored_payload);
                offset = offset
                    .checked_add(
                        u64::try_from(frame.frame_len).map_err(|_| CodecError::LengthOverflow)?,
                    )
                    .ok_or(CodecError::LengthOverflow)?;
                expected_lsn = expected_lsn
                    .checked_add(1)
                    .ok_or(DurabilityError::LsnExhausted)?;
            }
        }
    }

    Ok((
        state.finish(
            usize::try_from(offset).map_err(|_| CodecError::LengthOverflow)?,
            expected_lsn,
            TailStatus::Clean,
        ),
        freshness_hasher,
    ))
}

fn scan_wal_seeded(
    bytes: &[u8],
    base_revision: RevisionId,
    first_lsn: u64,
    seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
) -> Result<RecoveryScan, DurabilityError> {
    if first_lsn == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "WAL first LSN must be nonzero",
        });
    }
    let mut state = ScanState::new(base_revision);
    for (lsn, descriptor, payload_crc) in seeded_prepares {
        state.seed_prepare(*lsn, descriptor.clone(), *payload_crc)?;
    }
    let mut offset = 0_usize;
    let mut expected_lsn = first_lsn;
    while offset < bytes.len() {
        match read_frame(bytes, offset, expected_lsn)? {
            FrameRead::Tail(tail_status) => {
                return Ok(state.finish(offset, expected_lsn, tail_status));
            }
            FrameRead::Complete(frame) => {
                let frame_len = frame.frame_len;
                state.accept(&frame)?;
                offset = offset
                    .checked_add(frame_len)
                    .ok_or(DurabilityError::Corruption {
                        offset,
                        reason: "scan offset overflow",
                    })?;
                expected_lsn = expected_lsn
                    .checked_add(1)
                    .ok_or(DurabilityError::LsnExhausted)?;
            }
        }
    }
    Ok(state.finish(offset, expected_lsn, TailStatus::Clean))
}

#[derive(Debug)]
struct PreparedRevisionOwner {
    descriptor: DurableRevisionDescriptor,
}

#[derive(Debug, Clone, Copy)]
struct ScanCommittedRevision {
    prepare_owner: usize,
    prepare_lsn: u64,
    prepare_payload_crc32c: u32,
    commit_lsn: u64,
}

struct ScanState {
    base_revision: RevisionId,
    durable_head: RevisionId,
    prepare_owners: Vec<PreparedRevisionOwner>,
    prepares_by_lsn: BTreeMap<u64, (usize, u32)>,
    prepare_identity: BTreeMap<RevisionId, usize>,
    prepare_transaction_identity: BTreeMap<DurableTransactionKey, usize>,
    committed_transaction_keys: BTreeSet<DurableTransactionKey>,
    commits: BTreeMap<RevisionId, CommitRecord>,
    committed: Vec<ScanCommittedRevision>,
    replication_authority_frames: Vec<Vec<u8>>,
}

impl ScanState {
    fn new(base_revision: RevisionId) -> Self {
        Self {
            base_revision,
            durable_head: base_revision,
            prepare_owners: Vec::new(),
            prepares_by_lsn: BTreeMap::new(),
            prepare_identity: BTreeMap::new(),
            prepare_transaction_identity: BTreeMap::new(),
            committed_transaction_keys: BTreeSet::new(),
            commits: BTreeMap::new(),
            committed: Vec::new(),
            replication_authority_frames: Vec::new(),
        }
    }

    fn accept(&mut self, frame: &DecodedFrame<'_>) -> Result<(), DurabilityError> {
        match frame.kind {
            RecordKind::PrepareRevision => self.accept_prepare(frame),
            RecordKind::CommitRevision => self.accept_commit(frame),
            RecordKind::ReplicationAuthority => {
                if frame.revision != RevisionId::new(0) {
                    return Err(DurabilityError::Corruption {
                        offset: frame.offset,
                        reason: "replication authority WAL frame carries nonzero revision",
                    });
                }
                self.replication_authority_frames
                    .push(frame.payload.to_vec());
                Ok(())
            }
        }
    }

    fn seed_prepare(
        &mut self,
        lsn: u64,
        descriptor: DurableRevisionDescriptor,
        payload_crc: u32,
    ) -> Result<(), DurabilityError> {
        let transaction_key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        let target_owner = self
            .prepare_identity
            .get(&descriptor.target_revision)
            .copied();
        if let Some(owner) = target_owner
            && self.prepare_owners[owner].descriptor != descriptor
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "prepared cut capsule conflicts by target revision",
            });
        }
        let transaction_owner = self
            .prepare_transaction_identity
            .get(&transaction_key)
            .copied();
        if let Some(owner) = transaction_owner
            && self.prepare_owners[owner].descriptor != descriptor
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "prepared cut capsule conflicts by transaction identity",
            });
        }
        if let (Some(target_owner), Some(transaction_owner)) = (target_owner, transaction_owner)
            && target_owner != transaction_owner
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "prepared cut capsule has inconsistent prepare identity indices",
            });
        }
        let owner = if let Some(owner) = target_owner.or(transaction_owner) {
            owner
        } else {
            let owner = self.prepare_owners.len();
            self.prepare_owners
                .push(PreparedRevisionOwner { descriptor });
            self.prepare_identity
                .insert(self.prepare_owners[owner].descriptor.target_revision, owner);
            self.prepare_transaction_identity
                .insert(transaction_key, owner);
            owner
        };
        if let Some((existing_owner, existing_crc)) = self.prepares_by_lsn.get(&lsn) {
            if *existing_owner != owner || *existing_crc != payload_crc {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "prepared cut capsule conflicts by prepare lsn",
                });
            }
            return Ok(());
        }
        self.prepares_by_lsn.insert(lsn, (owner, payload_crc));
        Ok(())
    }

    fn accept_prepare(&mut self, frame: &DecodedFrame<'_>) -> Result<(), DurabilityError> {
        let descriptor =
            decode_prepare_payload(frame.revision, frame.payload).map_err(|reason| {
                DurabilityError::Corruption {
                    offset: frame.offset,
                    reason,
                }
            })?;
        let transaction_key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        if self.committed_transaction_keys.contains(&transaction_key)
            && self.prepare_owners[*self
                .prepare_transaction_identity
                .get(&transaction_key)
                .expect("committed transaction retains prepare owner")]
            .descriptor
                != descriptor
        {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "transaction id already committed to another exact intent",
            });
        }
        let target_owner = self.prepare_identity.get(&frame.revision).copied();
        if let Some(owner) = target_owner
            && self.prepare_owners[owner].descriptor != descriptor
        {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "conflicting duplicate prepare",
            });
        }
        let transaction_owner = self
            .prepare_transaction_identity
            .get(&transaction_key)
            .copied();
        if let Some(owner) = transaction_owner
            && self.prepare_owners[owner].descriptor != descriptor
        {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "transaction id reused by conflicting prepare",
            });
        }
        if let (Some(target_owner), Some(transaction_owner)) = (target_owner, transaction_owner)
            && target_owner != transaction_owner
        {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "prepare identity indices disagree",
            });
        }
        let owner = if let Some(owner) = target_owner.or(transaction_owner) {
            owner
        } else {
            let owner = self.prepare_owners.len();
            self.prepare_owners
                .push(PreparedRevisionOwner { descriptor });
            self.prepare_identity.insert(frame.revision, owner);
            self.prepare_transaction_identity
                .insert(transaction_key, owner);
            owner
        };
        if let Some((existing_owner, existing_crc)) = self.prepares_by_lsn.get(&frame.lsn) {
            if *existing_owner != owner || *existing_crc != frame.payload_crc {
                return Err(DurabilityError::Protocol {
                    offset: frame.offset,
                    reason: "conflicting duplicate prepare lsn",
                });
            }
            return Ok(());
        }
        self.prepares_by_lsn
            .insert(frame.lsn, (owner, frame.payload_crc));
        Ok(())
    }

    fn accept_commit(&mut self, frame: &DecodedFrame<'_>) -> Result<(), DurabilityError> {
        let record = decode_commit_payload(frame.revision, frame.payload).map_err(|reason| {
            DurabilityError::Corruption {
                offset: frame.offset,
                reason,
            }
        })?;
        let Some((prepare_owner, prepare_crc)) = self.prepares_by_lsn.get(&record.prepare_lsn)
        else {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "commit references missing prepare",
            });
        };
        let descriptor = &self.prepare_owners[*prepare_owner].descriptor;
        if descriptor.target_revision != frame.revision
            || *prepare_crc != record.prepare_payload_crc32c
        {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "commit does not bind the referenced prepare",
            });
        }
        if let Some(existing) = self.commits.get(&frame.revision) {
            if existing != &record {
                return Err(DurabilityError::Protocol {
                    offset: frame.offset,
                    reason: "conflicting duplicate commit",
                });
            }
            return Ok(());
        }
        if descriptor.source_revision != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "commit source revision does not match durable head",
            });
        }
        if descriptor.target_revision == descriptor.source_revision {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "revision transition does not advance identity",
            });
        }
        let target_revision = descriptor.target_revision;
        let transaction_key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        self.committed_transaction_keys.insert(transaction_key);
        self.commits.insert(frame.revision, record.clone());
        self.committed.push(ScanCommittedRevision {
            prepare_owner: *prepare_owner,
            prepare_lsn: record.prepare_lsn,
            prepare_payload_crc32c: record.prepare_payload_crc32c,
            commit_lsn: frame.lsn,
        });
        self.durable_head = target_revision;
        Ok(())
    }

    fn finish(
        self,
        last_good_offset: usize,
        next_lsn: u64,
        tail_status: TailStatus,
    ) -> RecoveryScan {
        let base_revision = self.base_revision;
        let committed_owners = self
            .committed
            .iter()
            .map(|revision| revision.prepare_owner)
            .collect::<BTreeSet<_>>();
        let unresolved_prepares = self
            .prepares_by_lsn
            .iter()
            .filter(|(_, (owner, _))| !committed_owners.contains(owner))
            .map(|(&prepare_lsn, &(owner, payload_crc32c))| {
                (
                    prepare_lsn,
                    self.prepare_owners[owner].descriptor.clone(),
                    payload_crc32c,
                )
            })
            .collect();
        let mut prepare_owners = self
            .prepare_owners
            .into_iter()
            .map(Some)
            .collect::<Vec<_>>();
        let mut committed = Vec::with_capacity(self.committed.len());
        let mut committed_transactions = BTreeMap::new();
        for revision in self.committed {
            let descriptor = prepare_owners[revision.prepare_owner]
                .take()
                .expect("one prepared owner can publish at most once")
                .descriptor;
            let transaction_key =
                DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
            let previous =
                committed_transactions.insert(transaction_key, descriptor.intent.clone());
            debug_assert!(previous.is_none());
            committed.push(CommittedRevision {
                descriptor,
                prepare_lsn: revision.prepare_lsn,
                prepare_payload_crc32c: revision.prepare_payload_crc32c,
                commit_lsn: revision.commit_lsn,
            });
        }
        RecoveryScan::recovered(
            base_revision,
            committed,
            last_good_offset,
            next_lsn,
            tail_status,
            RecoveredAuthorityState {
                committed_transactions,
                unresolved_prepares,
                replication_authority_frames: self.replication_authority_frames,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use kernel_model::{DatabaseState, Value};
    use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
    use kernel_semantics::SemanticRegistry;
    use kernel_types::{
        ClientTransactionId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId,
    };

    use super::ScanState;
    use crate::descriptor::DurableRevisionDescriptor;
    use crate::domain::DurableRelationMutation;
    use crate::runtime::DurabilityError;

    fn descriptor(source: u64, target: u64, value: Value) -> DurableRevisionDescriptor {
        let registry = SemanticRegistry::default();
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(7)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(9)),
        };
        let target_revision = kernel_revision::Revision::build(
            RevisionId::new(target),
            &context,
            &registry,
            DatabaseState::default(),
        )
        .unwrap();
        DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(u128::from(target)),
            RevisionId::new(source),
            &target_revision,
            target_revision.semantic_revision(),
            vec![DurableRelationMutation {
                relation: SemanticId::new(11),
                inserted: vec![vec![value]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap()
    }

    #[test]
    fn seeded_prepare_lsn_is_fail_closed_identity_not_overwrite_slot() {
        let mut state = ScanState::new(RevisionId::new(1));
        state
            .seed_prepare(7, descriptor(1, 2, Value::I64(7)), 11)
            .unwrap();

        assert!(matches!(
            state.seed_prepare(7, descriptor(1, 3, Value::I64(8)), 12),
            Err(DurabilityError::Protocol {
                reason: "prepared cut capsule conflicts by prepare lsn",
                ..
            })
        ));
    }

    #[test]
    fn exact_duplicate_prepares_share_one_recovery_owner() {
        let mut state = ScanState::new(RevisionId::new(1));
        let descriptor = descriptor(1, 2, Value::Text("large-owner".repeat(32)));
        state.seed_prepare(7, descriptor.clone(), 11).unwrap();
        state.seed_prepare(8, descriptor, 11).unwrap();

        assert_eq!(state.prepare_owners.len(), 1);
        assert_eq!(state.prepare_identity.len(), 1);
        assert_eq!(state.prepare_transaction_identity.len(), 1);
        assert_eq!(state.prepares_by_lsn.len(), 2);
    }
}
