use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::runtime::DurabilityError;

const LOCK_FILE_NAME: &str = ".cfmd-durability.lock";

pub(super) fn lock_directory(directory: &Path) -> Result<File, DurabilityError> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(LOCK_FILE_NAME))?;
    lock.lock()?;
    Ok(lock)
}

pub(super) fn checkpoint_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("checkpoint-{generation:020}.cfcp"))
}

pub(super) fn checkpoint_stream_spool_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("checkpoint-{generation:020}-stream.tmp"))
}

pub(super) fn checkpoint_chunk_path(directory: &Path, generation: u64, ordinal: usize) -> PathBuf {
    directory.join(format!(
        "checkpoint-{generation:020}-chunk-{ordinal:08}.cfck"
    ))
}

pub(super) fn prepared_capsule_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("prepared-{generation:020}.cfpc"))
}

pub(super) fn wal_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("wal-{generation:020}.cfmw"))
}

pub(super) fn manifest_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("manifest-{generation:020}.cfmf"))
}

pub(super) fn metadata_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("metadata-{generation:020}.cfdm"))
}

pub(super) fn realization_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("realization-{generation:020}.cfpr"))
}

pub(super) fn parse_generation_name(name: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let raw = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    (raw.len() == 20).then(|| raw.parse().ok()).flatten()
}

pub(super) fn parse_checkpoint_chunk_generation(name: &str) -> Option<u64> {
    let raw = name.strip_prefix("checkpoint-")?;
    let (generation, rest) = raw.split_once("-chunk-")?;
    let ordinal = rest.strip_suffix(".cfck")?;
    if generation.len() != 20
        || ordinal.len() != 8
        || !ordinal.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    generation.parse().ok()
}

pub(super) fn remove_orphan_checkpoint_stream_spools(
    directory: &Path,
) -> Result<(), DurabilityError> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if parse_generation_name(name, "checkpoint-", "-stream.tmp").is_some() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

pub(super) fn next_generation(directory: &Path) -> Result<u64, DurabilityError> {
    let mut highest = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let generation = parse_generation_name(name, "manifest-", ".cfmf")
            .or_else(|| parse_generation_name(name, "checkpoint-", ".cfcp"))
            .or_else(|| parse_generation_name(name, "wal-", ".cfmw"))
            .or_else(|| parse_generation_name(name, "metadata-", ".cfdm"))
            .or_else(|| parse_generation_name(name, "realization-", ".cfpr"))
            .or_else(|| parse_generation_name(name, "prepared-", ".cfpc"))
            .or_else(|| parse_checkpoint_chunk_generation(name))
            .or_else(|| parse_generation_name(name, "checkpoint-", "-stream.tmp"))
            .or_else(|| parse_generation_name(name, "pending-manifest-", ".tmp"));
        if let Some(generation) = generation {
            highest = highest.max(generation);
        }
    }
    highest.checked_add(1).ok_or(DurabilityError::LsnExhausted)
}
