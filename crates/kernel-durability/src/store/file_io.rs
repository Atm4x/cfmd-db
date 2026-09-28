use std::fs::File;
use std::io::Read;
use std::path::Path;

use crate::runtime::DurabilityError;

pub(super) fn read_exact_or_corruption(
    file: &mut File,
    bytes: &mut [u8],
    offset: usize,
    reason: &'static str,
) -> Result<(), DurabilityError> {
    file.read_exact(bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            DurabilityError::Corruption { offset, reason }
        } else {
            DurabilityError::Io(error)
        }
    })
}

pub(super) fn require_file_eof(
    file: &mut File,
    offset: usize,
    reason: &'static str,
) -> Result<(), DurabilityError> {
    let mut sentinel = [0_u8; 1];
    match file.read(&mut sentinel) {
        Ok(0) => Ok(()),
        Ok(_) => Err(DurabilityError::Corruption { offset, reason }),
        Err(error) => Err(DurabilityError::Io(error)),
    }
}

pub(super) fn read_exact_file_payload(
    file: &mut File,
    len: usize,
    offset: usize,
    truncated_reason: &'static str,
) -> Result<Vec<u8>, DurabilityError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(len)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    bytes.resize(len, 0);
    read_exact_or_corruption(file, &mut bytes, offset, truncated_reason)?;
    Ok(bytes)
}

pub(super) fn read_exact_sized_file(
    path: &Path,
    len: usize,
    truncated_reason: &'static str,
    trailing_reason: &'static str,
) -> Result<Vec<u8>, DurabilityError> {
    let mut file = File::open(path)?;
    let bytes = read_exact_file_payload(&mut file, len, 0, truncated_reason)?;
    require_file_eof(&mut file, len, trailing_reason)?;
    Ok(bytes)
}

pub(super) fn sync_directory(directory: &Path) -> Result<(), DurabilityError> {
    let directory = File::open(directory)?;
    directory.sync_all()?;
    Ok(())
}
