use std::io::{self, Read, Seek, SeekFrom};

use super::{
    ReplicationAuthorityFrameSource, ReplicationAuthoritySegmentId,
    ReplicationAuthoritySegmentPlan, collect_verified_segment_reader,
    replay_verified_segment_reader, verify_segment_reader,
};
use crate::replication::authority::ReplicationAuthorityJournal;
use crate::runtime::DurabilityError;
use crate::storage_encryption::{StorageAeadCodec, StorageEncryptionDomain, StorageNonceSequence};

const OBJECT_MAGIC: [u8; 4] = *b"CFAO";
const OBJECT_VERSION: u8 = 1;
const OBJECT_KIND_REPLICATION_AUTHORITY_SEGMENT: u8 = 1;
const OBJECT_FLAG_ENCRYPTED: u8 = 1;
const OBJECT_HEADER_LEN: usize = 88;
const OBJECT_CHUNK_SIZE: usize = 64 * 1024;
const OBJECT_CHUNK_SIZE_U32: u32 = 64 * 1024;
const OBJECT_CHUNK_SIZE_U64: u64 = 64 * 1024;
const OBJECT_AAD_LEN: usize = OBJECT_HEADER_LEN + 8;

fn corruption(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

fn object_chunk_count(plaintext_len: u64) -> Result<u32, DurabilityError> {
    if plaintext_len == 0 {
        return Ok(1);
    }
    let chunk_size = OBJECT_CHUNK_SIZE_U64;
    let count = plaintext_len
        .checked_add(chunk_size - 1)
        .ok_or(DurabilityError::PayloadTooLarge)?
        / chunk_size;
    u32::try_from(count).map_err(|_| DurabilityError::PayloadTooLarge)
}

fn object_stored_len(plaintext_len: u64, encrypted: bool) -> Result<u64, DurabilityError> {
    let mut len = u64::try_from(OBJECT_HEADER_LEN)
        .map_err(|_| DurabilityError::PayloadTooLarge)?
        .checked_add(plaintext_len)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    if encrypted {
        let envelope_overhead = StorageAeadCodec::sealed_len(0)?;
        let overhead = u64::try_from(envelope_overhead)
            .map_err(|_| DurabilityError::PayloadTooLarge)?
            .checked_mul(u64::from(object_chunk_count(plaintext_len)?))
            .ok_or(DurabilityError::PayloadTooLarge)?;
        len = len
            .checked_add(overhead)
            .ok_or(DurabilityError::PayloadTooLarge)?;
    }
    Ok(len)
}

fn object_header(
    plan: &ReplicationAuthoritySegmentPlan,
    encrypted: bool,
) -> Result<[u8; OBJECT_HEADER_LEN], DurabilityError> {
    let plaintext_len = plan.encoded_len()?;
    let chunk_count = object_chunk_count(plaintext_len)?;
    let mut header = [0_u8; OBJECT_HEADER_LEN];
    header[..4].copy_from_slice(&OBJECT_MAGIC);
    header[4] = OBJECT_VERSION;
    header[5] = OBJECT_KIND_REPLICATION_AUTHORITY_SEGMENT;
    header[6] = u8::from(encrypted) * OBJECT_FLAG_ENCRYPTED;
    header[8..40].copy_from_slice(&plan.id().bytes());
    if let Some(parent) = plan.parent() {
        header[40..72].copy_from_slice(&parent.bytes());
    }
    header[72..80].copy_from_slice(&plaintext_len.to_le_bytes());
    header[80..84].copy_from_slice(&OBJECT_CHUNK_SIZE_U32.to_le_bytes());
    header[84..88].copy_from_slice(&chunk_count.to_le_bytes());
    Ok(header)
}

fn chunk_aad(
    header: &[u8; OBJECT_HEADER_LEN],
    chunk_index: u32,
    chunk_len: u32,
) -> [u8; OBJECT_AAD_LEN] {
    let mut aad = [0_u8; OBJECT_AAD_LEN];
    aad[..OBJECT_HEADER_LEN].copy_from_slice(header);
    aad[OBJECT_HEADER_LEN..OBJECT_HEADER_LEN + 4].copy_from_slice(&chunk_index.to_le_bytes());
    aad[OBJECT_HEADER_LEN + 4..].copy_from_slice(&chunk_len.to_le_bytes());
    aad
}

pub(crate) fn write_segment_object<S: ReplicationAuthorityFrameSource + ?Sized>(
    plan: &ReplicationAuthoritySegmentPlan,
    source: &S,
    crypto: Option<&StorageAeadCodec>,
    nonce_sequence: Option<&mut StorageNonceSequence>,
    emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
) -> Result<u64, DurabilityError> {
    let encrypted = crypto.is_some();
    if encrypted != nonce_sequence.is_some() {
        return Err(protocol(
            "immutable authority object nonce source does not match encryption mode",
        ));
    }
    let header = object_header(plan, encrypted)?;
    emit(&header)?;

    match (crypto, nonce_sequence) {
        (None, None) => {
            plan.write_source_to(source, emit)?;
        }
        (Some(crypto), Some(nonce_sequence)) => {
            let mut chunk = Vec::with_capacity(OBJECT_CHUNK_SIZE);
            let mut chunk_index = 0_u32;
            plan.write_source_to(source, &mut |mut bytes| {
                while !bytes.is_empty() {
                    let take = (OBJECT_CHUNK_SIZE - chunk.len()).min(bytes.len());
                    chunk.extend_from_slice(&bytes[..take]);
                    bytes = &bytes[take..];
                    if chunk.len() == OBJECT_CHUNK_SIZE {
                        seal_chunk(&header, chunk_index, &chunk, crypto, nonce_sequence, emit)?;
                        chunk.clear();
                        chunk_index = chunk_index
                            .checked_add(1)
                            .ok_or(DurabilityError::PayloadTooLarge)?;
                    }
                }
                Ok(())
            })?;
            if !chunk.is_empty() {
                seal_chunk(&header, chunk_index, &chunk, crypto, nonce_sequence, emit)?;
                chunk_index = chunk_index
                    .checked_add(1)
                    .ok_or(DurabilityError::PayloadTooLarge)?;
            }
            if chunk_index != object_chunk_count(plan.encoded_len()?)? {
                return Err(corruption(
                    "immutable authority object emitted unexpected chunk count",
                ));
            }
        }
        _ => {
            return Err(protocol(
                "immutable authority object encryption state changed during publication",
            ));
        }
    }

    object_stored_len(plan.encoded_len()?, encrypted)
}

fn seal_chunk(
    header: &[u8; OBJECT_HEADER_LEN],
    chunk_index: u32,
    chunk: &[u8],
    crypto: &StorageAeadCodec,
    nonce_sequence: &mut StorageNonceSequence,
    emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
) -> Result<(), DurabilityError> {
    let chunk_len = u32::try_from(chunk.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let aad = chunk_aad(header, chunk_index, chunk_len);
    let envelope = crypto.seal(
        StorageEncryptionDomain::ImmutableObject,
        nonce_sequence.next_nonce()?,
        &aad,
        chunk,
    )?;
    emit(&envelope)
}

struct SegmentObjectReader<'a, R> {
    source: &'a mut R,
    header: [u8; OBJECT_HEADER_LEN],
    crypto: Option<&'a StorageAeadCodec>,
    plaintext_len: u64,
    delivered: u64,
    chunk_count: u32,
    next_chunk: u32,
    chunk: Vec<u8>,
    chunk_position: usize,
    pending_error: Option<DurabilityError>,
}

impl<'a, R: Read> SegmentObjectReader<'a, R> {
    fn open(
        source: &'a mut R,
        stored_len: u64,
        expected_id: ReplicationAuthoritySegmentId,
        expected_parent: Option<ReplicationAuthoritySegmentId>,
        crypto: Option<&'a StorageAeadCodec>,
    ) -> Result<Self, DurabilityError> {
        let mut header = [0_u8; OBJECT_HEADER_LEN];
        source.read_exact(&mut header)?;
        if header[..4] != OBJECT_MAGIC
            || header[4] != OBJECT_VERSION
            || header[5] != OBJECT_KIND_REPLICATION_AUTHORITY_SEGMENT
            || header[7] != 0
            || header[6] & !OBJECT_FLAG_ENCRYPTED != 0
        {
            return Err(corruption("immutable authority object header is invalid"));
        }
        let encrypted = header[6] == OBJECT_FLAG_ENCRYPTED;
        if encrypted != crypto.is_some() {
            return Err(corruption(
                "immutable authority object encryption mode does not match database",
            ));
        }
        let id =
            ReplicationAuthoritySegmentId::from_bytes(header[8..40].try_into().expect("32 bytes"))?;
        let parent_raw: [u8; 32] = header[40..72].try_into().expect("32 bytes");
        let parent = if parent_raw == ReplicationAuthoritySegmentId::ZERO_BYTES {
            None
        } else {
            Some(ReplicationAuthoritySegmentId::from_bytes(parent_raw)?)
        };
        if id != expected_id || parent != expected_parent {
            return Err(corruption(
                "immutable authority object identity binding mismatch",
            ));
        }
        let plaintext_len = u64::from_le_bytes(header[72..80].try_into().expect("8 bytes"));
        let chunk_size = u32::from_le_bytes(header[80..84].try_into().expect("4 bytes"));
        let chunk_count = u32::from_le_bytes(header[84..88].try_into().expect("4 bytes"));
        if chunk_size != OBJECT_CHUNK_SIZE_U32
            || chunk_count != object_chunk_count(plaintext_len)?
            || stored_len != object_stored_len(plaintext_len, encrypted)?
        {
            return Err(corruption(
                "immutable authority object physical length metadata is invalid",
            ));
        }
        Ok(Self {
            source,
            header,
            crypto,
            plaintext_len,
            delivered: 0,
            chunk_count,
            next_chunk: 0,
            chunk: Vec::new(),
            chunk_position: 0,
            pending_error: None,
        })
    }

    fn load_chunk(&mut self) -> Result<(), DurabilityError> {
        let crypto = self.crypto.ok_or_else(|| {
            corruption("encrypted immutable authority object is missing its database key")
        })?;
        if self.next_chunk >= self.chunk_count {
            return Err(corruption(
                "immutable authority object chunk stream ended unexpectedly",
            ));
        }
        let chunk_start = u64::from(self.next_chunk)
            .checked_mul(OBJECT_CHUNK_SIZE_U64)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let remaining = self
            .plaintext_len
            .checked_sub(chunk_start)
            .ok_or_else(|| corruption("immutable authority object chunk offset overflow"))?;
        let chunk_len = remaining.min(OBJECT_CHUNK_SIZE_U64);
        let chunk_len_usize =
            usize::try_from(chunk_len).map_err(|_| DurabilityError::PayloadTooLarge)?;
        let envelope_len = StorageAeadCodec::sealed_len(chunk_len_usize)?;
        let mut envelope = vec![0_u8; envelope_len];
        self.source.read_exact(&mut envelope)?;
        let aad = chunk_aad(
            &self.header,
            self.next_chunk,
            u32::try_from(chunk_len_usize).map_err(|_| DurabilityError::PayloadTooLarge)?,
        );
        let plaintext = crypto.open(StorageEncryptionDomain::ImmutableObject, &aad, &envelope)?;
        if plaintext.len() != chunk_len_usize {
            return Err(corruption(
                "immutable authority object chunk plaintext length mismatch",
            ));
        }
        self.chunk = plaintext;
        self.chunk_position = 0;
        self.next_chunk = self
            .next_chunk
            .checked_add(1)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        Ok(())
    }

    fn finish(&self) -> Result<(), DurabilityError> {
        if self.delivered != self.plaintext_len {
            return Err(corruption(
                "immutable authority object plaintext was not fully consumed",
            ));
        }
        if self.crypto.is_some() && self.next_chunk != self.chunk_count {
            return Err(corruption(
                "immutable authority object encrypted chunks were not fully consumed",
            ));
        }
        Ok(())
    }

    fn take_pending_error(&mut self) -> Option<DurabilityError> {
        self.pending_error.take()
    }
}

impl<R: Read> Read for SegmentObjectReader<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.delivered == self.plaintext_len {
            return Ok(0);
        }
        let result = if self.crypto.is_none() {
            let output_len =
                u64::try_from(output.len()).map_err(|_| DurabilityError::PayloadTooLarge);
            let remaining = output_len.and_then(|output_len| {
                usize::try_from((self.plaintext_len - self.delivered).min(output_len))
                    .map_err(|_| DurabilityError::PayloadTooLarge)
            });
            match remaining {
                Ok(take) => self
                    .source
                    .read(&mut output[..take])
                    .map_err(DurabilityError::Io)
                    .and_then(|read| {
                        if read == 0 {
                            Err(corruption(
                                "immutable authority object plaintext is truncated",
                            ))
                        } else {
                            self.delivered = self
                                .delivered
                                .checked_add(
                                    u64::try_from(read)
                                        .map_err(|_| DurabilityError::PayloadTooLarge)?,
                                )
                                .ok_or(DurabilityError::PayloadTooLarge)?;
                            Ok(read)
                        }
                    }),
                Err(error) => Err(error),
            }
        } else if self.chunk_position == self.chunk.len()
            && let Err(error) = self.load_chunk()
        {
            Err(error)
        } else {
            let available = self.chunk.len() - self.chunk_position;
            let take = available.min(output.len());
            output[..take]
                .copy_from_slice(&self.chunk[self.chunk_position..self.chunk_position + take]);
            self.chunk_position += take;
            u64::try_from(take)
                .map_err(|_| DurabilityError::PayloadTooLarge)
                .and_then(|take_u64| {
                    self.delivered
                        .checked_add(take_u64)
                        .ok_or(DurabilityError::PayloadTooLarge)
                })
                .map(|delivered| {
                    self.delivered = delivered;
                    take
                })
        };
        match result {
            Ok(read) => Ok(read),
            Err(error) => {
                self.pending_error = Some(error);
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "immutable authority object read failed",
                ))
            }
        }
    }
}

fn recover_reader_error<R: Read>(
    reader: &mut SegmentObjectReader<'_, R>,
    result: Result<(), DurabilityError>,
) -> Result<(), DurabilityError> {
    match result {
        Ok(()) => reader.finish(),
        Err(error) => Err(reader.take_pending_error().unwrap_or(error)),
    }
}

pub(super) fn verify_segment_object<R: Read>(
    reader: &mut R,
    stored_len: u64,
    expected_id: ReplicationAuthoritySegmentId,
    expected_parent: Option<ReplicationAuthoritySegmentId>,
    crypto: Option<&StorageAeadCodec>,
) -> Result<(), DurabilityError> {
    let mut object =
        SegmentObjectReader::open(reader, stored_len, expected_id, expected_parent, crypto)?;
    let plaintext_len = object.plaintext_len;
    let result = verify_segment_reader(&mut object, plaintext_len, expected_id, expected_parent);
    recover_reader_error(&mut object, result)
}

pub(crate) fn collect_segment_object_frames<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    stored_len: u64,
    expected_id: ReplicationAuthoritySegmentId,
    expected_parent: Option<ReplicationAuthoritySegmentId>,
    crypto: Option<&StorageAeadCodec>,
) -> Result<Vec<Vec<u8>>, DurabilityError> {
    reader.seek(SeekFrom::Start(offset))?;
    verify_segment_object(reader, stored_len, expected_id, expected_parent, crypto)?;
    reader.seek(SeekFrom::Start(offset))?;
    let mut object =
        SegmentObjectReader::open(reader, stored_len, expected_id, expected_parent, crypto)?;
    let plaintext_len = object.plaintext_len;
    let result =
        collect_verified_segment_reader(&mut object, plaintext_len, expected_id, expected_parent);
    match result {
        Ok(frames) => {
            object.finish()?;
            Ok(frames)
        }
        Err(error) => Err(object.take_pending_error().unwrap_or(error)),
    }
}

pub(crate) fn replay_segment_object<R: Read + Seek>(
    reader: &mut R,
    offset: u64,
    stored_len: u64,
    expected_id: ReplicationAuthoritySegmentId,
    expected_parent: Option<ReplicationAuthoritySegmentId>,
    crypto: Option<&StorageAeadCodec>,
    journal: &mut ReplicationAuthorityJournal,
) -> Result<(), DurabilityError> {
    reader.seek(SeekFrom::Start(offset))?;
    verify_segment_object(reader, stored_len, expected_id, expected_parent, crypto)?;
    reader.seek(SeekFrom::Start(offset))?;
    let mut object =
        SegmentObjectReader::open(reader, stored_len, expected_id, expected_parent, crypto)?;
    let plaintext_len = object.plaintext_len;
    let result = replay_verified_segment_reader(
        &mut object,
        plaintext_len,
        expected_id,
        expected_parent,
        journal,
    );
    recover_reader_error(&mut object, result)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::io::Cursor;

    use super::*;
    use crate::replication::authority::semantic_snapshot::ReplicationAuthoritySemanticSnapshot;
    use crate::replication::{ReplicaId, ReplicationMembership, ReplicationMembershipChange};
    use crate::storage_encryption::{StorageAeadAlgorithm, StorageAeadCodec, StorageEncryptionKey};

    fn frames() -> Vec<Vec<u8>> {
        let mut source =
            ReplicationAuthorityJournal::open_single_file("object-source", &[], &[]).unwrap();
        source
            .install_membership(ReplicationMembershipChange {
                next: ReplicationMembership {
                    epoch: 1,
                    members: [ReplicaId::new(1), ReplicaId::new(2)]
                        .into_iter()
                        .collect::<BTreeSet<_>>(),
                    quorum_size: 2,
                },
                acknowledged_by_previous: BTreeSet::new(),
            })
            .unwrap();
        source.take_pending_single_file_frames()
    }

    struct CountingFrameSource {
        frames: Vec<Vec<u8>>,
        traversals: Cell<usize>,
    }

    impl CountingFrameSource {
        fn new(frames: Vec<Vec<u8>>) -> Self {
            Self {
                frames,
                traversals: Cell::new(0),
            }
        }
    }

    impl ReplicationAuthorityFrameSource for CountingFrameSource {
        fn is_empty(&self) -> bool {
            self.frames.is_empty()
        }

        fn for_each_frame(
            &self,
            emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
        ) -> Result<(), DurabilityError> {
            self.traversals.set(self.traversals.get() + 1);
            for frame in &self.frames {
                emit(frame)?;
            }
            Ok(())
        }
    }

    #[test]
    fn segment_object_publication_traverses_replayable_source_exactly_twice() {
        let source = CountingFrameSource::new(frames());
        let plan = ReplicationAuthoritySegmentPlan::from_source(None, &source).unwrap();
        let mut stored = Vec::new();
        write_segment_object(&plan, &source, None, None, &mut |bytes| {
            stored.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
        assert_eq!(source.traversals.get(), 2);
        assert!(!stored.is_empty());
    }

    struct ChangingFrameSource {
        first: Vec<Vec<u8>>,
        second: Vec<Vec<u8>>,
        traversals: Cell<usize>,
    }

    impl ReplicationAuthorityFrameSource for ChangingFrameSource {
        fn is_empty(&self) -> bool {
            false
        }

        fn for_each_frame(
            &self,
            emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
        ) -> Result<(), DurabilityError> {
            let traversal = self.traversals.get();
            self.traversals.set(traversal + 1);
            let frames = if traversal == 0 {
                &self.first
            } else {
                &self.second
            };
            for frame in frames {
                emit(frame)?;
            }
            Ok(())
        }
    }

    #[test]
    fn segment_object_post_write_validation_rejects_changed_source() {
        let first = frames();
        let mut second = first.clone();
        *second.last_mut().unwrap().last_mut().unwrap() ^= 1;
        let source = ChangingFrameSource {
            first,
            second,
            traversals: Cell::new(0),
        };
        let plan = ReplicationAuthoritySegmentPlan::from_source(None, &source).unwrap();
        let mut stored = Vec::new();
        assert!(
            write_segment_object(&plan, &source, None, None, &mut |bytes| {
                stored.extend_from_slice(bytes);
                Ok(())
            })
            .is_err()
        );
        assert_eq!(source.traversals.get(), 2);
    }

    fn codec() -> StorageAeadCodec {
        StorageAeadCodec::with_salt(
            StorageAeadAlgorithm::Aes256GcmSiv,
            &StorageEncryptionKey::try_new([0x42; 32]).unwrap(),
            &[0x19; 32],
        )
        .unwrap()
    }

    #[test]
    fn encrypted_object_preserves_plaintext_segment_identity_and_replays() {
        let frames = frames();
        let plan = ReplicationAuthoritySegmentPlan::from_frames(None, &frames).unwrap();
        let crypto = codec();
        let mut nonces = StorageNonceSequence::random().unwrap();
        let mut stored = Vec::new();
        let stored_len = write_segment_object(
            &plan,
            &frames,
            Some(&crypto),
            Some(&mut nonces),
            &mut |bytes| {
                stored.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(stored_len, stored.len() as u64);
        assert_eq!(&stored[..4], b"CFAO");
        assert_ne!(&stored[OBJECT_HEADER_LEN..OBJECT_HEADER_LEN + 4], b"CFAS");

        verify_segment_object(
            &mut Cursor::new(&stored),
            stored_len,
            plan.id(),
            plan.parent(),
            Some(&crypto),
        )
        .unwrap();

        let mut restored =
            ReplicationAuthorityJournal::open_single_file("object-restored", &[], &[]).unwrap();
        replay_segment_object(
            &mut Cursor::new(&stored),
            0,
            stored_len,
            plan.id(),
            plan.parent(),
            Some(&crypto),
            &mut restored,
        )
        .unwrap();
        let archived = frames.concat();
        let expected =
            ReplicationAuthorityJournal::open_single_file("object-expected", &archived, &[])
                .unwrap();
        assert_eq!(
            ReplicationAuthoritySemanticSnapshot::capture(&restored),
            ReplicationAuthoritySemanticSnapshot::capture(&expected)
        );
    }

    #[test]
    fn encrypted_object_rejects_ciphertext_and_aad_tamper() {
        let frames = frames();
        let plan = ReplicationAuthoritySegmentPlan::from_frames(None, &frames).unwrap();
        let crypto = codec();
        let mut nonces = StorageNonceSequence::random().unwrap();
        let mut stored = Vec::new();
        let stored_len = write_segment_object(
            &plan,
            &frames,
            Some(&crypto),
            Some(&mut nonces),
            &mut |bytes| {
                stored.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();

        let mut ciphertext_tamper = stored.clone();
        *ciphertext_tamper.last_mut().unwrap() ^= 1;
        assert!(
            verify_segment_object(
                &mut Cursor::new(ciphertext_tamper),
                stored_len,
                plan.id(),
                plan.parent(),
                Some(&crypto),
            )
            .is_err()
        );

        let mut aad_tamper = stored;
        aad_tamper[72] ^= 1;
        assert!(
            verify_segment_object(
                &mut Cursor::new(aad_tamper),
                stored_len,
                plan.id(),
                plan.parent(),
                Some(&crypto),
            )
            .is_err()
        );
    }

    #[test]
    fn encrypted_object_rejects_every_single_header_bit_tamper() {
        let frames = frames();
        let plan = ReplicationAuthoritySegmentPlan::from_frames(None, &frames).unwrap();
        let crypto = codec();
        let mut nonces = StorageNonceSequence::random().unwrap();
        let mut stored = Vec::new();
        let stored_len = write_segment_object(
            &plan,
            &frames,
            Some(&crypto),
            Some(&mut nonces),
            &mut |bytes| {
                stored.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();

        for byte in 0..OBJECT_HEADER_LEN {
            for bit in 0..8 {
                let mut tampered = stored.clone();
                tampered[byte] ^= 1 << bit;
                assert!(
                    verify_segment_object(
                        &mut Cursor::new(tampered),
                        stored_len,
                        plan.id(),
                        plan.parent(),
                        Some(&crypto),
                    )
                    .is_err(),
                    "header byte {byte} bit {bit} unexpectedly survived"
                );
            }
        }
    }

    #[test]
    fn object_plan_mismatch_never_emits_beyond_frozen_plan() {
        let frames = frames();
        let plan = ReplicationAuthoritySegmentPlan::from_frames(None, &frames).unwrap();
        let mut changed = frames.clone();
        changed.push(frames[0].clone());
        let mut emitted = 0_usize;
        let error = write_segment_object(&plan, &changed, None, None, &mut |bytes| {
            emitted += bytes.len();
            Ok(())
        })
        .unwrap_err();
        assert!(matches!(error, DurabilityError::Corruption { .. }));
        assert_eq!(
            emitted as u64,
            object_stored_len(plan.encoded_len().unwrap(), false).unwrap()
        );
    }

    #[test]
    fn encrypted_object_streams_multi_chunk_segment_without_payload_sized_ciphertext() {
        let frame = frames().into_iter().next().unwrap();
        let frames = std::iter::repeat_n(frame, 3_000).collect::<Vec<_>>();
        let plan = ReplicationAuthoritySegmentPlan::from_frames(None, &frames).unwrap();
        assert!(plan.encoded_len().unwrap() > OBJECT_CHUNK_SIZE_U64 * 2);
        let crypto = codec();
        let mut nonces = StorageNonceSequence::random().unwrap();
        let mut stored = Vec::new();
        let mut largest_emit = 0_usize;
        let stored_len = write_segment_object(
            &plan,
            &frames,
            Some(&crypto),
            Some(&mut nonces),
            &mut |bytes| {
                largest_emit = largest_emit.max(bytes.len());
                stored.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        assert!(largest_emit <= StorageAeadCodec::sealed_len(OBJECT_CHUNK_SIZE).unwrap());
        verify_segment_object(
            &mut Cursor::new(&stored),
            stored_len,
            plan.id(),
            plan.parent(),
            Some(&crypto),
        )
        .unwrap();

        let envelope_len = StorageAeadCodec::sealed_len(OBJECT_CHUNK_SIZE).unwrap();
        let first_start = OBJECT_HEADER_LEN;
        let second_start = first_start + envelope_len;
        let mut reordered = stored;
        let first = reordered[first_start..second_start].to_vec();
        let second = reordered[second_start..second_start + envelope_len].to_vec();
        reordered[first_start..second_start].copy_from_slice(&second);
        reordered[second_start..second_start + envelope_len].copy_from_slice(&first);
        assert!(
            verify_segment_object(
                &mut Cursor::new(reordered),
                stored_len,
                plan.id(),
                plan.parent(),
                Some(&crypto),
            )
            .is_err()
        );
    }

    #[test]
    fn encryption_mode_is_fail_closed_and_plain_object_remains_valid() {
        let frames = frames();
        let plan = ReplicationAuthoritySegmentPlan::from_frames(None, &frames).unwrap();
        let crypto = codec();

        let mut plaintext = Vec::new();
        let plaintext_len = write_segment_object(&plan, &frames, None, None, &mut |bytes| {
            plaintext.extend_from_slice(bytes);
            Ok(())
        })
        .unwrap();
        verify_segment_object(
            &mut Cursor::new(&plaintext),
            plaintext_len,
            plan.id(),
            plan.parent(),
            None,
        )
        .unwrap();
        assert!(
            verify_segment_object(
                &mut Cursor::new(&plaintext),
                plaintext_len,
                plan.id(),
                plan.parent(),
                Some(&crypto),
            )
            .is_err()
        );

        let mut nonces = StorageNonceSequence::random().unwrap();
        let mut encrypted = Vec::new();
        let encrypted_len = write_segment_object(
            &plan,
            &frames,
            Some(&crypto),
            Some(&mut nonces),
            &mut |bytes| {
                encrypted.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        assert!(
            verify_segment_object(
                &mut Cursor::new(&encrypted),
                encrypted_len,
                plan.id(),
                plan.parent(),
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn encryption_nonce_and_relocation_do_not_change_segment_identity() {
        let frames = frames();
        let plan = ReplicationAuthoritySegmentPlan::from_frames(None, &frames).unwrap();
        let crypto = codec();
        let mut first_nonces = StorageNonceSequence::random().unwrap();
        let mut second_nonces = StorageNonceSequence::random().unwrap();
        let mut first = Vec::new();
        let mut second = Vec::new();
        let first_len = write_segment_object(
            &plan,
            &frames,
            Some(&crypto),
            Some(&mut first_nonces),
            &mut |bytes| {
                first.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        let second_len = write_segment_object(
            &plan,
            &frames,
            Some(&crypto),
            Some(&mut second_nonces),
            &mut |bytes| {
                second.extend_from_slice(bytes);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(first_len, second_len);
        assert_eq!(&first[8..40], &second[8..40]);
        assert_ne!(first, second);

        let mut relocated = vec![0xA5; 137];
        let offset = relocated.len() as u64;
        relocated.extend_from_slice(&first);
        let mut restored =
            ReplicationAuthorityJournal::open_single_file("object-relocated", &[], &[]).unwrap();
        replay_segment_object(
            &mut Cursor::new(relocated),
            offset,
            first_len,
            plan.id(),
            plan.parent(),
            Some(&crypto),
            &mut restored,
        )
        .unwrap();
        let archived = frames.concat();
        let expected = ReplicationAuthorityJournal::open_single_file(
            "object-relocated-expected",
            &archived,
            &[],
        )
        .unwrap();
        assert_eq!(
            ReplicationAuthoritySemanticSnapshot::capture(&restored),
            ReplicationAuthoritySemanticSnapshot::capture(&expected)
        );
    }
}
