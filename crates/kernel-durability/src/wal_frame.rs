use kernel_types::RevisionId;

use crate::binary_codec::{crc32c, read_u16, read_u32, read_u64};
use crate::runtime::{DurabilityError, TailStatus};

pub const MAGIC: [u8; 4] = *b"CFMW";
const WAL_FRAME_FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 36;
pub const MAX_PAYLOAD_LEN: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RecordKind {
    PrepareRevision = 1,
    CommitRevision = 2,
    ReplicationAuthority = 3,
    SealClientIntent = 4,
}

impl TryFrom<u8> for RecordKind {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::PrepareRevision),
            2 => Ok(Self::CommitRevision),
            3 => Ok(Self::ReplicationAuthority),
            4 => Ok(Self::SealClientIntent),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedFrame {
    pub(crate) lsn: u64,
    pub(crate) payload_crc32c: u32,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct DecodedFrame<'a> {
    pub(crate) offset: usize,
    pub(crate) kind: RecordKind,
    pub(crate) lsn: u64,
    pub(crate) revision: RevisionId,
    pub(crate) payload_crc: u32,
    pub(crate) payload: &'a [u8],
    pub(crate) frame_len: usize,
}

pub(crate) enum FrameRead<'a> {
    Complete(DecodedFrame<'a>),
    Tail(TailStatus),
}

pub(crate) fn read_frame(
    bytes: &[u8],
    offset: usize,
    expected_lsn: u64,
) -> Result<FrameRead<'_>, DurabilityError> {
    let remaining = bytes.len() - offset;
    if remaining < HEADER_LEN {
        let tail = &bytes[offset..];
        let prefix_len = tail.len().min(MAGIC.len());
        let looks_torn = tail[..prefix_len] == MAGIC[..prefix_len];
        return Ok(FrameRead::Tail(if looks_torn {
            TailStatus::Truncated { offset }
        } else {
            TailStatus::Garbage { offset }
        }));
    }
    if bytes[offset..offset + 4] != MAGIC {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "non-frame bytes large enough to hide a complete frame",
        });
    }
    let header = &bytes[offset..offset + HEADER_LEN];
    validate_frame_header(header, offset, expected_lsn)?;
    let payload_len =
        usize::try_from(read_u32(&header[8..12])).map_err(|_| DurabilityError::Corruption {
            offset,
            reason: "payload length overflow",
        })?;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "payload length exceeds hard limit",
        });
    }
    let frame_len = HEADER_LEN
        .checked_add(payload_len)
        .ok_or(DurabilityError::Corruption {
            offset,
            reason: "frame length overflow",
        })?;
    if remaining < frame_len {
        return Ok(FrameRead::Tail(TailStatus::Truncated { offset }));
    }
    let payload = &bytes[offset + HEADER_LEN..offset + frame_len];
    let payload_crc = read_u32(&header[28..32]);
    if crc32c(payload) != payload_crc {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "payload checksum mismatch",
        });
    }
    Ok(FrameRead::Complete(DecodedFrame {
        offset,
        kind: RecordKind::try_from(header[6]).map_err(|()| DurabilityError::Corruption {
            offset,
            reason: "unknown frame kind",
        })?,
        lsn: read_u64(&header[12..20]),
        revision: RevisionId::new(read_u64(&header[20..28])),
        payload_crc,
        payload,
        frame_len,
    }))
}

pub(crate) fn validate_frame_header(
    header: &[u8],
    offset: usize,
    expected_lsn: u64,
) -> Result<(), DurabilityError> {
    if read_u16(&header[4..6]) != WAL_FRAME_FORMAT_VERSION {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "unsupported/corrupt frame version",
        });
    }
    RecordKind::try_from(header[6]).map_err(|()| DurabilityError::Corruption {
        offset,
        reason: "unknown frame kind",
    })?;
    if header[7] != 0 {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "unknown frame flags",
        });
    }
    if crc32c(&header[..32]) != read_u32(&header[32..36]) {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "header checksum mismatch",
        });
    }
    if read_u64(&header[12..20]) != expected_lsn {
        return Err(DurabilityError::Protocol {
            offset,
            reason: "non-monotone or gapped LSN",
        });
    }
    Ok(())
}

pub(crate) fn encode_frame(
    lsn: u64,
    kind: RecordKind,
    revision: RevisionId,
    payload: &[u8],
) -> Result<EncodedFrame, DurabilityError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let payload_crc32c = crc32c(payload);
    let mut header = [0_u8; HEADER_LEN];
    header[0..4].copy_from_slice(&MAGIC);
    header[4..6].copy_from_slice(&WAL_FRAME_FORMAT_VERSION.to_le_bytes());
    header[6] = kind as u8;
    header[7] = 0;
    header[8..12].copy_from_slice(&payload_len.to_le_bytes());
    header[12..20].copy_from_slice(&lsn.to_le_bytes());
    header[20..28].copy_from_slice(&revision.raw().to_le_bytes());
    header[28..32].copy_from_slice(&payload_crc32c.to_le_bytes());
    let header_crc = crc32c(&header[..32]);
    header[32..36].copy_from_slice(&header_crc.to_le_bytes());
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(payload);
    Ok(EncodedFrame {
        lsn,
        payload_crc32c,
        bytes,
    })
}
