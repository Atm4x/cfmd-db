use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kernel_types::RevisionId;
use sha2::{Digest, Sha256};

use crate::descriptor::DurableRevisionDescriptor;
use crate::replication::authority::{
    ReplicationAuthorityJournal, ReplicationAuthorityLocatorRoot,
    ReplicationAuthoritySegmentExtent, ReplicationAuthoritySegmentId,
    ReplicationAuthoritySegmentPlan, locator_stored_len, recover_locator_chain,
    replay_indexed_segment_object_chain, write_locator_node, write_segment_object,
};
use crate::runtime::{DurabilityError, RecoveryScan, TailStatus};
use crate::storage_encryption::{
    StorageAeadAlgorithm, StorageAeadCodec, StorageEncryption, StorageEncryptionDomain,
    StorageEncryptionKey, StorageNonceSequence, WrappedDatabaseMasterKey,
    random_database_master_key, unwrap_database_master_key, wrap_database_master_key,
};
use crate::wal::{FileRevisionWal, WalRegionRecovery, WalRegionScanSpec, scan_wal_stream_seeded};

pub(crate) mod compaction_io;
mod compaction_protocol;
use compaction_io::{
    CopyRangeSteps, SingleFileCompactionIo, SingleFileCompactionIoStep, SingleFileCompactionReader,
    copy_exact_range as copy_exact_compaction_range, write_all as compaction_write_all,
};
#[cfg(test)]
use compaction_protocol::PUBLICATION_SEQUENCE as COMPACTION_PUBLICATION_SEQUENCE;
use compaction_protocol::{
    FinalRootDurable as CompactionFinalRootDurable, Publication as SingleFileCompactionPublication,
    SourceSealed as CompactionSourceSealed, SourceUnsealed as CompactionSourceUnsealed,
};

const PAGE_SIZE: u64 = 4096;
const PAGE_SIZE_USIZE: usize = 4096;
const PAGE_SIZE_U32: u32 = 4096;
const IO_BUFFER_SIZE: usize = 16 * 1024;
const HEADER_OFFSET: u64 = 0;
const ROOT_A_OFFSET: u64 = PAGE_SIZE;
const ROOT_B_OFFSET: u64 = PAGE_SIZE * 2;
const DATA_OFFSET: u64 = PAGE_SIZE * 3;
const HEADER_MAGIC: [u8; 8] = *b"CFMDSF01";
const ROOT_MAGIC: [u8; 4] = *b"CFSR";
const GENERATION_MAGIC: [u8; 4] = *b"CFSG";
const FORMAT_VERSION: u16 = 3;
const HEADER_DIGEST_OFFSET: usize = 96;
const KEY_SLOT_MAGIC: [u8; 4] = *b"CFKW";
const KEY_SLOT_VERSION: u8 = 1;
const KEY_SLOT_LEN: usize = 160;
const KEY_SLOT_DIGEST_OFFSET: usize = 128;
const KEY_SLOT_A_OFFSET: usize = 128;
const KEY_SLOT_B_OFFSET: usize = KEY_SLOT_A_OFFSET + KEY_SLOT_LEN;
const KEY_SLOTS_END: usize = KEY_SLOT_B_OFFSET + KEY_SLOT_LEN;
const ROOT_DIGEST_OFFSET: usize = 144;
const ROOT_AUTHORITY_OFFSET: usize = ROOT_DIGEST_OFFSET + 32;
const ROOT_AUTHORITY_BINDING_LEN: usize = 72;
const ROOT_AUTHORITY_DIGEST_OFFSET: usize = ROOT_AUTHORITY_OFFSET + ROOT_AUTHORITY_BINDING_LEN;
const ROOT_AUTHORITY_END: usize = ROOT_AUTHORITY_DIGEST_OFFSET + 32;
const ROOT_AUTHORITY_DOMAIN: &[u8] = b"CFMD/single-file-replication-authority-root/v1";
const GENERATION_HEADER_LEN: usize = 128;
const GENERATION_HEADER_DIGEST_OFFSET: usize = 88;
const SECTION_DESCRIPTOR_LEN: usize = 64;
const MAX_SECTION_COUNT: usize = 65_535;
const MAX_SECTION_LEN: u64 = 1_u64 << 40;
const MAX_GENERATION_LEN: u64 = 1_u64 << 44;
const ENCRYPTED_SECTION_MAGIC: [u8; 4] = *b"CFSC";
const ENCRYPTED_SECTION_VERSION: u8 = 1;
const ENCRYPTED_SECTION_HEADER_LEN: usize = 24;
const ENCRYPTED_SECTION_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SingleFileSectionKind {
    Checkpoint = 1,
    Metadata = 2,
    PreparedCapsule = 3,
    ReplicationAuthority = 4,
    PhysicalArtifact = 5,
    Auxiliary = 6,
    HistoricalEpochDescriptor = 7,
    HistoricalCheckpoint = 8,
    HistoricalMetadata = 9,
    HistoricalPreparedCapsule = 10,
    HistoricalWal = 11,
}

impl SingleFileSectionKind {
    fn decode(raw: u16) -> Result<Self, DurabilityError> {
        match raw {
            1 => Ok(Self::Checkpoint),
            2 => Ok(Self::Metadata),
            3 => Ok(Self::PreparedCapsule),
            4 => Ok(Self::ReplicationAuthority),
            5 => Ok(Self::PhysicalArtifact),
            6 => Ok(Self::Auxiliary),
            7 => Ok(Self::HistoricalEpochDescriptor),
            8 => Ok(Self::HistoricalCheckpoint),
            9 => Ok(Self::HistoricalMetadata),
            10 => Ok(Self::HistoricalPreparedCapsule),
            11 => Ok(Self::HistoricalWal),
            _ => Err(corruption("single-file section kind is unsupported")),
        }
    }
}

pub(crate) trait SingleFileSectionSource {
    fn plaintext_len(&self) -> Result<u64, DurabilityError>;

    fn write_to(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError>;
}

#[derive(Clone, Copy)]
pub(crate) enum SingleFileSectionContent<'a> {
    Bytes(&'a [u8]),
    Streaming(&'a dyn SingleFileSectionSource),
}

impl SingleFileSectionContent<'_> {
    fn plaintext_len(self) -> Result<u64, DurabilityError> {
        match self {
            Self::Bytes(bytes) => {
                u64::try_from(bytes.len()).map_err(|_| DurabilityError::PayloadTooLarge)
            }
            Self::Streaming(source) => source.plaintext_len(),
        }
    }

    fn write_to(
        self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        match self {
            Self::Bytes(bytes) => emit(bytes),
            Self::Streaming(source) => source.write_to(emit),
        }
    }
}

#[derive(Clone, Copy)]
pub struct SingleFileSectionInput<'a> {
    pub kind: SingleFileSectionKind,
    pub ordinal: u32,
    pub(crate) content: SingleFileSectionContent<'a>,
}

impl<'a> SingleFileSectionInput<'a> {
    #[must_use]
    pub const fn physical_realization(bytes: &'a [u8]) -> Self {
        Self::bytes(SingleFileSectionKind::PhysicalArtifact, 0, bytes)
    }

    pub(crate) const fn bytes(kind: SingleFileSectionKind, ordinal: u32, bytes: &'a [u8]) -> Self {
        Self {
            kind,
            ordinal,
            content: SingleFileSectionContent::Bytes(bytes),
        }
    }

    pub(crate) const fn streaming(
        kind: SingleFileSectionKind,
        ordinal: u32,
        source: &'a dyn SingleFileSectionSource,
    ) -> Self {
        Self {
            kind,
            ordinal,
            content: SingleFileSectionContent::Streaming(source),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CarriedWalPublication<'a> {
    pub(crate) start_offset: u64,
    pub(crate) first_lsn: u64,
    pub(crate) base_revision: RevisionId,
    pub(crate) expected_durable_revision: RevisionId,
    pub(crate) seeded_prepares: &'a [(u64, DurableRevisionDescriptor, u32)],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HistoricalGenerationArchive {
    pub(crate) generation: u64,
    pub(crate) checkpoint_revision: RevisionId,
    pub(crate) durable_head: RevisionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HistoricalArchiveDescriptor {
    generation: u64,
    checkpoint_revision: RevisionId,
    journal_first_lsn: u64,
    journal_next_lsn: u64,
    durable_head: RevisionId,
}

const HISTORICAL_ARCHIVE_MAGIC: [u8; 4] = *b"CFEA";
const HISTORICAL_ARCHIVE_VERSION: u16 = 1;
const HISTORICAL_ARCHIVE_DESCRIPTOR_LEN: usize = 48;

fn encode_historical_archive_descriptor(
    descriptor: HistoricalArchiveDescriptor,
) -> [u8; HISTORICAL_ARCHIVE_DESCRIPTOR_LEN] {
    let mut out = [0_u8; HISTORICAL_ARCHIVE_DESCRIPTOR_LEN];
    out[0..4].copy_from_slice(&HISTORICAL_ARCHIVE_MAGIC);
    out[4..6].copy_from_slice(&HISTORICAL_ARCHIVE_VERSION.to_le_bytes());
    out[8..16].copy_from_slice(&descriptor.generation.to_le_bytes());
    out[16..24].copy_from_slice(&descriptor.checkpoint_revision.raw().to_le_bytes());
    out[24..32].copy_from_slice(&descriptor.journal_first_lsn.to_le_bytes());
    out[32..40].copy_from_slice(&descriptor.journal_next_lsn.to_le_bytes());
    out[40..48].copy_from_slice(&descriptor.durable_head.raw().to_le_bytes());
    out
}

fn decode_historical_archive_descriptor(
    bytes: &[u8],
) -> Result<HistoricalArchiveDescriptor, DurabilityError> {
    if bytes.len() != HISTORICAL_ARCHIVE_DESCRIPTOR_LEN
        || bytes[0..4] != HISTORICAL_ARCHIVE_MAGIC
        || u16::from_le_bytes(bytes[4..6].try_into().expect("fixed slice"))
            != HISTORICAL_ARCHIVE_VERSION
        || bytes[6..8] != [0, 0]
    {
        return Err(corruption(
            "single-file historical epoch descriptor is invalid",
        ));
    }
    Ok(HistoricalArchiveDescriptor {
        generation: u64::from_le_bytes(bytes[8..16].try_into().expect("fixed slice")),
        checkpoint_revision: RevisionId::new(u64::from_le_bytes(
            bytes[16..24].try_into().expect("fixed slice"),
        )),
        journal_first_lsn: u64::from_le_bytes(bytes[24..32].try_into().expect("fixed slice")),
        journal_next_lsn: u64::from_le_bytes(bytes[32..40].try_into().expect("fixed slice")),
        durable_head: RevisionId::new(u64::from_le_bytes(
            bytes[40..48].try_into().expect("fixed slice"),
        )),
    })
}

#[derive(Clone)]
struct HistoricalSectionSource {
    path: PathBuf,
    generation: u64,
    section: SingleFileSectionDescriptor,
    crypto: Option<StorageAeadCodec>,
    plaintext_len: u64,
}

impl HistoricalSectionSource {
    fn open(
        path: &Path,
        generation: u64,
        section: SingleFileSectionDescriptor,
        crypto: Option<StorageAeadCodec>,
    ) -> Result<Self, DurabilityError> {
        let mut file = File::open(path)?;
        let reader =
            SingleFileSectionReader::open(&mut file, generation, section.clone(), crypto.clone())?;
        let plaintext_len = reader.plaintext_len();
        Ok(Self {
            path: path.to_path_buf(),
            generation,
            section,
            crypto,
            plaintext_len,
        })
    }
}

impl SingleFileSectionSource for HistoricalSectionSource {
    fn plaintext_len(&self) -> Result<u64, DurabilityError> {
        Ok(self.plaintext_len)
    }

    fn write_to(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        let mut file = File::open(&self.path)?;
        let mut reader = SingleFileSectionReader::open(
            &mut file,
            self.generation,
            self.section.clone(),
            self.crypto.clone(),
        )?;
        let mut buffer = [0_u8; IO_BUFFER_SIZE];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            emit(&buffer[..read])?;
        }
        reader.finish()
    }
}

struct HistoricalWalSource {
    path: PathBuf,
    offset: u64,
    len: u64,
}

impl SingleFileSectionSource for HistoricalWalSource {
    fn plaintext_len(&self) -> Result<u64, DurabilityError> {
        Ok(self.len)
    }

    fn write_to(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut remaining = self.len;
        let mut buffer = [0_u8; IO_BUFFER_SIZE];
        while remaining != 0 {
            let chunk = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
            file.read_exact(&mut buffer[..chunk]).map_err(|error| {
                eof_as_corruption(error, "single-file historical WAL is truncated")
            })?;
            emit(&buffer[..chunk])?;
            remaining -= chunk as u64;
        }
        Ok(())
    }
}

struct OutgoingHistoricalArchiveSources {
    ordinal: u32,
    descriptor: [u8; HISTORICAL_ARCHIVE_DESCRIPTOR_LEN],
    checkpoint: HistoricalSectionSource,
    metadata: HistoricalSectionSource,
    prepared: Option<HistoricalSectionSource>,
    wal: HistoricalWalSource,
}

struct HistoricalPublicationSources {
    carried: Vec<(SingleFileSectionKind, u32, HistoricalSectionSource)>,
    outgoing: Option<OutgoingHistoricalArchiveSources>,
}

impl HistoricalPublicationSources {
    fn append_inputs<'a>(&'a self, inputs: &mut Vec<SingleFileSectionInput<'a>>) {
        for (kind, ordinal, source) in &self.carried {
            inputs.push(SingleFileSectionInput::streaming(*kind, *ordinal, source));
        }
        let Some(outgoing) = &self.outgoing else {
            return;
        };
        inputs.push(SingleFileSectionInput::bytes(
            SingleFileSectionKind::HistoricalEpochDescriptor,
            outgoing.ordinal,
            &outgoing.descriptor,
        ));
        inputs.push(SingleFileSectionInput::streaming(
            SingleFileSectionKind::HistoricalCheckpoint,
            outgoing.ordinal,
            &outgoing.checkpoint,
        ));
        inputs.push(SingleFileSectionInput::streaming(
            SingleFileSectionKind::HistoricalMetadata,
            outgoing.ordinal,
            &outgoing.metadata,
        ));
        if let Some(prepared) = &outgoing.prepared {
            inputs.push(SingleFileSectionInput::streaming(
                SingleFileSectionKind::HistoricalPreparedCapsule,
                outgoing.ordinal,
                prepared,
            ));
        }
        inputs.push(SingleFileSectionInput::streaming(
            SingleFileSectionKind::HistoricalWal,
            outgoing.ordinal,
            &outgoing.wal,
        ));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleFileSectionDescriptor {
    pub kind: SingleFileSectionKind,
    pub ordinal: u32,
    pub offset: u64,
    pub len: u64,
    pub digest: [u8; 32],
}

struct SingleFileSectionReader<'a> {
    file: &'a mut File,
    section: SingleFileSectionDescriptor,
    generation: u64,
    crypto: Option<StorageAeadCodec>,
    stored_hasher: Sha256,
    plaintext_len: u64,
    remaining_plaintext: u64,
    chunk_count: u32,
    next_chunk: u32,
    chunk: Vec<u8>,
    chunk_position: usize,
    verified: bool,
}

impl<'a> SingleFileSectionReader<'a> {
    fn open(
        file: &'a mut File,
        generation: u64,
        section: SingleFileSectionDescriptor,
        crypto: Option<StorageAeadCodec>,
    ) -> Result<Self, DurabilityError> {
        file.seek(SeekFrom::Start(section.offset))?;
        if let Some(crypto) = crypto {
            let mut header = [0_u8; ENCRYPTED_SECTION_HEADER_LEN];
            file.read_exact(&mut header).map_err(|error| {
                eof_as_corruption(error, "encrypted section header is truncated")
            })?;
            if header[0..4] != ENCRYPTED_SECTION_MAGIC
                || header[4] != ENCRYPTED_SECTION_VERSION
                || header[5] != crypto.algorithm() as u8
                || header[6..8] != [0, 0]
            {
                return Err(corruption("encrypted section header is invalid"));
            }
            let chunk_size = usize::try_from(get_u32(&header[8..12]))
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
            if chunk_size != ENCRYPTED_SECTION_CHUNK_SIZE {
                return Err(corruption("encrypted section chunk size is unsupported"));
            }
            let plaintext_len = get_u64(&header[12..20]);
            let chunk_count = get_u32(&header[20..24]);
            if chunk_count != encrypted_section_chunk_count(plaintext_len)? {
                return Err(corruption("encrypted section chunk count is inconsistent"));
            }
            let envelope_overhead = u64::try_from(StorageAeadCodec::sealed_len(0)?)
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
            let expected_stored_len = u64::try_from(ENCRYPTED_SECTION_HEADER_LEN)
                .map_err(|_| DurabilityError::PayloadTooLarge)?
                .checked_add(plaintext_len)
                .and_then(|len| len.checked_add(envelope_overhead * u64::from(chunk_count)))
                .ok_or(DurabilityError::PayloadTooLarge)?;
            if expected_stored_len != section.len {
                return Err(corruption(
                    "encrypted section stored length is inconsistent",
                ));
            }
            let mut stored_hasher = Sha256::new();
            stored_hasher.update(header);
            Ok(Self {
                file,
                section,
                generation,
                crypto: Some(crypto),
                stored_hasher,
                plaintext_len,
                remaining_plaintext: plaintext_len,
                chunk_count,
                next_chunk: 0,
                chunk: Vec::new(),
                chunk_position: 0,
                verified: false,
            })
        } else {
            Ok(Self {
                file,
                plaintext_len: section.len,
                remaining_plaintext: section.len,
                section,
                generation,
                crypto: None,
                stored_hasher: Sha256::new(),
                chunk_count: 0,
                next_chunk: 0,
                chunk: Vec::new(),
                chunk_position: 0,
                verified: false,
            })
        }
    }

    const fn plaintext_len(&self) -> u64 {
        self.plaintext_len
    }

    fn verify_digest(&mut self) -> Result<(), DurabilityError> {
        if self.verified {
            return Ok(());
        }
        let digest: [u8; 32] = self.stored_hasher.clone().finalize().into();
        if digest != self.section.digest {
            return Err(corruption("single-file section digest mismatch"));
        }
        self.verified = true;
        Ok(())
    }

    fn load_next_encrypted_chunk(&mut self) -> Result<(), DurabilityError> {
        if self.next_chunk >= self.chunk_count {
            if self.remaining_plaintext != 0 {
                return Err(corruption("encrypted section plaintext length mismatch"));
            }
            self.verify_digest()?;
            return Ok(());
        }
        let chunk_plaintext_len = usize::try_from(
            self.remaining_plaintext
                .min(ENCRYPTED_SECTION_CHUNK_SIZE as u64),
        )
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
        let envelope_len = StorageAeadCodec::sealed_len(chunk_plaintext_len)?;
        let mut envelope = vec![0_u8; envelope_len];
        self.file
            .read_exact(&mut envelope)
            .map_err(|error| eof_as_corruption(error, "encrypted section chunk is truncated"))?;
        self.stored_hasher.update(&envelope);
        let context = section_chunk_aad_context(
            self.generation,
            self.section.kind,
            self.section.ordinal,
            self.next_chunk,
            self.plaintext_len,
            u32::try_from(chunk_plaintext_len).map_err(|_| DurabilityError::PayloadTooLarge)?,
        );
        let crypto = self
            .crypto
            .as_ref()
            .ok_or_else(|| corruption("encrypted section reader is missing crypto state"))?;
        self.chunk = crypto.open(StorageEncryptionDomain::Section, &context, &envelope)?;
        if self.chunk.len() != chunk_plaintext_len {
            return Err(corruption(
                "encrypted section chunk plaintext length mismatch",
            ));
        }
        self.chunk_position = 0;
        self.next_chunk += 1;
        self.remaining_plaintext = self
            .remaining_plaintext
            .checked_sub(
                u64::try_from(chunk_plaintext_len).map_err(|_| DurabilityError::PayloadTooLarge)?,
            )
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if self.next_chunk == self.chunk_count && self.remaining_plaintext == 0 {
            self.verify_digest()?;
        }
        Ok(())
    }

    fn read_plaintext(&mut self, out: &mut [u8]) -> Result<usize, DurabilityError> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.crypto.is_none() {
            if self.chunk_position == self.chunk.len() {
                self.chunk.clear();
                self.chunk_position = 0;
                if self.remaining_plaintext == 0 {
                    self.verify_digest()?;
                    return Ok(0);
                }
                let chunk_len = usize::try_from(
                    self.remaining_plaintext
                        .min(ENCRYPTED_SECTION_CHUNK_SIZE as u64),
                )
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
                self.chunk.resize(chunk_len, 0);
                self.file.read_exact(&mut self.chunk).map_err(|error| {
                    eof_as_corruption(error, "single-file section is truncated")
                })?;
                self.stored_hasher.update(&self.chunk);
                self.remaining_plaintext -= chunk_len as u64;
                if self.remaining_plaintext == 0 {
                    self.verify_digest()?;
                }
            }
            let available = self.chunk.len() - self.chunk_position;
            let take = available.min(out.len());
            out[..take]
                .copy_from_slice(&self.chunk[self.chunk_position..self.chunk_position + take]);
            self.chunk_position += take;
            return Ok(take);
        }

        if self.chunk_position == self.chunk.len() {
            self.chunk.clear();
            self.chunk_position = 0;
            self.load_next_encrypted_chunk()?;
            if self.chunk.is_empty() {
                return Ok(0);
            }
        }
        let available = self.chunk.len() - self.chunk_position;
        let take = available.min(out.len());
        out[..take].copy_from_slice(&self.chunk[self.chunk_position..self.chunk_position + take]);
        self.chunk_position += take;
        Ok(take)
    }

    fn finish(&mut self) -> Result<(), DurabilityError> {
        if self.crypto.is_some()
            && self.plaintext_len == 0
            && self.next_chunk == 0
            && self.chunk_count == 1
        {
            self.load_next_encrypted_chunk()?;
        }
        if self.remaining_plaintext != 0 || self.chunk_position != self.chunk.len() {
            return Err(corruption(
                "single-file section plaintext was not fully consumed",
            ));
        }
        if self.crypto.is_some() && self.next_chunk != self.chunk_count {
            return Err(corruption(
                "single-file encrypted section was not fully consumed",
            ));
        }
        self.verify_digest()
    }
}

impl Read for SingleFileSectionReader<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.read_plaintext(out).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "single-file section source failed authentication or integrity validation",
            )
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleFileGenerationView {
    pub sequence: u64,
    pub generation: u64,
    pub generation_offset: u64,
    pub generation_len: u64,
    pub generation_digest: [u8; 32],
    pub parent_digest: [u8; 32],
    pub journal_offset: u64,
    pub journal_end: Option<u64>,
    pub journal_first_lsn: u64,
    pub sealed_journal_next_lsn: Option<u64>,
    pub sections: Vec<SingleFileSectionDescriptor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootSlot {
    A,
    B,
}

impl RootSlot {
    const fn offset(self) -> u64 {
        match self {
            Self::A => ROOT_A_OFFSET,
            Self::B => ROOT_B_OFFSET,
        }
    }

    const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RootRecord {
    slot: RootSlot,
    sequence: u64,
    generation: u64,
    generation_offset: u64,
    generation_len: u64,
    journal_offset: u64,
    journal_end: u64,
    journal_first_lsn: u64,
    journal_next_lsn: u64,
    section_count: u32,
    generation_digest: [u8; 32],
    parent_digest: [u8; 32],
    replication_authority: Option<ReplicationAuthorityLocatorRoot>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SingleFileKeyMode {
    None,
    Direct,
    Wrapped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WrappedKeySlot {
    publication_sequence: u64,
    key_epoch: u64,
    provider_key_epoch: u64,
    provider_key_id: [u8; 16],
    wrapped: WrappedDatabaseMasterKey,
}

#[derive(Debug, Clone, Copy)]
struct SingleFileEncryptionHeader {
    algorithm: Option<StorageAeadAlgorithm>,
    key_mode: SingleFileKeyMode,
    database_salt: [u8; 32],
    key_commitment: [u8; 16],
    wrapped_slots: [Option<WrappedKeySlot>; 2],
}

#[derive(Debug, Clone)]
struct WrappedKeyState {
    active_slot: usize,
    slot: WrappedKeySlot,
}

struct OpenedEncryption {
    crypto: Option<StorageAeadCodec>,
    master_key: Option<StorageEncryptionKey>,
    wrapped_state: Option<WrappedKeyState>,
}

#[derive(Debug, Clone, Copy)]
struct ReplicationAuthorityCompactionEntry {
    id: ReplicationAuthoritySegmentId,
    parent: Option<ReplicationAuthoritySegmentId>,
    extent: ReplicationAuthoritySegmentExtent,
}

#[derive(Debug, Clone)]
struct SingleFileCompactionPlan {
    old_root: RootRecord,
    journal_len: u64,
    next_lsn: u64,
    authority_chain: Vec<ReplicationAuthorityCompactionEntry>,
}

#[derive(Clone, Copy)]
struct SingleFileCompactionValidation<'a> {
    base_revision: RevisionId,
    seeded_prepares: &'a [(u64, DurableRevisionDescriptor, u32)],
    expected_durable_revision: RevisionId,
    expected_next_lsn: u64,
    expected_journal_len: u64,
}

#[derive(Debug, Clone, Copy)]
enum SingleFileCompactionImageKind {
    Staging,
    Front,
}

impl SingleFileCompactionImageKind {
    const fn read_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingImageRead,
            Self::Front => SingleFileCompactionIoStep::FrontImageRead,
        }
    }

    const fn write_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingImageWrite,
            Self::Front => SingleFileCompactionIoStep::FrontImageWrite,
        }
    }

    const fn seek_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingSourceSeek,
            Self::Front => SingleFileCompactionIoStep::FrontSourceSeek,
        }
    }

    const fn validation_read_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingValidationRead,
            Self::Front => SingleFileCompactionIoStep::FrontValidationRead,
        }
    }

    const fn validation_seek_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingValidationSeek,
            Self::Front => SingleFileCompactionIoStep::FrontValidationSeek,
        }
    }

    const fn source_open_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingSourceOpen,
            Self::Front => SingleFileCompactionIoStep::FrontSourceOpen,
        }
    }

    const fn destination_seek_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingDestinationSeek,
            Self::Front => SingleFileCompactionIoStep::FrontDestinationSeek,
        }
    }

    const fn validation_open_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingValidationOpen,
            Self::Front => SingleFileCompactionIoStep::FrontValidationOpen,
        }
    }

    const fn wal_read_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingWalRead,
            Self::Front => SingleFileCompactionIoStep::FrontWalRead,
        }
    }

    const fn wal_seek_op(self) -> SingleFileCompactionIoStep {
        match self {
            Self::Staging => SingleFileCompactionIoStep::StagingWalSeek,
            Self::Front => SingleFileCompactionIoStep::FrontWalSeek,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SingleFileCompactionImagePlacement {
    source_generation_offset: u64,
    source_journal_offset: u64,
    destination_start: u64,
    sequence: u64,
    slot: RootSlot,
    kind: SingleFileCompactionImageKind,
}

struct PreparedSingleFileCompaction<'a, F> {
    plan: SingleFileCompactionPlan,
    publication: SingleFileCompactionPublication<'a, F, CompactionSourceSealed>,
}

fn recover_compaction_wal_with_io(
    mut region: WalRegionRecovery<'_>,
    read_step: SingleFileCompactionIoStep,
    seek_step: SingleFileCompactionIoStep,
    io: &mut impl SingleFileCompactionIo,
) -> Result<(FileRevisionWal, RecoveryScan), DurabilityError> {
    let scan_spec = WalRegionScanSpec {
        start_offset: region.start_offset,
        end_offset: region.end_offset,
        base_revision: region.base_revision,
        first_lsn: region.first_lsn,
        seeded_prepares: region.seeded_prepares,
        crypto: region.crypto.as_ref(),
    };
    let backing_len = region.end_offset;
    let scanned = {
        let mut reader = SingleFileCompactionReader {
            io,
            file: &mut region.file,
            read_step,
            seek_step,
        };
        FileRevisionWal::scan_region_recovery(&scan_spec, &mut reader, backing_len)?
    };
    io.seek(
        seek_step,
        &mut region.file,
        SeekFrom::Start(scanned.good_end()),
    )?;
    FileRevisionWal::from_region_scan(region, scanned)
}

#[derive(Debug)]
pub struct SingleFileContainer {
    path: PathBuf,
    file: File,
    root: RootRecord,
    crypto: Option<StorageAeadCodec>,
    master_key: Option<StorageEncryptionKey>,
    wrapped_key_state: Option<WrappedKeyState>,
}

impl SingleFileContainer {
    pub fn create(
        path: impl AsRef<Path>,
        sections: &[SingleFileSectionInput<'_>],
    ) -> Result<Self, DurabilityError> {
        Self::create_with_encryption(path, sections, &StorageEncryption::None)
    }

    pub fn create_with_encryption(
        path: impl AsRef<Path>,
        sections: &[SingleFileSectionInput<'_>],
        encryption: &StorageEncryption,
    ) -> Result<Self, DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let (encryption_header, opened) = prepare_encryption_header(encryption)?;
        let crypto = opened.crypto;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.lock()?;
        write_header(&mut file, &encryption_header)?;
        file.set_len(DATA_OFFSET)?;
        file.sync_all()?;
        sync_parent(&path)?;
        let root =
            publish_generation_to_file(&mut file, None, 1, 1, sections, None, crypto.as_ref())?;
        Ok(Self {
            path,
            file,
            root,
            crypto,
            master_key: opened.master_key,
            wrapped_key_state: opened.wrapped_state,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, DurabilityError> {
        Self::open_with_encryption(path, &StorageEncryption::None)
    }

    pub fn open_with_encryption(
        path: impl AsRef<Path>,
        encryption: &StorageEncryption,
    ) -> Result<Self, DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        file.lock()?;
        let encryption_header = read_and_validate_header(&mut file)?;
        let opened = open_encryption_codec(encryption, &encryption_header)?;
        let crypto = opened.crypto;
        let slot_a = read_root_slot(&mut file, RootSlot::A)?;
        let slot_b = read_root_slot(&mut file, RootSlot::B)?;
        let root = choose_authoritative_root(slot_a, slot_b)?;
        validate_authoritative_generation(&mut file, root)?;
        validate_replication_authority_objects(&mut file, &path, root, crypto.as_ref())?;
        Ok(Self {
            path,
            file,
            root,
            crypto,
            master_key: opened.master_key,
            wrapped_key_state: opened.wrapped_state,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.root.sequence
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.root.generation
    }

    pub fn rewrap_database_master_key(
        &mut self,
        next: &StorageEncryption,
    ) -> Result<u64, DurabilityError> {
        let state = self
            .wrapped_key_state
            .clone()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "database master key rewrap requires an existing wrapped-key store",
            })?;
        let master_key = self.master_key.as_ref().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "wrapped-key store has no unlocked database master key",
        })?;
        let next = wrapped_encryption_config(
            next,
            "database master key rewrap requires wrapped provider encryption",
        )?;
        let header = read_and_validate_header(&mut self.file)?;
        if header.key_mode != SingleFileKeyMode::Wrapped {
            return Err(corruption(
                "wrapped-key state disagrees with single-file header",
            ));
        }
        if header.algorithm != Some(next.algorithm) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "database master key rewrap cannot change storage AEAD algorithm",
            });
        }
        if let Some((pending_slot, pending)) = recover_pending_wrapped_key_handoff(
            &state,
            &header,
            &next.provider_key_id,
            next.provider_key_epoch,
            next.minimum_database_key_epoch,
        )? {
            self.wrapped_key_state = Some(WrappedKeyState {
                active_slot: pending_slot,
                slot: pending,
            });
            return Ok(pending.key_epoch);
        }
        if state.slot.provider_key_id == next.provider_key_id
            && state.slot.provider_key_epoch == next.provider_key_epoch
        {
            if state.slot.key_epoch >= next.minimum_database_key_epoch {
                return Ok(state.slot.key_epoch);
            }
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "provider database-key floor exceeds the active wrapped-key epoch",
            });
        }
        let publication_sequence =
            state
                .slot
                .publication_sequence
                .checked_add(1)
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "wrapped-key publication sequence exhausted",
                })?;
        let key_epoch = state
            .slot
            .key_epoch
            .checked_add(1)
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "database encryption key epoch exhausted",
            })?;
        if next.minimum_database_key_epoch == 0 || key_epoch < next.minimum_database_key_epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "next provider database-key epoch floor exceeds the rewrap epoch",
            });
        }
        let wrapped = wrap_database_master_key(
            next.wrapping_key,
            &header.database_salt,
            &next.provider_key_id,
            next.provider_key_epoch,
            key_epoch,
            publication_sequence,
            master_key,
        )?;
        let next_slot = WrappedKeySlot {
            publication_sequence,
            key_epoch,
            provider_key_epoch: next.provider_key_epoch,
            provider_key_id: next.provider_key_id,
            wrapped,
        };
        let inactive = 1 - state.active_slot;
        write_wrapped_key_slot(&mut self.file, inactive, next_slot)?;
        self.file.sync_all()?;
        self.wrapped_key_state = Some(WrappedKeyState {
            active_slot: inactive,
            slot: next_slot,
        });
        Ok(key_epoch)
    }

    pub fn retire_previous_wrapped_key_slot(
        &mut self,
        acknowledged_key_epoch: u64,
    ) -> Result<(), DurabilityError> {
        let state = self
            .wrapped_key_state
            .as_ref()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "wrapped-key predecessor retirement requires a wrapped-key store",
            })?;
        if state.slot.key_epoch != acknowledged_key_epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external acknowledgement does not match active database-key epoch",
            });
        }
        let obsolete = 1 - state.active_slot;
        let offset = u64::try_from(key_slot_range(obsolete).start)
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.write_all(&[0_u8; KEY_SLOT_LEN])?;
        self.file.sync_all()?;
        Ok(())
    }

    pub fn generation_view(&mut self) -> Result<SingleFileGenerationView, DurabilityError> {
        read_generation_view(&mut self.file, self.root)
    }

    pub(crate) fn recover_replication_authority_journal(
        &mut self,
        live_frames: &[Vec<u8>],
    ) -> Result<ReplicationAuthorityJournal, DurabilityError> {
        let mut journal = ReplicationAuthorityJournal::open_single_file(&self.path, &[], &[])?;
        if let Some(authority_root) = self.root.replication_authority {
            let index = recover_locator_chain(&mut self.file, authority_root)?;
            replay_indexed_segment_object_chain(
                &mut self.file,
                &index,
                self.crypto.as_ref(),
                &mut journal,
            )?;
        }
        journal.replay_single_file_live_frames(live_frames)?;
        Ok(journal)
    }

    pub fn read_section(
        &mut self,
        kind: SingleFileSectionKind,
        ordinal: u32,
    ) -> Result<Option<Vec<u8>>, DurabilityError> {
        let view = self.generation_view()?;
        let Some(section) = view
            .sections
            .iter()
            .find(|section| section.kind == kind && section.ordinal == ordinal)
            .cloned()
        else {
            return Ok(None);
        };
        let mut bytes = Vec::new();
        self.copy_section_descriptor_plaintext_to(view.generation, &section, &mut bytes)?;
        Ok(Some(bytes))
    }

    pub(crate) fn with_section_reader<T>(
        &mut self,
        kind: SingleFileSectionKind,
        ordinal: u32,
        decode: impl FnOnce(&mut dyn Read, u64) -> Result<T, DurabilityError>,
    ) -> Result<Option<T>, DurabilityError> {
        let view = self.generation_view()?;
        let Some(section) = view
            .sections
            .iter()
            .find(|section| section.kind == kind && section.ordinal == ordinal)
            .cloned()
        else {
            return Ok(None);
        };
        let mut reader = SingleFileSectionReader::open(
            &mut self.file,
            view.generation,
            section,
            self.crypto.clone(),
        )?;
        let plaintext_len = reader.plaintext_len();
        let decoded = decode(&mut reader, plaintext_len)?;
        reader.finish()?;
        Ok(Some(decoded))
    }

    pub fn copy_section_to(
        &mut self,
        kind: SingleFileSectionKind,
        ordinal: u32,
        output: &mut impl Write,
    ) -> Result<Option<u64>, DurabilityError> {
        let view = self.generation_view()?;
        let Some(section) = view
            .sections
            .iter()
            .find(|section| section.kind == kind && section.ordinal == ordinal)
            .cloned()
        else {
            return Ok(None);
        };
        self.copy_section_descriptor_plaintext_to(view.generation, &section, output)
            .map(Some)
    }

    fn copy_section_descriptor_plaintext_to(
        &mut self,
        generation: u64,
        section: &SingleFileSectionDescriptor,
        output: &mut impl Write,
    ) -> Result<u64, DurabilityError> {
        let Some(crypto) = self.crypto.clone() else {
            self.copy_section_descriptor_to(section, output)?;
            return Ok(section.len);
        };

        self.file.seek(SeekFrom::Start(section.offset))?;
        let mut magic = [0_u8; 4];
        self.file
            .read_exact(&mut magic)
            .map_err(|error| eof_as_corruption(error, "single-file section is truncated"))?;
        if magic != ENCRYPTED_SECTION_MAGIC {
            return Err(corruption("encrypted section format is unsupported"));
        }
        self.copy_chunked_encrypted_section_to(generation, section, &crypto, output)
    }

    fn copy_chunked_encrypted_section_to(
        &mut self,
        generation: u64,
        section: &SingleFileSectionDescriptor,
        crypto: &StorageAeadCodec,
        output: &mut impl Write,
    ) -> Result<u64, DurabilityError> {
        self.file.seek(SeekFrom::Start(section.offset))?;
        let mut header = [0_u8; ENCRYPTED_SECTION_HEADER_LEN];
        self.file
            .read_exact(&mut header)
            .map_err(|error| eof_as_corruption(error, "encrypted section header is truncated"))?;
        if header[0..4] != ENCRYPTED_SECTION_MAGIC
            || header[4] != ENCRYPTED_SECTION_VERSION
            || header[5] != crypto.algorithm() as u8
            || header[6..8] != [0, 0]
        {
            return Err(corruption("encrypted section header is invalid"));
        }
        let chunk_size = usize::try_from(get_u32(&header[8..12]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        if chunk_size != ENCRYPTED_SECTION_CHUNK_SIZE {
            return Err(corruption("encrypted section chunk size is unsupported"));
        }
        let plaintext_len = get_u64(&header[12..20]);
        let chunk_count = get_u32(&header[20..24]);
        let expected_chunk_count = encrypted_section_chunk_count(plaintext_len)?;
        if chunk_count != expected_chunk_count {
            return Err(corruption("encrypted section chunk count is inconsistent"));
        }

        let envelope_overhead = u64::try_from(StorageAeadCodec::sealed_len(0)?)
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        let expected_stored_len = u64::try_from(ENCRYPTED_SECTION_HEADER_LEN)
            .map_err(|_| DurabilityError::PayloadTooLarge)?
            .checked_add(plaintext_len)
            .and_then(|len| len.checked_add(envelope_overhead * u64::from(chunk_count)))
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if expected_stored_len != section.len {
            return Err(corruption(
                "encrypted section stored length is inconsistent",
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(header);
        let mut remaining_plaintext = plaintext_len;
        let mut written = 0_u64;
        for chunk_index in 0..chunk_count {
            let chunk_plaintext_len = if plaintext_len == 0 {
                0
            } else {
                usize::try_from(remaining_plaintext.min(ENCRYPTED_SECTION_CHUNK_SIZE as u64))
                    .map_err(|_| DurabilityError::PayloadTooLarge)?
            };
            let envelope_len = StorageAeadCodec::sealed_len(chunk_plaintext_len)?;
            let mut envelope = vec![0_u8; envelope_len];
            self.file.read_exact(&mut envelope).map_err(|error| {
                eof_as_corruption(error, "encrypted section chunk is truncated")
            })?;
            hasher.update(&envelope);
            let context = section_chunk_aad_context(
                generation,
                section.kind,
                section.ordinal,
                chunk_index,
                plaintext_len,
                u32::try_from(chunk_plaintext_len).map_err(|_| DurabilityError::PayloadTooLarge)?,
            );
            let plaintext = crypto.open(StorageEncryptionDomain::Section, &context, &envelope)?;
            if plaintext.len() != chunk_plaintext_len {
                return Err(corruption(
                    "encrypted section chunk plaintext length mismatch",
                ));
            }
            // Each chunk is authenticated before any bytes from that chunk are released.
            output.write_all(&plaintext)?;
            let chunk_plaintext_len_u64 =
                u64::try_from(chunk_plaintext_len).map_err(|_| DurabilityError::PayloadTooLarge)?;
            remaining_plaintext = remaining_plaintext
                .checked_sub(chunk_plaintext_len_u64)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            written = written
                .checked_add(chunk_plaintext_len_u64)
                .ok_or(DurabilityError::PayloadTooLarge)?;
        }
        if remaining_plaintext != 0 || written != plaintext_len {
            return Err(corruption("encrypted section plaintext length mismatch"));
        }
        let digest: [u8; 32] = hasher.finalize().into();
        if digest != section.digest {
            return Err(corruption("single-file section digest mismatch"));
        }
        Ok(written)
    }

    fn copy_section_descriptor_to(
        &mut self,
        section: &SingleFileSectionDescriptor,
        output: &mut impl Write,
    ) -> Result<(), DurabilityError> {
        self.file.seek(SeekFrom::Start(section.offset))?;
        let mut remaining = section.len;
        let mut buffer = [0_u8; IO_BUFFER_SIZE];
        let mut hasher = Sha256::new();
        while remaining != 0 {
            let chunk = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
            self.file
                .read_exact(&mut buffer[..chunk])
                .map_err(|error| eof_as_corruption(error, "single-file section is truncated"))?;
            hasher.update(&buffer[..chunk]);
            output.write_all(&buffer[..chunk])?;
            remaining -= chunk as u64;
        }
        let digest: [u8; 32] = hasher.finalize().into();
        if digest != section.digest {
            return Err(corruption("single-file section digest mismatch"));
        }
        Ok(())
    }

    fn prepare_historical_publication_sources(
        &mut self,
        old_journal_end: u64,
        next_lsn: u64,
        archive_outgoing: Option<HistoricalGenerationArchive>,
        retained_generations: &BTreeSet<u64>,
    ) -> Result<HistoricalPublicationSources, DurabilityError> {
        let current_view = read_generation_view(&mut self.file, self.root)?;
        let mut retained_ordinals = BTreeSet::new();
        let mut max_archive_ordinal = None::<u32>;
        let descriptor_sections: Vec<_> = current_view
            .sections
            .iter()
            .filter(|section| section.kind == SingleFileSectionKind::HistoricalEpochDescriptor)
            .cloned()
            .collect();
        for section in descriptor_sections {
            let mut bytes = Vec::with_capacity(HISTORICAL_ARCHIVE_DESCRIPTOR_LEN);
            self.copy_section_descriptor_plaintext_to(
                current_view.generation,
                &section,
                &mut bytes,
            )?;
            let descriptor = decode_historical_archive_descriptor(&bytes)?;
            if retained_generations.contains(&descriptor.generation) {
                retained_ordinals.insert(section.ordinal);
                max_archive_ordinal = Some(
                    max_archive_ordinal
                        .map_or(section.ordinal, |current| current.max(section.ordinal)),
                );
            }
        }
        let mut carried = Vec::new();
        for section in &current_view.sections {
            if retained_ordinals.contains(&section.ordinal)
                && matches!(
                    section.kind,
                    SingleFileSectionKind::HistoricalEpochDescriptor
                        | SingleFileSectionKind::HistoricalCheckpoint
                        | SingleFileSectionKind::HistoricalMetadata
                        | SingleFileSectionKind::HistoricalPreparedCapsule
                        | SingleFileSectionKind::HistoricalWal
                )
            {
                carried.push((
                    section.kind,
                    section.ordinal,
                    HistoricalSectionSource::open(
                        &self.path,
                        current_view.generation,
                        section.clone(),
                        self.crypto.clone(),
                    )?,
                ));
            }
        }

        let outgoing = if let Some(archive) = archive_outgoing {
            if archive.generation != current_view.generation {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "single-file historical archive generation does not match active generation",
                });
            }
            let ordinal = match max_archive_ordinal {
                Some(value) => value
                    .checked_add(1)
                    .ok_or(DurabilityError::PayloadTooLarge)?,
                None => 0,
            };
            let checkpoint_section = current_view
                .sections
                .iter()
                .find(|section| {
                    section.kind == SingleFileSectionKind::Checkpoint && section.ordinal == 0
                })
                .cloned()
                .ok_or_else(|| {
                    corruption(
                        "single-file checkpoint section is missing while archiving historical epoch",
                    )
                })?;
            let metadata_section = current_view
                .sections
                .iter()
                .find(|section| {
                    section.kind == SingleFileSectionKind::Metadata && section.ordinal == 0
                })
                .cloned()
                .ok_or_else(|| {
                    corruption(
                        "single-file metadata section is missing while archiving historical epoch",
                    )
                })?;
            let prepared_section = current_view
                .sections
                .iter()
                .find(|section| {
                    section.kind == SingleFileSectionKind::PreparedCapsule && section.ordinal == 0
                })
                .cloned();
            Some(OutgoingHistoricalArchiveSources {
                ordinal,
                descriptor: encode_historical_archive_descriptor(HistoricalArchiveDescriptor {
                    generation: archive.generation,
                    checkpoint_revision: archive.checkpoint_revision,
                    journal_first_lsn: self.root.journal_first_lsn,
                    journal_next_lsn: next_lsn,
                    durable_head: archive.durable_head,
                }),
                checkpoint: HistoricalSectionSource::open(
                    &self.path,
                    current_view.generation,
                    checkpoint_section,
                    self.crypto.clone(),
                )?,
                metadata: HistoricalSectionSource::open(
                    &self.path,
                    current_view.generation,
                    metadata_section,
                    self.crypto.clone(),
                )?,
                prepared: prepared_section
                    .map(|section| {
                        HistoricalSectionSource::open(
                            &self.path,
                            current_view.generation,
                            section,
                            self.crypto.clone(),
                        )
                    })
                    .transpose()?,
                wal: HistoricalWalSource {
                    path: self.path.clone(),
                    offset: self.root.journal_offset,
                    len: old_journal_end
                        .checked_sub(self.root.journal_offset)
                        .ok_or(DurabilityError::PayloadTooLarge)?,
                },
            })
        } else {
            None
        };

        Ok(HistoricalPublicationSources { carried, outgoing })
    }

    pub fn publish_generation(
        &mut self,
        sections: &[SingleFileSectionInput<'_>],
    ) -> Result<SingleFileGenerationView, DurabilityError> {
        let file_len = self.file.metadata()?.len();
        if self.root.journal_end == 0 && file_len != self.root.journal_offset {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file live journal requires WAL-aware generation publication",
            });
        }
        if self.root.journal_end == 0 {
            self.seal_journal_boundary(self.root.journal_first_lsn)?;
        }
        self.publish_next_generation(sections)
    }

    pub fn publish_generation_after_wal(
        &mut self,
        mut wal: FileRevisionWal,
        sections: &[SingleFileSectionInput<'_>],
    ) -> Result<SingleFileGenerationView, DurabilityError> {
        if wal.path() != self.path || wal.start_offset() != self.root.journal_offset {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file WAL does not belong to the active journal",
            });
        }
        wal.durability_barrier()?;
        let next_lsn = wal.next_lsn();
        drop(wal);
        self.seal_journal_boundary(next_lsn)?;
        self.publish_next_generation(sections)
    }

    pub(crate) fn publish_generation_after_active_wal(
        &mut self,
        wal: &mut FileRevisionWal,
        sections: &[SingleFileSectionInput<'_>],
        replication_frames: &[Vec<u8>],
        archive_outgoing: Option<HistoricalGenerationArchive>,
        retained_historical_generations: &BTreeSet<u64>,
    ) -> Result<SingleFileGenerationView, DurabilityError> {
        if wal.path() != self.path || wal.start_offset() != self.root.journal_offset {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file WAL does not belong to the active journal",
            });
        }
        let next_lsn = wal.seal_for_generation_rotation()?;
        let old_journal_end = self.seal_journal_boundary(next_lsn)?;
        let replication_authority =
            self.append_replication_authority_segment(replication_frames)?;
        let historical_sources = self.prepare_historical_publication_sources(
            old_journal_end,
            next_lsn,
            archive_outgoing,
            retained_historical_generations,
        )?;
        let mut historical_inputs = Vec::new();
        historical_sources.append_inputs(&mut historical_inputs);
        let mut all_sections = Vec::with_capacity(sections.len() + historical_inputs.len());
        all_sections.extend_from_slice(sections);
        all_sections.extend_from_slice(&historical_inputs);
        self.publish_next_generation_with_authority(&all_sections, replication_authority)
    }

    pub(crate) fn publish_generation_with_carried_wal(
        &mut self,
        wal: &mut FileRevisionWal,
        carry: CarriedWalPublication<'_>,
        sections: &[SingleFileSectionInput<'_>],
        replication_frames: &[Vec<u8>],
        archive_outgoing: Option<HistoricalGenerationArchive>,
        retained_historical_generations: &BTreeSet<u64>,
    ) -> Result<SingleFileGenerationView, DurabilityError> {
        if wal.path() != self.path || wal.start_offset() != self.root.journal_offset {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file WAL does not belong to the active journal",
            });
        }
        if carry.start_offset < self.root.journal_offset
            || carry.first_lsn < self.root.journal_first_lsn
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file carried WAL cut precedes active journal authority",
            });
        }

        let next_lsn = wal.seal_for_generation_rotation()?;
        let old_journal_end = self.seal_journal_boundary(next_lsn)?;
        if carry.start_offset > old_journal_end || carry.first_lsn > next_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file carried WAL cut exceeds sealed journal endpoint",
            });
        }
        let replication_authority =
            self.append_replication_authority_segment(replication_frames)?;

        let historical_sources = self.prepare_historical_publication_sources(
            old_journal_end,
            next_lsn,
            archive_outgoing,
            retained_historical_generations,
        )?;
        let mut historical_inputs = Vec::new();
        historical_sources.append_inputs(&mut historical_inputs);
        let mut all_sections = Vec::with_capacity(sections.len() + historical_inputs.len());
        all_sections.extend_from_slice(sections);
        all_sections.extend_from_slice(&historical_inputs);

        let generation = self
            .root
            .generation
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?;
        let (generation_offset, generation_len, generation_digest, section_count) =
            append_generation_without_root(
                &mut self.file,
                self.root,
                generation,
                &all_sections,
                self.crypto.as_ref(),
            )?;
        let journal_offset = generation_offset
            .checked_add(generation_len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let carried_len = old_journal_end
            .checked_sub(carry.start_offset)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let journal_end = journal_offset
            .checked_add(carried_len)
            .ok_or(DurabilityError::PayloadTooLarge)?;

        // `File::try_clone` may share a seek cursor with the original file on
        // Unix. Carry-forward needs independent source/destination cursors.
        let mut source = File::open(&self.path)?;
        source.seek(SeekFrom::Start(carry.start_offset))?;
        self.file.seek(SeekFrom::Start(journal_offset))?;
        let copied = std::io::copy(&mut source.take(carried_len), &mut self.file)?;
        if copied != carried_len {
            return Err(corruption("single-file carried WAL copy was truncated"));
        }
        self.file.sync_all()?;

        let (_, scan) = FileRevisionWal::open_region_recovered(WalRegionRecovery {
            path: self.path.clone(),
            file: OpenOptions::new().read(true).write(true).open(&self.path)?,
            start_offset: journal_offset,
            end_offset: journal_end,
            base_revision: carry.base_revision,
            first_lsn: carry.first_lsn,
            seeded_prepares: carry.seeded_prepares,
            writable: false,
            crypto: self.crypto.clone(),
        })?;
        let logical_len =
            u64::try_from(scan.last_good_offset()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        if scan.durable_revision() != carry.expected_durable_revision
            || scan.next_lsn() != next_lsn
            || !matches!(scan.tail_status(), TailStatus::Clean)
            || logical_len != carried_len
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "carried WAL does not certify the requested publish endpoint",
            });
        }

        let root = RootRecord {
            slot: self.root.slot.other(),
            sequence: self
                .root
                .sequence
                .checked_add(1)
                .ok_or(DurabilityError::LsnExhausted)?,
            generation,
            generation_offset,
            generation_len,
            journal_offset,
            journal_end,
            journal_first_lsn: carry.first_lsn,
            journal_next_lsn: next_lsn,
            section_count,
            generation_digest,
            parent_digest: self.root.generation_digest,
            replication_authority,
        };
        write_root_slot(&mut self.file, root)?;
        self.file.sync_all()?;
        self.root = root;
        self.generation_view()
    }

    fn publish_next_generation(
        &mut self,
        sections: &[SingleFileSectionInput<'_>],
    ) -> Result<SingleFileGenerationView, DurabilityError> {
        self.publish_next_generation_with_authority(sections, self.root.replication_authority)
    }

    fn publish_next_generation_with_authority(
        &mut self,
        sections: &[SingleFileSectionInput<'_>],
        replication_authority: Option<ReplicationAuthorityLocatorRoot>,
    ) -> Result<SingleFileGenerationView, DurabilityError> {
        let next_first_lsn = self.root.journal_next_lsn;
        if self.root.journal_end == 0 || next_first_lsn == 0 {
            return Err(corruption(
                "single-file generation publication requires sealed journal",
            ));
        }
        let generation = self
            .root
            .generation
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?;
        let next = publish_generation_to_file(
            &mut self.file,
            Some(self.root),
            generation,
            next_first_lsn,
            sections,
            replication_authority,
            self.crypto.as_ref(),
        )?;
        self.root = next;
        self.generation_view()
    }

    fn append_replication_authority_segment(
        &mut self,
        frames: &[Vec<u8>],
    ) -> Result<Option<ReplicationAuthorityLocatorRoot>, DurabilityError> {
        if frames.is_empty() {
            return Ok(self.root.replication_authority);
        }
        if self.root.journal_end == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication authority object publication requires a sealed journal",
            });
        }
        let parent = self.root.replication_authority.map(|root| root.segment_id);
        let plan = ReplicationAuthoritySegmentPlan::from_frames(parent, frames)?;
        let object_offset = self.file.metadata()?.len();
        self.file.seek(SeekFrom::Start(object_offset))?;
        let crypto = self.crypto.clone();
        let mut nonce_sequence = crypto
            .as_ref()
            .map(|_| StorageNonceSequence::random())
            .transpose()?;
        let object_len = write_segment_object(
            &plan,
            frames,
            crypto.as_ref(),
            nonce_sequence.as_mut(),
            &mut |bytes| {
                self.file.write_all(bytes)?;
                Ok(())
            },
        )?;
        let locator_offset = object_offset
            .checked_add(object_len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if self.file.stream_position()? != locator_offset {
            return Err(corruption(
                "replication authority object writer length mismatch",
            ));
        }
        let root = write_locator_node(
            locator_offset,
            plan.id(),
            plan.parent(),
            ReplicationAuthoritySegmentExtent {
                offset: object_offset,
                len: object_len,
            },
            self.root.replication_authority,
            &mut |bytes| {
                self.file.write_all(bytes)?;
                Ok(())
            },
        )?;
        self.file.sync_all()?;
        Ok(Some(root))
    }

    fn historical_archive_descriptor(
        &mut self,
        generation: u64,
    ) -> Result<Option<(u32, HistoricalArchiveDescriptor)>, DurabilityError> {
        let view = self.generation_view()?;
        let descriptors: Vec<_> = view
            .sections
            .iter()
            .filter(|section| section.kind == SingleFileSectionKind::HistoricalEpochDescriptor)
            .cloned()
            .collect();
        for section in descriptors {
            let ordinal = section.ordinal;
            let mut bytes = Vec::with_capacity(HISTORICAL_ARCHIVE_DESCRIPTOR_LEN);
            self.copy_section_descriptor_plaintext_to(view.generation, &section, &mut bytes)?;
            let descriptor = decode_historical_archive_descriptor(&bytes)?;
            if descriptor.generation == generation {
                return Ok(Some((ordinal, descriptor)));
            }
        }
        Ok(None)
    }

    pub(crate) fn has_historical_epoch_archive(
        &mut self,
        generation: u64,
    ) -> Result<bool, DurabilityError> {
        Ok(self.historical_archive_descriptor(generation)?.is_some())
    }

    pub(crate) fn with_historical_epoch_section_reader<T>(
        &mut self,
        generation: u64,
        kind: SingleFileSectionKind,
        decode: impl FnOnce(&mut dyn Read, u64) -> Result<T, DurabilityError>,
    ) -> Result<Option<T>, DurabilityError> {
        let Some((ordinal, _)) = self.historical_archive_descriptor(generation)? else {
            return Ok(None);
        };
        self.with_section_reader(kind, ordinal, decode)
    }

    pub(crate) fn read_historical_epoch_section(
        &mut self,
        generation: u64,
        kind: SingleFileSectionKind,
    ) -> Result<Option<Vec<u8>>, DurabilityError> {
        let Some((ordinal, _)) = self.historical_archive_descriptor(generation)? else {
            return Ok(None);
        };
        self.read_section(kind, ordinal)
    }

    pub(crate) fn scan_historical_epoch_journal(
        &mut self,
        generation: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<Option<RecoveryScan>, DurabilityError> {
        let Some((ordinal, descriptor)) = self.historical_archive_descriptor(generation)? else {
            return Ok(None);
        };
        let view = self.generation_view()?;
        let section = view
            .sections
            .iter()
            .find(|section| {
                section.kind == SingleFileSectionKind::HistoricalWal && section.ordinal == ordinal
            })
            .cloned()
            .ok_or_else(|| corruption("single-file historical epoch WAL section is missing"))?;
        let mut reader = SingleFileSectionReader::open(
            &mut self.file,
            view.generation,
            section,
            self.crypto.clone(),
        )?;
        let plaintext_len = reader.plaintext_len();
        let scan = scan_wal_stream_seeded(
            &mut reader,
            plaintext_len,
            descriptor.checkpoint_revision,
            descriptor.journal_first_lsn,
            seeded_prepares,
            self.crypto.as_ref(),
        )?;
        reader.finish()?;
        if !matches!(scan.tail_status(), TailStatus::Clean)
            || scan.next_lsn() != descriptor.journal_next_lsn
            || scan.durable_revision() != descriptor.durable_head
            || u64::try_from(scan.last_good_offset())
                .map_err(|_| DurabilityError::PayloadTooLarge)?
                != plaintext_len
        {
            return Err(corruption(
                "single-file historical epoch WAL certificate mismatch",
            ));
        }
        Ok(Some(scan))
    }

    pub(crate) fn scan_active_journal_read_only(
        &self,
        base_revision: RevisionId,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<RecoveryScan, DurabilityError> {
        let end_offset = if self.root.journal_end == 0 {
            self.file.metadata()?.len()
        } else {
            self.root.journal_end
        };
        let scan = FileRevisionWal::scan_region_seeded_read_only(
            &self.path,
            self.root.journal_offset,
            end_offset,
            base_revision,
            self.root.journal_first_lsn,
            seeded_prepares,
            self.crypto.as_ref(),
        )?;
        if self.root.journal_end != 0
            && (!matches!(scan.tail_status(), TailStatus::Clean)
                || u64::try_from(scan.last_good_offset())
                    .map_err(|_| DurabilityError::PayloadTooLarge)?
                    != self.root.journal_end - self.root.journal_offset)
        {
            return Err(corruption(
                "single-file sealed journal is not a complete historical WAL prefix",
            ));
        }
        Ok(scan)
    }

    pub fn open_journal_recovered(
        &mut self,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<(FileRevisionWal, RecoveryScan), DurabilityError> {
        if first_lsn != self.root.journal_first_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file journal first LSN disagrees with root authority",
            });
        }
        if self.root.journal_end != 0 {
            let sealed_len = self.root.journal_end - self.root.journal_offset;
            let (wal, scan) = FileRevisionWal::open_region_recovered(WalRegionRecovery {
                path: self.path.clone(),
                file: OpenOptions::new().read(true).write(true).open(&self.path)?,
                start_offset: self.root.journal_offset,
                end_offset: self.root.journal_end,
                base_revision,
                first_lsn,
                seeded_prepares,
                writable: false,
                crypto: self.crypto.clone(),
            })?;
            if !matches!(scan.tail_status(), TailStatus::Clean)
                || u64::try_from(scan.last_good_offset())
                    .map_err(|_| DurabilityError::PayloadTooLarge)?
                    != sealed_len
            {
                return Err(corruption(
                    "single-file sealed journal is not a complete WAL prefix",
                ));
            }
            self.reopen_sealed_journal()?;
            return Ok((wal, scan));
        }
        let end = self.file.metadata()?.len();
        FileRevisionWal::open_region_recovered(WalRegionRecovery {
            path: self.path.clone(),
            file: OpenOptions::new().read(true).write(true).open(&self.path)?,
            start_offset: self.root.journal_offset,
            end_offset: end,
            base_revision,
            first_lsn,
            seeded_prepares,
            writable: true,
            crypto: self.crypto.clone(),
        })
    }

    pub(crate) fn compact_active_generation(
        &mut self,
        wal: &mut FileRevisionWal,
        base_revision: RevisionId,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
        expected_durable_revision: RevisionId,
        fault: &mut impl FnMut(SingleFileCompactionIoStep) -> Result<(), DurabilityError>,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<bool, DurabilityError> {
        let Some(prepared) = self.prepare_compaction_plan(wal, fault, io)? else {
            return Ok(false);
        };
        let PreparedSingleFileCompaction { plan, publication } = prepared;
        let validation = SingleFileCompactionValidation {
            base_revision,
            seeded_prepares,
            expected_durable_revision,
            expected_next_lsn: plan.next_lsn,
            expected_journal_len: plan.journal_len,
        };
        let (relocated, publication) =
            self.relocate_compaction_image(&plan, validation, publication, io)?;
        self.root = relocated;

        let sealed_len = relocated.journal_end - relocated.journal_offset;
        let (replacement, reopened_scan) = recover_compaction_wal_with_io(
            WalRegionRecovery {
                path: self.path.clone(),
                file: io
                    .open_read_write(SingleFileCompactionIoStep::JournalReopenOpen, &self.path)
                    .map_err(DurabilityError::Io)?,
                start_offset: relocated.journal_offset,
                end_offset: relocated.journal_end,
                base_revision,
                first_lsn: relocated.journal_first_lsn,
                seeded_prepares,
                writable: false,
                crypto: self.crypto.clone(),
            },
            SingleFileCompactionIoStep::JournalReopenWalRead,
            SingleFileCompactionIoStep::JournalReopenWalSeek,
            io,
        )?;
        if !matches!(reopened_scan.tail_status(), TailStatus::Clean)
            || u64::try_from(reopened_scan.last_good_offset())
                .map_err(|_| DurabilityError::PayloadTooLarge)?
                != sealed_len
        {
            return Err(corruption(
                "single-file compacted sealed journal is not a complete WAL prefix",
            ));
        }
        let publication =
            publication.advance_after(|step| self.reclaim_sealed_journal_tail_with_io(io, step))?;
        let next = RootRecord {
            slot: self.root.slot.other(),
            sequence: self
                .root
                .sequence
                .checked_add(1)
                .ok_or(DurabilityError::LsnExhausted)?,
            journal_end: 0,
            journal_next_lsn: 0,
            ..self.root
        };
        let publication = publication.advance_after(|step| {
            write_root_slot_with_compaction_io(&mut self.file, next, io, step)
        })?;
        let _publication = publication
            .advance_after(|step| io.sync_all(step, &self.file).map_err(DurabilityError::Io))?;
        self.root = next;
        if reopened_scan.durable_revision() != expected_durable_revision
            || reopened_scan.next_lsn() != plan.next_lsn
            || !matches!(reopened_scan.tail_status(), TailStatus::Clean)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "compacted single-file journal did not reopen at certified endpoint",
            });
        }
        *wal = replacement;
        Ok(true)
    }

    fn prepare_compaction_plan<'a, F>(
        &mut self,
        wal: &mut FileRevisionWal,
        fault: &'a mut F,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<Option<PreparedSingleFileCompaction<'a, F>>, DurabilityError>
    where
        F: FnMut(SingleFileCompactionIoStep) -> Result<(), DurabilityError>,
    {
        if wal.path() != self.path || wal.start_offset() != self.root.journal_offset {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file WAL does not belong to the active journal",
            });
        }
        if self.root.journal_end != 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file compaction requires an open active journal",
            });
        }

        let active_end = io
            .metadata_len(SingleFileCompactionIoStep::SourceLength, &self.file)
            .map_err(DurabilityError::Io)?;
        let journal_len = active_end
            .checked_sub(self.root.journal_offset)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let authority_chain = if let Some(authority_root) = self.root.replication_authority {
            let path = self.path.clone();
            let crypto = self.crypto.clone();
            let mut reader = SingleFileCompactionReader {
                io,
                file: &mut self.file,
                read_step: SingleFileCompactionIoStep::SourceValidationRead,
                seek_step: SingleFileCompactionIoStep::SourceValidationSeek,
            };
            let index = recover_locator_chain(&mut reader, authority_root)?;
            validate_replication_authority_objects(&mut reader, &path, self.root, crypto.as_ref())?;
            index
                .reachable_chain()?
                .into_iter()
                .map(|(id, parent, extent)| ReplicationAuthorityCompactionEntry {
                    id,
                    parent,
                    extent,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let authority_len = authority_chain.iter().try_fold(0_u64, |total, entry| {
            total
                .checked_add(entry.extent.len)
                .and_then(|total| total.checked_add(locator_stored_len()))
                .ok_or(DurabilityError::PayloadTooLarge)
        })?;
        let compacted_generation_offset = align_up(
            DATA_OFFSET
                .checked_add(authority_len)
                .ok_or(DurabilityError::PayloadTooLarge)?,
            PAGE_SIZE,
        )?;
        let compacted_end = compacted_generation_offset
            .checked_add(self.root.generation_len)
            .and_then(|end| end.checked_add(journal_len))
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if compacted_end >= active_end {
            return Ok(None);
        }

        let next_lsn = wal.seal_for_generation_rotation_with(|file| {
            io.sync_data(SingleFileCompactionIoStep::SourceWalSync, file)
        })?;
        let publication = SingleFileCompactionPublication::new(fault);
        let (old_journal_end, publication) =
            self.seal_journal_boundary_with_io(next_lsn, publication, io)?;
        let old_root = self.root;
        if old_journal_end - old_root.journal_offset != journal_len {
            return Err(corruption(
                "single-file journal changed while entering compaction barrier",
            ));
        }
        Ok(Some(PreparedSingleFileCompaction {
            plan: SingleFileCompactionPlan {
                old_root,
                journal_len,
                next_lsn,
                authority_chain,
            },
            publication,
        }))
    }

    fn relocate_compaction_image<'a, F>(
        &mut self,
        plan: &SingleFileCompactionPlan,
        validation: SingleFileCompactionValidation<'_>,
        publication: SingleFileCompactionPublication<'a, F, CompactionSourceSealed>,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<
        (
            RootRecord,
            SingleFileCompactionPublication<'a, F, CompactionFinalRootDurable>,
        ),
        DurabilityError,
    >
    where
        F: FnMut(SingleFileCompactionIoStep) -> Result<(), DurabilityError>,
    {
        let stage_start = io
            .metadata_len(SingleFileCompactionIoStep::StagingLength, &self.file)
            .map_err(DurabilityError::Io)?;
        if stage_start != plan.old_root.journal_end {
            return Err(corruption(
                "single-file compaction staging must begin at sealed journal end",
            ));
        }
        let staged = self.write_compaction_image(
            plan,
            SingleFileCompactionImagePlacement {
                source_generation_offset: plan.old_root.generation_offset,
                source_journal_offset: plan.old_root.journal_offset,
                destination_start: stage_start,
                sequence: plan
                    .old_root
                    .sequence
                    .checked_add(1)
                    .ok_or(DurabilityError::LsnExhausted)?,
                slot: plan.old_root.slot.other(),
                kind: SingleFileCompactionImageKind::Staging,
            },
            io,
        )?;
        let publication = publication
            .advance_after(|step| io.sync_all(step, &self.file).map_err(DurabilityError::Io))?;
        self.validate_compaction_candidate(
            staged,
            SingleFileCompactionImageKind::Staging,
            validation,
            io,
        )?;
        let publication = publication.advance_after(|step| {
            write_root_slot_with_compaction_io(&mut self.file, staged, io, step)
        })?;
        let publication = publication
            .advance_after(|step| io.sync_all(step, &self.file).map_err(DurabilityError::Io))?;
        self.root = staged;

        let final_root = self.write_compaction_image(
            plan,
            SingleFileCompactionImagePlacement {
                source_generation_offset: staged.generation_offset,
                source_journal_offset: staged.journal_offset,
                destination_start: DATA_OFFSET,
                sequence: staged
                    .sequence
                    .checked_add(1)
                    .ok_or(DurabilityError::LsnExhausted)?,
                slot: staged.slot.other(),
                kind: SingleFileCompactionImageKind::Front,
            },
            io,
        )?;
        if final_root.journal_end > stage_start {
            return Err(corruption(
                "single-file compacted image overlaps crash-safe staging authority",
            ));
        }
        let publication = publication
            .advance_after(|step| io.sync_all(step, &self.file).map_err(DurabilityError::Io))?;
        self.validate_compaction_candidate(
            final_root,
            SingleFileCompactionImageKind::Front,
            validation,
            io,
        )?;
        let publication = publication.advance_after(|step| {
            write_root_slot_with_compaction_io(&mut self.file, final_root, io, step)
        })?;
        let publication = publication
            .advance_after(|step| io.sync_all(step, &self.file).map_err(DurabilityError::Io))?;
        self.root = final_root;
        Ok((final_root, publication))
    }

    fn write_compaction_image(
        &mut self,
        plan: &SingleFileCompactionPlan,
        placement: SingleFileCompactionImagePlacement,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<RootRecord, DurabilityError> {
        let source_authority = self.compaction_source_authority(plan, placement.kind, io)?;
        let mut source = io
            .open_read(placement.kind.source_open_op(), &self.path)
            .map_err(DurabilityError::Io)?;
        io.seek(
            placement.kind.destination_seek_op(),
            &mut self.file,
            SeekFrom::Start(placement.destination_start),
        )
        .map_err(DurabilityError::Io)?;
        let relocated_authority = self.relocate_compaction_authority(
            plan,
            &source_authority,
            &mut source,
            placement.kind,
            io,
        )?;

        let current = io
            .seek(
                placement.kind.destination_seek_op(),
                &mut self.file,
                SeekFrom::Current(0),
            )
            .map_err(DurabilityError::Io)?;
        let generation_offset = align_up(current, PAGE_SIZE)?;
        if generation_offset > current {
            let padding = usize::try_from(generation_offset - current)
                .map_err(|_| DurabilityError::PayloadTooLarge)?;
            compaction_write_all(
                io,
                placement.kind.write_op(),
                &mut self.file,
                &vec![0_u8; padding],
            )
            .map_err(DurabilityError::Io)?;
        }
        copy_exact_compaction_range(
            io,
            CopyRangeSteps {
                seek: placement.kind.seek_op(),
                read: placement.kind.read_op(),
                write: placement.kind.write_op(),
            },
            &mut source,
            placement.source_generation_offset,
            plan.old_root.generation_len,
            &mut self.file,
        )
        .map_err(|error| {
            eof_as_corruption(error, "single-file compaction source range is truncated")
        })?;

        let journal_offset = io
            .seek(
                placement.kind.destination_seek_op(),
                &mut self.file,
                SeekFrom::Current(0),
            )
            .map_err(DurabilityError::Io)?;
        copy_exact_compaction_range(
            io,
            CopyRangeSteps {
                seek: placement.kind.seek_op(),
                read: placement.kind.read_op(),
                write: placement.kind.write_op(),
            },
            &mut source,
            placement.source_journal_offset,
            plan.journal_len,
            &mut self.file,
        )
        .map_err(|error| {
            eof_as_corruption(error, "single-file compaction source range is truncated")
        })?;
        let journal_end = journal_offset
            .checked_add(plan.journal_len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if io
            .seek(
                placement.kind.destination_seek_op(),
                &mut self.file,
                SeekFrom::Current(0),
            )
            .map_err(DurabilityError::Io)?
            != journal_end
        {
            return Err(corruption("single-file compacted WAL copy was truncated"));
        }
        Ok(RootRecord {
            slot: placement.slot,
            sequence: placement.sequence,
            generation_offset,
            journal_offset,
            journal_end,
            replication_authority: relocated_authority,
            ..plan.old_root
        })
    }

    fn compaction_source_authority(
        &mut self,
        plan: &SingleFileCompactionPlan,
        kind: SingleFileCompactionImageKind,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<Vec<ReplicationAuthorityCompactionEntry>, DurabilityError> {
        if matches!(kind, SingleFileCompactionImageKind::Staging) {
            return Ok(plan.authority_chain.clone());
        }
        if plan.authority_chain.is_empty() {
            return Ok(Vec::new());
        }
        let root = self
            .root
            .replication_authority
            .ok_or_else(|| corruption("staged compaction root is missing replication authority"))?;
        let mut reader = SingleFileCompactionReader {
            io,
            file: &mut self.file,
            read_step: kind.validation_read_op(),
            seek_step: kind.validation_seek_op(),
        };
        Ok(recover_locator_chain(&mut reader, root)?
            .reachable_chain()?
            .into_iter()
            .map(|(id, parent, extent)| ReplicationAuthorityCompactionEntry { id, parent, extent })
            .collect())
    }

    fn relocate_compaction_authority(
        &mut self,
        plan: &SingleFileCompactionPlan,
        source_authority: &[ReplicationAuthorityCompactionEntry],
        source: &mut File,
        kind: SingleFileCompactionImageKind,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<Option<ReplicationAuthorityLocatorRoot>, DurabilityError> {
        let mut relocated_authority = None;
        for entry in source_authority {
            let object_offset = io
                .seek(
                    kind.destination_seek_op(),
                    &mut self.file,
                    SeekFrom::Current(0),
                )
                .map_err(DurabilityError::Io)?;
            copy_exact_compaction_range(
                io,
                CopyRangeSteps {
                    seek: kind.seek_op(),
                    read: kind.read_op(),
                    write: kind.write_op(),
                },
                source,
                entry.extent.offset,
                entry.extent.len,
                &mut self.file,
            )
            .map_err(|error| {
                eof_as_corruption(error, "single-file compaction source range is truncated")
            })?;
            let locator_offset = object_offset
                .checked_add(entry.extent.len)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            if io
                .seek(
                    kind.destination_seek_op(),
                    &mut self.file,
                    SeekFrom::Current(0),
                )
                .map_err(DurabilityError::Io)?
                != locator_offset
            {
                return Err(corruption(
                    "single-file authority relocation object length mismatch",
                ));
            }
            relocated_authority = Some(write_locator_node(
                locator_offset,
                entry.id,
                entry.parent,
                ReplicationAuthoritySegmentExtent {
                    offset: object_offset,
                    len: entry.extent.len,
                },
                relocated_authority,
                &mut |bytes| {
                    compaction_write_all(io, kind.write_op(), &mut self.file, bytes)
                        .map_err(DurabilityError::Io)
                },
            )?);
        }
        if plan.authority_chain.is_empty() != relocated_authority.is_none() {
            return Err(corruption(
                "single-file authority relocation root presence mismatch",
            ));
        }
        Ok(relocated_authority)
    }

    fn validate_compaction_candidate(
        &mut self,
        candidate: RootRecord,
        kind: SingleFileCompactionImageKind,
        validation: SingleFileCompactionValidation<'_>,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<(), DurabilityError> {
        let path = self.path.clone();
        let crypto = self.crypto.clone();
        {
            let mut reader = SingleFileCompactionReader {
                io,
                file: &mut self.file,
                read_step: kind.validation_read_op(),
                seek_step: kind.validation_seek_op(),
            };
            let digest = hash_file_range(
                &mut reader,
                candidate.generation_offset,
                candidate.generation_len,
            )?;
            if digest != candidate.generation_digest {
                return Err(corruption(
                    "single-file compacted generation digest mismatch",
                ));
            }
            validate_replication_authority_objects(&mut reader, &path, candidate, crypto.as_ref())?;
        }
        let validation_file = io
            .open_read_write(kind.validation_open_op(), &path)
            .map_err(DurabilityError::Io)?;
        let (_, scan) = recover_compaction_wal_with_io(
            WalRegionRecovery {
                path,
                file: validation_file,
                start_offset: candidate.journal_offset,
                end_offset: candidate.journal_end,
                base_revision: validation.base_revision,
                first_lsn: candidate.journal_first_lsn,
                seeded_prepares: validation.seeded_prepares,
                writable: false,
                crypto: self.crypto.clone(),
            },
            kind.wal_read_op(),
            kind.wal_seek_op(),
            io,
        )?;
        let logical_len =
            u64::try_from(scan.last_good_offset()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        if scan.durable_revision() != validation.expected_durable_revision
            || scan.next_lsn() != validation.expected_next_lsn
            || !matches!(scan.tail_status(), TailStatus::Clean)
            || logical_len != validation.expected_journal_len
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "compacted single-file WAL does not certify the active durable endpoint",
            });
        }
        Ok(())
    }

    fn seal_journal_boundary(&mut self, next_lsn: u64) -> Result<u64, DurabilityError> {
        if next_lsn == 0 || next_lsn < self.root.journal_first_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file journal next LSN is invalid",
            });
        }
        if self.root.journal_end != 0 {
            if self.root.journal_next_lsn != next_lsn {
                return Err(corruption(
                    "single-file sealed journal LSN boundary disagrees",
                ));
            }
            return Ok(self.root.journal_end);
        }
        self.file.sync_all()?;
        let journal_end = self.file.metadata()?.len();
        if journal_end < self.root.journal_offset {
            return Err(corruption("single-file journal end precedes journal start"));
        }
        let next = rewrite_root_journal_boundary(&mut self.file, self.root, journal_end, next_lsn)?;
        self.root = next;
        Ok(journal_end)
    }

    fn seal_journal_boundary_with_io<'a, F>(
        &mut self,
        next_lsn: u64,
        publication: SingleFileCompactionPublication<'a, F, CompactionSourceUnsealed>,
        io: &mut impl SingleFileCompactionIo,
    ) -> Result<
        (
            u64,
            SingleFileCompactionPublication<'a, F, CompactionSourceSealed>,
        ),
        DurabilityError,
    >
    where
        F: FnMut(SingleFileCompactionIoStep) -> Result<(), DurabilityError>,
    {
        if next_lsn == 0 || next_lsn < self.root.journal_first_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file journal next LSN is invalid",
            });
        }
        if self.root.journal_end != 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file compaction seal requires an open journal",
            });
        }
        io.sync_all(SingleFileCompactionIoStep::SourceContainerSync, &self.file)
            .map_err(DurabilityError::Io)?;
        let journal_end = io
            .metadata_len(SingleFileCompactionIoStep::SourceLength, &self.file)
            .map_err(DurabilityError::Io)?;
        if journal_end < self.root.journal_offset {
            return Err(corruption("single-file journal end precedes journal start"));
        }
        let sequence = self
            .root
            .sequence
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?;
        let next = RootRecord {
            slot: self.root.slot.other(),
            sequence,
            journal_end,
            journal_next_lsn: next_lsn,
            ..self.root
        };
        let publication = publication.advance_after(|step| {
            write_root_slot_with_compaction_io(&mut self.file, next, io, step)
        })?;
        let publication = publication
            .advance_after(|step| io.sync_all(step, &self.file).map_err(DurabilityError::Io))?;
        self.root = next;
        Ok((journal_end, publication))
    }

    fn reopen_sealed_journal(&mut self) -> Result<(), DurabilityError> {
        self.reclaim_sealed_journal_tail()?;
        self.publish_open_journal_root()
    }

    fn reclaim_sealed_journal_tail(&mut self) -> Result<(), DurabilityError> {
        let journal_end = self.root.journal_end;
        if journal_end < self.root.journal_offset {
            return Err(corruption("single-file sealed journal range is invalid"));
        }
        if self.file.metadata()?.len() > journal_end {
            self.file.set_len(journal_end)?;
            self.file.sync_all()?;
        }
        Ok(())
    }

    fn reclaim_sealed_journal_tail_with_io(
        &mut self,
        io: &mut impl SingleFileCompactionIo,
        sync_step: SingleFileCompactionIoStep,
    ) -> Result<(), DurabilityError> {
        let journal_end = self.root.journal_end;
        if journal_end < self.root.journal_offset {
            return Err(corruption("single-file sealed journal range is invalid"));
        }
        if io
            .metadata_len(SingleFileCompactionIoStep::TailLength, &self.file)
            .map_err(DurabilityError::Io)?
            > journal_end
        {
            io.set_len(
                SingleFileCompactionIoStep::TailTruncate,
                &self.file,
                journal_end,
            )?;
            io.sync_all(sync_step, &self.file)?;
        }
        Ok(())
    }

    fn publish_open_journal_root(&mut self) -> Result<(), DurabilityError> {
        let next = rewrite_root_journal_boundary(&mut self.file, self.root, 0, 0)?;
        self.root = next;
        Ok(())
    }
}

fn prepare_encryption_header(
    encryption: &StorageEncryption,
) -> Result<(SingleFileEncryptionHeader, OpenedEncryption), DurabilityError> {
    match encryption {
        StorageEncryption::None => Ok((
            SingleFileEncryptionHeader {
                algorithm: None,
                key_mode: SingleFileKeyMode::None,
                database_salt: [0_u8; 32],
                key_commitment: [0_u8; 16],
                wrapped_slots: [None, None],
            },
            OpenedEncryption {
                crypto: None,
                master_key: None,
                wrapped_state: None,
            },
        )),
        StorageEncryption::Direct { algorithm, key } => {
            let database_salt = random_database_salt()?;
            let crypto = StorageAeadCodec::with_salt(*algorithm, key, &database_salt)?;
            Ok((
                SingleFileEncryptionHeader {
                    algorithm: Some(crypto.algorithm()),
                    key_mode: SingleFileKeyMode::Direct,
                    database_salt,
                    key_commitment: crypto.key_commitment(),
                    wrapped_slots: [None, None],
                },
                OpenedEncryption {
                    crypto: Some(crypto),
                    master_key: Some(key.clone()),
                    wrapped_state: None,
                },
            ))
        }
        StorageEncryption::Wrapped {
            algorithm,
            wrapping_key,
            provider_key_id,
            provider_key_epoch,
            minimum_database_key_epoch,
        } => {
            if *minimum_database_key_epoch == 0 || *minimum_database_key_epoch > 1 {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "new wrapped-key database must admit database-key epoch 1",
                });
            }
            let database_salt = random_database_salt()?;
            let master_key = random_database_master_key()?;
            let crypto = StorageAeadCodec::with_salt(*algorithm, &master_key, &database_salt)?;
            let slot = WrappedKeySlot {
                publication_sequence: 1,
                key_epoch: 1,
                provider_key_epoch: *provider_key_epoch,
                provider_key_id: *provider_key_id,
                wrapped: wrap_database_master_key(
                    wrapping_key,
                    &database_salt,
                    provider_key_id,
                    *provider_key_epoch,
                    1,
                    1,
                    &master_key,
                )?,
            };
            Ok((
                SingleFileEncryptionHeader {
                    algorithm: Some(crypto.algorithm()),
                    key_mode: SingleFileKeyMode::Wrapped,
                    database_salt,
                    key_commitment: crypto.key_commitment(),
                    wrapped_slots: [Some(slot), None],
                },
                OpenedEncryption {
                    crypto: Some(crypto),
                    master_key: Some(master_key),
                    wrapped_state: Some(WrappedKeyState {
                        active_slot: 0,
                        slot,
                    }),
                },
            ))
        }
    }
}

fn random_database_salt() -> Result<[u8; 32], DurabilityError> {
    let mut database_salt = [0_u8; 32];
    getrandom::fill(&mut database_salt).map_err(|_| DurabilityError::Protocol {
        offset: 0,
        reason: "OS CSPRNG failed while creating encrypted database",
    })?;
    Ok(database_salt)
}

fn open_encryption_codec(
    encryption: &StorageEncryption,
    header: &SingleFileEncryptionHeader,
) -> Result<OpenedEncryption, DurabilityError> {
    match header.key_mode {
        SingleFileKeyMode::None => open_unencrypted(encryption),
        SingleFileKeyMode::Direct => open_direct_encryption(encryption, header),
        SingleFileKeyMode::Wrapped => open_wrapped_encryption(encryption, header),
    }
}

fn open_unencrypted(encryption: &StorageEncryption) -> Result<OpenedEncryption, DurabilityError> {
    if !matches!(encryption, StorageEncryption::None) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "encryption key supplied for an unencrypted single-file database",
        });
    }
    Ok(OpenedEncryption {
        crypto: None,
        master_key: None,
        wrapped_state: None,
    })
}

fn open_direct_encryption(
    encryption: &StorageEncryption,
    header: &SingleFileEncryptionHeader,
) -> Result<OpenedEncryption, DurabilityError> {
    let (algorithm, key) = match encryption {
        StorageEncryption::Direct { algorithm, key } => (*algorithm, key.clone()),
        StorageEncryption::None => {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "encrypted single-file database requires an encryption key",
            });
        }
        StorageEncryption::Wrapped { .. } => {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "direct-key database cannot be opened as a wrapped-key store",
            });
        }
    };
    require_header_algorithm(header, algorithm)?;
    let crypto = StorageAeadCodec::with_salt(algorithm, &key, &header.database_salt)?;
    validate_key_commitment(&crypto, header.key_commitment)?;
    Ok(OpenedEncryption {
        crypto: Some(crypto),
        master_key: Some(key),
        wrapped_state: None,
    })
}

fn open_wrapped_encryption(
    encryption: &StorageEncryption,
    header: &SingleFileEncryptionHeader,
) -> Result<OpenedEncryption, DurabilityError> {
    let config = wrapped_encryption_config(
        encryption,
        "wrapped-key database requires a provider wrapping key",
    )?;
    require_header_algorithm(header, config.algorithm)?;
    if config.minimum_database_key_epoch == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "provider database-key epoch floor must be non-zero",
        });
    }
    let Some((active_slot, slot)) = newest_matching_wrapped_key_slot(
        header.wrapped_slots,
        &config.provider_key_id,
        config.provider_key_epoch,
        config.minimum_database_key_epoch,
    )?
    else {
        let matching_below_floor = header.wrapped_slots.iter().flatten().any(|slot| {
            slot.provider_key_id == config.provider_key_id
                && slot.provider_key_epoch == config.provider_key_epoch
                && slot.key_epoch < config.minimum_database_key_epoch
        });
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: if matching_below_floor {
                "wrapped database key was rolled back below provider authority floor"
            } else {
                "provider key identity/epoch does not match any admissible wrapped database key"
            },
        });
    };
    let master_key = unwrap_database_master_key(
        config.wrapping_key,
        &header.database_salt,
        &config.provider_key_id,
        config.provider_key_epoch,
        slot.key_epoch,
        slot.publication_sequence,
        &slot.wrapped,
    )?;
    let crypto = StorageAeadCodec::with_salt(config.algorithm, &master_key, &header.database_salt)?;
    validate_key_commitment(&crypto, header.key_commitment)?;
    Ok(OpenedEncryption {
        crypto: Some(crypto),
        master_key: Some(master_key),
        wrapped_state: Some(WrappedKeyState { active_slot, slot }),
    })
}

fn require_header_algorithm(
    header: &SingleFileEncryptionHeader,
    algorithm: StorageAeadAlgorithm,
) -> Result<(), DurabilityError> {
    if header.algorithm != Some(algorithm) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "configured storage AEAD algorithm does not match database header",
        });
    }
    Ok(())
}

struct WrappedEncryptionConfig<'a> {
    algorithm: StorageAeadAlgorithm,
    wrapping_key: &'a StorageEncryptionKey,
    provider_key_id: [u8; 16],
    provider_key_epoch: u64,
    minimum_database_key_epoch: u64,
}

fn wrapped_encryption_config<'a>(
    encryption: &'a StorageEncryption,
    reason: &'static str,
) -> Result<WrappedEncryptionConfig<'a>, DurabilityError> {
    let StorageEncryption::Wrapped {
        algorithm,
        wrapping_key,
        provider_key_id,
        provider_key_epoch,
        minimum_database_key_epoch,
    } = encryption
    else {
        return Err(DurabilityError::Protocol { offset: 0, reason });
    };
    Ok(WrappedEncryptionConfig {
        algorithm: *algorithm,
        wrapping_key,
        provider_key_id: *provider_key_id,
        provider_key_epoch: *provider_key_epoch,
        minimum_database_key_epoch: *minimum_database_key_epoch,
    })
}

fn validate_key_commitment(
    crypto: &StorageAeadCodec,
    expected: [u8; 16],
) -> Result<(), DurabilityError> {
    if crypto.key_commitment() != expected {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "database encryption key does not match encrypted store",
        });
    }
    Ok(())
}

fn recover_pending_wrapped_key_handoff(
    state: &WrappedKeyState,
    header: &SingleFileEncryptionHeader,
    provider_key_id: &[u8; 16],
    provider_key_epoch: u64,
    minimum_database_key_epoch: u64,
) -> Result<Option<(usize, WrappedKeySlot)>, DurabilityError> {
    let Some((pending_slot, pending)) = newest_matching_wrapped_key_slot(
        header.wrapped_slots,
        provider_key_id,
        provider_key_epoch,
        minimum_database_key_epoch,
    )?
    else {
        return Ok(None);
    };
    if pending.publication_sequence <= state.slot.publication_sequence {
        return Ok(None);
    }
    let expected_sequence =
        state
            .slot
            .publication_sequence
            .checked_add(1)
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "wrapped-key publication sequence exhausted",
            })?;
    let expected_epoch = state
        .slot
        .key_epoch
        .checked_add(1)
        .ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "database encryption key epoch exhausted",
        })?;
    if pending.publication_sequence != expected_sequence || pending.key_epoch != expected_epoch {
        return Err(corruption(
            "pending wrapped-key handoff is not consecutive with active authority",
        ));
    }
    Ok(Some((pending_slot, pending)))
}

fn newest_matching_wrapped_key_slot(
    slots: [Option<WrappedKeySlot>; 2],
    provider_key_id: &[u8; 16],
    provider_key_epoch: u64,
    minimum_database_key_epoch: u64,
) -> Result<Option<(usize, WrappedKeySlot)>, DurabilityError> {
    let mut best: Option<(usize, WrappedKeySlot)> = None;
    for (index, slot) in slots.into_iter().enumerate() {
        let Some(slot) = slot else {
            continue;
        };
        if slot.provider_key_id != *provider_key_id
            || slot.provider_key_epoch != provider_key_epoch
            || slot.key_epoch < minimum_database_key_epoch
        {
            continue;
        }
        match best {
            None => best = Some((index, slot)),
            Some((_, current)) if slot.publication_sequence > current.publication_sequence => {
                best = Some((index, slot));
            }
            Some((_, current)) if slot.publication_sequence < current.publication_sequence => {}
            Some((_, current)) if slot == current => {}
            Some(_) => {
                return Err(corruption(
                    "matching wrapped-key slots have conflicting publication sequence",
                ));
            }
        }
    }
    Ok(best)
}

fn write_header(
    file: &mut File,
    encryption: &SingleFileEncryptionHeader,
) -> Result<(), DurabilityError> {
    let mut page = [0_u8; PAGE_SIZE_USIZE];
    page[0..8].copy_from_slice(&HEADER_MAGIC);
    put_u16(&mut page[8..10], FORMAT_VERSION);
    put_u16(&mut page[10..12], 0);
    put_u32(&mut page[12..16], PAGE_SIZE_U32);
    put_u64(&mut page[16..24], ROOT_A_OFFSET);
    put_u64(&mut page[24..32], ROOT_B_OFFSET);
    put_u64(&mut page[32..40], DATA_OFFSET);
    page[40] = encryption.algorithm.map_or(0, |algorithm| algorithm as u8);
    page[41] = match encryption.key_mode {
        SingleFileKeyMode::None => 0,
        SingleFileKeyMode::Direct => 1,
        SingleFileKeyMode::Wrapped => 2,
    };
    put_u16(&mut page[42..44], 0);
    page[44..76].copy_from_slice(&encryption.database_salt);
    page[76..92].copy_from_slice(&encryption.key_commitment);
    put_u32(&mut page[92..96], 0);
    let digest = sha256(&page[..HEADER_DIGEST_OFFSET]);
    page[HEADER_DIGEST_OFFSET..HEADER_DIGEST_OFFSET + 32].copy_from_slice(&digest);
    for (index, slot) in encryption.wrapped_slots.iter().copied().enumerate() {
        if let Some(slot) = slot {
            encode_wrapped_key_slot(&mut page[key_slot_range(index)], slot)?;
        }
    }
    file.seek(SeekFrom::Start(HEADER_OFFSET))?;
    file.write_all(&page)?;
    Ok(())
}

fn read_and_validate_header(
    file: &mut File,
) -> Result<SingleFileEncryptionHeader, DurabilityError> {
    let mut page = [0_u8; PAGE_SIZE_USIZE];
    file.seek(SeekFrom::Start(HEADER_OFFSET))?;
    file.read_exact(&mut page)
        .map_err(|error| eof_as_corruption(error, "single-file header is truncated"))?;
    if page[0..8] != HEADER_MAGIC {
        return Err(corruption("single-file header magic mismatch"));
    }
    if get_u16(&page[8..10]) != FORMAT_VERSION {
        return Err(corruption("single-file format version is unsupported"));
    }
    if get_u16(&page[10..12]) != 0
        || get_u32(&page[12..16]) != PAGE_SIZE_U32
        || get_u64(&page[16..24]) != ROOT_A_OFFSET
        || get_u64(&page[24..32]) != ROOT_B_OFFSET
        || get_u64(&page[32..40]) != DATA_OFFSET
        || get_u16(&page[42..44]) != 0
        || get_u32(&page[92..96]) != 0
    {
        return Err(corruption("single-file header layout mismatch"));
    }
    if sha256(&page[..HEADER_DIGEST_OFFSET])
        != page[HEADER_DIGEST_OFFSET..HEADER_DIGEST_OFFSET + 32]
    {
        return Err(corruption("single-file header digest mismatch"));
    }
    let algorithm = match page[40] {
        0 => None,
        raw => Some(StorageAeadAlgorithm::decode(raw)?),
    };
    let key_mode = match page[41] {
        0 => SingleFileKeyMode::None,
        1 => SingleFileKeyMode::Direct,
        2 => SingleFileKeyMode::Wrapped,
        _ => return Err(corruption("single-file key mode is unsupported")),
    };
    let mut database_salt = [0_u8; 32];
    database_salt.copy_from_slice(&page[44..76]);
    let mut key_commitment = [0_u8; 16];
    key_commitment.copy_from_slice(&page[76..92]);
    let key_slot_bytes_nonzero = page[KEY_SLOT_A_OFFSET..KEY_SLOTS_END]
        .iter()
        .any(|byte| *byte != 0);
    if page[KEY_SLOTS_END..].iter().any(|byte| *byte != 0) {
        return Err(corruption("single-file header reserved bytes are non-zero"));
    }
    let wrapped_slots = [
        decode_wrapped_key_slot(&page[key_slot_range(0)])?,
        decode_wrapped_key_slot(&page[key_slot_range(1)])?,
    ];
    match key_mode {
        SingleFileKeyMode::None => {
            if algorithm.is_some()
                || database_salt.iter().any(|byte| *byte != 0)
                || key_commitment.iter().any(|byte| *byte != 0)
                || key_slot_bytes_nonzero
            {
                return Err(corruption(
                    "unencrypted single-file header carries encryption material",
                ));
            }
        }
        SingleFileKeyMode::Direct => {
            if algorithm.is_none() || key_slot_bytes_nonzero {
                return Err(corruption("direct-key single-file header is inconsistent"));
            }
        }
        SingleFileKeyMode::Wrapped => {
            if algorithm.is_none() || wrapped_slots.iter().all(Option::is_none) {
                return Err(corruption("wrapped-key single-file header is inconsistent"));
            }
        }
    }
    Ok(SingleFileEncryptionHeader {
        algorithm,
        key_mode,
        database_salt,
        key_commitment,
        wrapped_slots,
    })
}

fn key_slot_range(index: usize) -> std::ops::Range<usize> {
    let start = match index {
        0 => KEY_SLOT_A_OFFSET,
        1 => KEY_SLOT_B_OFFSET,
        _ => unreachable!("wrapped-key slot index is bounded"),
    };
    start..start + KEY_SLOT_LEN
}

fn encode_wrapped_key_slot(bytes: &mut [u8], slot: WrappedKeySlot) -> Result<(), DurabilityError> {
    if bytes.len() != KEY_SLOT_LEN {
        return Err(corruption("wrapped-key slot has invalid length"));
    }
    bytes.fill(0);
    bytes[0..4].copy_from_slice(&KEY_SLOT_MAGIC);
    bytes[4] = KEY_SLOT_VERSION;
    bytes[5] = 1;
    bytes[6] = StorageAeadAlgorithm::Aes256GcmSiv as u8;
    bytes[7] = 0;
    put_u64(&mut bytes[8..16], slot.publication_sequence);
    put_u64(&mut bytes[16..24], slot.key_epoch);
    put_u64(&mut bytes[24..32], slot.provider_key_epoch);
    bytes[32..48].copy_from_slice(&slot.provider_key_id);
    bytes[48..60].copy_from_slice(&slot.wrapped.nonce);
    bytes[60..108].copy_from_slice(&slot.wrapped.ciphertext_and_tag);
    let digest = sha256(&bytes[..KEY_SLOT_DIGEST_OFFSET]);
    bytes[KEY_SLOT_DIGEST_OFFSET..KEY_SLOT_DIGEST_OFFSET + 32].copy_from_slice(&digest);
    Ok(())
}

fn decode_wrapped_key_slot(bytes: &[u8]) -> Result<Option<WrappedKeySlot>, DurabilityError> {
    if bytes.len() != KEY_SLOT_LEN {
        return Err(corruption("wrapped-key slot has invalid length"));
    }
    if bytes.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if bytes[0..4] != KEY_SLOT_MAGIC
        || bytes[4] != KEY_SLOT_VERSION
        || bytes[5] != 1
        || bytes[6] != StorageAeadAlgorithm::Aes256GcmSiv as u8
        || bytes[7] != 0
        || bytes[108..KEY_SLOT_DIGEST_OFFSET]
            .iter()
            .any(|byte| *byte != 0)
    {
        return Ok(None);
    }
    if sha256(&bytes[..KEY_SLOT_DIGEST_OFFSET])
        != bytes[KEY_SLOT_DIGEST_OFFSET..KEY_SLOT_DIGEST_OFFSET + 32]
    {
        return Ok(None);
    }
    let publication_sequence = get_u64(&bytes[8..16]);
    let key_epoch = get_u64(&bytes[16..24]);
    let provider_key_epoch = get_u64(&bytes[24..32]);
    if publication_sequence == 0 || key_epoch == 0 {
        return Ok(None);
    }
    let mut provider_key_id = [0_u8; 16];
    provider_key_id.copy_from_slice(&bytes[32..48]);
    let mut nonce = [0_u8; 12];
    nonce.copy_from_slice(&bytes[48..60]);
    let mut ciphertext_and_tag = [0_u8; 48];
    ciphertext_and_tag.copy_from_slice(&bytes[60..108]);
    Ok(Some(WrappedKeySlot {
        publication_sequence,
        key_epoch,
        provider_key_epoch,
        provider_key_id,
        wrapped: WrappedDatabaseMasterKey {
            nonce,
            ciphertext_and_tag,
        },
    }))
}

fn write_wrapped_key_slot(
    file: &mut File,
    index: usize,
    slot: WrappedKeySlot,
) -> Result<(), DurabilityError> {
    let mut bytes = [0_u8; KEY_SLOT_LEN];
    encode_wrapped_key_slot(&mut bytes, slot)?;
    let offset =
        u64::try_from(key_slot_range(index).start).map_err(|_| DurabilityError::PayloadTooLarge)?;
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(&bytes)?;
    Ok(())
}

fn stored_section_len(
    plaintext_len: u64,
    crypto: Option<&StorageAeadCodec>,
) -> Result<u64, DurabilityError> {
    let Some(_) = crypto else {
        return Ok(plaintext_len);
    };
    let chunk_count = encrypted_section_chunk_count(plaintext_len)?;
    let envelope_overhead = u64::try_from(StorageAeadCodec::sealed_len(0)?)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    u64::try_from(ENCRYPTED_SECTION_HEADER_LEN)
        .map_err(|_| DurabilityError::PayloadTooLarge)?
        .checked_add(plaintext_len)
        .and_then(|len| len.checked_add(envelope_overhead.checked_mul(u64::from(chunk_count))?))
        .ok_or(DurabilityError::PayloadTooLarge)
}

fn write_stored_section(
    file: &mut File,
    generation_hasher: &mut Sha256,
    crypto: Option<&StorageAeadCodec>,
    nonce_sequence: Option<&mut StorageNonceSequence>,
    generation: u64,
    plaintext_len: u64,
    section: &SingleFileSectionInput<'_>,
) -> Result<[u8; 32], DurabilityError> {
    let mut digest = Sha256::new();
    let Some(crypto) = crypto else {
        let mut written = 0_u64;
        section.content.write_to(&mut |bytes| {
            let len = u64::try_from(bytes.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
            written = written
                .checked_add(len)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            write_dual_hashed(file, &mut digest, generation_hasher, bytes)
        })?;
        if written != plaintext_len {
            return Err(corruption(
                "single-file section source length changed while writing",
            ));
        }
        return Ok(digest.finalize().into());
    };
    let nonce_sequence = nonce_sequence.ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "encrypted section writer is missing its nonce sequence",
    })?;
    let chunk_count = encrypted_section_chunk_count(plaintext_len)?;
    let mut header = [0_u8; ENCRYPTED_SECTION_HEADER_LEN];
    header[0..4].copy_from_slice(&ENCRYPTED_SECTION_MAGIC);
    header[4] = ENCRYPTED_SECTION_VERSION;
    header[5] = crypto.algorithm() as u8;
    put_u32(
        &mut header[8..12],
        u32::try_from(ENCRYPTED_SECTION_CHUNK_SIZE)
            .map_err(|_| DurabilityError::PayloadTooLarge)?,
    );
    put_u64(&mut header[12..20], plaintext_len);
    put_u32(&mut header[20..24], chunk_count);
    write_dual_hashed(file, &mut digest, generation_hasher, &header)?;

    let mut chunk_index = 0_u32;
    let mut written_plaintext = 0_u64;
    let mut chunk_buffer = Vec::with_capacity(ENCRYPTED_SECTION_CHUNK_SIZE);
    let mut emit_chunk = |chunk: &[u8]| -> Result<(), DurabilityError> {
        let chunk_len = u32::try_from(chunk.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        let context = section_chunk_aad_context(
            generation,
            section.kind,
            section.ordinal,
            chunk_index,
            plaintext_len,
            chunk_len,
        );
        let envelope = crypto.seal(
            StorageEncryptionDomain::Section,
            nonce_sequence.next_nonce()?,
            &context,
            chunk,
        )?;
        write_dual_hashed(file, &mut digest, generation_hasher, &envelope)?;
        chunk_index = chunk_index
            .checked_add(1)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        Ok(())
    };

    section.content.write_to(&mut |mut bytes| {
        let len = u64::try_from(bytes.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        written_plaintext = written_plaintext
            .checked_add(len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        while !bytes.is_empty() {
            let remaining = ENCRYPTED_SECTION_CHUNK_SIZE - chunk_buffer.len();
            let take = remaining.min(bytes.len());
            chunk_buffer.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if chunk_buffer.len() == ENCRYPTED_SECTION_CHUNK_SIZE {
                emit_chunk(&chunk_buffer)?;
                chunk_buffer.clear();
            }
        }
        Ok(())
    })?;

    if written_plaintext != plaintext_len {
        return Err(corruption(
            "single-file section source length changed while writing",
        ));
    }
    if plaintext_len == 0 {
        emit_chunk(&[])?;
    } else if !chunk_buffer.is_empty() {
        emit_chunk(&chunk_buffer)?;
    }
    if chunk_index != chunk_count {
        return Err(corruption(
            "single-file section source produced an unexpected chunk count",
        ));
    }
    Ok(digest.finalize().into())
}

fn encrypted_section_chunk_count(plaintext_len: u64) -> Result<u32, DurabilityError> {
    if plaintext_len == 0 {
        return Ok(1);
    }
    let chunk_size = ENCRYPTED_SECTION_CHUNK_SIZE as u64;
    let count = plaintext_len
        .checked_add(chunk_size - 1)
        .ok_or(DurabilityError::PayloadTooLarge)?
        / chunk_size;
    u32::try_from(count).map_err(|_| DurabilityError::PayloadTooLarge)
}

fn section_aad_context(generation: u64, kind: SingleFileSectionKind, ordinal: u32) -> [u8; 14] {
    let mut context = [0_u8; 14];
    context[..8].copy_from_slice(&generation.to_le_bytes());
    context[8..10].copy_from_slice(&(kind as u16).to_le_bytes());
    context[10..14].copy_from_slice(&ordinal.to_le_bytes());
    context
}

fn section_chunk_aad_context(
    generation: u64,
    kind: SingleFileSectionKind,
    ordinal: u32,
    chunk_index: u32,
    plaintext_len: u64,
    chunk_len: u32,
) -> [u8; 30] {
    let mut context = [0_u8; 30];
    context[..14].copy_from_slice(&section_aad_context(generation, kind, ordinal));
    context[14..18].copy_from_slice(&chunk_index.to_le_bytes());
    context[18..26].copy_from_slice(&plaintext_len.to_le_bytes());
    context[26..30].copy_from_slice(&chunk_len.to_le_bytes());
    context
}

fn publish_generation_to_file(
    file: &mut File,
    current: Option<RootRecord>,
    generation: u64,
    journal_first_lsn: u64,
    sections: &[SingleFileSectionInput<'_>],
    replication_authority: Option<ReplicationAuthorityLocatorRoot>,
    crypto: Option<&StorageAeadCodec>,
) -> Result<RootRecord, DurabilityError> {
    if journal_first_lsn == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "single-file journal first LSN must be nonzero",
        });
    }
    validate_section_inputs(sections)?;
    let sequence = current.map_or(Ok(1), |root| {
        root.sequence
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)
    })?;
    let parent_digest = current.map_or([0_u8; 32], |root| root.generation_digest);
    let file_len = file.metadata()?.len();
    let generation_offset = align_up(file_len.max(DATA_OFFSET), PAGE_SIZE)?;
    if generation_offset > file_len {
        file.set_len(generation_offset)?;
    }
    let mut layout = generation_layout(
        generation,
        parent_digest,
        generation_offset,
        sections,
        crypto,
    )?;
    let generation_digest =
        write_generation_streaming(file, generation_offset, &mut layout, sections, crypto)?;
    file.sync_all()?;

    let slot = current.map_or(RootSlot::A, |root| root.slot.other());
    let root = RootRecord {
        slot,
        sequence,
        generation,
        generation_offset,
        generation_len: layout.total_len,
        journal_offset: generation_offset
            .checked_add(layout.total_len)
            .ok_or(DurabilityError::PayloadTooLarge)?,
        journal_end: 0,
        journal_first_lsn,
        journal_next_lsn: 0,
        section_count: u32::try_from(sections.len())
            .map_err(|_| DurabilityError::PayloadTooLarge)?,
        generation_digest,
        parent_digest,
        replication_authority,
    };
    write_root_slot(file, root)?;
    file.sync_all()?;
    Ok(root)
}

fn append_generation_without_root(
    file: &mut File,
    current: RootRecord,
    generation: u64,
    sections: &[SingleFileSectionInput<'_>],
    crypto: Option<&StorageAeadCodec>,
) -> Result<(u64, u64, [u8; 32], u32), DurabilityError> {
    validate_section_inputs(sections)?;
    let file_len = file.metadata()?.len();
    let generation_offset = align_up(file_len.max(DATA_OFFSET), PAGE_SIZE)?;
    if generation_offset > file_len {
        file.set_len(generation_offset)?;
    }
    let mut layout = generation_layout(
        generation,
        current.generation_digest,
        generation_offset,
        sections,
        crypto,
    )?;
    let generation_digest =
        write_generation_streaming(file, generation_offset, &mut layout, sections, crypto)?;
    file.sync_all()?;
    Ok((
        generation_offset,
        layout.total_len,
        generation_digest,
        u32::try_from(sections.len()).map_err(|_| DurabilityError::PayloadTooLarge)?,
    ))
}

struct GenerationLayout {
    generation: u64,
    header: [u8; GENERATION_HEADER_LEN],
    descriptors: Vec<SingleFileSectionDescriptor>,
    plaintext_lens: Vec<u64>,
    data_start: u64,
    table_offset: u64,
    table_len: u64,
    total_len: u64,
}

fn generation_layout(
    generation: u64,
    parent_digest: [u8; 32],
    generation_offset: u64,
    sections: &[SingleFileSectionInput<'_>],
    crypto: Option<&StorageAeadCodec>,
) -> Result<GenerationLayout, DurabilityError> {
    let table_len = SECTION_DESCRIPTOR_LEN
        .checked_mul(sections.len())
        .ok_or(DurabilityError::PayloadTooLarge)?;
    let table_len_u64 = u64::try_from(table_len).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let data_start = align_up(GENERATION_HEADER_LEN as u64, PAGE_SIZE)?;
    let mut descriptors = Vec::with_capacity(sections.len());
    let mut plaintext_lens = Vec::with_capacity(sections.len());
    let mut cursor = data_start;
    for section in sections {
        let plaintext_len = section.content.plaintext_len()?;
        if plaintext_len > MAX_SECTION_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        plaintext_lens.push(plaintext_len);
        let len = stored_section_len(plaintext_len, crypto)?;
        if len > MAX_SECTION_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        descriptors.push(SingleFileSectionDescriptor {
            kind: section.kind,
            ordinal: section.ordinal,
            offset: generation_offset
                .checked_add(cursor)
                .ok_or(DurabilityError::PayloadTooLarge)?,
            len,
            digest: [0_u8; 32],
        });
        cursor = align_up(
            cursor
                .checked_add(len)
                .ok_or(DurabilityError::PayloadTooLarge)?,
            PAGE_SIZE,
        )?;
    }
    let table_offset = cursor;
    let total_len = align_up(
        table_offset
            .checked_add(table_len_u64)
            .ok_or(DurabilityError::PayloadTooLarge)?,
        PAGE_SIZE,
    )?;
    if total_len > MAX_GENERATION_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }

    let mut header = [0_u8; GENERATION_HEADER_LEN];
    header[0..4].copy_from_slice(&GENERATION_MAGIC);
    put_u16(&mut header[4..6], FORMAT_VERSION);
    put_u16(&mut header[6..8], 0);
    put_u64(&mut header[8..16], generation);
    header[16..48].copy_from_slice(&parent_digest);
    put_u32(
        &mut header[48..52],
        u32::try_from(sections.len()).map_err(|_| DurabilityError::PayloadTooLarge)?,
    );
    put_u32(&mut header[52..56], 0);
    put_u64(&mut header[56..64], table_len_u64);
    put_u64(&mut header[64..72], data_start);
    put_u64(&mut header[72..80], total_len);
    put_u64(&mut header[80..88], table_offset);
    let header_digest = sha256(&header[..GENERATION_HEADER_DIGEST_OFFSET]);
    header[GENERATION_HEADER_DIGEST_OFFSET..GENERATION_HEADER_DIGEST_OFFSET + 32]
        .copy_from_slice(&header_digest);

    Ok(GenerationLayout {
        generation,
        header,
        descriptors,
        plaintext_lens,
        data_start,
        table_offset,
        table_len: table_len_u64,
        total_len,
    })
}

fn write_section_table_hashed(
    file: &mut File,
    generation_hasher: &mut Sha256,
    generation_offset: u64,
    descriptors: &[SingleFileSectionDescriptor],
) -> Result<(), DurabilityError> {
    for descriptor in descriptors {
        let mut descriptor_bytes = [0_u8; SECTION_DESCRIPTOR_LEN];
        put_u16(&mut descriptor_bytes[0..2], descriptor.kind as u16);
        put_u16(&mut descriptor_bytes[2..4], 0);
        put_u32(&mut descriptor_bytes[4..8], descriptor.ordinal);
        put_u64(
            &mut descriptor_bytes[8..16],
            descriptor.offset - generation_offset,
        );
        put_u64(&mut descriptor_bytes[16..24], descriptor.len);
        descriptor_bytes[24..56].copy_from_slice(&descriptor.digest);
        write_hashed(file, generation_hasher, &descriptor_bytes)?;
    }
    Ok(())
}

fn write_generation_streaming(
    file: &mut File,
    generation_offset: u64,
    layout: &mut GenerationLayout,
    sections: &[SingleFileSectionInput<'_>],
    crypto: Option<&StorageAeadCodec>,
) -> Result<[u8; 32], DurabilityError> {
    let generation_end = generation_offset
        .checked_add(layout.total_len)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    file.set_len(generation_end)?;
    file.seek(SeekFrom::Start(generation_offset))?;
    let mut generation_hasher = Sha256::new();
    write_hashed(file, &mut generation_hasher, &layout.header)?;
    write_zeroes_hashed(
        file,
        &mut generation_hasher,
        layout.data_start - GENERATION_HEADER_LEN as u64,
    )?;
    let mut nonce_sequence = crypto.map(|_| StorageNonceSequence::random()).transpose()?;
    let mut relative_cursor = layout.data_start;
    for ((section, descriptor), plaintext_len) in sections
        .iter()
        .zip(layout.descriptors.iter_mut())
        .zip(layout.plaintext_lens.iter().copied())
    {
        let relative_offset = descriptor.offset - generation_offset;
        if relative_offset < relative_cursor {
            return Err(corruption("single-file generation section layout overlaps"));
        }
        write_zeroes_hashed(
            file,
            &mut generation_hasher,
            relative_offset - relative_cursor,
        )?;
        descriptor.digest = write_stored_section(
            file,
            &mut generation_hasher,
            crypto,
            nonce_sequence.as_mut(),
            layout.generation,
            plaintext_len,
            section,
        )?;
        let actual_end = file.stream_position()?;
        let expected_end = descriptor
            .offset
            .checked_add(descriptor.len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if actual_end != expected_end {
            return Err(corruption(
                "single-file generation section writer length mismatch",
            ));
        }
        relative_cursor = relative_offset
            .checked_add(descriptor.len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let aligned = align_up(relative_cursor, PAGE_SIZE)?;
        write_zeroes_hashed(file, &mut generation_hasher, aligned - relative_cursor)?;
        relative_cursor = aligned;
    }
    if relative_cursor != layout.table_offset {
        return Err(corruption("single-file generation writer length mismatch"));
    }
    let encoded_table_len = u64::try_from(
        SECTION_DESCRIPTOR_LEN
            .checked_mul(layout.descriptors.len())
            .ok_or(DurabilityError::PayloadTooLarge)?,
    )
    .map_err(|_| DurabilityError::PayloadTooLarge)?;
    if encoded_table_len != layout.table_len {
        return Err(corruption(
            "single-file generation section table length changed",
        ));
    }
    write_section_table_hashed(
        file,
        &mut generation_hasher,
        generation_offset,
        &layout.descriptors,
    )?;
    let after_table = layout
        .table_offset
        .checked_add(layout.table_len)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    write_zeroes_hashed(file, &mut generation_hasher, layout.total_len - after_table)?;
    Ok(generation_hasher.finalize().into())
}

fn write_hashed(file: &mut File, hasher: &mut Sha256, bytes: &[u8]) -> Result<(), DurabilityError> {
    file.write_all(bytes)?;
    hasher.update(bytes);
    Ok(())
}

fn write_dual_hashed(
    file: &mut File,
    first: &mut Sha256,
    second: &mut Sha256,
    bytes: &[u8],
) -> Result<(), DurabilityError> {
    file.write_all(bytes)?;
    first.update(bytes);
    second.update(bytes);
    Ok(())
}

fn write_zeroes_hashed(
    file: &mut File,
    hasher: &mut Sha256,
    mut len: u64,
) -> Result<(), DurabilityError> {
    let zeroes = [0_u8; PAGE_SIZE_USIZE];
    while len != 0 {
        let chunk =
            usize::try_from(len.min(PAGE_SIZE)).map_err(|_| DurabilityError::PayloadTooLarge)?;
        write_hashed(file, hasher, &zeroes[..chunk])?;
        len -= chunk as u64;
    }
    Ok(())
}

fn encode_root_page(root: RootRecord) -> [u8; PAGE_SIZE_USIZE] {
    let mut page = [0_u8; PAGE_SIZE_USIZE];
    page[0..4].copy_from_slice(&ROOT_MAGIC);
    put_u16(&mut page[4..6], FORMAT_VERSION);
    put_u16(&mut page[6..8], 0);
    put_u64(&mut page[8..16], root.sequence);
    put_u64(&mut page[16..24], root.generation);
    put_u64(&mut page[24..32], root.generation_offset);
    put_u64(&mut page[32..40], root.generation_len);
    put_u64(&mut page[40..48], root.journal_offset);
    put_u64(&mut page[48..56], root.journal_end);
    put_u64(&mut page[56..64], root.journal_first_lsn);
    put_u64(&mut page[64..72], root.journal_next_lsn);
    put_u32(&mut page[72..76], root.section_count);
    put_u32(&mut page[76..80], 0);
    page[80..112].copy_from_slice(&root.generation_digest);
    page[112..144].copy_from_slice(&root.parent_digest);
    let digest = sha256(&page[..ROOT_DIGEST_OFFSET]);
    page[ROOT_DIGEST_OFFSET..ROOT_DIGEST_OFFSET + 32].copy_from_slice(&digest);
    if let Some(authority) = root.replication_authority {
        page[ROOT_AUTHORITY_OFFSET..ROOT_AUTHORITY_OFFSET + 32]
            .copy_from_slice(&authority.segment_id.bytes());
        put_u64(
            &mut page[ROOT_AUTHORITY_OFFSET + 32..ROOT_AUTHORITY_OFFSET + 40],
            authority.offset,
        );
        page[ROOT_AUTHORITY_OFFSET + 40..ROOT_AUTHORITY_OFFSET + 72]
            .copy_from_slice(&authority.digest);
        let authority_digest = root_authority_digest(&page);
        page[ROOT_AUTHORITY_DIGEST_OFFSET..ROOT_AUTHORITY_END].copy_from_slice(&authority_digest);
    }

    page
}

fn write_root_slot(file: &mut File, root: RootRecord) -> Result<(), DurabilityError> {
    let page = encode_root_page(root);
    file.seek(SeekFrom::Start(root.slot.offset()))?;
    file.write_all(&page)?;
    Ok(())
}

fn write_root_slot_with_compaction_io(
    file: &mut File,
    root: RootRecord,
    io: &mut impl SingleFileCompactionIo,
    op: SingleFileCompactionIoStep,
) -> Result<(), DurabilityError> {
    let page = encode_root_page(root);
    file.seek(SeekFrom::Start(root.slot.offset()))?;
    compaction_write_all(io, op, file, &page).map_err(DurabilityError::Io)
}

fn root_authority_digest(page: &[u8; PAGE_SIZE_USIZE]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(ROOT_AUTHORITY_DOMAIN);
    hasher.update(&page[..ROOT_AUTHORITY_OFFSET]);
    hasher.update(&page[ROOT_AUTHORITY_OFFSET..ROOT_AUTHORITY_DIGEST_OFFSET]);
    hasher.finalize().into()
}

fn read_root_slot(file: &mut File, slot: RootSlot) -> Result<Option<RootRecord>, DurabilityError> {
    let mut page = [0_u8; PAGE_SIZE_USIZE];
    file.seek(SeekFrom::Start(slot.offset()))?;
    file.read_exact(&mut page)
        .map_err(|error| eof_as_corruption(error, "single-file root slot is truncated"))?;
    if page.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    if page[0..4] != ROOT_MAGIC
        || get_u16(&page[4..6]) != FORMAT_VERSION
        || get_u16(&page[6..8]) != 0
        || get_u32(&page[76..80]) != 0
        || page[ROOT_AUTHORITY_END..].iter().any(|byte| *byte != 0)
    {
        return Ok(None);
    }
    if sha256(&page[..ROOT_DIGEST_OFFSET]) != page[ROOT_DIGEST_OFFSET..ROOT_DIGEST_OFFSET + 32] {
        return Ok(None);
    }
    let mut generation_digest = [0_u8; 32];
    generation_digest.copy_from_slice(&page[80..112]);
    let mut parent_digest = [0_u8; 32];
    parent_digest.copy_from_slice(&page[112..144]);
    let authority_binding = &page[ROOT_AUTHORITY_OFFSET..ROOT_AUTHORITY_DIGEST_OFFSET];
    let authority_digest = &page[ROOT_AUTHORITY_DIGEST_OFFSET..ROOT_AUTHORITY_END];
    let replication_authority = if authority_binding.iter().all(|byte| *byte == 0)
        && authority_digest.iter().all(|byte| *byte == 0)
    {
        None
    } else {
        if authority_binding.iter().all(|byte| *byte == 0)
            || authority_digest.iter().all(|byte| *byte == 0)
            || root_authority_digest(&page) != authority_digest
        {
            return Ok(None);
        }
        let segment_id = ReplicationAuthoritySegmentId::from_bytes(
            page[ROOT_AUTHORITY_OFFSET..ROOT_AUTHORITY_OFFSET + 32]
                .try_into()
                .expect("32 bytes"),
        )?;
        let offset = get_u64(&page[ROOT_AUTHORITY_OFFSET + 32..ROOT_AUTHORITY_OFFSET + 40]);
        let digest: [u8; 32] = page[ROOT_AUTHORITY_OFFSET + 40..ROOT_AUTHORITY_OFFSET + 72]
            .try_into()
            .expect("32 bytes");
        if offset < DATA_OFFSET || digest == [0; 32] {
            return Ok(None);
        }
        Some(ReplicationAuthorityLocatorRoot {
            segment_id,
            offset,
            digest,
        })
    };
    let root = RootRecord {
        slot,
        sequence: get_u64(&page[8..16]),
        generation: get_u64(&page[16..24]),
        generation_offset: get_u64(&page[24..32]),
        generation_len: get_u64(&page[32..40]),
        journal_offset: get_u64(&page[40..48]),
        journal_end: get_u64(&page[48..56]),
        journal_first_lsn: get_u64(&page[56..64]),
        journal_next_lsn: get_u64(&page[64..72]),
        section_count: get_u32(&page[72..76]),
        generation_digest,
        parent_digest,
        replication_authority,
    };
    if root.sequence == 0
        || root.generation == 0
        || root.generation_offset < DATA_OFFSET
        || !root.generation_offset.is_multiple_of(PAGE_SIZE)
        || root.generation_len == 0
        || root.generation_len > MAX_GENERATION_LEN
        || root.journal_offset != root.generation_offset.saturating_add(root.generation_len)
        || (root.journal_end != 0 && root.journal_end < root.journal_offset)
        || root.journal_first_lsn == 0
        || (root.journal_end == 0 && root.journal_next_lsn != 0)
        || (root.journal_end != 0 && root.journal_next_lsn < root.journal_first_lsn)
        || usize::try_from(root.section_count).map_or(true, |count| count > MAX_SECTION_COUNT)
    {
        return Ok(None);
    }
    Ok(Some(root))
}

fn choose_authoritative_root(
    slot_a: Option<RootRecord>,
    slot_b: Option<RootRecord>,
) -> Result<RootRecord, DurabilityError> {
    match (slot_a, slot_b) {
        (None, None) => Err(corruption("single-file store has no valid root slot")),
        (Some(root), None) | (None, Some(root)) => Ok(root),
        (Some(a), Some(b)) => {
            let (older, newer) = if a.sequence < b.sequence {
                (a, b)
            } else {
                (b, a)
            };
            if older.sequence == newer.sequence {
                return Err(corruption("single-file root slots fork at equal sequence"));
            }
            if older.sequence.checked_add(1) != Some(newer.sequence)
                || !roots_form_valid_transition(older, newer)
            {
                return Err(corruption(
                    "single-file root slots do not form a consecutive chain",
                ));
            }
            Ok(newer)
        }
    }
}

#[cfg(test)]
pub(crate) fn test_corrupt_newest_root_slot(path: &Path) -> Result<(), DurabilityError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let root = choose_authoritative_root(
        read_root_slot(&mut file, RootSlot::A)?,
        read_root_slot(&mut file, RootSlot::B)?,
    )?;
    file.seek(SeekFrom::Start(root.slot.offset()))?;
    file.write_all(&[0_u8; 32])?;
    file.sync_all()?;
    Ok(())
}

fn roots_form_valid_transition(older: RootRecord, newer: RootRecord) -> bool {
    if older.generation == newer.generation {
        let same_generation = older.generation_len == newer.generation_len
            && older.journal_first_lsn == newer.journal_first_lsn
            && older.section_count == newer.section_count
            && older.generation_digest == newer.generation_digest
            && older.parent_digest == newer.parent_digest
            && older.replication_authority.map(|root| root.segment_id)
                == newer.replication_authority.map(|root| root.segment_id);
        let same_location_transition = older.generation_offset == newer.generation_offset
            && older.journal_offset == newer.journal_offset
            && ((older.journal_end == 0
                && older.journal_next_lsn == 0
                && newer.journal_end >= newer.journal_offset
                && newer.journal_next_lsn >= newer.journal_first_lsn)
                || (older.journal_end >= older.journal_offset
                    && older.journal_next_lsn >= older.journal_first_lsn
                    && newer.journal_end == 0
                    && newer.journal_next_lsn == 0));
        let relocation = older.journal_end >= older.journal_offset
            && newer.journal_end >= newer.journal_offset
            && older.journal_next_lsn == newer.journal_next_lsn
            && newer.generation_offset != older.generation_offset
            && older.journal_end - older.journal_offset == newer.journal_end - newer.journal_offset;
        return same_generation && (same_location_transition || relocation);
    }
    if older.generation.checked_add(1) != Some(newer.generation)
        || older.journal_end < older.journal_offset
        || older.journal_next_lsn < older.journal_first_lsn
        || newer.parent_digest != older.generation_digest
    {
        return false;
    }
    let ordinary_rotation = newer.journal_end == 0
        && newer.journal_next_lsn == 0
        && newer.journal_first_lsn == older.journal_next_lsn;
    let certified_carry_forward = newer.journal_end >= newer.journal_offset
        && newer.journal_next_lsn == older.journal_next_lsn
        && newer.journal_first_lsn >= older.journal_first_lsn
        && newer.journal_first_lsn <= older.journal_next_lsn;
    ordinary_rotation || certified_carry_forward
}

fn rewrite_root_journal_boundary(
    file: &mut File,
    current: RootRecord,
    journal_end: u64,
    journal_next_lsn: u64,
) -> Result<RootRecord, DurabilityError> {
    let sequence = current
        .sequence
        .checked_add(1)
        .ok_or(DurabilityError::LsnExhausted)?;
    let next = RootRecord {
        slot: current.slot.other(),
        sequence,
        journal_end,
        journal_next_lsn,
        ..current
    };
    write_root_slot(file, next)?;
    file.sync_all()?;
    Ok(next)
}

fn validate_authoritative_generation(
    file: &mut File,
    root: RootRecord,
) -> Result<(), DurabilityError> {
    let file_len = file.metadata()?.len();
    let end = root
        .generation_offset
        .checked_add(root.generation_len)
        .ok_or_else(|| corruption("single-file generation range overflow"))?;
    if end > file_len {
        return Err(corruption(
            "authoritative single-file generation is truncated",
        ));
    }
    if root.journal_end != 0 && root.journal_end > file_len {
        return Err(corruption("authoritative single-file journal is truncated"));
    }
    let digest = hash_file_range(file, root.generation_offset, root.generation_len)?;
    if digest != root.generation_digest {
        return Err(corruption(
            "authoritative single-file generation digest mismatch",
        ));
    }
    let view = read_generation_view(file, root)?;
    if view.generation != root.generation
        || view.parent_digest != root.parent_digest
        || view.sections.len() != usize::try_from(root.section_count).unwrap_or(usize::MAX)
    {
        return Err(corruption("single-file root and generation disagree"));
    }
    Ok(())
}

fn validate_replication_authority_objects<R: Read + Seek>(
    file: &mut R,
    path: &Path,
    root: RootRecord,
    crypto: Option<&StorageAeadCodec>,
) -> Result<(), DurabilityError> {
    let Some(authority_root) = root.replication_authority else {
        return Ok(());
    };
    let index = recover_locator_chain(file, authority_root)?;
    let mut journal = ReplicationAuthorityJournal::open_single_file(path, &[], &[])?;
    replay_indexed_segment_object_chain(file, &index, crypto, &mut journal)
}

fn read_generation_view(
    file: &mut File,
    root: RootRecord,
) -> Result<SingleFileGenerationView, DurabilityError> {
    let header = read_generation_header(file, root)?;
    let table_len_usize =
        usize::try_from(header.table_len).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let mut table = vec![0_u8; table_len_usize];
    file.seek(SeekFrom::Start(
        root.generation_offset
            .checked_add(header.table_offset)
            .ok_or(DurabilityError::PayloadTooLarge)?,
    ))?;
    file.read_exact(&mut table)
        .map_err(|error| eof_as_corruption(error, "single-file section table is truncated"))?;
    let sections = decode_section_table(
        &table,
        header.section_count,
        header.data_start,
        header.table_offset,
        root,
    )?;
    Ok(SingleFileGenerationView {
        sequence: root.sequence,
        generation: header.generation,
        generation_offset: root.generation_offset,
        generation_len: root.generation_len,
        generation_digest: root.generation_digest,
        parent_digest: header.parent_digest,
        journal_offset: root.journal_offset,
        journal_end: (root.journal_end != 0).then_some(root.journal_end),
        journal_first_lsn: root.journal_first_lsn,
        sealed_journal_next_lsn: (root.journal_next_lsn != 0).then_some(root.journal_next_lsn),
        sections,
    })
}

struct DecodedGenerationHeader {
    generation: u64,
    parent_digest: [u8; 32],
    section_count: usize,
    table_len: u64,
    data_start: u64,
    table_offset: u64,
}

fn read_generation_header(
    file: &mut File,
    root: RootRecord,
) -> Result<DecodedGenerationHeader, DurabilityError> {
    let mut header = [0_u8; GENERATION_HEADER_LEN];
    file.seek(SeekFrom::Start(root.generation_offset))?;
    file.read_exact(&mut header)
        .map_err(|error| eof_as_corruption(error, "single-file generation header is truncated"))?;
    if header[0..4] != GENERATION_MAGIC
        || get_u16(&header[4..6]) != FORMAT_VERSION
        || get_u16(&header[6..8]) != 0
        || get_u32(&header[52..56]) != 0
        || header[GENERATION_HEADER_DIGEST_OFFSET + 32..]
            .iter()
            .any(|byte| *byte != 0)
    {
        return Err(corruption("single-file generation header mismatch"));
    }
    if sha256(&header[..GENERATION_HEADER_DIGEST_OFFSET])
        != header[GENERATION_HEADER_DIGEST_OFFSET..GENERATION_HEADER_DIGEST_OFFSET + 32]
    {
        return Err(corruption("single-file generation header digest mismatch"));
    }
    let generation = get_u64(&header[8..16]);
    let mut parent_digest = [0_u8; 32];
    parent_digest.copy_from_slice(&header[16..48]);
    let section_count =
        usize::try_from(get_u32(&header[48..52])).map_err(|_| DurabilityError::PayloadTooLarge)?;
    if section_count > MAX_SECTION_COUNT {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let table_len = get_u64(&header[56..64]);
    let data_start = get_u64(&header[64..72]);
    let total_len = get_u64(&header[72..80]);
    let table_offset = get_u64(&header[80..88]);
    let expected_table_len = u64::try_from(
        section_count
            .checked_mul(SECTION_DESCRIPTOR_LEN)
            .ok_or(DurabilityError::PayloadTooLarge)?,
    )
    .map_err(|_| DurabilityError::PayloadTooLarge)?;
    if table_len != expected_table_len
        || data_start != align_up(GENERATION_HEADER_LEN as u64, PAGE_SIZE)?
        || table_offset < data_start
        || !table_offset.is_multiple_of(PAGE_SIZE)
        || total_len
            != align_up(
                table_offset
                    .checked_add(table_len)
                    .ok_or(DurabilityError::PayloadTooLarge)?,
                PAGE_SIZE,
            )?
        || total_len != root.generation_len
        || total_len > MAX_GENERATION_LEN
    {
        return Err(corruption("single-file generation layout mismatch"));
    }
    Ok(DecodedGenerationHeader {
        generation,
        parent_digest,
        section_count,
        table_len,
        data_start,
        table_offset,
    })
}

fn decode_section_table(
    table: &[u8],
    section_count: usize,
    data_start: u64,
    table_offset: u64,
    root: RootRecord,
) -> Result<Vec<SingleFileSectionDescriptor>, DurabilityError> {
    let mut sections = Vec::with_capacity(section_count);
    let mut keys = BTreeSet::new();
    let mut prior_end = data_start;
    let (descriptors, remainder) = table.as_chunks::<SECTION_DESCRIPTOR_LEN>();
    if !remainder.is_empty() {
        return Err(corruption("single-file section table length mismatch"));
    }
    for descriptor in descriptors {
        if get_u16(&descriptor[2..4]) != 0 || descriptor[56..64].iter().any(|byte| *byte != 0) {
            return Err(corruption(
                "single-file section descriptor reserved bytes are non-zero",
            ));
        }
        let kind = SingleFileSectionKind::decode(get_u16(&descriptor[0..2]))?;
        let ordinal = get_u32(&descriptor[4..8]);
        if !keys.insert((kind, ordinal)) {
            return Err(corruption("single-file section descriptor is duplicated"));
        }
        let relative_offset = get_u64(&descriptor[8..16]);
        let len = get_u64(&descriptor[16..24]);
        if len > MAX_SECTION_LEN
            || relative_offset < data_start
            || !relative_offset.is_multiple_of(PAGE_SIZE)
            || relative_offset < prior_end
        {
            return Err(corruption("single-file section range is invalid"));
        }
        let end = relative_offset
            .checked_add(len)
            .ok_or_else(|| corruption("single-file section range overflow"))?;
        if end > table_offset {
            return Err(corruption("single-file section exceeds generation"));
        }
        prior_end = align_up(end, PAGE_SIZE)?;
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(&descriptor[24..56]);
        sections.push(SingleFileSectionDescriptor {
            kind,
            ordinal,
            offset: root
                .generation_offset
                .checked_add(relative_offset)
                .ok_or(DurabilityError::PayloadTooLarge)?,
            len,
            digest,
        });
    }
    if sections.len() != section_count {
        return Err(corruption("single-file section table length mismatch"));
    }
    if prior_end != table_offset {
        return Err(corruption(
            "single-file section table is not at the canonical payload boundary",
        ));
    }
    Ok(sections)
}

fn validate_section_inputs(sections: &[SingleFileSectionInput<'_>]) -> Result<(), DurabilityError> {
    if sections.len() > MAX_SECTION_COUNT {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let mut keys = BTreeSet::new();
    for section in sections {
        if !keys.insert((section.kind, section.ordinal)) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file section key is duplicated",
            });
        }
    }
    Ok(())
}

fn hash_file_range<R: Read + Seek>(
    file: &mut R,
    offset: u64,
    len: u64,
) -> Result<[u8; 32], DurabilityError> {
    file.seek(SeekFrom::Start(offset))?;
    let mut remaining = len;
    let mut buffer = [0_u8; IO_BUFFER_SIZE];
    let mut hasher = Sha256::new();
    while remaining != 0 {
        let chunk = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        file.read_exact(&mut buffer[..chunk])
            .map_err(|error| eof_as_corruption(error, "single-file generation is truncated"))?;
        hasher.update(&buffer[..chunk]);
        remaining -= chunk as u64;
    }
    Ok(hasher.finalize().into())
}

fn sync_parent(path: &Path) -> Result<(), DurabilityError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn align_up(value: u64, alignment: u64) -> Result<u64, DurabilityError> {
    let mask = alignment - 1;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(DurabilityError::PayloadTooLarge)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn corruption(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

fn eof_as_corruption(error: std::io::Error, reason: &'static str) -> DurabilityError {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        corruption(reason)
    } else {
        DurabilityError::Io(error)
    }
}

fn put_u16(bytes: &mut [u8], value: u16) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], value: u32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], value: u64) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn get_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("u16 slice length"))
}

fn get_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("u32 slice length"))
}

fn get_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("u64 slice length"))
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::time::{SystemTime, UNIX_EPOCH};

    use kernel_model::{DatabaseState, Value};
    use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
    use kernel_semantics::SemanticRegistry;
    use kernel_types::{
        ClientTransactionId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId,
    };

    use crate::descriptor::DurableRevisionDescriptor;
    use crate::domain::DurableRelationMutation;
    use crate::runtime::RevisionDurability;

    use super::*;

    fn test_file(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cfmd-single-file-{label}-{}-{nonce}.cfmd",
            std::process::id()
        ))
    }

    fn sections<'a>(checkpoint: &'a [u8], metadata: &'a [u8]) -> [SingleFileSectionInput<'a>; 2] {
        [
            SingleFileSectionInput::bytes(SingleFileSectionKind::Checkpoint, 0, checkpoint),
            SingleFileSectionInput::bytes(SingleFileSectionKind::Metadata, 0, metadata),
        ]
    }

    fn descriptor(source: u64, target: u64, value: i64) -> DurableRevisionDescriptor {
        let registry = SemanticRegistry::default();
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(7)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(9)),
        };
        let target_revision = kernel_revision::Revision::build(
            RevisionId::new(target),
            &context,
            &registry,
            DatabaseState::default(),
        )
        .unwrap();
        DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(u128::from(target)),
            RevisionId::new(source),
            &target_revision,
            target_revision.semantic_revision(),
            vec![DurableRelationMutation {
                relation: SemanticId::new(11),
                inserted: vec![vec![Value::I64(value)]],
                removed: Vec::new(),
            object_field_writes: Vec::new(),
            authorization: Default::default(),
            }],
            &registry,
        )
        .unwrap()
    }

    #[test]
    fn compaction_typestate_sequence_matches_declarative_publication_law() {
        let declared = SingleFileCompactionIoStep::ALL
            .iter()
            .copied()
            .filter(|step| step.publication_boundary().is_some())
            .collect::<Vec<_>>();
        assert_eq!(declared, COMPACTION_PUBLICATION_SEQUENCE);
        assert!(
            declared.iter().all(|step| {
                step.is_publication_sync_boundary() ^ step.is_uncertain_root_write()
            })
        );
        let crash_names = declared
            .iter()
            .map(|step| {
                step.crash_name()
                    .expect("publication boundary has crash name")
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(crash_names.len(), declared.len());
    }

    #[test]
    fn single_file_roundtrip_and_alternating_root_slots() {
        let path = test_file("roundtrip");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        assert_eq!(store.sequence(), 1);
        assert_eq!(store.generation(), 1);
        assert_eq!(
            store
                .read_section(SingleFileSectionKind::Checkpoint, 0)
                .unwrap(),
            Some(b"checkpoint-1".to_vec())
        );
        let view2 = store
            .publish_generation(&sections(b"checkpoint-2", b"metadata-2"))
            .unwrap();
        assert_eq!(view2.sequence, 3);
        assert_eq!(view2.generation, 2);
        let view3 = store
            .publish_generation(&sections(b"checkpoint-3", b"metadata-3"))
            .unwrap();
        assert_eq!(view3.sequence, 5);
        drop(store);
        let mut reopened = SingleFileContainer::open(&path).unwrap();
        assert_eq!(reopened.sequence(), 5);
        assert_eq!(reopened.generation(), 3);
        assert_eq!(
            reopened
                .read_section(SingleFileSectionKind::Metadata, 0)
                .unwrap(),
            Some(b"metadata-3".to_vec())
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn encrypted_sections_are_chunked_and_stream_copy_without_whole_section_envelope() {
        let path = test_file("encrypted-chunks");
        let key = crate::storage_encryption::StorageEncryptionKey::try_new([0x51; 32]).unwrap();
        let encryption = StorageEncryption::aes256_gcm_siv(key);
        let mut checkpoint = vec![0_u8; ENCRYPTED_SECTION_CHUNK_SIZE * 2 + 777];
        for (index, byte) in checkpoint.iter_mut().enumerate() {
            *byte = u8::try_from(index % 251).expect("modulo 251 fits in u8");
        }
        let mut store = SingleFileContainer::create_with_encryption(
            &path,
            &sections(&checkpoint, b"encrypted-metadata"),
            &encryption,
        )
        .unwrap();
        let view = store.generation_view().unwrap();
        let descriptor = view
            .sections
            .iter()
            .find(|section| section.kind == SingleFileSectionKind::Checkpoint)
            .unwrap()
            .clone();
        let mut raw = vec![0_u8; 4];
        store.file.seek(SeekFrom::Start(descriptor.offset)).unwrap();
        store.file.read_exact(&mut raw).unwrap();
        assert_eq!(raw.as_slice(), ENCRYPTED_SECTION_MAGIC);

        let mut generation_header = [0_u8; GENERATION_HEADER_LEN];
        store
            .file
            .seek(SeekFrom::Start(view.generation_offset))
            .unwrap();
        store.file.read_exact(&mut generation_header).unwrap();
        let table_offset = get_u64(&generation_header[80..88]);
        let table_absolute = view.generation_offset + table_offset;
        let last_section_end = view
            .sections
            .iter()
            .map(|section| section.offset + section.len)
            .max()
            .unwrap();
        assert!(table_absolute >= align_up(last_section_end, PAGE_SIZE).unwrap());
        assert!(table_absolute < view.journal_offset);

        let mut streamed = Vec::new();
        assert_eq!(
            store
                .copy_section_to(SingleFileSectionKind::Checkpoint, 0, &mut streamed)
                .unwrap(),
            Some(checkpoint.len() as u64)
        );
        assert_eq!(streamed, checkpoint);
        drop(store);

        let mut reopened = SingleFileContainer::open_with_encryption(&path, &encryption).unwrap();
        assert_eq!(
            reopened
                .read_section(SingleFileSectionKind::Checkpoint, 0)
                .unwrap(),
            Some(checkpoint)
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn single_file_live_wal_roundtrips_without_sidecar() {
        let path = test_file("live-wal");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        let (mut wal, initial) = store
            .open_journal_recovered(RevisionId::new(1), 1, &[])
            .unwrap();
        assert_eq!(initial.durable_revision(), RevisionId::new(1));
        let prepared = wal.durably_prepare(&descriptor(1, 2, 11)).unwrap();
        wal.durably_commit(prepared).unwrap();
        drop(wal);
        drop(store);

        let mut reopened = SingleFileContainer::open(&path).unwrap();
        let (_wal, recovered) = reopened
            .open_journal_recovered(RevisionId::new(1), 1, &[])
            .unwrap();
        assert_eq!(recovered.durable_revision(), RevisionId::new(2));
        assert_eq!(recovered.next_lsn(), 3);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn torn_single_file_wal_tail_is_truncated_inside_container() {
        let path = test_file("torn-wal");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        let journal_offset = store.generation_view().unwrap().journal_offset;
        let (mut wal, _) = store
            .open_journal_recovered(RevisionId::new(1), 1, &[])
            .unwrap();
        let prepared = wal.durably_prepare(&descriptor(1, 2, 12)).unwrap();
        wal.durably_commit(prepared).unwrap();
        drop(wal);
        drop(store);

        let good_len = fs::metadata(&path).unwrap().len();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(b"CFWL\0\0\0").unwrap();
        file.sync_all().unwrap();
        drop(file);

        let mut reopened = SingleFileContainer::open(&path).unwrap();
        let (mut wal, recovered) = reopened
            .open_journal_recovered(RevisionId::new(1), 1, &[])
            .unwrap();
        assert_eq!(recovered.durable_revision(), RevisionId::new(2));
        assert_eq!(fs::metadata(&path).unwrap().len(), good_len);
        assert!(good_len > journal_offset);
        let prepared = wal.durably_prepare(&descriptor(2, 3, 13)).unwrap();
        wal.durably_commit(prepared).unwrap();
        drop(wal);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn sealed_journal_makes_orphan_generation_recoverable_without_guessing() {
        let path = test_file("sealed-orphan");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        let (mut wal, _) = store
            .open_journal_recovered(RevisionId::new(1), 1, &[])
            .unwrap();
        let prepared = wal.durably_prepare(&descriptor(1, 2, 14)).unwrap();
        wal.durably_commit(prepared).unwrap();
        wal.durability_barrier().unwrap();
        let next_lsn = wal.next_lsn();
        drop(wal);
        let sealed_end = store.seal_journal_boundary(next_lsn).unwrap();
        let root = store.root;
        let orphan_offset = align_up(sealed_end, PAGE_SIZE).unwrap();
        let orphan_sections = sections(b"checkpoint-2", b"metadata-2");
        let mut layout = generation_layout(
            2,
            root.generation_digest,
            orphan_offset,
            &orphan_sections,
            None,
        )
        .unwrap();
        store.file.set_len(orphan_offset).unwrap();
        write_generation_streaming(
            &mut store.file,
            orphan_offset,
            &mut layout,
            &orphan_sections,
            None,
        )
        .unwrap();
        store.file.sync_all().unwrap();
        drop(store);

        let mut reopened = SingleFileContainer::open(&path).unwrap();
        assert_eq!(reopened.generation(), 1);
        let (mut wal, recovered) = reopened
            .open_journal_recovered(RevisionId::new(1), 1, &[])
            .unwrap();
        assert_eq!(recovered.durable_revision(), RevisionId::new(2));
        assert_eq!(recovered.next_lsn(), 3);
        assert_eq!(fs::metadata(&path).unwrap().len(), sealed_end);
        let prepared = wal.durably_prepare(&descriptor(2, 3, 15)).unwrap();
        wal.durably_commit(prepared).unwrap();
        drop(wal);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn wal_rotation_carries_exact_next_lsn_into_new_generation() {
        let path = test_file("wal-rotation");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        let (mut wal, _) = store
            .open_journal_recovered(RevisionId::new(1), 1, &[])
            .unwrap();
        let prepared = wal.durably_prepare(&descriptor(1, 2, 16)).unwrap();
        wal.durably_commit(prepared).unwrap();
        let view = store
            .publish_generation_after_wal(wal, &sections(b"checkpoint-2", b"metadata-2"))
            .unwrap();
        assert_eq!(view.generation, 2);
        assert_eq!(view.journal_first_lsn, 3);
        assert_eq!(view.journal_end, None);
        let (mut wal2, scan2) = store
            .open_journal_recovered(RevisionId::new(2), 3, &[])
            .unwrap();
        assert_eq!(scan2.next_lsn(), 3);
        let prepared = wal2.durably_prepare(&descriptor(2, 3, 17)).unwrap();
        wal2.durably_commit(prepared).unwrap();
        drop(wal2);
        drop(store);

        let mut reopened = SingleFileContainer::open(&path).unwrap();
        let (_wal, recovered) = reopened
            .open_journal_recovered(RevisionId::new(2), 3, &[])
            .unwrap();
        assert_eq!(recovered.durable_revision(), RevisionId::new(3));
        assert_eq!(recovered.next_lsn(), 5);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn every_single_file_wal_prefix_exposes_only_fully_durable_commits() {
        let source = test_file("wal-prefix-source");
        let mut store =
            SingleFileContainer::create(&source, &sections(b"checkpoint-10", b"metadata-10"))
                .unwrap();
        let journal_offset = store.generation_view().unwrap().journal_offset;
        let (mut wal, _) = store
            .open_journal_recovered(RevisionId::new(10), 1, &[])
            .unwrap();
        let first = wal.durably_prepare(&descriptor(10, 20, 20)).unwrap();
        wal.durably_commit(first).unwrap();
        let first_end = fs::metadata(&source).unwrap().len();
        let second = wal.durably_prepare(&descriptor(20, 30, 30)).unwrap();
        wal.durably_commit(second).unwrap();
        let second_end = fs::metadata(&source).unwrap().len();
        drop(wal);
        drop(store);

        for cut in journal_offset..=second_end {
            let candidate = test_file("wal-prefix-cut");
            fs::copy(&source, &candidate).unwrap();
            OpenOptions::new()
                .write(true)
                .open(&candidate)
                .unwrap()
                .set_len(cut)
                .unwrap();
            let mut reopened = SingleFileContainer::open(&candidate).unwrap();
            let (_wal, scan) = reopened
                .open_journal_recovered(RevisionId::new(10), 1, &[])
                .unwrap();
            let expected = if cut >= second_end {
                RevisionId::new(30)
            } else if cut >= first_end {
                RevisionId::new(20)
            } else {
                RevisionId::new(10)
            };
            assert_eq!(scan.durable_revision(), expected, "cut={cut}");
            fs::remove_file(candidate).unwrap();
        }
        fs::remove_file(source).unwrap();
    }

    #[test]
    fn orphan_generation_without_root_switch_is_ignored() {
        let path = test_file("orphan");
        let store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        let root = store.root;
        drop(store);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.lock().unwrap();
        let orphan_offset = align_up(file.metadata().unwrap().len(), PAGE_SIZE).unwrap();
        let orphan_sections = sections(b"checkpoint-2", b"metadata-2");
        let mut layout = generation_layout(
            2,
            root.generation_digest,
            orphan_offset,
            &orphan_sections,
            None,
        )
        .unwrap();
        file.set_len(orphan_offset).unwrap();
        write_generation_streaming(
            &mut file,
            orphan_offset,
            &mut layout,
            &orphan_sections,
            None,
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        let mut reopened = SingleFileContainer::open(&path).unwrap();
        assert_eq!(reopened.generation(), 1);
        assert_eq!(
            reopened
                .read_section(SingleFileSectionKind::Checkpoint, 0)
                .unwrap(),
            Some(b"checkpoint-1".to_vec())
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn torn_new_root_slot_falls_back_to_previous_valid_root() {
        let path = test_file("torn-root");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        store
            .publish_generation(&sections(b"checkpoint-2", b"metadata-2"))
            .unwrap();
        let root2 = store.root;
        drop(store);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut torn = [0_u8; 37];
        torn[0..4].copy_from_slice(&ROOT_MAGIC);
        file.seek(SeekFrom::Start(root2.slot.other().offset()))
            .unwrap();
        file.write_all(&torn).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let mut reopened = SingleFileContainer::open(&path).unwrap();
        assert_eq!(reopened.generation(), 2);
        assert_eq!(
            reopened
                .read_section(SingleFileSectionKind::Checkpoint, 0)
                .unwrap(),
            Some(b"checkpoint-2".to_vec())
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn valid_new_root_pointing_to_corrupt_generation_never_falls_back() {
        let path = test_file("no-fallback");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        store
            .publish_generation(&sections(b"checkpoint-2", b"metadata-2"))
            .unwrap();
        let root2 = store.root;
        drop(store);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.seek(SeekFrom::Start(
            root2.generation_offset + root2.generation_len - 1,
        ))
        .unwrap();
        file.write_all(&[0x7f]).unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert!(matches!(
            SingleFileContainer::open(&path),
            Err(DurabilityError::Corruption {
                reason: "authoritative single-file generation digest mismatch",
                ..
            })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn section_corruption_is_detected_even_after_root_generation_validation() {
        let path = test_file("section-digest");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        let view = store.generation_view().unwrap();
        let checkpoint = view
            .sections
            .iter()
            .find(|section| section.kind == SingleFileSectionKind::Checkpoint)
            .unwrap()
            .clone();
        let root = store.root;
        drop(store);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.seek(SeekFrom::Start(checkpoint.offset)).unwrap();
        file.write_all(b"X").unwrap();
        let generation_digest =
            hash_file_range(&mut file, root.generation_offset, root.generation_len).unwrap();
        let mut rewritten = root;
        rewritten.generation_digest = generation_digest;
        write_root_slot(&mut file, rewritten).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let mut reopened = SingleFileContainer::open(&path).unwrap();
        assert!(matches!(
            reopened.read_section(SingleFileSectionKind::Checkpoint, 0),
            Err(DurabilityError::Corruption {
                reason: "single-file section digest mismatch",
                ..
            })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn valid_root_slots_must_form_one_parent_digest_chain() {
        let path = test_file("root-chain");
        let mut store =
            SingleFileContainer::create(&path, &sections(b"checkpoint-1", b"metadata-1")).unwrap();
        store
            .publish_generation(&sections(b"checkpoint-2", b"metadata-2"))
            .unwrap();
        let newer = store.root;
        drop(store);

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        let mut forged = newer;
        forged.parent_digest = [0x5a; 32];
        write_root_slot(&mut file, forged).unwrap();
        file.sync_all().unwrap();
        drop(file);

        assert!(matches!(
            SingleFileContainer::open(&path),
            Err(DurabilityError::Corruption {
                reason: "single-file root slots do not form a consecutive chain",
                ..
            })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn torn_rewrap_slot_recovers_previous_wrapped_key_without_data_rewrite() {
        let path = test_file("torn-key-rewrap");
        let old = StorageEncryption::aes256_gcm_siv_wrapped(
            StorageEncryptionKey::try_new([0x31; 32]).unwrap(),
            [0x11; 16],
            1,
        );
        let next = StorageEncryption::aes256_gcm_siv_wrapped(
            StorageEncryptionKey::try_new([0x32; 32]).unwrap(),
            [0x22; 16],
            2,
        );
        let mut store = SingleFileContainer::create_with_encryption(
            &path,
            &sections(b"checkpoint", b"metadata"),
            &old,
        )
        .unwrap();
        assert_eq!(store.rewrap_database_master_key(&next).unwrap(), 2);
        drop(store);

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.seek(SeekFrom::Start(KEY_SLOT_B_OFFSET as u64))
            .unwrap();
        file.write_all(&[0_u8; KEY_SLOT_LEN]).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let mut recovered = SingleFileContainer::open_with_encryption(&path, &old).unwrap();
        assert_eq!(
            recovered
                .read_section(SingleFileSectionKind::Checkpoint, 0)
                .unwrap(),
            Some(b"checkpoint".to_vec())
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn pending_wrapped_key_handoff_recovers_under_old_authority_and_retries_without_rewrite() {
        let path = test_file("pending-key-handoff-recovery");
        let old = StorageEncryption::aes256_gcm_siv_wrapped(
            StorageEncryptionKey::try_new([0x61; 32]).unwrap(),
            [0x71; 16],
            1,
        );
        let next = StorageEncryption::aes256_gcm_siv_wrapped(
            StorageEncryptionKey::try_new([0x62; 32]).unwrap(),
            [0x72; 16],
            2,
        );
        let mut store = SingleFileContainer::create_with_encryption(
            &path,
            &sections(b"checkpoint", b"metadata"),
            &old,
        )
        .unwrap();
        assert_eq!(store.rewrap_database_master_key(&next).unwrap(), 2);
        drop(store);

        let before_retry = fs::read(&path).unwrap();
        let mut recovered_old = SingleFileContainer::open_with_encryption(&path, &old).unwrap();
        assert_eq!(
            recovered_old
                .read_section(SingleFileSectionKind::Checkpoint, 0)
                .unwrap(),
            Some(b"checkpoint".to_vec())
        );
        assert_eq!(recovered_old.rewrap_database_master_key(&next).unwrap(), 2);
        let after_retry = fs::read(&path).unwrap();
        assert_eq!(
            &before_retry[..PAGE_SIZE_USIZE],
            &after_retry[..PAGE_SIZE_USIZE],
            "retry must adopt the already-durable pending slot instead of publishing another wrap"
        );
        recovered_old.retire_previous_wrapped_key_slot(2).unwrap();
        drop(recovered_old);

        assert!(SingleFileContainer::open_with_encryption(&path, &old).is_err());
        let mut reopened = SingleFileContainer::open_with_encryption(&path, &next).unwrap();
        assert_eq!(
            reopened
                .read_section(SingleFileSectionKind::Metadata, 0)
                .unwrap(),
            Some(b"metadata".to_vec())
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn provider_database_key_epoch_floor_rejects_complete_header_rollback() {
        let path = test_file("key-epoch-floor-rollback");
        let wrapping_key = StorageEncryptionKey::try_new([0x41; 32]).unwrap();
        let provider_key_id = [0x51; 16];
        let provider =
            StorageEncryption::aes256_gcm_siv_wrapped(wrapping_key.clone(), provider_key_id, 7);
        let mut store = SingleFileContainer::create_with_encryption(
            &path,
            &sections(b"checkpoint", b"metadata"),
            &provider,
        )
        .unwrap();

        let mut old_header = [0_u8; PAGE_SIZE_USIZE];
        {
            let mut file = File::open(&path).unwrap();
            file.read_exact(&mut old_header).unwrap();
        }

        let next_provider =
            StorageEncryption::aes256_gcm_siv_wrapped(wrapping_key.clone(), provider_key_id, 8);
        assert_eq!(store.rewrap_database_master_key(&next_provider).unwrap(), 2);
        drop(store);

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.seek(SeekFrom::Start(HEADER_OFFSET)).unwrap();
        file.write_all(&old_header).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let enforcing = StorageEncryption::aes256_gcm_siv_wrapped_with_minimum_database_key_epoch(
            wrapping_key,
            provider_key_id,
            7,
            2,
        );
        assert!(matches!(
            SingleFileContainer::open_with_encryption(&path, &enforcing),
            Err(DurabilityError::Protocol {
                reason: "wrapped database key was rolled back below provider authority floor",
                ..
            })
        ));

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn duplicate_section_keys_are_rejected_before_file_publication() {
        let path = test_file("duplicates");
        let duplicate = [
            SingleFileSectionInput::bytes(SingleFileSectionKind::Checkpoint, 0, b"a"),
            SingleFileSectionInput::bytes(SingleFileSectionKind::Checkpoint, 0, b"b"),
        ];
        assert!(matches!(
            SingleFileContainer::create(&path, &duplicate),
            Err(DurabilityError::Protocol {
                reason: "single-file section key is duplicated",
                ..
            })
        ));
        let _ = fs::remove_file(path);
    }
}
