use std::io::{Read, Seek, SeekFrom, Write};

use super::{ReplicationAuthorityJournal, corruption};
use crate::binary_codec::{crc32c, read_u16, read_u32};
use crate::replication::codec::{FRAME_HEADER_LEN, REPLICATION_MAGIC, REPLICATION_VERSION};
use crate::runtime::{CodecError, DurabilityError};
use crate::wal_frame::MAX_PAYLOAD_LEN;

impl ReplicationAuthorityJournal {
    pub(super) fn append_frame(&mut self, kind: u8, payload: &[u8]) -> Result<(), DurabilityError> {
        if payload.len() > MAX_PAYLOAD_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        let len = u32::try_from(payload.len()).map_err(|_| CodecError::LengthOverflow)?;
        let mut header = [0_u8; FRAME_HEADER_LEN];
        header[..4].copy_from_slice(&REPLICATION_MAGIC);
        header[4..6].copy_from_slice(&REPLICATION_VERSION.to_le_bytes());
        header[6] = kind;
        header[8..12].copy_from_slice(&len.to_le_bytes());
        header[12..16].copy_from_slice(&crc32c(payload).to_le_bytes());
        if let Err(error) = self
            .file
            .write_all(&header)
            .and_then(|()| self.file.write_all(payload))
        {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        if let Err(error) = self.file.sync_data() {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        Ok(())
    }

    pub(super) fn replay_file(&mut self) -> Result<(u64, u64), DurabilityError> {
        self.file.seek(SeekFrom::Start(0))?;
        let original_len = self.file.metadata()?.len();
        let header_len = u64::try_from(FRAME_HEADER_LEN).map_err(|_| CodecError::LengthOverflow)?;
        let mut offset = 0_u64;
        while original_len.saturating_sub(offset) >= header_len {
            let offset_usize = usize::try_from(offset).map_err(|_| CodecError::LengthOverflow)?;
            let mut header = [0_u8; FRAME_HEADER_LEN];
            self.file.read_exact(&mut header)?;
            if header[..4] != REPLICATION_MAGIC {
                return Err(corruption(
                    offset_usize,
                    "replication journal magic mismatch",
                ));
            }
            if read_u16(&header[4..6]) != REPLICATION_VERSION {
                return Err(corruption(
                    offset_usize,
                    "unsupported replication journal version",
                ));
            }
            if header[7] != 0 {
                return Err(corruption(
                    offset_usize,
                    "replication journal reserved header byte is non-zero",
                ));
            }
            let kind = header[6];
            let len = usize::try_from(read_u32(&header[8..12]))
                .map_err(|_| CodecError::LengthOverflow)?;
            if len > MAX_PAYLOAD_LEN {
                return Err(DurabilityError::PayloadTooLarge);
            }
            let frame_len = FRAME_HEADER_LEN
                .checked_add(len)
                .ok_or(CodecError::LengthOverflow)?;
            let end = offset
                .checked_add(u64::try_from(frame_len).map_err(|_| CodecError::LengthOverflow)?)
                .ok_or(CodecError::LengthOverflow)?;
            if end > original_len {
                break;
            }
            let mut payload = Vec::new();
            payload
                .try_reserve_exact(len)
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
            payload.resize(len, 0);
            self.file.read_exact(&mut payload)?;
            if crc32c(&payload) != read_u32(&header[12..16]) {
                return Err(corruption(
                    offset_usize,
                    "replication journal payload checksum mismatch",
                ));
            }
            self.apply_replay_frame(kind, &payload, offset_usize)?;
            offset = end;
        }
        Ok((offset, original_len))
    }
}
