use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;
use kernel_types::RevisionId;

use crate::binary_codec::{crc32c, crc32c_update, read_u16, read_u32, read_u64};
use crate::checkpoint;

use super::file_io::{
    read_exact_file_payload, read_exact_or_corruption, read_exact_sized_file, require_file_eof,
};
use super::format_registry::{
    CHECKPOINT_FORMAT_VERSION, DurableFormatRegistry, LEGACY_CHECKPOINT_FORMAT_VERSION,
};
use super::generation_layout::{checkpoint_chunk_path, checkpoint_path};
use super::manifest::ManifestRecord;
use crate::runtime::DurabilityError;

pub(super) const CHECKPOINT_MAGIC: [u8; 4] = *b"CFCP";
pub(super) const CHECKPOINT_HEADER_LEN: usize = 32;
pub(super) const MAX_CHECKPOINT_LEN: usize = 512 * 1024 * 1024;
pub(super) const DEFAULT_CHECKPOINT_CHUNK_SIZE: usize = 1024 * 1024;
pub(super) const CHECKPOINT_CHUNK_DESCRIPTOR_LEN: usize = 16;

pub(super) fn read_checkpoint_root_bounded(path: &Path) -> Result<Vec<u8>, DurabilityError> {
    let mut file = File::open(path)?;
    let mut header = [0_u8; CHECKPOINT_HEADER_LEN];
    read_exact_or_corruption(&mut file, &mut header, 0, "checkpoint header truncated")?;
    if header[..4] != CHECKPOINT_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint magic mismatch",
        });
    }
    let version = read_u16(&header[4..6]);
    if crc32c(&header[..28]) != read_u32(&header[28..32]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header checksum mismatch",
        });
    }
    let tail_len = if version == LEGACY_CHECKPOINT_FORMAT_VERSION {
        if read_u16(&header[6..8]) != 0 {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "unsupported checkpoint file flags",
            });
        }
        let payload_len = usize::try_from(read_u64(&header[8..16]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        if payload_len > MAX_CHECKPOINT_LEN {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "checkpoint payload length exceeds hard limit",
            });
        }
        payload_len
    } else {
        DurableFormatRegistry::require_checkpoint_current(version)?;
        let logical_len = usize::try_from(read_u64(&header[8..16]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        if logical_len > MAX_CHECKPOINT_LEN {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "checkpoint logical stream exceeds configured bound",
            });
        }
        usize::from(read_u16(&header[6..8]))
            .checked_mul(CHECKPOINT_CHUNK_DESCRIPTOR_LEN)
            .ok_or(DurabilityError::PayloadTooLarge)?
    };
    let tail = read_exact_file_payload(
        &mut file,
        tail_len,
        CHECKPOINT_HEADER_LEN,
        "checkpoint file truncated",
    )?;
    require_file_eof(
        &mut file,
        CHECKPOINT_HEADER_LEN + tail_len,
        "checkpoint file has trailing bytes",
    )?;
    let mut root = Vec::new();
    root.try_reserve_exact(CHECKPOINT_HEADER_LEN + tail_len)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    root.extend_from_slice(&header);
    root.extend_from_slice(&tail);
    Ok(root)
}

pub(super) fn checkpoint_chunk_count(
    logical_len: usize,
    chunk_size: usize,
) -> Result<usize, DurabilityError> {
    if chunk_size == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint chunk size must be nonzero",
        });
    }
    let chunk_count = logical_len.div_ceil(chunk_size);
    u16::try_from(chunk_count).map_err(|_| DurabilityError::PayloadTooLarge)?;
    Ok(chunk_count)
}

pub(super) fn write_chunked_checkpoint_root(
    directory: &Path,
    generation: u64,
    revision: RevisionId,
    logical_len: usize,
    chunk_crcs: &[u32],
    chunk_size: usize,
) -> Result<u32, DurabilityError> {
    let expected_chunk_count = checkpoint_chunk_count(logical_len, chunk_size)?;
    if chunk_crcs.len() != expected_chunk_count {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint chunk count disagrees with logical stream layout",
        });
    }
    let descriptor_capacity = chunk_crcs
        .len()
        .checked_mul(CHECKPOINT_CHUNK_DESCRIPTOR_LEN)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    let mut descriptors = Vec::with_capacity(descriptor_capacity);
    for (ordinal, crc) in chunk_crcs.iter().copied().enumerate() {
        let start = ordinal.saturating_mul(chunk_size);
        let len = logical_len.saturating_sub(start).min(chunk_size);
        descriptors.extend_from_slice(
            &u32::try_from(ordinal)
                .map_err(|_| DurabilityError::PayloadTooLarge)?
                .to_le_bytes(),
        );
        descriptors.extend_from_slice(
            &u64::try_from(len)
                .map_err(|_| DurabilityError::PayloadTooLarge)?
                .to_le_bytes(),
        );
        descriptors.extend_from_slice(&crc.to_le_bytes());
    }
    let mut header = [0_u8; CHECKPOINT_HEADER_LEN];
    header[0..4].copy_from_slice(&CHECKPOINT_MAGIC);
    header[4..6].copy_from_slice(&CHECKPOINT_FORMAT_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(
        &u16::try_from(chunk_crcs.len())
            .map_err(|_| DurabilityError::PayloadTooLarge)?
            .to_le_bytes(),
    );
    header[8..16].copy_from_slice(
        &u64::try_from(logical_len)
            .map_err(|_| DurabilityError::PayloadTooLarge)?
            .to_le_bytes(),
    );
    header[16..24].copy_from_slice(&revision.raw().to_le_bytes());
    header[24..28].copy_from_slice(&crc32c(&descriptors).to_le_bytes());
    let header_crc = crc32c(&header[..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());
    let path = checkpoint_path(directory, generation);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(&header)?;
    file.write_all(&descriptors)?;
    file.sync_all()?;
    let crc = crc32c_update(!0_u32, &header);
    Ok(!crc32c_update(crc, &descriptors))
}

pub(super) fn validate_checkpoint_chunk_descriptors(
    descriptors: &[u8],
    chunk_count: usize,
    logical_len: usize,
) -> Result<(), DurabilityError> {
    let mut described_len = 0_usize;
    for ordinal in 0..chunk_count {
        let off = ordinal * CHECKPOINT_CHUNK_DESCRIPTOR_LEN;
        if usize::try_from(read_u32(&descriptors[off..off + 4])).ok() != Some(ordinal) {
            return Err(DurabilityError::Corruption {
                offset: off,
                reason: "checkpoint chunk ordinal mismatch",
            });
        }
        let len = usize::try_from(read_u64(&descriptors[off + 4..off + 12]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        described_len = described_len
            .checked_add(len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if described_len > MAX_CHECKPOINT_LEN {
            return Err(DurabilityError::Corruption {
                offset: off,
                reason: "checkpoint chunk descriptors exceed configured bound",
            });
        }
    }
    if described_len != logical_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root logical length disagrees with chunk descriptors",
        });
    }
    Ok(())
}

pub(super) fn read_checkpoint_generation(
    directory: &Path,
    manifest: ManifestRecord,
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    let root = read_checkpoint_root_bounded(&checkpoint_path(directory, manifest.generation))?;
    if crc32c(&root) != manifest.checkpoint_crc32c {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published checkpoint file checksum mismatch",
        });
    }
    if root.len() < CHECKPOINT_HEADER_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header truncated",
        });
    }
    let version = read_u16(&root[4..6]);
    if version == LEGACY_CHECKPOINT_FORMAT_VERSION {
        return decode_checkpoint_file(&root, registry);
    }
    DurableFormatRegistry::require_checkpoint_current(version)?;
    let header = &root[..CHECKPOINT_HEADER_LEN];
    if header[..4] != CHECKPOINT_MAGIC || crc32c(&header[..28]) != read_u32(&header[28..32]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root header mismatch",
        });
    }
    let chunk_count = usize::from(read_u16(&header[6..8]));
    let descriptor_len = chunk_count
        .checked_mul(CHECKPOINT_CHUNK_DESCRIPTOR_LEN)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    if root.len() != CHECKPOINT_HEADER_LEN + descriptor_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root descriptor length mismatch",
        });
    }
    let descriptors = &root[CHECKPOINT_HEADER_LEN..];
    if crc32c(descriptors) != read_u32(&header[24..28]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root descriptor checksum mismatch",
        });
    }
    let logical_len =
        usize::try_from(read_u64(&header[8..16])).map_err(|_| DurabilityError::PayloadTooLarge)?;
    if logical_len > MAX_CHECKPOINT_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint logical stream exceeds configured bound",
        });
    }
    validate_checkpoint_chunk_descriptors(descriptors, chunk_count, logical_len)?;
    let mut logical = Vec::new();
    logical
        .try_reserve_exact(logical_len)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    for ordinal in 0..chunk_count {
        let off = ordinal * CHECKPOINT_CHUNK_DESCRIPTOR_LEN;
        let len = usize::try_from(read_u64(&descriptors[off + 4..off + 12]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        let expected_crc = read_u32(&descriptors[off + 12..off + 16]);
        let chunk = read_exact_sized_file(
            &checkpoint_chunk_path(directory, manifest.generation, ordinal),
            len,
            "checkpoint chunk truncated",
            "checkpoint chunk has trailing bytes",
        )?;
        if chunk.len() != len || crc32c(&chunk) != expected_crc {
            return Err(DurabilityError::Corruption {
                offset: ordinal,
                reason: "checkpoint chunk integrity mismatch",
            });
        }
        logical.extend_from_slice(&chunk);
    }
    if logical.len() != logical_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint logical stream length mismatch",
        });
    }
    let revision = checkpoint::decode_revision(&logical, registry)?;
    if revision.id().raw() != read_u64(&header[16..24]) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint root cut revision mismatch",
        });
    }
    Ok(revision)
}

pub(super) fn write_checkpoint_file(
    path: &Path,
    revision: &Revision,
) -> Result<u32, DurabilityError> {
    let mut payload_len = 0_u64;
    let mut payload_crc_state = !0_u32;
    checkpoint::stream_revision(revision, &mut |bytes| {
        let len = u64::try_from(bytes.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        payload_len = payload_len
            .checked_add(len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        payload_crc_state = crc32c_update(payload_crc_state, bytes);
        Ok(())
    })?;
    if payload_len > MAX_CHECKPOINT_LEN as u64 {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let payload_crc = !payload_crc_state;
    let mut header = [0_u8; CHECKPOINT_HEADER_LEN];
    header[0..4].copy_from_slice(&CHECKPOINT_MAGIC);
    header[4..6].copy_from_slice(&LEGACY_CHECKPOINT_FORMAT_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(&0_u16.to_le_bytes());
    header[8..16].copy_from_slice(&payload_len.to_le_bytes());
    header[16..24].copy_from_slice(&revision.id().raw().to_le_bytes());
    header[24..28].copy_from_slice(&payload_crc.to_le_bytes());
    let header_crc = crc32c(&header[..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&header)?;
    let mut file_crc = crc32c_update(!0_u32, &header);
    checkpoint::stream_revision(revision, &mut |bytes| {
        file.write_all(bytes)?;
        file_crc = crc32c_update(file_crc, bytes);
        Ok(())
    })?;
    file.sync_all()?;
    Ok(!file_crc)
}

fn decode_checkpoint_file(
    bytes: &[u8],
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    if bytes.len() < CHECKPOINT_HEADER_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header truncated",
        });
    }
    let header = &bytes[..CHECKPOINT_HEADER_LEN];
    if header[..4] != CHECKPOINT_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint magic mismatch",
        });
    }
    let version = read_u16(&header[4..6]);
    DurableFormatRegistry::require_checkpoint_legacy(version)?;
    if read_u16(&header[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported checkpoint file flags",
        });
    }
    if crc32c(&header[..28]) != read_u32(&header[28..32]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header checksum mismatch",
        });
    }
    let payload_len =
        usize::try_from(read_u64(&header[8..16])).map_err(|_| DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint payload length overflow",
        })?;
    if payload_len > MAX_CHECKPOINT_LEN
        || CHECKPOINT_HEADER_LEN.checked_add(payload_len) != Some(bytes.len())
    {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint payload length mismatch",
        });
    }
    let payload = &bytes[CHECKPOINT_HEADER_LEN..];
    if crc32c(payload) != read_u32(&header[24..28]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint payload checksum mismatch",
        });
    }
    let revision = checkpoint::decode_revision(payload, registry)?;
    if revision.id().raw() != read_u64(&header[16..24]) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint header revision mismatch",
        });
    }
    Ok(revision)
}
