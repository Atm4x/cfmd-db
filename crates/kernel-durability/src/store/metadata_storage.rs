use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::binary_codec::{crc32c, crc32c_update, read_u16, read_u32, read_u64};
use crate::metadata;

use super::file_io::{read_exact_file_payload, read_exact_or_corruption, require_file_eof};
use super::format_registry::{DurableFormatRegistry, METADATA_FILE_TAG};
use super::generation_layout::metadata_path;
use super::manifest::ManifestRecord;
use crate::runtime::DurabilityError;

pub(super) const METADATA_MAGIC: [u8; 4] = *b"CFDM";
pub(super) const METADATA_HEADER_LEN: usize = 20;
const MAX_METADATA_LEN: usize = 64 * 1024 * 1024;

pub(super) fn read_metadata_bytes_bounded(path: &Path) -> Result<Vec<u8>, DurabilityError> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; METADATA_HEADER_LEN];
    read_exact_or_corruption(
        &mut file,
        &mut header,
        0,
        "durable metadata header truncated",
    )?;
    if header[..4] != METADATA_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata magic mismatch",
        });
    }
    DurableFormatRegistry::require_metadata(read_u16(&header[4..6]))?;
    if read_u16(&header[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported durable metadata flags",
        });
    }
    let payload_len =
        usize::try_from(read_u64(&header[8..16])).map_err(|_| DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload length overflow",
        })?;
    if payload_len > MAX_METADATA_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload length mismatch",
        });
    }
    let payload = read_exact_file_payload(
        &mut file,
        payload_len,
        METADATA_HEADER_LEN,
        "durable metadata payload truncated",
    )?;
    require_file_eof(
        &mut file,
        METADATA_HEADER_LEN + payload_len,
        "durable metadata file has trailing bytes",
    )?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(METADATA_HEADER_LEN + payload_len)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

pub(super) fn read_published_metadata(
    directory: &Path,
    manifest: ManifestRecord,
) -> Result<metadata::DurableStoreMetadata, DurabilityError> {
    let metadata_file = metadata_path(directory, manifest.generation);
    let metadata_bytes =
        read_metadata_bytes_bounded(&metadata_file).map_err(|error| match error {
            DurabilityError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
                DurabilityError::Corruption {
                    offset: 0,
                    reason: "published durable metadata file is missing",
                }
            }
            other => other,
        })?;
    if crc32c(&metadata_bytes) != manifest.metadata_crc32c {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published durable metadata file checksum mismatch",
        });
    }
    decode_metadata_file(&metadata_bytes)
}

pub(super) fn write_metadata_file(
    path: &Path,
    metadata: &metadata::DurableStoreMetadata,
) -> Result<u32, DurabilityError> {
    let mut payload_len = 0_u64;
    let mut payload_crc_state = !0_u32;
    metadata::stream(metadata, &mut |bytes| {
        let len = u64::try_from(bytes.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        payload_len = payload_len
            .checked_add(len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        payload_crc_state = crc32c_update(payload_crc_state, bytes);
        Ok(())
    })?;
    if payload_len > MAX_METADATA_LEN as u64 {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let payload_crc = !payload_crc_state;
    let mut header = [0_u8; METADATA_HEADER_LEN];
    header[0..4].copy_from_slice(&METADATA_MAGIC);
    header[4..6].copy_from_slice(&METADATA_FILE_TAG.to_le_bytes());
    header[6..8].copy_from_slice(&0_u16.to_le_bytes());
    header[8..16].copy_from_slice(&payload_len.to_le_bytes());
    header[16..20].copy_from_slice(&payload_crc.to_le_bytes());
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&header)?;
    let mut file_crc = crc32c_update(!0_u32, &header);
    metadata::stream(metadata, &mut |bytes| {
        file.write_all(bytes)?;
        file_crc = crc32c_update(file_crc, bytes);
        Ok(())
    })?;
    file.sync_all()?;
    Ok(!file_crc)
}

fn decode_metadata_file(bytes: &[u8]) -> Result<metadata::DurableStoreMetadata, DurabilityError> {
    if bytes.len() < METADATA_HEADER_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata header truncated",
        });
    }
    let header = &bytes[..METADATA_HEADER_LEN];
    if header[..4] != METADATA_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata magic mismatch",
        });
    }
    let version = read_u16(&header[4..6]);
    DurableFormatRegistry::require_metadata(version)?;
    if read_u16(&header[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported durable metadata flags",
        });
    }
    let payload_len =
        usize::try_from(read_u64(&header[8..16])).map_err(|_| DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload length overflow",
        })?;
    if payload_len > MAX_METADATA_LEN
        || METADATA_HEADER_LEN.checked_add(payload_len) != Some(bytes.len())
    {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload length mismatch",
        });
    }
    let payload = &bytes[METADATA_HEADER_LEN..];
    if crc32c(payload) != read_u32(&header[16..20]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload checksum mismatch",
        });
    }
    metadata::decode(payload).map_err(|reason| DurabilityError::Corruption { offset: 0, reason })
}
