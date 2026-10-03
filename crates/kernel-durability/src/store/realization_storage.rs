use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use crate::binary_codec::crc32c_update;
use crate::metadata::DurableCheckpointRealizationBinding;
use crate::realization::DurableFactorizedRealization;
use crate::runtime::DurabilityError;

use super::generation_layout::realization_path;

pub(super) fn write_factorized_realization_file(
    path: &Path,
    realization: &DurableFactorizedRealization,
) -> Result<DurableCheckpointRealizationBinding, DurabilityError> {
    let expected_len = realization.encoded_len()?;
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    let mut len = 0_u64;
    let mut crc = !0_u32;
    realization.stream(&mut |bytes| {
        file.write_all(bytes)?;
        len = len
            .checked_add(u64::try_from(bytes.len()).map_err(|_| DurabilityError::PayloadTooLarge)?)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        crc = crc32c_update(crc, bytes);
        Ok(())
    })?;
    if len != expected_len {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "durable realization streaming length changed during publication",
        });
    }
    file.sync_all()?;
    Ok(DurableCheckpointRealizationBinding {
        revision: realization.revision(),
        encoded_len: len,
        crc32c: !crc,
    })
}

pub(super) fn read_published_factorized_realization(
    directory: &Path,
    generation: u64,
    binding: DurableCheckpointRealizationBinding,
) -> Result<DurableFactorizedRealization, DurabilityError> {
    let path = realization_path(directory, generation);
    let mut file = File::open(&path).map_err(|error| match error {
        error if error.kind() == std::io::ErrorKind::NotFound => DurabilityError::Corruption {
            offset: 0,
            reason: "published durable realization file is missing",
        },
        error => DurabilityError::Io(error),
    })?;
    let actual_len = file.metadata()?.len();
    if actual_len != binding.encoded_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published durable realization length mismatch",
        });
    }
    let mut crc = !0_u32;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        crc = crc32c_update(crc, &buffer[..read]);
    }
    if !crc != binding.crc32c {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published durable realization checksum mismatch",
        });
    }
    let mut file = File::open(path)?;
    let realization =
        DurableFactorizedRealization::decode_from_reader(&mut file, binding.encoded_len)?;
    if realization.revision() != binding.revision {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published durable realization revision binding mismatch",
        });
    }
    Ok(realization)
}
