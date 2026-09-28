use super::checkpoint_storage::{
    DEFAULT_CHECKPOINT_CHUNK_SIZE, MAX_CHECKPOINT_LEN, checkpoint_chunk_count,
    read_checkpoint_generation, write_chunked_checkpoint_root,
};
use super::file_io::sync_directory;
use super::freshness::ExternalFreshnessState;
use super::generation_layout::{
    checkpoint_chunk_path, metadata_path, next_generation, prepared_capsule_path, wal_path,
};
use super::manifest::{ManifestRecord, publish_manifest_with_hook};
use super::metadata_storage::{read_metadata_bytes_bounded, write_metadata_file};
use super::prepared_capsule::{
    PreparedCutCapsule, encode_prepared_cut_capsule, read_prepared_cut_capsule_file,
    write_prepared_cut_capsule,
};
use super::publication_protocol::{NoStoreFault, PublicationAttempt};
use super::{DurableGenerationReceipt, DurableRevisionStore, StreamingCheckpointProgress};
use crate::binary_codec::crc32c;
use crate::runtime::{CodecError, DurabilityError, TailStatus};
use crate::single_file::{CarriedWalPublication, SingleFileSectionInput, SingleFileSectionKind};
use crate::wal::FileRevisionWal;
use crate::wal_frame::EncodedFrame;
use crate::{checkpoint, metadata};
use kernel_revision::Revision;
use std::fs::OpenOptions;
use std::io::Write;

#[derive(Debug)]
enum StreamingCheckpointPhysical {
    Directory {
        metadata_crc32c: u32,
        capsule_crc32c: u32,
        shadow_wal: FileRevisionWal,
    },
    SingleFile {
        carry_start_offset: u64,
        metadata_bytes: Vec<u8>,
        prepared_bytes: Vec<u8>,
        replication_archive: Vec<u8>,
        replication_cut_frames: usize,
    },
}

#[derive(Debug)]
pub(super) struct StreamingCheckpointJob {
    generation: u64,
    cut_revision: Revision,
    payload: Vec<u8>,
    chunk_size: usize,
    chunk_crcs: Vec<u32>,
    next_chunk: usize,
    checkpoint_crc32c: Option<u32>,
    prepared_capsule: PreparedCutCapsule,
    physical: Option<StreamingCheckpointPhysical>,
    wal_first_lsn: u64,
    mirrored_lsn: u64,
    durable_shadow_lsn: u64,
    failed: bool,
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
        let payload = checkpoint::encode_revision(revision)?;
        if payload.len() > MAX_CHECKPOINT_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        checkpoint_chunk_count(payload.len(), chunk_size)?;
        let prepared_capsule = PreparedCutCapsule::from_prepared_transactions(
            &self.prepared_transactions,
            self.durable_head,
        );
        let metadata_record = self.streaming_metadata_record(revision)?;
        let (generation, physical) = if self.backend.is_single_file() {
            let generation = self
                .generation
                .checked_add(1)
                .ok_or(DurabilityError::LsnExhausted)?;
            let carry_start_offset = self.wal.current_end_offset()?;
            let metadata_bytes = metadata::encode(&metadata_record)?;
            let prepared_bytes = encode_prepared_cut_capsule(&prepared_capsule)?;
            let replication_cut_frames = self.replication.single_file_live_frame_count();
            let replication_archive =
                self.single_file_replication_archive_prefix(replication_cut_frames)?;
            (
                generation,
                StreamingCheckpointPhysical::SingleFile {
                    carry_start_offset,
                    metadata_bytes,
                    prepared_bytes,
                    replication_archive,
                    replication_cut_frames,
                },
            )
        } else {
            let directory = self.backend.directory_root()?.to_path_buf();
            let generation = next_generation(&directory)?;
            let capsule_crc32c = write_prepared_cut_capsule(
                &prepared_capsule_path(&directory, generation),
                &prepared_capsule,
            )?;
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
                    shadow_wal,
                },
            )
        };
        self.streaming_checkpoint = Some(StreamingCheckpointJob {
            generation,
            cut_revision: revision.clone(),
            payload,
            chunk_size,
            chunk_crcs: Vec::new(),
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

    fn streaming_metadata_record(
        &self,
        revision: &Revision,
    ) -> Result<metadata::DurableStoreMetadata, DurabilityError> {
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
            committed_transactions: self.committed_transactions.clone(),
            semantic_modules: self
                .semantic_registry
                .builtin_modules_for_context(revision.semantic_context())
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "checkpoint revision requires unavailable semantic implementation",
                })?,
            causal_coverage_root: Some(self.causal_coverage_root),
            revision_effects: self.revision_effects.clone(),
            revision_effect_frontiers: self.revision_effect_frontiers.clone(),
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
        let total = job.payload.len().div_ceil(job.chunk_size);
        let end_chunk = total.min(job.next_chunk.saturating_add(max_chunks));
        while job.next_chunk < end_chunk {
            let ordinal = job.next_chunk;
            let start = ordinal * job.chunk_size;
            let end = job.payload.len().min(start + job.chunk_size);
            let chunk = &job.payload[start..end];
            if let Some(directory) = &directory {
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(checkpoint_chunk_path(directory, job.generation, ordinal))?;
                file.write_all(chunk)?;
                file.sync_all()?;
            }
            job.chunk_crcs.push(crc32c(chunk));
            job.next_chunk += 1;
        }
        if job.next_chunk == total && job.checkpoint_crc32c.is_none() {
            if let Some(directory) = &directory {
                job.checkpoint_crc32c = Some(write_chunked_checkpoint_root(
                    directory,
                    job.generation,
                    job.cut_revision.id(),
                    job.payload.len(),
                    &job.chunk_crcs,
                    job.chunk_size,
                )?);
                sync_directory(directory)?;
            } else {
                job.checkpoint_crc32c = Some(crc32c(&job.payload));
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
            } => self.finalize_directory_streaming_checkpoint(
                job,
                checkpoint_crc32c,
                metadata_crc32c,
                capsule_crc32c,
                shadow_wal,
            ),
            StreamingCheckpointPhysical::SingleFile {
                carry_start_offset,
                metadata_bytes,
                prepared_bytes,
                replication_archive,
                replication_cut_frames,
            } => self.finalize_single_file_streaming_checkpoint(
                job,
                carry_start_offset,
                &metadata_bytes,
                &prepared_bytes,
                &replication_archive,
                replication_cut_frames,
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
        metadata_bytes: &[u8],
        prepared_bytes: &[u8],
        replication_archive: &[u8],
        replication_cut_frames: usize,
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
        let verified_cut = checkpoint::decode_revision(&job.payload, &self.semantic_registry)?;
        if verified_cut.id() != job.cut_revision.id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file streaming checkpoint does not decode to pinned cut",
            });
        }
        let sections = [
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Checkpoint,
                ordinal: 0,
                bytes: &job.payload,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::Metadata,
                ordinal: 0,
                bytes: metadata_bytes,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::PreparedCapsule,
                ordinal: 0,
                bytes: prepared_bytes,
            },
            SingleFileSectionInput {
                kind: SingleFileSectionKind::ReplicationAuthority,
                ordinal: 0,
                bytes: replication_archive,
            },
        ];
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
        let total = job.payload.len().div_ceil(job.chunk_size);
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
