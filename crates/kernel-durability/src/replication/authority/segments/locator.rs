use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};

use sha2::{Digest, Sha256};

use super::{
    ReplicationAuthoritySegmentExtent, ReplicationAuthoritySegmentId,
    ReplicationAuthoritySegmentIndex,
};
use crate::runtime::DurabilityError;

const LOCATOR_DOMAIN: &[u8] = b"CFMD/replication-authority-locator/v1";
const LOCATOR_MAGIC: [u8; 4] = *b"CFLN";
const LOCATOR_VERSION: u8 = 1;
const LOCATOR_LEN: usize = 160;
const LOCATOR_DIGEST_OFFSET: usize = 128;

pub(crate) const fn locator_stored_len() -> u64 {
    LOCATOR_LEN as u64
}

fn corruption(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplicationAuthorityLocatorRoot {
    pub(crate) segment_id: ReplicationAuthoritySegmentId,
    pub(crate) offset: u64,
    pub(crate) digest: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplicationAuthorityLocatorNode {
    segment_id: ReplicationAuthoritySegmentId,
    parent: Option<ReplicationAuthoritySegmentId>,
    object: ReplicationAuthoritySegmentExtent,
    previous: Option<ReplicationAuthorityLocatorRoot>,
}

fn locator_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(LOCATOR_DOMAIN);
    hasher.update(bytes);
    hasher.finalize().into()
}

pub(crate) fn write_locator_node(
    offset: u64,
    segment_id: ReplicationAuthoritySegmentId,
    parent: Option<ReplicationAuthoritySegmentId>,
    object: ReplicationAuthoritySegmentExtent,
    previous: Option<ReplicationAuthorityLocatorRoot>,
    emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
) -> Result<ReplicationAuthorityLocatorRoot, DurabilityError> {
    if parent.is_some() != previous.is_some() {
        return Err(corruption(
            "replication authority locator parent/link presence mismatch",
        ));
    }
    if let (Some(parent), Some(previous)) = (parent, previous)
        && parent != previous.segment_id
    {
        return Err(corruption(
            "replication authority locator parent does not match previous segment",
        ));
    }
    object
        .offset
        .checked_add(object.len)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    let mut bytes = [0_u8; LOCATOR_LEN];
    bytes[0..4].copy_from_slice(&LOCATOR_MAGIC);
    bytes[4] = LOCATOR_VERSION;
    bytes[8..40].copy_from_slice(&segment_id.bytes());
    if let Some(parent) = parent {
        bytes[40..72].copy_from_slice(&parent.bytes());
    }
    bytes[72..80].copy_from_slice(&object.offset.to_le_bytes());
    bytes[80..88].copy_from_slice(&object.len.to_le_bytes());
    if let Some(previous) = previous {
        bytes[88..96].copy_from_slice(&previous.offset.to_le_bytes());
        bytes[96..128].copy_from_slice(&previous.digest);
    }
    let digest = locator_digest(&bytes[..LOCATOR_DIGEST_OFFSET]);
    bytes[LOCATOR_DIGEST_OFFSET..].copy_from_slice(&digest);
    emit(&bytes)?;
    Ok(ReplicationAuthorityLocatorRoot {
        segment_id,
        offset,
        digest,
    })
}

fn read_locator_node<R: Read + Seek>(
    reader: &mut R,
    expected: ReplicationAuthorityLocatorRoot,
) -> Result<ReplicationAuthorityLocatorNode, DurabilityError> {
    reader.seek(SeekFrom::Start(expected.offset))?;
    let mut bytes = [0_u8; LOCATOR_LEN];
    reader.read_exact(&mut bytes)?;
    if bytes[0..4] != LOCATOR_MAGIC || bytes[4] != LOCATOR_VERSION || bytes[5..8] != [0, 0, 0] {
        return Err(corruption(
            "replication authority locator header is invalid",
        ));
    }
    let digest = locator_digest(&bytes[..LOCATOR_DIGEST_OFFSET]);
    if digest != expected.digest || bytes[LOCATOR_DIGEST_OFFSET..] != expected.digest {
        return Err(corruption("replication authority locator digest mismatch"));
    }
    let segment_id =
        ReplicationAuthoritySegmentId::from_bytes(bytes[8..40].try_into().expect("32 bytes"))?;
    if segment_id != expected.segment_id {
        return Err(corruption(
            "replication authority locator root segment binding mismatch",
        ));
    }
    let parent_raw: [u8; 32] = bytes[40..72].try_into().expect("32 bytes");
    let parent = if parent_raw == ReplicationAuthoritySegmentId::ZERO_BYTES {
        None
    } else {
        Some(ReplicationAuthoritySegmentId::from_bytes(parent_raw)?)
    };
    let object = ReplicationAuthoritySegmentExtent {
        offset: u64::from_le_bytes(bytes[72..80].try_into().expect("8 bytes")),
        len: u64::from_le_bytes(bytes[80..88].try_into().expect("8 bytes")),
    };
    object
        .offset
        .checked_add(object.len)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    let previous_offset = u64::from_le_bytes(bytes[88..96].try_into().expect("8 bytes"));
    let previous_digest: [u8; 32] = bytes[96..128].try_into().expect("32 bytes");
    let previous = match parent {
        None => {
            if previous_offset != 0 || previous_digest != [0; 32] {
                return Err(corruption(
                    "root replication authority locator has a previous physical link",
                ));
            }
            None
        }
        Some(parent) => {
            if previous_offset == 0 || previous_digest == [0; 32] {
                return Err(corruption(
                    "replication authority locator is missing previous physical link",
                ));
            }
            Some(ReplicationAuthorityLocatorRoot {
                segment_id: parent,
                offset: previous_offset,
                digest: previous_digest,
            })
        }
    };
    Ok(ReplicationAuthorityLocatorNode {
        segment_id,
        parent,
        object,
        previous,
    })
}

pub(crate) fn recover_locator_chain<R: Read + Seek>(
    reader: &mut R,
    root: ReplicationAuthorityLocatorRoot,
) -> Result<ReplicationAuthoritySegmentIndex, DurabilityError> {
    let mut index = ReplicationAuthoritySegmentIndex::empty();
    let mut current = Some(root);
    let mut seen_offsets = BTreeSet::new();
    while let Some(expected) = current {
        if !seen_offsets.insert(expected.offset) {
            return Err(corruption(
                "replication authority locator chain contains a physical cycle",
            ));
        }
        let node = read_locator_node(reader, expected)?;
        index.insert(node.segment_id, node.parent, node.object)?;
        current = node.previous;
    }
    index.set_root(Some(root.segment_id))?;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use kernel_auth::Sha256Digest;

    use super::*;

    fn id(byte: u8) -> ReplicationAuthoritySegmentId {
        ReplicationAuthoritySegmentId(Sha256Digest([byte; 32]))
    }

    #[test]
    fn linked_locator_chain_recovers_relocatable_index() {
        let mut physical = vec![0_u8; 64];
        let first_offset = physical.len() as u64;
        let first = write_locator_node(
            first_offset,
            id(1),
            None,
            ReplicationAuthoritySegmentExtent { offset: 8, len: 88 },
            None,
            &mut |bytes| {
                physical.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        let second_offset = physical.len() as u64;
        let second = write_locator_node(
            second_offset,
            id(2),
            Some(id(1)),
            ReplicationAuthoritySegmentExtent {
                offset: 256,
                len: 96,
            },
            Some(first),
            &mut |bytes| {
                physical.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        let index = recover_locator_chain(&mut Cursor::new(physical), second).unwrap();
        assert_eq!(index.root(), Some(id(2)));
        assert_eq!(index.chain_oldest_first().unwrap().len(), 2);
    }
}
