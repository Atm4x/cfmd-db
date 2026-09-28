use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kernel_types::RevisionId;
use sha2::{Digest, Sha256};

use crate::descriptor::DurableRevisionDescriptor;
use crate::runtime::{DurabilityError, RecoveryScan, TailStatus};
use crate::wal::{FileRevisionWal, WalRegionRecovery};

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
const FORMAT_VERSION: u16 = 2;
const HEADER_DIGEST_OFFSET: usize = 64;
const ROOT_DIGEST_OFFSET: usize = 144;
const GENERATION_HEADER_LEN: usize = 128;
const GENERATION_HEADER_DIGEST_OFFSET: usize = 80;
const SECTION_DESCRIPTOR_LEN: usize = 64;
const MAX_SECTION_COUNT: usize = 65_535;
const MAX_SECTION_LEN: u64 = 1_u64 << 40;
const MAX_GENERATION_LEN: u64 = 1_u64 << 44;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SingleFileSectionKind {
    Checkpoint = 1,
    Metadata = 2,
    PreparedCapsule = 3,
    ReplicationAuthority = 4,
    PhysicalArtifact = 5,
    Auxiliary = 6,
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
            _ => Err(corruption("single-file section kind is unsupported")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SingleFileSectionInput<'a> {
    pub kind: SingleFileSectionKind,
    pub ordinal: u32,
    pub bytes: &'a [u8],
}

#[derive(Clone, Copy)]
pub(crate) struct CarriedWalPublication<'a> {
    pub(crate) start_offset: u64,
    pub(crate) first_lsn: u64,
    pub(crate) base_revision: RevisionId,
    pub(crate) expected_durable_revision: RevisionId,
    pub(crate) seeded_prepares: &'a [(u64, DurableRevisionDescriptor, u32)],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleFileSectionDescriptor {
    pub kind: SingleFileSectionKind,
    pub ordinal: u32,
    pub offset: u64,
    pub len: u64,
    pub digest: [u8; 32],
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
}

#[derive(Debug, Clone, Copy)]
struct SingleFileCompactionPlan {
    old_root: RootRecord,
    journal_len: u64,
    compacted_journal_offset: u64,
    compacted_journal_end: u64,
    next_lsn: u64,
}

#[derive(Debug)]
pub struct SingleFileContainer {
    path: PathBuf,
    file: File,
    root: RootRecord,
}

impl SingleFileContainer {
    pub fn create(
        path: impl AsRef<Path>,
        sections: &[SingleFileSectionInput<'_>],
    ) -> Result<Self, DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.lock()?;
        write_header(&mut file)?;
        file.set_len(DATA_OFFSET)?;
        file.sync_all()?;
        sync_parent(&path)?;
        let root = publish_generation_to_file(&mut file, None, 1, 1, sections)?;
        Ok(Self { path, file, root })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        file.lock()?;
        read_and_validate_header(&mut file)?;
        let slot_a = read_root_slot(&mut file, RootSlot::A)?;
        let slot_b = read_root_slot(&mut file, RootSlot::B)?;
        let root = choose_authoritative_root(slot_a, slot_b)?;
        validate_authoritative_generation(&mut file, root)?;
        Ok(Self { path, file, root })
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

    pub fn generation_view(&mut self) -> Result<SingleFileGenerationView, DurabilityError> {
        read_generation_view(&mut self.file, self.root)
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
        let len = usize::try_from(section.len).map_err(|_| DurabilityError::PayloadTooLarge)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        self.copy_section_descriptor_to(&section, &mut bytes)?;
        Ok(Some(bytes))
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
        self.copy_section_descriptor_to(&section, output)?;
        Ok(Some(section.len))
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
    ) -> Result<SingleFileGenerationView, DurabilityError> {
        if wal.path() != self.path || wal.start_offset() != self.root.journal_offset {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file WAL does not belong to the active journal",
            });
        }
        let next_lsn = wal.seal_for_generation_rotation()?;
        self.seal_journal_boundary(next_lsn)?;
        self.publish_next_generation(sections)
    }

    pub(crate) fn publish_generation_with_carried_wal(
        &mut self,
        wal: &mut FileRevisionWal,
        carry: CarriedWalPublication<'_>,
        sections: &[SingleFileSectionInput<'_>],
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

        let generation = self
            .root
            .generation
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?;
        let (generation_offset, generation_len, generation_digest, section_count) =
            append_generation_without_root(&mut self.file, self.root, generation, sections)?;
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
        )?;
        self.root = next;
        self.generation_view()
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
        })
    }

    pub(crate) fn compact_active_generation(
        &mut self,
        wal: &mut FileRevisionWal,
        base_revision: RevisionId,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
        expected_durable_revision: RevisionId,
    ) -> Result<bool, DurabilityError> {
        let Some(plan) = self.prepare_compaction_plan(wal)? else {
            return Ok(false);
        };
        self.copy_compaction_authority(plan)?;
        let (_, scan) = FileRevisionWal::open_region_recovered(WalRegionRecovery {
            path: self.path.clone(),
            file: OpenOptions::new().read(true).write(true).open(&self.path)?,
            start_offset: plan.compacted_journal_offset,
            end_offset: plan.compacted_journal_end,
            base_revision,
            first_lsn: plan.old_root.journal_first_lsn,
            seeded_prepares,
            writable: false,
        })?;
        let logical_len =
            u64::try_from(scan.last_good_offset()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        if scan.durable_revision() != expected_durable_revision
            || scan.next_lsn() != plan.next_lsn
            || !matches!(scan.tail_status(), TailStatus::Clean)
            || logical_len != plan.journal_len
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "compacted single-file WAL does not certify the active durable endpoint",
            });
        }

        let relocated = RootRecord {
            slot: plan.old_root.slot.other(),
            sequence: plan
                .old_root
                .sequence
                .checked_add(1)
                .ok_or(DurabilityError::LsnExhausted)?,
            generation_offset: DATA_OFFSET,
            journal_offset: plan.compacted_journal_offset,
            journal_end: plan.compacted_journal_end,
            ..plan.old_root
        };
        write_root_slot(&mut self.file, relocated)?;
        self.file.sync_all()?;
        self.root = relocated;

        let (replacement, reopened_scan) = self.open_journal_recovered(
            base_revision,
            relocated.journal_first_lsn,
            seeded_prepares,
        )?;
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

    fn prepare_compaction_plan(
        &mut self,
        wal: &mut FileRevisionWal,
    ) -> Result<Option<SingleFileCompactionPlan>, DurabilityError> {
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
        let active_end = wal.current_end_offset()?;
        let journal_len = active_end
            .checked_sub(self.root.journal_offset)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let compacted_journal_offset = DATA_OFFSET
            .checked_add(self.root.generation_len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let compacted_journal_end = compacted_journal_offset
            .checked_add(journal_len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if compacted_journal_end > self.root.generation_offset {
            return Ok(None);
        }
        let next_lsn = wal.seal_for_generation_rotation()?;
        let old_journal_end = self.seal_journal_boundary(next_lsn)?;
        let old_root = self.root;
        if old_journal_end - old_root.journal_offset != journal_len {
            return Err(corruption(
                "single-file journal changed while entering compaction barrier",
            ));
        }
        Ok(Some(SingleFileCompactionPlan {
            old_root,
            journal_len,
            compacted_journal_offset,
            compacted_journal_end,
            next_lsn,
        }))
    }

    fn copy_compaction_authority(
        &mut self,
        plan: SingleFileCompactionPlan,
    ) -> Result<(), DurabilityError> {
        let copy_len = plan
            .old_root
            .generation_len
            .checked_add(plan.journal_len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let mut source = File::open(&self.path)?;
        source.seek(SeekFrom::Start(plan.old_root.generation_offset))?;
        self.file.seek(SeekFrom::Start(DATA_OFFSET))?;
        let copied = std::io::copy(&mut source.take(copy_len), &mut self.file)?;
        if copied != copy_len {
            return Err(corruption("single-file compaction copy was truncated"));
        }
        self.file.sync_all()?;
        let digest = hash_file_range(&mut self.file, DATA_OFFSET, plan.old_root.generation_len)?;
        if digest != plan.old_root.generation_digest {
            return Err(corruption(
                "single-file compacted generation digest mismatch",
            ));
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

    fn reopen_sealed_journal(&mut self) -> Result<(), DurabilityError> {
        let journal_end = self.root.journal_end;
        if journal_end < self.root.journal_offset {
            return Err(corruption("single-file sealed journal range is invalid"));
        }
        if self.file.metadata()?.len() > journal_end {
            self.file.set_len(journal_end)?;
            self.file.sync_all()?;
        }
        let next = rewrite_root_journal_boundary(&mut self.file, self.root, 0, 0)?;
        self.root = next;
        Ok(())
    }
}

fn write_header(file: &mut File) -> Result<(), DurabilityError> {
    let mut page = [0_u8; PAGE_SIZE_USIZE];
    page[0..8].copy_from_slice(&HEADER_MAGIC);
    put_u16(&mut page[8..10], FORMAT_VERSION);
    put_u16(&mut page[10..12], 0);
    put_u32(&mut page[12..16], PAGE_SIZE_U32);
    put_u64(&mut page[16..24], ROOT_A_OFFSET);
    put_u64(&mut page[24..32], ROOT_B_OFFSET);
    put_u64(&mut page[32..40], DATA_OFFSET);
    let digest = sha256(&page[..HEADER_DIGEST_OFFSET]);
    page[HEADER_DIGEST_OFFSET..HEADER_DIGEST_OFFSET + 32].copy_from_slice(&digest);
    file.seek(SeekFrom::Start(HEADER_OFFSET))?;
    file.write_all(&page)?;
    Ok(())
}

fn read_and_validate_header(file: &mut File) -> Result<(), DurabilityError> {
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
    {
        return Err(corruption("single-file header layout mismatch"));
    }
    if sha256(&page[..HEADER_DIGEST_OFFSET])
        != page[HEADER_DIGEST_OFFSET..HEADER_DIGEST_OFFSET + 32]
    {
        return Err(corruption("single-file header digest mismatch"));
    }
    if page[HEADER_DIGEST_OFFSET + 32..]
        .iter()
        .any(|byte| *byte != 0)
    {
        return Err(corruption("single-file header reserved bytes are non-zero"));
    }
    Ok(())
}

fn publish_generation_to_file(
    file: &mut File,
    current: Option<RootRecord>,
    generation: u64,
    journal_first_lsn: u64,
    sections: &[SingleFileSectionInput<'_>],
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
    let layout = generation_layout(generation, parent_digest, generation_offset, sections)?;
    let generation_digest = write_generation(file, generation_offset, &layout, sections)?;
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
) -> Result<(u64, u64, [u8; 32], u32), DurabilityError> {
    validate_section_inputs(sections)?;
    let file_len = file.metadata()?.len();
    let generation_offset = align_up(file_len.max(DATA_OFFSET), PAGE_SIZE)?;
    if generation_offset > file_len {
        file.set_len(generation_offset)?;
    }
    let layout = generation_layout(
        generation,
        current.generation_digest,
        generation_offset,
        sections,
    )?;
    let generation_digest = write_generation(file, generation_offset, &layout, sections)?;
    file.sync_all()?;
    Ok((
        generation_offset,
        layout.total_len,
        generation_digest,
        u32::try_from(sections.len()).map_err(|_| DurabilityError::PayloadTooLarge)?,
    ))
}

struct GenerationLayout {
    header: [u8; GENERATION_HEADER_LEN],
    table: Vec<u8>,
    descriptors: Vec<SingleFileSectionDescriptor>,
    data_start: u64,
    total_len: u64,
}

fn generation_layout(
    generation: u64,
    parent_digest: [u8; 32],
    generation_offset: u64,
    sections: &[SingleFileSectionInput<'_>],
) -> Result<GenerationLayout, DurabilityError> {
    let table_len = SECTION_DESCRIPTOR_LEN
        .checked_mul(sections.len())
        .ok_or(DurabilityError::PayloadTooLarge)?;
    let data_start = align_up(
        u64::try_from(GENERATION_HEADER_LEN + table_len)
            .map_err(|_| DurabilityError::PayloadTooLarge)?,
        PAGE_SIZE,
    )?;
    let mut descriptors = Vec::with_capacity(sections.len());
    let mut cursor = data_start;
    for section in sections {
        let len =
            u64::try_from(section.bytes.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
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
            digest: sha256(section.bytes),
        });
        cursor = align_up(
            cursor
                .checked_add(len)
                .ok_or(DurabilityError::PayloadTooLarge)?,
            PAGE_SIZE,
        )?;
    }
    let total_len = cursor;
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
    put_u64(
        &mut header[56..64],
        u64::try_from(table_len).map_err(|_| DurabilityError::PayloadTooLarge)?,
    );
    put_u64(&mut header[64..72], data_start);
    put_u64(&mut header[72..80], total_len);
    let header_digest = sha256(&header[..GENERATION_HEADER_DIGEST_OFFSET]);
    header[GENERATION_HEADER_DIGEST_OFFSET..GENERATION_HEADER_DIGEST_OFFSET + 32]
        .copy_from_slice(&header_digest);

    let mut table = vec![0_u8; table_len];
    for (index, descriptor) in descriptors.iter().enumerate() {
        let start = index * SECTION_DESCRIPTOR_LEN;
        let descriptor_bytes = &mut table[start..start + SECTION_DESCRIPTOR_LEN];
        put_u16(&mut descriptor_bytes[0..2], descriptor.kind as u16);
        put_u16(&mut descriptor_bytes[2..4], 0);
        put_u32(&mut descriptor_bytes[4..8], descriptor.ordinal);
        put_u64(
            &mut descriptor_bytes[8..16],
            descriptor.offset - generation_offset,
        );
        put_u64(&mut descriptor_bytes[16..24], descriptor.len);
        descriptor_bytes[24..56].copy_from_slice(&descriptor.digest);
    }
    Ok(GenerationLayout {
        header,
        table,
        descriptors,
        data_start,
        total_len,
    })
}

fn write_generation(
    file: &mut File,
    generation_offset: u64,
    layout: &GenerationLayout,
    sections: &[SingleFileSectionInput<'_>],
) -> Result<[u8; 32], DurabilityError> {
    file.seek(SeekFrom::Start(generation_offset))?;
    let mut hasher = Sha256::new();
    write_hashed(file, &mut hasher, &layout.header)?;
    write_hashed(file, &mut hasher, &layout.table)?;
    let prefix_len = u64::try_from(GENERATION_HEADER_LEN + layout.table.len())
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    write_zeroes_hashed(file, &mut hasher, layout.data_start - prefix_len)?;
    let mut relative_cursor = layout.data_start;
    for (section, descriptor) in sections.iter().zip(layout.descriptors.iter()) {
        let relative_offset = descriptor.offset - generation_offset;
        write_zeroes_hashed(file, &mut hasher, relative_offset - relative_cursor)?;
        write_hashed(file, &mut hasher, section.bytes)?;
        relative_cursor = relative_offset
            .checked_add(descriptor.len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let aligned = align_up(relative_cursor, PAGE_SIZE)?;
        write_zeroes_hashed(file, &mut hasher, aligned - relative_cursor)?;
        relative_cursor = aligned;
    }
    if relative_cursor != layout.total_len {
        return Err(corruption("single-file generation writer length mismatch"));
    }
    Ok(hasher.finalize().into())
}

fn write_hashed(file: &mut File, hasher: &mut Sha256, bytes: &[u8]) -> Result<(), DurabilityError> {
    file.write_all(bytes)?;
    hasher.update(bytes);
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

fn write_root_slot(file: &mut File, root: RootRecord) -> Result<(), DurabilityError> {
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
    file.seek(SeekFrom::Start(root.slot.offset()))?;
    file.write_all(&page)?;
    Ok(())
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
        || page[ROOT_DIGEST_OFFSET + 32..]
            .iter()
            .any(|byte| *byte != 0)
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

fn roots_form_valid_transition(older: RootRecord, newer: RootRecord) -> bool {
    if older.generation == newer.generation {
        let same_generation = older.generation_len == newer.generation_len
            && older.journal_first_lsn == newer.journal_first_lsn
            && older.section_count == newer.section_count
            && older.generation_digest == newer.generation_digest
            && older.parent_digest == newer.parent_digest;
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
            && newer.generation_offset < older.generation_offset
            && newer.generation_offset == DATA_OFFSET
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

fn read_generation_view(
    file: &mut File,
    root: RootRecord,
) -> Result<SingleFileGenerationView, DurabilityError> {
    let header = read_generation_header(file, root)?;
    let table_len_usize =
        usize::try_from(header.table_len).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let mut table = vec![0_u8; table_len_usize];
    file.read_exact(&mut table)
        .map_err(|error| eof_as_corruption(error, "single-file section table is truncated"))?;
    let sections = decode_section_table(
        &table,
        header.section_count,
        header.data_start,
        header.total_len,
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
    total_len: u64,
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
    let expected_table_len = u64::try_from(
        section_count
            .checked_mul(SECTION_DESCRIPTOR_LEN)
            .ok_or(DurabilityError::PayloadTooLarge)?,
    )
    .map_err(|_| DurabilityError::PayloadTooLarge)?;
    if table_len != expected_table_len
        || data_start != align_up(GENERATION_HEADER_LEN as u64 + table_len, PAGE_SIZE)?
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
        total_len,
    })
}

fn decode_section_table(
    table: &[u8],
    section_count: usize,
    data_start: u64,
    total_len: u64,
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
        if end > total_len {
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
        if u64::try_from(section.bytes.len()).map_or(true, |len| len > MAX_SECTION_LEN) {
            return Err(DurabilityError::PayloadTooLarge);
        }
    }
    Ok(())
}

fn hash_file_range(file: &mut File, offset: u64, len: u64) -> Result<[u8; 32], DurabilityError> {
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
    use std::io::{Seek, SeekFrom, Write};
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
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Checkpoint,
                ordinal: 0,
                bytes: checkpoint,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Metadata,
                ordinal: 0,
                bytes: metadata,
            },
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
            }],
            &registry,
        )
        .unwrap()
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
        let layout =
            generation_layout(2, root.generation_digest, orphan_offset, &orphan_sections).unwrap();
        store.file.set_len(orphan_offset).unwrap();
        write_generation(&mut store.file, orphan_offset, &layout, &orphan_sections).unwrap();
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
        let layout =
            generation_layout(2, root.generation_digest, orphan_offset, &orphan_sections).unwrap();
        file.set_len(orphan_offset).unwrap();
        write_generation(&mut file, orphan_offset, &layout, &orphan_sections).unwrap();
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
    fn duplicate_section_keys_are_rejected_before_file_publication() {
        let path = test_file("duplicates");
        let duplicate = [
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Checkpoint,
                ordinal: 0,
                bytes: b"a",
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Checkpoint,
                ordinal: 0,
                bytes: b"b",
            },
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
