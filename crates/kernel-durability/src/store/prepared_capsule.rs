use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use kernel_types::RevisionId;

use crate::binary_codec::{crc32c, crc32c_update, read_u16, read_u32, read_u64};
use crate::descriptor::DurableRevisionDescriptor;
use crate::wal_frame::MAX_PAYLOAD_LEN;
use crate::wal_payload::{decode_prepare_payload, encode_prepare_payload};

use super::file_io::{read_exact_file_payload, read_exact_or_corruption, require_file_eof};
use super::prepared_lifecycle::PreparedTransactionLedger;
use crate::runtime::DurabilityError;

pub(super) const PREPARED_CAPSULE_MAGIC: [u8; 4] = *b"CFPC";
pub(super) const PREPARED_CAPSULE_VERSION: u16 = 1;
pub(super) const PREPARED_CAPSULE_HEADER_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedCutEntry {
    prepare_lsn: u64,
    payload_crc32c: u32,
    descriptor: DurableRevisionDescriptor,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedCutCapsule {
    entries: Vec<PreparedCutEntry>,
}

impl PreparedCutCapsule {
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn from_prepared_transactions(
        prepared: &PreparedTransactionLedger,
        durable_head: RevisionId,
    ) -> Self {
        let mut entries = Vec::new();
        for (prepare_lsn, descriptor, payload_crc32c) in prepared.checkpoint_entries(durable_head) {
            entries.push(PreparedCutEntry {
                prepare_lsn,
                payload_crc32c,
                descriptor: descriptor.clone(),
            });
        }
        Self { entries }
    }

    pub(super) fn prepare_lsns(&self) -> impl Iterator<Item = u64> + '_ {
        self.entries.iter().map(|entry| entry.prepare_lsn)
    }

    pub(super) fn matches_recovery_scan(&self, scan: &crate::runtime::RecoveryScan) -> bool {
        let expected = self.scan_seeds();
        let actual = scan
            .unresolved_prepares()
            .iter()
            .map(|(lsn, descriptor, crc)| (*lsn, descriptor.clone(), *crc))
            .collect::<Vec<_>>();
        expected == actual
    }

    pub(super) fn scan_seeds(&self) -> Vec<(u64, DurableRevisionDescriptor, u32)> {
        self.entries
            .iter()
            .map(|entry| {
                (
                    entry.prepare_lsn,
                    entry.descriptor.clone(),
                    entry.payload_crc32c,
                )
            })
            .collect()
    }
}

pub(super) fn read_prepared_cut_capsule_file(
    path: &Path,
    expected_file_crc32c: u32,
) -> Result<PreparedCutCapsule, DurabilityError> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; PREPARED_CAPSULE_HEADER_LEN];
    read_exact_or_corruption(
        &mut file,
        &mut header,
        0,
        "prepared cut capsule header truncated",
    )?;
    if header[..4] != PREPARED_CAPSULE_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "prepared cut capsule header mismatch",
        });
    }
    if read_u16(&header[4..6]) != PREPARED_CAPSULE_VERSION || read_u16(&header[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported prepared cut capsule format",
        });
    }
    let count =
        usize::try_from(read_u32(&header[8..12])).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let expected_payload_crc32c = read_u32(&header[12..16]);
    let mut file_crc = crc32c_update(!0_u32, &header);
    let mut payload_crc = !0_u32;
    let mut entries = Vec::new();
    let mut offset = PREPARED_CAPSULE_HEADER_LEN;
    for _ in 0..count {
        let mut entry_header = [0_u8; 24];
        read_exact_or_corruption(
            &mut file,
            &mut entry_header,
            offset,
            "prepared cut capsule entry truncated",
        )?;
        file_crc = crc32c_update(file_crc, &entry_header);
        payload_crc = crc32c_update(payload_crc, &entry_header);
        let prepare_lsn = read_u64(&entry_header[0..8]);
        let payload_crc32c = read_u32(&entry_header[8..12]);
        let target_revision = RevisionId::new(read_u64(&entry_header[12..20]));
        let len = usize::try_from(read_u32(&entry_header[20..24]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        if len > MAX_PAYLOAD_LEN {
            return Err(DurabilityError::Corruption {
                offset,
                reason: "prepared cut capsule payload exceeds hard limit",
            });
        }
        let encoded = read_exact_file_payload(
            &mut file,
            len,
            offset + 24,
            "prepared cut capsule payload truncated",
        )?;
        file_crc = crc32c_update(file_crc, &encoded);
        payload_crc = crc32c_update(payload_crc, &encoded);
        let descriptor = decode_prepare_payload(target_revision, &encoded).map_err(|reason| {
            DurabilityError::Corruption {
                offset: offset + 24,
                reason,
            }
        })?;
        entries.push(PreparedCutEntry {
            prepare_lsn,
            payload_crc32c,
            descriptor,
        });
        offset = offset
            .checked_add(24)
            .and_then(|value| value.checked_add(len))
            .ok_or(DurabilityError::PayloadTooLarge)?;
    }
    require_file_eof(&mut file, offset, "prepared cut capsule has trailing bytes")?;
    if !payload_crc != expected_payload_crc32c {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "prepared cut capsule checksum mismatch",
        });
    }
    if !file_crc != expected_file_crc32c {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published prepared cut capsule checksum mismatch",
        });
    }
    Ok(PreparedCutCapsule { entries })
}

pub(super) fn encode_prepared_cut_capsule(
    capsule: &PreparedCutCapsule,
) -> Result<Vec<u8>, DurabilityError> {
    let mut out = vec![0_u8; PREPARED_CAPSULE_HEADER_LEN];
    for entry in &capsule.entries {
        let encoded = encode_prepare_payload(&entry.descriptor)?;
        let encoded_len =
            u32::try_from(encoded.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        out.extend_from_slice(&entry.prepare_lsn.to_le_bytes());
        out.extend_from_slice(&entry.payload_crc32c.to_le_bytes());
        out.extend_from_slice(&entry.descriptor.target_revision.raw().to_le_bytes());
        out.extend_from_slice(&encoded_len.to_le_bytes());
        out.extend_from_slice(&encoded);
    }
    let count =
        u32::try_from(capsule.entries.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let payload_crc32c = crc32c(&out[PREPARED_CAPSULE_HEADER_LEN..]);
    out[..4].copy_from_slice(&PREPARED_CAPSULE_MAGIC);
    out[4..6].copy_from_slice(&PREPARED_CAPSULE_VERSION.to_le_bytes());
    out[6..8].copy_from_slice(&0_u16.to_le_bytes());
    out[8..12].copy_from_slice(&count.to_le_bytes());
    out[12..16].copy_from_slice(&payload_crc32c.to_le_bytes());
    Ok(out)
}

pub(super) fn decode_prepared_cut_capsule(
    bytes: &[u8],
) -> Result<PreparedCutCapsule, DurabilityError> {
    if bytes.len() < PREPARED_CAPSULE_HEADER_LEN || bytes[..4] != PREPARED_CAPSULE_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "prepared cut capsule header mismatch",
        });
    }
    if read_u16(&bytes[4..6]) != PREPARED_CAPSULE_VERSION || read_u16(&bytes[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported prepared cut capsule format",
        });
    }
    let count =
        usize::try_from(read_u32(&bytes[8..12])).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let payload = &bytes[PREPARED_CAPSULE_HEADER_LEN..];
    if crc32c(payload) != read_u32(&bytes[12..16]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "prepared cut capsule checksum mismatch",
        });
    }
    if count > payload.len() / 24 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "prepared cut capsule entry count exceeds payload structure",
        });
    }
    let mut cursor = 0_usize;
    let mut entries = Vec::new();
    entries
        .try_reserve_exact(count)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    for _ in 0..count {
        if payload.len().saturating_sub(cursor) < 24 {
            return Err(DurabilityError::Corruption {
                offset: cursor,
                reason: "prepared cut capsule entry truncated",
            });
        }
        let prepare_lsn = read_u64(&payload[cursor..cursor + 8]);
        let payload_crc32c = read_u32(&payload[cursor + 8..cursor + 12]);
        let target_revision = RevisionId::new(read_u64(&payload[cursor + 12..cursor + 20]));
        let len = usize::try_from(read_u32(&payload[cursor + 20..cursor + 24]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        cursor += 24;
        let end = cursor
            .checked_add(len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let encoded = payload
            .get(cursor..end)
            .ok_or(DurabilityError::Corruption {
                offset: cursor,
                reason: "prepared cut capsule payload truncated",
            })?;
        let descriptor = decode_prepare_payload(target_revision, encoded).map_err(|reason| {
            DurabilityError::Corruption {
                offset: cursor,
                reason,
            }
        })?;
        entries.push(PreparedCutEntry {
            prepare_lsn,
            payload_crc32c,
            descriptor,
        });
        cursor = end;
    }
    if cursor != payload.len() {
        return Err(DurabilityError::Corruption {
            offset: cursor,
            reason: "prepared cut capsule has trailing bytes",
        });
    }
    Ok(PreparedCutCapsule { entries })
}

pub(super) fn write_prepared_cut_capsule(
    path: &Path,
    capsule: &PreparedCutCapsule,
) -> Result<u32, DurabilityError> {
    let bytes = encode_prepared_cut_capsule(capsule)?;
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(crc32c(&bytes))
}
