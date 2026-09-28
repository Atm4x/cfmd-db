use std::io::{Read, Seek, SeekFrom, Write};

use super::{ReplicationAuthorityJournal, corruption};
use crate::binary_codec::{crc32c, read_u16, read_u32};
use crate::replication::codec::{FRAME_HEADER_LEN, REPLICATION_MAGIC, REPLICATION_VERSION};
use crate::runtime::{CodecError, DurabilityError};
use crate::wal_frame::MAX_PAYLOAD_LEN;

fn encode_replication_frame(kind: u8, payload: &[u8]) -> Result<Vec<u8>, DurabilityError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let len = u32::try_from(payload.len()).map_err(|_| CodecError::LengthOverflow)?;
    let total = FRAME_HEADER_LEN
        .checked_add(payload.len())
        .ok_or(CodecError::LengthOverflow)?;
    let mut frame = vec![0_u8; total];
    frame[..4].copy_from_slice(&REPLICATION_MAGIC);
    frame[4..6].copy_from_slice(&REPLICATION_VERSION.to_le_bytes());
    frame[6] = kind;
    frame[8..12].copy_from_slice(&len.to_le_bytes());
    frame[12..16].copy_from_slice(&crc32c(payload).to_le_bytes());
    frame[FRAME_HEADER_LEN..].copy_from_slice(payload);
    Ok(frame)
}

fn decode_replication_frame(frame: &[u8], offset: usize) -> Result<(u8, &[u8]), DurabilityError> {
    if frame.len() < FRAME_HEADER_LEN {
        return Err(corruption(offset, "replication journal frame is truncated"));
    }
    if frame[..4] != REPLICATION_MAGIC {
        return Err(corruption(offset, "replication journal magic mismatch"));
    }
    if !matches!(read_u16(&frame[4..6]), 1 | REPLICATION_VERSION) {
        return Err(corruption(
            offset,
            "unsupported replication journal version",
        ));
    }
    if frame[7] != 0 {
        return Err(corruption(
            offset,
            "replication journal reserved header byte is non-zero",
        ));
    }
    let len = usize::try_from(read_u32(&frame[8..12])).map_err(|_| CodecError::LengthOverflow)?;
    if len > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let frame_len = FRAME_HEADER_LEN
        .checked_add(len)
        .ok_or(CodecError::LengthOverflow)?;
    if frame.len() != frame_len {
        return Err(corruption(
            offset,
            "replication journal frame length mismatch",
        ));
    }
    let payload = &frame[FRAME_HEADER_LEN..];
    if crc32c(payload) != read_u32(&frame[12..16]) {
        return Err(corruption(
            offset,
            "replication journal payload checksum mismatch",
        ));
    }
    Ok((frame[6], payload))
}

impl ReplicationAuthorityJournal {
    pub(super) fn append_frame(&mut self, kind: u8, payload: &[u8]) -> Result<(), DurabilityError> {
        let encoded = encode_replication_frame(kind, payload)?;
        if self.single_file_capture {
            self.pending_single_file_frames.push(encoded);
            return Ok(());
        }
        let file = self.file.as_mut().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "replication journal has no persistence backend",
        })?;
        if let Err(error) = file.write_all(&encoded) {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        if let Err(error) = file.sync_data() {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        Ok(())
    }

    pub(super) fn replay_single_file_archive(
        &mut self,
        bytes: &[u8],
    ) -> Result<(), DurabilityError> {
        let mut offset = 0_usize;
        while offset < bytes.len() {
            if bytes.len() - offset < FRAME_HEADER_LEN {
                return Err(corruption(
                    offset,
                    "single-file replication archive has truncated frame header",
                ));
            }
            let len = usize::try_from(read_u32(&bytes[offset + 8..offset + 12]))
                .map_err(|_| CodecError::LengthOverflow)?;
            let frame_len = FRAME_HEADER_LEN
                .checked_add(len)
                .ok_or(CodecError::LengthOverflow)?;
            let end = offset
                .checked_add(frame_len)
                .ok_or(CodecError::LengthOverflow)?;
            if end > bytes.len() {
                return Err(corruption(
                    offset,
                    "single-file replication archive has truncated frame",
                ));
            }
            self.replay_single_file_frame(&bytes[offset..end], false)?;
            offset = end;
        }
        Ok(())
    }

    pub(super) fn replay_single_file_frame(
        &mut self,
        frame: &[u8],
        retain_live: bool,
    ) -> Result<(), DurabilityError> {
        let (kind, payload) = decode_replication_frame(frame, 0)?;
        self.apply_replay_frame(kind, payload, 0)?;
        if retain_live {
            self.live_single_file_frames.push(frame.to_vec());
        }
        Ok(())
    }

    pub(super) fn replay_file(&mut self) -> Result<(u64, u64), DurabilityError> {
        let Some(file) = self.file.as_mut() else {
            return Ok((0, 0));
        };
        file.seek(SeekFrom::Start(0))?;
        let original_len = file.metadata()?.len();
        let _ = file;
        let header_len = u64::try_from(FRAME_HEADER_LEN).map_err(|_| CodecError::LengthOverflow)?;
        let mut offset = 0_u64;
        while original_len.saturating_sub(offset) >= header_len {
            let offset_usize = usize::try_from(offset).map_err(|_| CodecError::LengthOverflow)?;
            let mut header = [0_u8; FRAME_HEADER_LEN];
            self.file
                .as_mut()
                .expect("file-backed replication journal")
                .read_exact(&mut header)?;
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
            let mut frame = Vec::new();
            frame
                .try_reserve_exact(frame_len)
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
            frame.extend_from_slice(&header);
            frame.resize(frame_len, 0);
            self.file
                .as_mut()
                .expect("file-backed replication journal")
                .read_exact(&mut frame[FRAME_HEADER_LEN..])?;
            let (kind, payload) = decode_replication_frame(&frame, offset_usize)?;
            self.apply_replay_frame(kind, payload, offset_usize)?;
            offset = end;
        }
        Ok((offset, original_len))
    }
}
