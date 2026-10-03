use super::checkpoint_storage::{
    DEFAULT_CHECKPOINT_CHUNK_SIZE, MAX_CHECKPOINT_LEN, checkpoint_chunk_count,
    read_checkpoint_generation, write_chunked_checkpoint_root,
};
use super::file_io::sync_directory;
use super::freshness::ExternalFreshnessState;
use super::generation_layout::{
    checkpoint_chunk_path, checkpoint_stream_spool_path, metadata_path, next_generation,
    prepared_capsule_path, realization_path, wal_path,
};
use super::manifest::{ManifestRecord, publish_manifest_with_hook};
use super::metadata_storage::{read_metadata_bytes_bounded, write_metadata_file};
use super::prepared_capsule::{
    PreparedCutCapsule, encode_prepared_cut_capsule, read_prepared_cut_capsule_file,
    write_prepared_cut_capsule,
};
use super::publication_protocol::{NoStoreFault, PublicationAttempt};
use super::realization_storage::{
    read_published_factorized_realization, write_factorized_realization_file,
};
use super::single_file_backend::{
    FactorizedRealizationSectionSource, MetadataSectionSource, RevisionSectionSource,
};
use super::{DurableGenerationReceipt, DurableRevisionStore, StreamingCheckpointProgress};
use crate::binary_codec::{crc32c, crc32c_update};
use crate::realization::DurableFactorizedRealization;
use crate::runtime::{CodecError, DurabilityError, TailStatus};
use crate::single_file::{
    CarriedWalPublication, HistoricalGenerationArchive, SingleFileSectionInput,
    SingleFileSectionKind,
};
use crate::wal::FileRevisionWal;
use crate::wal_frame::EncodedFrame;
use crate::{checkpoint, metadata};
use kernel_realization::{FactorizedRealizationRoot, PhysicalAtomStore};
use kernel_revision::Revision;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
enum StreamingCheckpointPhysical {
    Directory {
        metadata_crc32c: u32,
        capsule_crc32c: u32,
        shadow_wal: Box<FileRevisionWal>,
        physical_realization: Option<DurableFactorizedRealization>,
        physical_binding: Option<metadata::DurableCheckpointRealizationBinding>,
    },
    SingleFile {
        carry_start_offset: u64,
        metadata_record: Box<metadata::DurableStoreMetadata>,
        prepared_bytes: Vec<u8>,
        replication_cut_frames: usize,
        physical_realization: Option<DurableFactorizedRealization>,
    },
}

#[derive(Debug)]
struct CheckpointSpool {
    file: Option<File>,
    path: PathBuf,
}

impl CheckpointSpool {
    fn file_mut(&mut self) -> Result<&mut File, DurabilityError> {
        self.file.as_mut().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint canonical spool is already closed",
        })
    }
}

impl Drop for CheckpointSpool {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = fs::remove_file(&self.path);
    }
}

struct PreparedCheckpointEncoding {
    spool: Option<CheckpointSpool>,
    spool_digest: Option<[u8; 32]>,
    encoded_len: usize,
    chunk_crcs: Vec<u32>,
    precomputed_checkpoint_crc32c: Option<u32>,
}

fn replication_prefix(
    replication: &crate::replication::authority::ReplicationAuthorityJournal,
    count: usize,
) -> Result<&[Vec<u8>], DurabilityError> {
    replication.single_file_live_frames_prefix(count)
}

#[derive(Debug)]
pub(super) struct StreamingCheckpointJob {
    generation: u64,
    cut_revision: Revision,
    spool: Option<CheckpointSpool>,
    expected_spool_digest: Option<[u8; 32]>,
    spool_hasher: Option<Sha256>,
    encoded_len: usize,
    chunk_size: usize,
    chunk_crcs: Vec<u32>,
    precomputed_checkpoint_crc32c: Option<u32>,
    next_chunk: usize,
    checkpoint_crc32c: Option<u32>,
    prepared_capsule: PreparedCutCapsule,
    physical: Option<StreamingCheckpointPhysical>,
    wal_first_lsn: u64,
    mirrored_lsn: u64,
    durable_shadow_lsn: u64,
    failed: bool,
}

fn checkpoint_stream_summary(revision: &Revision) -> Result<(usize, u32), DurabilityError> {
    let mut encoded_len = 0_usize;
    let mut full_crc = !0_u32;
    checkpoint::stream_revision(revision, &mut |bytes| {
        encoded_len = encoded_len
            .checked_add(bytes.len())
            .ok_or(DurabilityError::PayloadTooLarge)?;
        full_crc = crc32c_update(full_crc, bytes);
        Ok(())
    })?;
    Ok((encoded_len, !full_crc))
}

fn prepare_checkpoint_encoding(
    revision: &Revision,
    chunk_size: usize,
    directory_spool: Option<(PathBuf, u64)>,
) -> Result<PreparedCheckpointEncoding, DurabilityError> {
    if directory_spool.is_none() {
        let (encoded_len, checkpoint_crc32c) = checkpoint_stream_summary(revision)?;
        if encoded_len > MAX_CHECKPOINT_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        checkpoint_chunk_count(encoded_len, chunk_size)?;
        return Ok(PreparedCheckpointEncoding {
            spool: None,
            spool_digest: None,
            encoded_len,
            chunk_crcs: Vec::new(),
            precomputed_checkpoint_crc32c: Some(checkpoint_crc32c),
        });
    }

    let encoded_len = usize::try_from(checkpoint::encoded_revision_len(revision)?)
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    if encoded_len > MAX_CHECKPOINT_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    checkpoint_chunk_count(encoded_len, chunk_size)?;

    let Some((directory, generation)) = directory_spool else {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "directory checkpoint encoding requires a canonical spool location",
        });
    };
    let path = checkpoint_stream_spool_path(&directory, generation);
    let mut file = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)?;
    let mut emitted = 0_usize;
    let mut hasher = Sha256::new();
    let stream_result = checkpoint::stream_revision(revision, &mut |bytes| {
        emitted = emitted
            .checked_add(bytes.len())
            .ok_or(DurabilityError::PayloadTooLarge)?;
        if emitted > encoded_len {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint canonical stream exceeded counted length",
            });
        }
        file.write_all(bytes)?;
        hasher.update(bytes);
        Ok(())
    });
    if let Err(error) = stream_result {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    if emitted != encoded_len {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint canonical stream disagrees with counted length",
        });
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(PreparedCheckpointEncoding {
        spool: Some(CheckpointSpool {
            file: Some(file),
            path,
        }),
        spool_digest: Some(hasher.finalize().into()),
        encoded_len,
        chunk_crcs: Vec::new(),
        precomputed_checkpoint_crc32c: None,
    })
}

fn write_spooled_checkpoint_chunk(
    spool: &mut File,
    output: &mut File,
    start: u64,
    len: usize,
    stream_hasher: &mut Sha256,
) -> Result<u32, DurabilityError> {
    spool.seek(SeekFrom::Start(start))?;
    let mut remaining = len;
    let mut crc = !0_u32;
    let mut buffer = vec![0_u8; 64 * 1024];
    while remaining != 0 {
        let take = remaining.min(buffer.len());
        spool.read_exact(&mut buffer[..take])?;
        output.write_all(&buffer[..take])?;
        crc = crc32c_update(crc, &buffer[..take]);
        stream_hasher.update(&buffer[..take]);
        remaining -= take;
    }
    Ok(!crc)
}

fn prepare_store_checkpoint_encoding(
    store: &DurableRevisionStore,
    revision: &Revision,
    chunk_size: usize,
) -> Result<(u64, PreparedCheckpointEncoding), DurabilityError> {
    let generation = if store.backend.is_single_file() {
        store
            .generation
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?
    } else {
        next_generation(store.backend.directory_root()?)?
    };
    let directory_spool = if store.backend.is_single_file() {
        None
    } else {
        Some((store.backend.directory_root()?.to_path_buf(), generation))
    };
    let encoding = prepare_checkpoint_encoding(revision, chunk_size, directory_spool)?;
    Ok((generation, encoding))
}

fn write_next_directory_checkpoint_chunk(
    job: &mut StreamingCheckpointJob,
    directory: &Path,
) -> Result<u32, DurabilityError> {
    let ordinal = job.next_chunk;
    let start = ordinal
        .checked_mul(job.chunk_size)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    let end = job.encoded_len.min(
        start
            .checked_add(job.chunk_size)
            .ok_or(DurabilityError::PayloadTooLarge)?,
    );
    let spool = job.spool.as_mut().ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "directory streaming checkpoint lost its canonical spool",
    })?;
    let stream_hasher = job.spool_hasher.as_mut().ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "directory streaming checkpoint lost its spool digest state",
    })?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(checkpoint_chunk_path(directory, job.generation, ordinal))?;
    let crc = write_spooled_checkpoint_chunk(
        spool.file_mut()?,
        &mut file,
        u64::try_from(start).map_err(|_| DurabilityError::PayloadTooLarge)?,
        end - start,
        stream_hasher,
    )?;
    file.sync_all()?;
    Ok(crc)
}

impl DurableRevisionStore {
    /// Captures an immutable authority cut and starts an unpublished chunked
    /// checkpoint generation. Commits may continue between subsequent chunk
    /// writes; their exact WAL frames are mirrored into the shadow WAL.
    pub fn begin_streaming_checkpoint(
        &mut self,
        revision: &Revision,
    ) -> Result<StreamingCheckpointProgress, DurabilityError> {
        self.begin_streaming_checkpoint_with_chunk_size(revision, DEFAULT_CHECKPOINT_CHUNK_SIZE)
    }

    pub fn begin_streaming_checkpoint_with_chunk_size(
        &mut self,
        revision: &Revision,
        chunk_size: usize,
    ) -> Result<StreamingCheckpointProgress, DurabilityError> {
        let physical = self
            .checkpoint_realization
            .as_ref()
            .filter(|physical| physical.revision() == revision.id())
            .cloned();
        self.begin_streaming_checkpoint_with_chunk_size_and_physical(revision, chunk_size, physical)
    }

    pub fn begin_streaming_checkpoint_with_factorized_realization(
        &mut self,
        revision: &Revision,
        atoms: &PhysicalAtomStore,
        root: &FactorizedRealizationRoot,
    ) -> Result<StreamingCheckpointProgress, DurabilityError> {
        self.begin_streaming_checkpoint_with_factorized_realization_and_chunk_size(
            revision,
            atoms,
            root,
            DEFAULT_CHECKPOINT_CHUNK_SIZE,
        )
    }

    pub fn begin_streaming_checkpoint_with_factorized_realization_and_chunk_size(
        &mut self,
        revision: &Revision,
        atoms: &PhysicalAtomStore,
        root: &FactorizedRealizationRoot,
        chunk_size: usize,
    ) -> Result<StreamingCheckpointProgress, DurabilityError> {
        let mut physical =
            DurableFactorizedRealization::new(revision.id(), atoms.clone(), root.clone())?;
        if let Some(previous) = self.checkpoint_realization.as_ref() {
            physical.inherit_retained_historical_roots(
                previous,
                self.checkpoint.semantic_context(),
                self.historical_epoch_anchors
                    .values()
                    .map(|anchor| (anchor.effect_id, anchor.source_revision)),
            )?;
        }
        self.begin_streaming_checkpoint_with_chunk_size_and_physical(
            revision,
            chunk_size,
            Some(physical),
        )
    }

    fn begin_streaming_checkpoint_with_chunk_size_and_physical(
        &mut self,
        revision: &Revision,
        chunk_size: usize,
        physical_realization: Option<DurableFactorizedRealization>,
    ) -> Result<StreamingCheckpointProgress, DurabilityError> {
        if !self.backend.capabilities().streaming_checkpoint {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durability backend does not support streaming checkpoints",
            });
        }
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.streaming_checkpoint.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "streaming checkpoint job already active",
            });
        }
        if chunk_size == 0 || revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "streaming checkpoint requires nonzero chunk size at durable head",
            });
        }
        self.wal
            .durability_barrier()
            .inspect_err(|_| self.poisoned = true)?;
        let wal_first_lsn = self.wal.next_lsn();
        let (planned_generation, encoding) =
            prepare_store_checkpoint_encoding(self, revision, chunk_size)?;
        let PreparedCheckpointEncoding {
            spool,
            spool_digest,
            encoded_len,
            chunk_crcs,
            precomputed_checkpoint_crc32c,
        } = encoding;
        let prepared_capsule = PreparedCutCapsule::from_prepared_transactions(
            &self.prepared_transactions,
            self.durable_head,
        );
        let mut metadata_record = self.streaming_metadata_record()?;
        let (generation, physical) = if self.backend.is_single_file() {
            let generation = planned_generation;
            let carry_start_offset = self.wal.current_end_offset()?;
            let prepared_bytes = encode_prepared_cut_capsule(&prepared_capsule)?;
            let replication_cut_frames = self.replication.single_file_live_frame_count();
            (
                generation,
                StreamingCheckpointPhysical::SingleFile {
                    carry_start_offset,
                    metadata_record: Box::new(metadata_record),
                    prepared_bytes,
                    replication_cut_frames,
                    physical_realization,
                },
            )
        } else {
            let directory = self.backend.directory_root()?.to_path_buf();
            let generation = planned_generation;
            let capsule_crc32c = write_prepared_cut_capsule(
                &prepared_capsule_path(&directory, generation),
                &prepared_capsule,
            )?;
            let physical_binding = physical_realization
                .as_ref()
                .map(|physical| {
                    write_factorized_realization_file(
                        &realization_path(&directory, generation),
                        physical,
                    )
                })
                .transpose()?;
            metadata_record.checkpoint_realization = physical_binding;
            let metadata_crc32c =
                write_metadata_file(&metadata_path(&directory, generation), &metadata_record)?;
            let mut shadow_wal =
                FileRevisionWal::create_at_lsn(wal_path(&directory, generation), wal_first_lsn)?;
            shadow_wal.durability_barrier()?;
            sync_directory(&directory)?;
            (
                generation,
                StreamingCheckpointPhysical::Directory {
                    metadata_crc32c,
                    capsule_crc32c,
                    shadow_wal: Box::new(shadow_wal),
                    physical_realization,
                    physical_binding,
                },
            )
        };
        self.streaming_checkpoint = Some(StreamingCheckpointJob {
            generation,
            cut_revision: revision.clone(),
            spool,
            expected_spool_digest: spool_digest,
            spool_hasher: spool_digest.map(|_| Sha256::new()),
            encoded_len,
            chunk_size,
            chunk_crcs,
            precomputed_checkpoint_crc32c,
            next_chunk: 0,
            checkpoint_crc32c: None,
            prepared_capsule,
            physical: Some(physical),
            wal_first_lsn,
            mirrored_lsn: wal_first_lsn - 1,
            durable_shadow_lsn: wal_first_lsn - 1,
            failed: false,
        });
        self.streaming_checkpoint_progress()
    }

    fn streaming_metadata_record(&self) -> Result<metadata::DurableStoreMetadata, DurabilityError> {
        Ok(metadata::DurableStoreMetadata {
            external_freshness: self
                .external_freshness
                .as_ref()
                .map(ExternalFreshnessState::metadata_binding),
            current_idempotency_epoch: self.current_idempotency_epoch,
            minimum_retry_epoch: self.minimum_retry_epoch,
            materializations: self.materialization_specs.clone(),
            physical_artifacts: self.physical_artifact_specs.clone(),
            artifact_cores: self.artifact_cores.clone(),
            migration_complements: self.migration_complements.clone(),
            historical_epoch_anchors: self.historical_epoch_anchors.clone(),
            committed_transactions: self.committed_transactions.clone(),
            semantic_modules: self.semantic_registry.builtin_module_specs(),
            next_revision_effect_id: self.next_revision_effect_id,
            causal_coverage_root: Some(self.causal_coverage_root),
            revision_effects: self.revision_effects.clone(),
            revision_effect_frontiers: self.revision_effect_frontiers.clone(),
            checkpoint_realization: None,
        })
    }

    pub fn write_streaming_checkpoint_chunks(
        &mut self,
        max_chunks: usize,
    ) -> Result<StreamingCheckpointProgress, DurabilityError> {
        if max_chunks == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "streaming checkpoint chunk budget must be nonzero",
            });
        }
        let directory = if self.backend.is_single_file() {
            None
        } else {
            Some(self.backend.directory_root()?.to_path_buf())
        };
        let job = self
            .streaming_checkpoint
            .as_mut()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "no streaming checkpoint job is active",
            })?;
        if job.failed {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "streaming checkpoint job has failed",
            });
        }
        let total = job.encoded_len.div_ceil(job.chunk_size);
        let end_chunk = total.min(job.next_chunk.saturating_add(max_chunks));
        if let Some(directory) = &directory {
            while job.next_chunk < end_chunk {
                let crc = match write_next_directory_checkpoint_chunk(job, directory) {
                    Ok(crc) => crc,
                    Err(error) => {
                        job.failed = true;
                        return Err(error);
                    }
                };
                job.chunk_crcs.push(crc);
                job.next_chunk += 1;
            }
        } else {
            job.next_chunk = end_chunk;
        }
        if job.next_chunk == total && job.checkpoint_crc32c.is_none() {
            if let Some(expected) = job.expected_spool_digest {
                let actual: [u8; 32] = job
                    .spool_hasher
                    .as_ref()
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "directory streaming checkpoint lost its spool digest state",
                    })?
                    .clone()
                    .finalize()
                    .into();
                if actual != expected {
                    job.failed = true;
                    return Err(DurabilityError::Corruption {
                        offset: 0,
                        reason: "checkpoint canonical spool changed during resumable publication",
                    });
                }
            }
            if let Some(directory) = &directory {
                job.checkpoint_crc32c = Some(write_chunked_checkpoint_root(
                    directory,
                    job.generation,
                    job.cut_revision.id(),
                    job.encoded_len,
                    &job.chunk_crcs,
                    job.chunk_size,
                )?);
                sync_directory(directory)?;
            } else {
                job.checkpoint_crc32c = job.precomputed_checkpoint_crc32c;
            }
        }
        self.streaming_checkpoint_progress()
    }

    pub fn finalize_streaming_checkpoint(
        &mut self,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.wal
            .durability_barrier()
            .inspect_err(|_| self.poisoned = true)?;
        self.barrier_streaming_shadow();
        let checkpoint_crc32c = self.ready_streaming_checkpoint_crc32c()?;
        let mut job = self
            .streaming_checkpoint
            .take()
            .expect("streaming checkpoint presence was validated above");
        let physical = job
            .physical
            .take()
            .expect("streaming checkpoint physical state is present");
        match physical {
            StreamingCheckpointPhysical::Directory {
                metadata_crc32c,
                capsule_crc32c,
                shadow_wal,
                physical_realization,
                physical_binding,
            } => self.finalize_directory_streaming_checkpoint(
                job,
                checkpoint_crc32c,
                metadata_crc32c,
                capsule_crc32c,
                *shadow_wal,
                physical_realization,
                physical_binding,
            ),
            StreamingCheckpointPhysical::SingleFile {
                carry_start_offset,
                metadata_record,
                prepared_bytes,
                replication_cut_frames,
                physical_realization,
            } => self.finalize_single_file_streaming_checkpoint(
                job,
                carry_start_offset,
                &metadata_record,
                &prepared_bytes,
                replication_cut_frames,
                physical_realization,
            ),
        }
    }

    fn finalize_directory_streaming_checkpoint(
        &mut self,
        job: StreamingCheckpointJob,
        checkpoint_crc32c: u32,
        metadata_crc32c: u32,
        capsule_crc32c: u32,
        mut shadow_wal: FileRevisionWal,
        physical_realization: Option<DurableFactorizedRealization>,
        physical_binding: Option<metadata::DurableCheckpointRealizationBinding>,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let directory = self.backend.directory_root()?.to_path_buf();
        let seeds = job.prepared_capsule.scan_seeds();
        let (scan, shadow_len) =
            shadow_wal.scan_recovery_seeded(job.cut_revision.id(), job.wal_first_lsn, &seeds)?;
        let last_good =
            u64::try_from(scan.last_good_offset()).map_err(|_| CodecError::LengthOverflow)?;
        if scan.durable_revision() != self.durable_head
            || scan.next_lsn() <= job.mirrored_lsn
            || !matches!(scan.tail_status(), TailStatus::Clean)
            || last_good != shadow_len
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint cut plus shadow WAL does not recover exact publish endpoint",
            });
        }
        let candidate_manifest = ManifestRecord {
            generation: job.generation,
            base_revision: job.cut_revision.id(),
            published_head: self.durable_head,
            wal_first_lsn: job.wal_first_lsn,
            published_tail_lsn: job.mirrored_lsn,
            checkpoint_crc32c,
            metadata_crc32c,
            prepared_capsule_crc32c: capsule_crc32c,
        };
        let verified_cut =
            read_checkpoint_generation(&directory, candidate_manifest, &self.semantic_registry)?;
        if verified_cut.id() != job.cut_revision.id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "streaming checkpoint root does not decode to pinned cut",
            });
        }
        let metadata_bytes =
            read_metadata_bytes_bounded(&metadata_path(&directory, job.generation))?;
        if crc32c(&metadata_bytes) != metadata_crc32c {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "streaming checkpoint metadata changed before publication",
            });
        }
        let _ = read_prepared_cut_capsule_file(
            &prepared_capsule_path(&directory, job.generation),
            capsule_crc32c,
        )?;
        if let Some(binding) = physical_binding {
            let verified =
                read_published_factorized_realization(&directory, job.generation, binding)?;
            if verified.revision() != job.cut_revision.id() {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "streaming durable realization does not match pinned checkpoint cut",
                });
            }
        }
        sync_directory(&directory)?;
        let mut publication = PublicationAttempt::default();
        if let Err(error) = publish_manifest_with_hook(
            &directory,
            candidate_manifest,
            &mut NoStoreFault,
            &mut publication,
        ) {
            if publication.requires_recovery_after_error() {
                self.poisoned = true;
            }
            return Err(error);
        }

        let freshness_digest = shadow_wal.freshness_digest();
        let published_generation = job.generation;
        let published_tail_lsn = job.mirrored_lsn;
        self.prepared_transactions
            .retain_published_generation(job.wal_first_lsn, job.prepared_capsule.prepare_lsns());
        self.generation = job.generation;
        self.checkpoint = job.cut_revision;
        self.checkpoint_realization = physical_realization;
        self.wal = shadow_wal;
        self.advance_external_freshness_generation_with_digest(
            published_generation,
            published_tail_lsn,
            freshness_digest,
        )?;
        Ok(DurableGenerationReceipt {
            generation: self.generation,
            base_revision: self.checkpoint.id(),
        })
    }

    fn finalize_single_file_streaming_checkpoint(
        &mut self,
        job: StreamingCheckpointJob,
        carry_start_offset: u64,
        metadata_record: &metadata::DurableStoreMetadata,
        prepared_bytes: &[u8],
        replication_cut_frames: usize,
        physical_realization: Option<DurableFactorizedRealization>,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let seeds = job.prepared_capsule.scan_seeds();
        let active_end = self.wal.current_end_offset()?;
        let scan = self.wal.scan_subregion_seeded(
            carry_start_offset,
            active_end,
            job.cut_revision.id(),
            job.wal_first_lsn,
            &seeds,
        )?;
        let carried_len = active_end
            .checked_sub(carry_start_offset)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let last_good =
            u64::try_from(scan.last_good_offset()).map_err(|_| CodecError::LengthOverflow)?;
        if scan.durable_revision() != self.durable_head
            || scan.next_lsn() != self.wal.next_lsn()
            || !matches!(scan.tail_status(), TailStatus::Clean)
            || last_good != carried_len
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint cut plus carried WAL does not recover exact publish endpoint",
            });
        }
        let checkpoint_source = RevisionSectionSource(&job.cut_revision);
        let metadata_source = MetadataSectionSource(metadata_record);
        let replication_frames = replication_prefix(&self.replication, replication_cut_frames)?;
        let retained_historical_generations =
            self.pinned_historical_generations_for(physical_realization.as_ref());
        let archive_outgoing = retained_historical_generations
            .contains(&self.generation)
            .then_some(HistoricalGenerationArchive {
                generation: self.generation,
                checkpoint_revision: self.checkpoint.id(),
                durable_head: self.durable_head,
            });
        let mut sections = vec![
            SingleFileSectionInput::streaming(
                SingleFileSectionKind::Checkpoint,
                0,
                &checkpoint_source,
            ),
            SingleFileSectionInput::streaming(SingleFileSectionKind::Metadata, 0, &metadata_source),
            SingleFileSectionInput::bytes(
                SingleFileSectionKind::PreparedCapsule,
                0,
                prepared_bytes,
            ),
        ];
        let physical_source = physical_realization
            .as_ref()
            .map(FactorizedRealizationSectionSource);
        if let Some(source) = physical_source.as_ref() {
            sections.push(SingleFileSectionInput::streaming(
                SingleFileSectionKind::PhysicalArtifact,
                0,
                source,
            ));
        }
        let (backend, wal) = (&mut self.backend, &mut self.wal);
        let container = backend.single_file_container()?;
        let view = container
            .publish_generation_with_carried_wal(
                wal,
                CarriedWalPublication {
                    start_offset: carry_start_offset,
                    first_lsn: job.wal_first_lsn,
                    base_revision: job.cut_revision.id(),
                    expected_durable_revision: self.durable_head,
                    seeded_prepares: &seeds,
                },
                &sections,
                replication_frames,
                archive_outgoing,
                &retained_historical_generations,
            )
            .inspect_err(|_| self.poisoned = true)?;
        let (wal, reopened_scan) = container
            .open_journal_recovered(job.cut_revision.id(), view.journal_first_lsn, &seeds)
            .inspect_err(|_| self.poisoned = true)?;
        if reopened_scan.durable_revision() != self.durable_head
            || reopened_scan.next_lsn() != scan.next_lsn()
            || !matches!(reopened_scan.tail_status(), TailStatus::Clean)
        {
            self.poisoned = true;
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "published carried WAL does not reopen at certified endpoint",
            });
        }
        self.prepared_transactions
            .retain_published_generation(job.wal_first_lsn, job.prepared_capsule.prepare_lsns());
        self.generation = view.generation;
        self.checkpoint = job.cut_revision;
        self.checkpoint_realization = physical_realization;
        self.wal = wal;
        self.replication
            .advance_single_file_generation_prefix(replication_cut_frames);
        let freshness_digest = self.wal.freshness_digest();
        self.advance_external_freshness_generation_with_digest(
            self.generation,
            self.wal.last_lsn(),
            freshness_digest,
        )?;
        Ok(DurableGenerationReceipt {
            generation: self.generation,
            base_revision: self.checkpoint.id(),
        })
    }

    fn ready_streaming_checkpoint_crc32c(&self) -> Result<u32, DurabilityError> {
        let job = self
            .streaming_checkpoint
            .as_ref()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "no streaming checkpoint job is active",
            })?;
        let checkpoint_crc32c = job.checkpoint_crc32c.ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "streaming checkpoint chunks are not complete",
        })?;
        if job.failed
            || job.mirrored_lsn != self.wal.last_lsn()
            || job.durable_shadow_lsn != job.mirrored_lsn
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "streaming WAL projection is not durably caught up to active WAL",
            });
        }
        Ok(checkpoint_crc32c)
    }

    #[must_use]
    pub fn has_streaming_checkpoint(&self) -> bool {
        self.streaming_checkpoint.is_some()
    }

    fn streaming_checkpoint_progress(
        &self,
    ) -> Result<StreamingCheckpointProgress, DurabilityError> {
        let job = self
            .streaming_checkpoint
            .as_ref()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "no streaming checkpoint job is active",
            })?;
        let total = job.encoded_len.div_ceil(job.chunk_size);
        Ok(StreamingCheckpointProgress {
            generation: job.generation,
            chunks_written: u32::try_from(job.next_chunk)
                .map_err(|_| DurabilityError::PayloadTooLarge)?,
            chunks_total: u32::try_from(total).map_err(|_| DurabilityError::PayloadTooLarge)?,
            mirrored_lsn: job.mirrored_lsn,
            durable_shadow_lsn: job.durable_shadow_lsn,
            ready_to_publish: job.checkpoint_crc32c.is_some() && !job.failed,
        })
    }

    pub(super) fn mirror_streaming_frame(&mut self, frame: &EncodedFrame) {
        let Some(job) = &mut self.streaming_checkpoint else {
            return;
        };
        if job.failed {
            return;
        }
        match job
            .physical
            .as_mut()
            .expect("streaming checkpoint physical state is present")
        {
            StreamingCheckpointPhysical::Directory { shadow_wal, .. } => {
                if shadow_wal.append_exact_frame(frame).is_err() {
                    job.failed = true;
                    return;
                }
            }
            StreamingCheckpointPhysical::SingleFile { .. } => {}
        }
        job.mirrored_lsn = frame.lsn;
    }

    pub(super) fn barrier_streaming_shadow(&mut self) {
        let single_file = self.streaming_checkpoint.as_ref().is_some_and(|job| {
            matches!(
                job.physical.as_ref(),
                Some(StreamingCheckpointPhysical::SingleFile { .. })
            )
        });
        if single_file && self.wal.durability_barrier().is_err() {
            if let Some(job) = &mut self.streaming_checkpoint {
                job.failed = true;
            }
            return;
        }
        let active_last_lsn = self.wal.last_lsn();
        let Some(job) = &mut self.streaming_checkpoint else {
            return;
        };
        if job.failed {
            return;
        }
        match job
            .physical
            .as_mut()
            .expect("streaming checkpoint physical state is present")
        {
            StreamingCheckpointPhysical::Directory { shadow_wal, .. } => {
                if job.durable_shadow_lsn == job.mirrored_lsn {
                    return;
                }
                if shadow_wal.durability_barrier().is_err() {
                    job.failed = true;
                    return;
                }
            }
            StreamingCheckpointPhysical::SingleFile { .. } => {
                job.mirrored_lsn = active_last_lsn;
            }
        }
        job.durable_shadow_lsn = job.mirrored_lsn;
    }

    #[cfg(test)]
    pub(super) fn test_mark_streaming_checkpoint_failed(&mut self) {
        self.streaming_checkpoint
            .as_mut()
            .expect("test requires active streaming checkpoint")
            .failed = true;
    }
}
