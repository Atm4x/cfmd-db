use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use kernel_types::RevisionId;
use sha2::{Digest, Sha256};

use crate::binary_codec::{crc32c, read_u32, read_u64};
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::DurableTransactionKey;
use crate::runtime::{CodecError, CommittedRevision, DurabilityError, RecoveryScan, TailStatus};
use crate::wal_frame::{
    DecodedFrame, FrameRead, HEADER_LEN, MAGIC, MAX_PAYLOAD_LEN, RecordKind, read_frame,
    validate_frame_header,
};
use crate::wal_payload::{CommitRecord, decode_commit_payload, decode_prepare_payload};

use super::WAL_FRESHNESS_PREFIX_DOMAIN;

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
    frame_len: usize,
}

enum FileFrameRead {
    Complete(OwnedWalFrame),
    Tail(TailStatus),
}

fn read_wal_file_frame(
    file: &mut File,
    file_len: u64,
    offset: u64,
    expected_lsn: u64,
) -> Result<FileFrameRead, DurabilityError> {
    let remaining = file_len.saturating_sub(offset);
    let offset_usize = usize::try_from(offset).map_err(|_| CodecError::LengthOverflow)?;
    if remaining < u64::try_from(HEADER_LEN).expect("WAL header length fits u64") {
        let tail_len = usize::try_from(remaining).map_err(|_| CodecError::LengthOverflow)?;
        let mut tail = vec![0_u8; tail_len];
        file.read_exact(&mut tail)?;
        let prefix_len = tail.len().min(MAGIC.len());
        let looks_torn = tail[..prefix_len] == MAGIC[..prefix_len];
        return Ok(FileFrameRead::Tail(if looks_torn {
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
    file.read_exact(&mut header)?;
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
        return Ok(FileFrameRead::Tail(TailStatus::Truncated {
            offset: offset_usize,
        }));
    }
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(payload_len)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    payload.resize(payload_len, 0);
    file.read_exact(&mut payload)?;
    let payload_crc = read_u32(&header[28..32]);
    if crc32c(&payload) != payload_crc {
        return Err(DurabilityError::Corruption {
            offset: offset_usize,
            reason: "payload checksum mismatch",
        });
    }
    Ok(FileFrameRead::Complete(OwnedWalFrame {
        header,
        kind: RecordKind::try_from(header[6]).map_err(|()| DurabilityError::Corruption {
            offset: offset_usize,
            reason: "unknown frame kind",
        })?,
        lsn: read_u64(&header[12..20]),
        revision: RevisionId::new(read_u64(&header[20..28])),
        payload_crc,
        payload,
        frame_len,
    }))
}

pub(super) fn scan_wal_file_seeded(
    file: &mut File,
    base_revision: RevisionId,
    first_lsn: u64,
    seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
) -> Result<(RecoveryScan, Sha256, u64), DurabilityError> {
    if first_lsn == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "WAL first LSN must be nonzero",
        });
    }
    file.seek(SeekFrom::Start(0))?;
    let file_len = file.metadata()?.len();
    let mut state = ScanState::new(base_revision);
    for (lsn, descriptor, payload_crc) in seeded_prepares {
        state.seed_prepare(*lsn, descriptor.clone(), *payload_crc)?;
    }
    let mut freshness_hasher = Sha256::new();
    freshness_hasher.update(WAL_FRESHNESS_PREFIX_DOMAIN);
    let mut offset = 0_u64;
    let mut expected_lsn = first_lsn;

    while offset < file_len {
        match read_wal_file_frame(file, file_len, offset, expected_lsn)? {
            FileFrameRead::Tail(tail_status) => {
                return Ok((
                    state.finish(
                        usize::try_from(offset).map_err(|_| CodecError::LengthOverflow)?,
                        expected_lsn,
                        tail_status,
                    ),
                    freshness_hasher,
                    file_len,
                ));
            }
            FileFrameRead::Complete(frame) => {
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
                freshness_hasher.update(&frame.payload);
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
        file_len,
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
        }
    }

    fn accept(&mut self, frame: &DecodedFrame<'_>) -> Result<(), DurabilityError> {
        match frame.kind {
            RecordKind::PrepareRevision => self.accept_prepare(frame),
            RecordKind::CommitRevision => self.accept_commit(frame),
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
            committed_transactions,
            unresolved_prepares,
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
