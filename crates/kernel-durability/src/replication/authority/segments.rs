use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::io::SeekFrom;
use std::io::{Read, Seek};

use kernel_auth::Sha256Digest;
use sha2::{Digest, Sha256};

use super::ReplicationAuthorityJournal;
use crate::binary_codec::read_u32;
use crate::replication::codec::FRAME_HEADER_LEN;
use crate::runtime::{CodecError, DurabilityError};
use crate::wal_frame::MAX_PAYLOAD_LEN;

const SEGMENT_DOMAIN: &[u8] = b"CFMD/replication-authority-segment/v2";
const SEGMENT_MAGIC: [u8; 4] = *b"CFAS";
const SEGMENT_VERSION: u8 = 2;
const SEGMENT_HEADER_LEN: usize = 88;
const SEGMENT_ID_OFFSET: usize = 40;
const SEGMENT_DELTA_LEN_OFFSET: usize = 72;
const SEGMENT_FRAME_COUNT_OFFSET: usize = 80;
const MAX_SEGMENT_DELTA_LEN: u64 = 1_u64 << 34;
const MAX_SEGMENT_FRAME_COUNT: u32 = 1 << 20;

#[cfg(test)]
const INDEX_MAGIC: [u8; 4] = *b"CFAI";
#[cfg(test)]
const INDEX_VERSION: u8 = 1;
#[cfg(test)]
const INDEX_HEADER_LEN: usize = 48;
#[cfg(test)]
const INDEX_ENTRY_LEN: usize = 80;
const MAX_INDEX_ENTRIES: usize = 65_535;

mod locator;
mod object_codec;

pub(crate) use locator::{
    ReplicationAuthorityLocatorRoot, locator_stored_len, recover_locator_chain, write_locator_node,
};
pub(crate) use object_codec::{replay_segment_object, write_segment_object};

fn corruption(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ReplicationAuthoritySegmentId(pub(crate) Sha256Digest);

impl ReplicationAuthoritySegmentId {
    const ZERO_BYTES: [u8; 32] = [0; 32];

    pub(crate) fn from_bytes(bytes: [u8; 32]) -> Result<Self, DurabilityError> {
        if bytes == Self::ZERO_BYTES {
            return Err(corruption("replication authority segment id is zero"));
        }
        Ok(Self(Sha256Digest(bytes)))
    }

    pub(crate) const fn bytes(self) -> [u8; 32] {
        self.0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplicationAuthoritySegmentExtent {
    pub(crate) offset: u64,
    pub(crate) len: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplicationAuthoritySegmentIndexEntry {
    parent: Option<ReplicationAuthoritySegmentId>,
    extent: ReplicationAuthoritySegmentExtent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplicationAuthoritySegmentIndex {
    root: Option<ReplicationAuthoritySegmentId>,
    entries: BTreeMap<ReplicationAuthoritySegmentId, ReplicationAuthoritySegmentIndexEntry>,
}

impl ReplicationAuthoritySegmentIndex {
    pub(crate) fn empty() -> Self {
        Self {
            root: None,
            entries: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> Option<ReplicationAuthoritySegmentId> {
        self.root
    }

    pub(crate) fn insert(
        &mut self,
        id: ReplicationAuthoritySegmentId,
        parent: Option<ReplicationAuthoritySegmentId>,
        extent: ReplicationAuthoritySegmentExtent,
    ) -> Result<(), DurabilityError> {
        if extent.len < u64::try_from(SEGMENT_HEADER_LEN).map_err(|_| CodecError::LengthOverflow)? {
            return Err(corruption(
                "replication authority segment extent is too short",
            ));
        }
        extent
            .offset
            .checked_add(extent.len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if let Some(existing) = self.entries.get(&id) {
            return if *existing == (ReplicationAuthoritySegmentIndexEntry { parent, extent }) {
                Ok(())
            } else {
                Err(corruption(
                    "replication authority segment id has conflicting extent",
                ))
            };
        }
        if self.entries.len() >= MAX_INDEX_ENTRIES {
            return Err(DurabilityError::PayloadTooLarge);
        }
        self.entries
            .insert(id, ReplicationAuthoritySegmentIndexEntry { parent, extent });
        Ok(())
    }

    pub(crate) fn set_root(
        &mut self,
        root: Option<ReplicationAuthoritySegmentId>,
    ) -> Result<(), DurabilityError> {
        if let Some(root) = root
            && !self.entries.contains_key(&root)
        {
            return Err(corruption(
                "replication authority root segment is absent from index",
            ));
        }
        self.root = root;
        self.validate_chain()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn relocate(
        &mut self,
        id: ReplicationAuthoritySegmentId,
        extent: ReplicationAuthoritySegmentExtent,
    ) -> Result<(), DurabilityError> {
        if extent.len < u64::try_from(SEGMENT_HEADER_LEN).map_err(|_| CodecError::LengthOverflow)? {
            return Err(corruption(
                "replication authority relocated extent is too short",
            ));
        }
        extent
            .offset
            .checked_add(extent.len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let entry = self.entries.get_mut(&id).ok_or_else(|| {
            corruption("replication authority relocation references unknown segment")
        })?;
        entry.extent = extent;
        Ok(())
    }

    fn validate_chain(&self) -> Result<(), DurabilityError> {
        let Some(mut current) = self.root else {
            if self.entries.is_empty() {
                return Ok(());
            }
            return Err(corruption(
                "replication authority index has entries without an authority root",
            ));
        };
        let mut seen = BTreeSet::new();
        loop {
            if !seen.insert(current) {
                return Err(corruption(
                    "replication authority segment chain contains a cycle",
                ));
            }
            let entry = self.entries.get(&current).ok_or_else(|| {
                corruption("replication authority segment chain has missing parent")
            })?;
            let Some(parent) = entry.parent else {
                break;
            };
            current = parent;
        }
        if seen.len() != self.entries.len() {
            return Err(corruption(
                "replication authority index contains unreachable segments",
            ));
        }
        let mut extents = self
            .entries
            .values()
            .map(|entry| entry.extent)
            .collect::<Vec<_>>();
        extents.sort_by_key(|extent| extent.offset);
        for pair in extents.windows(2) {
            let previous_end = pair[0]
                .offset
                .checked_add(pair[0].len)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            if previous_end > pair[1].offset {
                return Err(corruption("replication authority segment extents overlap"));
            }
        }
        Ok(())
    }

    fn chain_oldest_first(
        &self,
    ) -> Result<
        Vec<(
            ReplicationAuthoritySegmentId,
            ReplicationAuthoritySegmentIndexEntry,
        )>,
        DurabilityError,
    > {
        self.validate_chain()?;
        let Some(mut current) = self.root else {
            return Ok(Vec::new());
        };
        let mut chain = Vec::new();
        loop {
            let entry = *self.entries.get(&current).ok_or_else(|| {
                corruption("replication authority segment chain has missing entry")
            })?;
            chain.push((current, entry));
            let Some(parent) = entry.parent else {
                break;
            };
            current = parent;
        }
        chain.reverse();
        Ok(chain)
    }

    pub(crate) fn reachable_chain(
        &self,
    ) -> Result<
        Vec<(
            ReplicationAuthoritySegmentId,
            Option<ReplicationAuthoritySegmentId>,
            ReplicationAuthoritySegmentExtent,
        )>,
        DurabilityError,
    > {
        self.chain_oldest_first().map(|chain| {
            chain
                .into_iter()
                .map(|(id, entry)| (id, entry.parent, entry.extent))
                .collect()
        })
    }

    #[cfg(test)]
    pub(crate) fn encode(&self) -> Result<Vec<u8>, DurabilityError> {
        self.validate_chain()?;
        let count = u32::try_from(self.entries.len()).map_err(|_| CodecError::LengthOverflow)?;
        let total_len = INDEX_HEADER_LEN
            .checked_add(
                self.entries
                    .len()
                    .checked_mul(INDEX_ENTRY_LEN)
                    .ok_or(CodecError::LengthOverflow)?,
            )
            .ok_or(CodecError::LengthOverflow)?;
        let mut bytes = vec![0_u8; total_len];
        bytes[0..4].copy_from_slice(&INDEX_MAGIC);
        bytes[4] = INDEX_VERSION;
        if let Some(root) = self.root {
            bytes[8..40].copy_from_slice(&root.bytes());
        }
        bytes[40..44].copy_from_slice(&count.to_le_bytes());
        let mut cursor = INDEX_HEADER_LEN;
        for (id, entry) in &self.entries {
            bytes[cursor..cursor + 32].copy_from_slice(&id.bytes());
            if let Some(parent) = entry.parent {
                bytes[cursor + 32..cursor + 64].copy_from_slice(&parent.bytes());
            }
            bytes[cursor + 64..cursor + 72].copy_from_slice(&entry.extent.offset.to_le_bytes());
            bytes[cursor + 72..cursor + 80].copy_from_slice(&entry.extent.len.to_le_bytes());
            cursor += INDEX_ENTRY_LEN;
        }
        Ok(bytes)
    }

    #[cfg(test)]
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, DurabilityError> {
        if bytes.len() < INDEX_HEADER_LEN
            || bytes[0..4] != INDEX_MAGIC
            || bytes[4] != INDEX_VERSION
            || bytes[5..8] != [0, 0, 0]
            || bytes[44..48] != [0, 0, 0, 0]
        {
            return Err(corruption(
                "replication authority segment index header is invalid",
            ));
        }
        let count = usize::try_from(u32::from_le_bytes(
            bytes[40..44].try_into().expect("4 bytes"),
        ))
        .map_err(|_| CodecError::LengthOverflow)?;
        if count > MAX_INDEX_ENTRIES {
            return Err(DurabilityError::PayloadTooLarge);
        }
        let expected_len = INDEX_HEADER_LEN
            .checked_add(
                count
                    .checked_mul(INDEX_ENTRY_LEN)
                    .ok_or(CodecError::LengthOverflow)?,
            )
            .ok_or(CodecError::LengthOverflow)?;
        if bytes.len() != expected_len {
            return Err(corruption(
                "replication authority segment index length is invalid",
            ));
        }
        let root_raw: [u8; 32] = bytes[8..40].try_into().expect("32 bytes");
        let root = if root_raw == ReplicationAuthoritySegmentId::ZERO_BYTES {
            None
        } else {
            Some(ReplicationAuthoritySegmentId::from_bytes(root_raw)?)
        };
        let mut index = Self::empty();
        let mut cursor = INDEX_HEADER_LEN;
        for _ in 0..count {
            let id = ReplicationAuthoritySegmentId::from_bytes(
                bytes[cursor..cursor + 32].try_into().expect("32 bytes"),
            )?;
            let parent_raw: [u8; 32] = bytes[cursor + 32..cursor + 64]
                .try_into()
                .expect("32 bytes");
            let parent = if parent_raw == ReplicationAuthoritySegmentId::ZERO_BYTES {
                None
            } else {
                Some(ReplicationAuthoritySegmentId::from_bytes(parent_raw)?)
            };
            let offset =
                u64::from_le_bytes(bytes[cursor + 64..cursor + 72].try_into().expect("8 bytes"));
            let len =
                u64::from_le_bytes(bytes[cursor + 72..cursor + 80].try_into().expect("8 bytes"));
            index.insert(
                id,
                parent,
                ReplicationAuthoritySegmentExtent { offset, len },
            )?;
            cursor += INDEX_ENTRY_LEN;
        }
        index.set_root(root)?;
        Ok(index)
    }
}

pub(crate) trait ReplicationAuthorityFrameSource {
    fn is_empty(&self) -> bool;

    fn for_each_frame(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError>;
}

pub(crate) struct ReplicationAuthorityFrameSlice<'a> {
    frames: &'a [Vec<u8>],
}

impl<'a> ReplicationAuthorityFrameSlice<'a> {
    pub(crate) const fn new(frames: &'a [Vec<u8>]) -> Self {
        Self { frames }
    }
}

impl ReplicationAuthorityFrameSource for ReplicationAuthorityFrameSlice<'_> {
    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    fn for_each_frame(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        for frame in self.frames {
            emit(frame)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReplicationAuthoritySegmentPlan {
    id: ReplicationAuthoritySegmentId,
    parent: Option<ReplicationAuthoritySegmentId>,
    delta_len: u64,
    frame_count: u32,
}

impl ReplicationAuthoritySegmentPlan {
    pub(crate) fn from_source<S: ReplicationAuthorityFrameSource + ?Sized>(
        parent: Option<ReplicationAuthoritySegmentId>,
        source: &S,
    ) -> Result<Self, DurabilityError> {
        let mut measurement = ReplicationAuthoritySegmentMeasurement::new();
        source.for_each_frame(&mut |frame| measurement.observe(frame))?;
        measurement.finish(parent)
    }

    pub(crate) fn id(&self) -> ReplicationAuthoritySegmentId {
        self.id
    }

    pub(crate) fn parent(&self) -> Option<ReplicationAuthoritySegmentId> {
        self.parent
    }

    pub(crate) fn encoded_len(&self) -> Result<u64, DurabilityError> {
        u64::try_from(SEGMENT_HEADER_LEN)
            .map_err(|_| DurabilityError::PayloadTooLarge)?
            .checked_add(self.delta_len)
            .ok_or(DurabilityError::PayloadTooLarge)
    }

    pub(crate) fn write_source_to<S: ReplicationAuthorityFrameSource + ?Sized>(
        &self,
        source: &S,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        let mut header = [0_u8; SEGMENT_HEADER_LEN];
        header[0..4].copy_from_slice(&SEGMENT_MAGIC);
        header[4] = SEGMENT_VERSION;
        if let Some(parent) = self.parent {
            header[8..40].copy_from_slice(&parent.bytes());
        }
        header[SEGMENT_ID_OFFSET..SEGMENT_ID_OFFSET + 32].copy_from_slice(&self.id.bytes());
        header[SEGMENT_DELTA_LEN_OFFSET..SEGMENT_DELTA_LEN_OFFSET + 8]
            .copy_from_slice(&self.delta_len.to_le_bytes());
        header[SEGMENT_FRAME_COUNT_OFFSET..SEGMENT_FRAME_COUNT_OFFSET + 4]
            .copy_from_slice(&self.frame_count.to_le_bytes());
        emit(&header)?;

        let mut measurement = ReplicationAuthoritySegmentMeasurement::new();
        source.for_each_frame(&mut |frame| {
            measurement.observe(frame)?;
            if measurement.delta_len > self.delta_len || measurement.frame_count > self.frame_count
            {
                return Err(corruption(
                    "replication authority segment source exceeded its frozen plan",
                ));
            }
            emit(frame)
        })?;
        let actual = measurement.finish(self.parent)?;
        if actual.id != self.id
            || actual.delta_len != self.delta_len
            || actual.frame_count != self.frame_count
        {
            return Err(corruption(
                "replication authority segment plan no longer matches frame source",
            ));
        }
        Ok(())
    }
}

struct ReplicationAuthoritySegmentMeasurement {
    delta_len: u64,
    frame_count: u32,
    frame_hasher: Sha256,
}

impl ReplicationAuthoritySegmentMeasurement {
    fn new() -> Self {
        Self {
            delta_len: 0,
            frame_count: 0,
            frame_hasher: Sha256::new(),
        }
    }

    fn observe(&mut self, frame: &[u8]) -> Result<(), DurabilityError> {
        validate_frame_shape(frame)?;
        let len = u64::try_from(frame.len()).map_err(|_| CodecError::LengthOverflow)?;
        self.delta_len = self
            .delta_len
            .checked_add(len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        self.frame_count = self
            .frame_count
            .checked_add(1)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if self.delta_len > MAX_SEGMENT_DELTA_LEN || self.frame_count > MAX_SEGMENT_FRAME_COUNT {
            return Err(DurabilityError::PayloadTooLarge);
        }
        self.frame_hasher.update(frame);
        Ok(())
    }

    fn finish(
        self,
        parent: Option<ReplicationAuthoritySegmentId>,
    ) -> Result<ReplicationAuthoritySegmentPlan, DurabilityError> {
        if self.frame_count == 0 {
            return Err(DurabilityError::PayloadTooLarge);
        }
        let frame_digest: [u8; 32] = self.frame_hasher.finalize().into();
        let mut hasher = Sha256::new();
        hasher.update(SEGMENT_DOMAIN);
        hasher.update(parent.map_or(
            ReplicationAuthoritySegmentId::ZERO_BYTES,
            ReplicationAuthoritySegmentId::bytes,
        ));
        hasher.update(self.delta_len.to_le_bytes());
        hasher.update(self.frame_count.to_le_bytes());
        hasher.update(frame_digest);
        let id = ReplicationAuthoritySegmentId::from_bytes(hasher.finalize().into())?;
        Ok(ReplicationAuthoritySegmentPlan {
            id,
            parent,
            delta_len: self.delta_len,
            frame_count: self.frame_count,
        })
    }
}

fn validate_frame_shape(frame: &[u8]) -> Result<(), DurabilityError> {
    if frame.len() < FRAME_HEADER_LEN {
        return Err(corruption(
            "replication authority segment contains truncated frame header",
        ));
    }
    let payload_len =
        usize::try_from(read_u32(&frame[8..12])).map_err(|_| CodecError::LengthOverflow)?;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let expected = FRAME_HEADER_LEN
        .checked_add(payload_len)
        .ok_or(CodecError::LengthOverflow)?;
    if frame.len() != expected {
        return Err(corruption(
            "replication authority segment frame length is invalid",
        ));
    }
    Ok(())
}

fn decode_segment_header(
    header: &[u8; SEGMENT_HEADER_LEN],
) -> Result<
    (
        ReplicationAuthoritySegmentId,
        Option<ReplicationAuthoritySegmentId>,
        u64,
        u32,
    ),
    DurabilityError,
> {
    if header[0..4] != SEGMENT_MAGIC
        || header[4] != SEGMENT_VERSION
        || header[5..8] != [0, 0, 0]
        || header[84..88] != [0, 0, 0, 0]
    {
        return Err(corruption(
            "replication authority segment header is invalid",
        ));
    }
    let parent_raw: [u8; 32] = header[8..40].try_into().expect("32 bytes");
    let parent = if parent_raw == ReplicationAuthoritySegmentId::ZERO_BYTES {
        None
    } else {
        Some(ReplicationAuthoritySegmentId::from_bytes(parent_raw)?)
    };
    let id = ReplicationAuthoritySegmentId::from_bytes(
        header[SEGMENT_ID_OFFSET..SEGMENT_ID_OFFSET + 32]
            .try_into()
            .expect("32 bytes"),
    )?;
    let delta_len = u64::from_le_bytes(
        header[SEGMENT_DELTA_LEN_OFFSET..SEGMENT_DELTA_LEN_OFFSET + 8]
            .try_into()
            .expect("8 bytes"),
    );
    let frame_count = u32::from_le_bytes(
        header[SEGMENT_FRAME_COUNT_OFFSET..SEGMENT_FRAME_COUNT_OFFSET + 4]
            .try_into()
            .expect("4 bytes"),
    );
    if delta_len > MAX_SEGMENT_DELTA_LEN
        || frame_count == 0
        || frame_count > MAX_SEGMENT_FRAME_COUNT
    {
        return Err(DurabilityError::PayloadTooLarge);
    }
    Ok((id, parent, delta_len, frame_count))
}

fn verify_segment_reader(
    reader: &mut dyn Read,
    expected_len: u64,
    expected_id: ReplicationAuthoritySegmentId,
    expected_parent: Option<ReplicationAuthoritySegmentId>,
) -> Result<(), DurabilityError> {
    let mut header = [0_u8; SEGMENT_HEADER_LEN];
    reader.read_exact(&mut header)?;
    let (id, parent, delta_len, frame_count) = decode_segment_header(&header)?;
    if id != expected_id || parent != expected_parent {
        return Err(corruption(
            "replication authority segment index/header binding mismatch",
        ));
    }
    let encoded_len = u64::try_from(SEGMENT_HEADER_LEN)
        .map_err(|_| CodecError::LengthOverflow)?
        .checked_add(delta_len)
        .ok_or(CodecError::LengthOverflow)?;
    if encoded_len != expected_len {
        return Err(corruption(
            "replication authority segment extent length mismatch",
        ));
    }
    let mut frame_hasher = Sha256::new();
    let mut consumed = 0_u64;
    for _ in 0..frame_count {
        if delta_len.saturating_sub(consumed)
            < u64::try_from(FRAME_HEADER_LEN).map_err(|_| CodecError::LengthOverflow)?
        {
            return Err(corruption(
                "replication authority segment delta has truncated frame header",
            ));
        }
        let mut frame_header = [0_u8; FRAME_HEADER_LEN];
        reader.read_exact(&mut frame_header)?;
        let payload_len = usize::try_from(read_u32(&frame_header[8..12]))
            .map_err(|_| CodecError::LengthOverflow)?;
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        let frame_len = FRAME_HEADER_LEN
            .checked_add(payload_len)
            .ok_or(CodecError::LengthOverflow)?;
        let frame_len_u64 = u64::try_from(frame_len).map_err(|_| CodecError::LengthOverflow)?;
        let next_consumed = consumed
            .checked_add(frame_len_u64)
            .ok_or(CodecError::LengthOverflow)?;
        if next_consumed > delta_len {
            return Err(corruption(
                "replication authority segment frame exceeds delta length",
            ));
        }
        frame_hasher.update(frame_header);
        let mut remaining = payload_len;
        let mut buffer = [0_u8; 16 * 1024];
        while remaining != 0 {
            let take = remaining.min(buffer.len());
            reader.read_exact(&mut buffer[..take])?;
            frame_hasher.update(&buffer[..take]);
            remaining -= take;
        }
        consumed = next_consumed;
    }
    if consumed != delta_len {
        return Err(corruption(
            "replication authority segment delta has unframed trailing bytes",
        ));
    }
    let frame_digest: [u8; 32] = frame_hasher.finalize().into();
    let mut hasher = Sha256::new();
    hasher.update(SEGMENT_DOMAIN);
    hasher.update(parent.map_or(
        ReplicationAuthoritySegmentId::ZERO_BYTES,
        ReplicationAuthoritySegmentId::bytes,
    ));
    hasher.update(delta_len.to_le_bytes());
    hasher.update(frame_count.to_le_bytes());
    hasher.update(frame_digest);
    let actual = ReplicationAuthoritySegmentId::from_bytes(hasher.finalize().into())?;
    if actual != id {
        return Err(corruption("replication authority segment digest mismatch"));
    }
    Ok(())
}

fn replay_verified_segment_reader(
    reader: &mut dyn Read,
    expected_len: u64,
    expected_id: ReplicationAuthoritySegmentId,
    expected_parent: Option<ReplicationAuthoritySegmentId>,
    journal: &mut ReplicationAuthorityJournal,
) -> Result<(), DurabilityError> {
    let mut header = [0_u8; SEGMENT_HEADER_LEN];
    reader.read_exact(&mut header)?;
    let (id, parent, delta_len, frame_count) = decode_segment_header(&header)?;
    if id != expected_id || parent != expected_parent {
        return Err(corruption(
            "verified replication authority segment binding changed before replay",
        ));
    }
    let encoded_len = u64::try_from(SEGMENT_HEADER_LEN)
        .map_err(|_| CodecError::LengthOverflow)?
        .checked_add(delta_len)
        .ok_or(CodecError::LengthOverflow)?;
    if encoded_len != expected_len {
        return Err(corruption(
            "verified replication authority segment extent changed before replay",
        ));
    }
    let mut consumed = 0_u64;
    for _ in 0..frame_count {
        let mut frame_header = [0_u8; FRAME_HEADER_LEN];
        reader.read_exact(&mut frame_header)?;
        let payload_len = usize::try_from(read_u32(&frame_header[8..12]))
            .map_err(|_| CodecError::LengthOverflow)?;
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        let frame_len = FRAME_HEADER_LEN
            .checked_add(payload_len)
            .ok_or(CodecError::LengthOverflow)?;
        let frame_len_u64 = u64::try_from(frame_len).map_err(|_| CodecError::LengthOverflow)?;
        consumed = consumed
            .checked_add(frame_len_u64)
            .ok_or(CodecError::LengthOverflow)?;
        if consumed > delta_len {
            return Err(corruption(
                "verified replication authority segment frame exceeds delta length",
            ));
        }
        let mut frame = Vec::new();
        frame
            .try_reserve_exact(frame_len)
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        frame.extend_from_slice(&frame_header);
        frame.resize(frame_len, 0);
        reader.read_exact(&mut frame[FRAME_HEADER_LEN..])?;
        journal.replay_single_file_frame(&frame, false)?;
    }
    if consumed != delta_len {
        return Err(corruption(
            "verified replication authority segment delta has trailing bytes",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn replay_indexed_segment_chain<R: Read + Seek>(
    reader: &mut R,
    index: &ReplicationAuthoritySegmentIndex,
    journal: &mut ReplicationAuthorityJournal,
) -> Result<(), DurabilityError> {
    for (id, entry) in index.chain_oldest_first()? {
        reader.seek(SeekFrom::Start(entry.extent.offset))?;
        verify_segment_reader(reader, entry.extent.len, id, entry.parent)?;
        reader.seek(SeekFrom::Start(entry.extent.offset))?;
        replay_verified_segment_reader(reader, entry.extent.len, id, entry.parent, journal)?;
    }
    Ok(())
}

pub(crate) fn replay_indexed_segment_object_chain<R: Read + Seek>(
    reader: &mut R,
    index: &ReplicationAuthoritySegmentIndex,
    crypto: Option<&crate::storage_encryption::StorageAeadCodec>,
    journal: &mut ReplicationAuthorityJournal,
) -> Result<(), DurabilityError> {
    for (id, entry) in index.chain_oldest_first()? {
        replay_segment_object(
            reader,
            entry.extent.offset,
            entry.extent.len,
            id,
            entry.parent,
            crypto,
            journal,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::io::Cursor;

    use super::*;
    use crate::replication::authority::semantic_snapshot::ReplicationAuthoritySemanticSnapshot;
    use crate::replication::{
        ReplicaId, ReplicationMembership, ReplicationMembershipChange, ReplicationQuorumLoss,
    };

    #[test]
    #[allow(clippy::too_many_lines)]
    fn segment_chain_replay_matches_flat_journal_and_index_relocation_preserves_identity() {
        let mut source = ReplicationAuthorityJournal::open_single_file("segment-source", &[], &[])
            .expect("open source journal");
        source
            .install_membership(ReplicationMembershipChange {
                next: ReplicationMembership {
                    epoch: 1,
                    members: [ReplicaId::new(1), ReplicaId::new(2), ReplicaId::new(3)]
                        .into_iter()
                        .collect::<BTreeSet<_>>(),
                    quorum_size: 2,
                },
                acknowledged_by_previous: BTreeSet::new(),
            })
            .expect("bootstrap membership");
        let first_frames = source.take_pending_single_file_frames();
        source.commit_single_file_frames(first_frames.clone());
        source
            .mark_quorum_lost(ReplicationQuorumLoss {
                membership_epoch: 1,
                observed_term: 7,
            })
            .expect("fence quorum");
        let second_frames = source.take_pending_single_file_frames();
        source.commit_single_file_frames(second_frames.clone());
        let expected = ReplicationAuthoritySemanticSnapshot::capture(&source);

        let first = ReplicationAuthoritySegmentPlan::from_source(
            None,
            &ReplicationAuthorityFrameSlice::new(&first_frames),
        )
        .expect("plan first segment");
        let second = ReplicationAuthoritySegmentPlan::from_source(
            Some(first.id()),
            &ReplicationAuthorityFrameSlice::new(&second_frames),
        )
        .expect("plan second segment");

        let mut physical = vec![0xA5; 37];
        let first_offset = u64::try_from(physical.len()).unwrap();
        first
            .write_source_to(
                &ReplicationAuthorityFrameSlice::new(&first_frames),
                &mut |bytes| {
                    physical.extend_from_slice(bytes);
                    Ok(())
                },
            )
            .expect("write first segment");
        let second_offset = u64::try_from(physical.len()).unwrap();
        second
            .write_source_to(
                &ReplicationAuthorityFrameSlice::new(&second_frames),
                &mut |bytes| {
                    physical.extend_from_slice(bytes);
                    Ok(())
                },
            )
            .expect("write second segment");

        let mut index = ReplicationAuthoritySegmentIndex::empty();
        index
            .insert(
                first.id(),
                None,
                ReplicationAuthoritySegmentExtent {
                    offset: first_offset,
                    len: first.encoded_len().unwrap(),
                },
            )
            .unwrap();
        index
            .insert(
                second.id(),
                Some(first.id()),
                ReplicationAuthoritySegmentExtent {
                    offset: second_offset,
                    len: second.encoded_len().unwrap(),
                },
            )
            .unwrap();
        index.set_root(Some(second.id())).unwrap();

        let encoded_index = index.encode().unwrap();
        let decoded_index = ReplicationAuthoritySegmentIndex::decode(&encoded_index).unwrap();
        assert_eq!(decoded_index.root(), Some(second.id()));

        let mut restored =
            ReplicationAuthorityJournal::open_single_file("segment-restored", &[], &[])
                .expect("open restored journal");
        replay_indexed_segment_chain(&mut Cursor::new(&physical), &decoded_index, &mut restored)
            .expect("replay segment chain");
        assert_eq!(
            ReplicationAuthoritySemanticSnapshot::capture(&restored),
            expected
        );

        let mut relocated_bytes = vec![0x5A; 113];
        let relocated_first = u64::try_from(relocated_bytes.len()).unwrap();
        first
            .write_source_to(
                &ReplicationAuthorityFrameSlice::new(&first_frames),
                &mut |bytes| {
                    relocated_bytes.extend_from_slice(bytes);
                    Ok(())
                },
            )
            .unwrap();
        let relocated_second = u64::try_from(relocated_bytes.len()).unwrap();
        second
            .write_source_to(
                &ReplicationAuthorityFrameSlice::new(&second_frames),
                &mut |bytes| {
                    relocated_bytes.extend_from_slice(bytes);
                    Ok(())
                },
            )
            .unwrap();
        let mut relocated_index = decoded_index.clone();
        relocated_index
            .relocate(
                first.id(),
                ReplicationAuthoritySegmentExtent {
                    offset: relocated_first,
                    len: first.encoded_len().unwrap(),
                },
            )
            .unwrap();
        relocated_index
            .relocate(
                second.id(),
                ReplicationAuthoritySegmentExtent {
                    offset: relocated_second,
                    len: second.encoded_len().unwrap(),
                },
            )
            .unwrap();
        assert_eq!(relocated_index.root(), Some(second.id()));

        let mut relocated =
            ReplicationAuthorityJournal::open_single_file("segment-relocated", &[], &[])
                .expect("open relocated journal");
        replay_indexed_segment_chain(
            &mut Cursor::new(&relocated_bytes),
            &relocated_index,
            &mut relocated,
        )
        .expect("replay relocated chain");
        assert_eq!(
            ReplicationAuthoritySemanticSnapshot::capture(&relocated),
            expected
        );
    }

    #[test]
    fn segment_replay_rejects_exact_byte_tamper() {
        let mut source =
            ReplicationAuthorityJournal::open_single_file("segment-tamper", &[], &[]).unwrap();
        source
            .install_membership(ReplicationMembershipChange {
                next: ReplicationMembership {
                    epoch: 1,
                    members: [ReplicaId::new(1)].into_iter().collect(),
                    quorum_size: 1,
                },
                acknowledged_by_previous: BTreeSet::new(),
            })
            .unwrap();
        let frames = source.take_pending_single_file_frames();
        let plan = ReplicationAuthoritySegmentPlan::from_source(
            None,
            &ReplicationAuthorityFrameSlice::new(&frames),
        )
        .unwrap();
        let mut physical = Vec::new();
        plan.write_source_to(
            &ReplicationAuthorityFrameSlice::new(&frames),
            &mut |bytes| {
                physical.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        *physical.last_mut().expect("non-empty segment") ^= 1;

        let mut index = ReplicationAuthoritySegmentIndex::empty();
        index
            .insert(
                plan.id(),
                None,
                ReplicationAuthoritySegmentExtent {
                    offset: 0,
                    len: plan.encoded_len().unwrap(),
                },
            )
            .unwrap();
        index.set_root(Some(plan.id())).unwrap();
        let mut restored =
            ReplicationAuthorityJournal::open_single_file("segment-tampered", &[], &[]).unwrap();
        assert!(
            replay_indexed_segment_chain(&mut Cursor::new(physical), &index, &mut restored)
                .is_err()
        );
    }

    #[test]
    fn segment_index_rejects_unreachable_authority_extent() {
        let mut source =
            ReplicationAuthorityJournal::open_single_file("segment-index", &[], &[]).unwrap();
        source
            .install_membership(ReplicationMembershipChange {
                next: ReplicationMembership {
                    epoch: 1,
                    members: [ReplicaId::new(1)].into_iter().collect(),
                    quorum_size: 1,
                },
                acknowledged_by_previous: BTreeSet::new(),
            })
            .unwrap();
        let frames = source.take_pending_single_file_frames();
        let base = ReplicationAuthoritySegmentPlan::from_source(
            None,
            &ReplicationAuthorityFrameSlice::new(&frames),
        )
        .unwrap();
        let child = ReplicationAuthoritySegmentPlan::from_source(
            Some(base.id()),
            &ReplicationAuthorityFrameSlice::new(&frames),
        )
        .unwrap();
        let mut index = ReplicationAuthoritySegmentIndex::empty();
        index
            .insert(
                base.id(),
                None,
                ReplicationAuthoritySegmentExtent {
                    offset: 0,
                    len: base.encoded_len().unwrap(),
                },
            )
            .unwrap();
        index
            .insert(
                child.id(),
                Some(base.id()),
                ReplicationAuthoritySegmentExtent {
                    offset: base.encoded_len().unwrap(),
                    len: child.encoded_len().unwrap(),
                },
            )
            .unwrap();
        assert!(index.set_root(Some(base.id())).is_err());
    }

    #[test]
    fn segment_parent_binding_changes_identity() {
        let mut source =
            ReplicationAuthorityJournal::open_single_file("segment-binding", &[], &[]).unwrap();
        source
            .install_membership(ReplicationMembershipChange {
                next: ReplicationMembership {
                    epoch: 1,
                    members: [ReplicaId::new(1)].into_iter().collect(),
                    quorum_size: 1,
                },
                acknowledged_by_previous: BTreeSet::new(),
            })
            .unwrap();
        let frames = source.take_pending_single_file_frames();
        let base = ReplicationAuthoritySegmentPlan::from_source(
            None,
            &ReplicationAuthorityFrameSlice::new(&frames),
        )
        .unwrap();
        let child = ReplicationAuthoritySegmentPlan::from_source(
            Some(base.id()),
            &ReplicationAuthorityFrameSlice::new(&frames),
        )
        .unwrap();
        assert_ne!(base.id(), child.id());
    }
}
