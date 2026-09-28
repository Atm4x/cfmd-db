use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use kernel_types::RevisionId;

use crate::binary_codec::{crc32c, read_u16, read_u32, read_u64};

use super::file_io::sync_directory;
use super::format_registry::{
    DurableFormatRegistry, LEGACY_MANIFEST_FORMAT_VERSION, MANIFEST_FORMAT_VERSION,
};
use super::generation_layout::{manifest_path, parse_generation_name};
use super::publication_protocol::{PublicationAttempt, StoreFaultHook, StoreFaultPoint};
use crate::runtime::DurabilityError;

const MANIFEST_MAGIC: [u8; 4] = *b"CFMF";
const LEGACY_MANIFEST_LEN: usize = 36;
const MANIFEST_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ManifestRecord {
    pub(super) generation: u64,
    pub(super) base_revision: RevisionId,
    pub(super) published_head: RevisionId,
    pub(super) wal_first_lsn: u64,
    pub(super) published_tail_lsn: u64,
    pub(super) checkpoint_crc32c: u32,
    pub(super) metadata_crc32c: u32,
    pub(super) prepared_capsule_crc32c: u32,
}

pub(super) fn read_manifest_bytes_bounded(path: &Path) -> Result<Vec<u8>, DurabilityError> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(u64::try_from(MANIFEST_LEN + 1).expect("manifest bound fits u64"))
        .read_to_end(&mut bytes)?;
    if bytes.len() > MANIFEST_LEN {
        return Err(DurabilityError::Corruption {
            offset: MANIFEST_LEN,
            reason: "manifest length mismatch",
        });
    }
    Ok(bytes)
}

pub(super) fn publish_manifest_with_hook(
    directory: &Path,
    manifest: ManifestRecord,
    hook: &mut impl StoreFaultHook,
    publication: &mut PublicationAttempt,
) -> Result<(), DurabilityError> {
    let final_path = manifest_path(directory, manifest.generation);
    let pending_path = directory.join(format!("pending-manifest-{:020}.tmp", manifest.generation));
    if pending_path.exists() {
        fs::remove_file(&pending_path)?;
    }
    let bytes = encode_manifest(manifest);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    hook.hit(StoreFaultPoint::AfterPendingManifestSync)?;
    // From the rename attempt onward, an I/O error cannot be interpreted as
    // proof that the old manifest is still the only authoritative generation.
    publication.mark_manifest_rename_attempted();
    fs::rename(&pending_path, &final_path)?;
    hook.hit(StoreFaultPoint::AfterManifestRename)?;
    sync_directory(directory)?;
    hook.hit(StoreFaultPoint::AfterManifestDirectorySync)?;
    Ok(())
}

pub(super) fn encode_manifest(manifest: ManifestRecord) -> [u8; MANIFEST_LEN] {
    let mut bytes = [0_u8; MANIFEST_LEN];
    bytes[0..4].copy_from_slice(&MANIFEST_MAGIC);
    bytes[4..6].copy_from_slice(&MANIFEST_FORMAT_VERSION.to_le_bytes());
    bytes[6..8].copy_from_slice(&0_u16.to_le_bytes());
    bytes[8..16].copy_from_slice(&manifest.generation.to_le_bytes());
    bytes[16..24].copy_from_slice(&manifest.base_revision.raw().to_le_bytes());
    bytes[24..32].copy_from_slice(&manifest.published_head.raw().to_le_bytes());
    bytes[32..40].copy_from_slice(&manifest.wal_first_lsn.to_le_bytes());
    bytes[40..48].copy_from_slice(&manifest.published_tail_lsn.to_le_bytes());
    bytes[48..52].copy_from_slice(&manifest.checkpoint_crc32c.to_le_bytes());
    bytes[52..56].copy_from_slice(&manifest.metadata_crc32c.to_le_bytes());
    bytes[56..60].copy_from_slice(&manifest.prepared_capsule_crc32c.to_le_bytes());
    let checksum = crc32c(&bytes[..60]);
    bytes[60..64].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

pub(super) fn decode_manifest(bytes: &[u8]) -> Result<ManifestRecord, DurabilityError> {
    if bytes.len() != MANIFEST_LEN && bytes.len() != LEGACY_MANIFEST_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "manifest length mismatch",
        });
    }
    if bytes[..4] != MANIFEST_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "manifest magic mismatch",
        });
    }
    let version = read_u16(&bytes[4..6]);
    DurableFormatRegistry::require_manifest(version)?;
    if read_u16(&bytes[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported manifest flags",
        });
    }
    if version == LEGACY_MANIFEST_FORMAT_VERSION {
        if bytes.len() != LEGACY_MANIFEST_LEN || crc32c(&bytes[..32]) != read_u32(&bytes[32..36]) {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "manifest checksum mismatch",
            });
        }
        let base_revision = RevisionId::new(read_u64(&bytes[16..24]));
        return Ok(ManifestRecord {
            generation: read_u64(&bytes[8..16]),
            base_revision,
            published_head: base_revision,
            wal_first_lsn: 1,
            published_tail_lsn: 0,
            checkpoint_crc32c: read_u32(&bytes[24..28]),
            metadata_crc32c: read_u32(&bytes[28..32]),
            prepared_capsule_crc32c: 0,
        });
    }
    if bytes.len() != MANIFEST_LEN || crc32c(&bytes[..60]) != read_u32(&bytes[60..64]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "manifest checksum mismatch",
        });
    }
    Ok(ManifestRecord {
        generation: read_u64(&bytes[8..16]),
        base_revision: RevisionId::new(read_u64(&bytes[16..24])),
        published_head: RevisionId::new(read_u64(&bytes[24..32])),
        wal_first_lsn: read_u64(&bytes[32..40]),
        published_tail_lsn: read_u64(&bytes[40..48]),
        checkpoint_crc32c: read_u32(&bytes[48..52]),
        metadata_crc32c: read_u32(&bytes[52..56]),
        prepared_capsule_crc32c: read_u32(&bytes[56..60]),
    })
}

pub(super) fn read_current_manifest(directory: &Path) -> Result<ManifestRecord, DurabilityError> {
    let generation = highest_manifest_generation(directory)?.ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "durable store has no published manifest",
    })?;
    let bytes = read_manifest_bytes_bounded(&manifest_path(directory, generation))?;
    let manifest = decode_manifest(&bytes)?;
    if manifest.generation != generation {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "manifest filename generation mismatch",
        });
    }
    Ok(manifest)
}

pub(super) fn highest_manifest_generation(
    directory: &Path,
) -> Result<Option<u64>, DurabilityError> {
    let mut highest = None;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(generation) = parse_generation_name(name, "manifest-", ".cfmf") {
            highest = Some(highest.map_or(generation, |current: u64| current.max(generation)));
        }
    }
    Ok(highest)
}
