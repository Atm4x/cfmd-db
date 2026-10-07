use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use kernel_auth::AuthorityDigest;
use kernel_types::RevisionId;

use super::freshness::{
    FreshnessGenerationMaterial, FreshnessRecoveryMaterial, WalFreshnessSource,
    generation_material_digest,
};
use super::generation_layout::wal_path;
use super::manifest::read_current_manifest;
use super::metadata_storage::read_published_metadata;
use super::prepared_capsule::{PreparedCutCapsule, decode_prepared_cut_capsule};
use super::publication_protocol::{StoreFaultHook, StoreFaultPoint};
use crate::metadata;
use crate::runtime::DurabilityError;
use crate::single_file::{
    SingleFileContainer, SingleFileSectionKind, compaction_io::SingleFileCompactionIo,
};
use crate::storage_encryption::StorageEncryption;
use crate::wal::RuntimeRevisionWal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DurabilityBackendCapabilities {
    pub(super) streaming_checkpoint: bool,
    pub(super) external_freshness: bool,
    pub(super) physical_compaction: bool,
}

#[derive(Debug)]
pub(super) struct DirectoryDurabilityBackend {
    root: PathBuf,
    _lock: File,
}

#[derive(Debug)]
pub(super) struct SingleFileDurabilityBackend {
    container: SingleFileContainer,
}

#[derive(Debug)]
pub(super) struct VolatileDurabilityBackend {
    protection_floor: crate::storage_encryption::StorageProtectionProfile,
}

/// Owns physical durability resources. Semantic store code must not infer
/// durability rules from paths or optional sidecars; layout-specific access is
/// centralized here and explicit.
#[derive(Debug)]
pub(super) enum DurabilityBackend {
    Directory(DirectoryDurabilityBackend),
    SingleFile(Box<SingleFileDurabilityBackend>),
    Volatile(VolatileDurabilityBackend),
}

impl DurabilityBackend {
    pub(super) fn directory(root: PathBuf, lock: File) -> Self {
        Self::Directory(DirectoryDurabilityBackend { root, _lock: lock })
    }

    pub(super) fn single_file(container: SingleFileContainer) -> Self {
        Self::SingleFile(Box::new(SingleFileDurabilityBackend { container }))
    }

    pub(super) const fn volatile(
        protection_floor: crate::storage_encryption::StorageProtectionProfile,
    ) -> Self {
        Self::Volatile(VolatileDurabilityBackend { protection_floor })
    }

    pub(super) const fn capabilities(&self) -> DurabilityBackendCapabilities {
        match self {
            Self::Directory(_) | Self::SingleFile(_) => DurabilityBackendCapabilities {
                streaming_checkpoint: true,
                external_freshness: true,
                physical_compaction: true,
            },
            Self::Volatile(_) => DurabilityBackendCapabilities {
                streaming_checkpoint: false,
                external_freshness: false,
                physical_compaction: false,
            },
        }
    }

    pub(super) fn historical_directory_root(&self) -> Result<&Path, DurabilityError> {
        match self {
            Self::Directory(backend) => Ok(&backend.root),
            Self::SingleFile(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "historical epoch materialization is not yet representable by the single-file backend",
            }),
            Self::Volatile(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile backend has no historical directory root",
            }),
        }
    }

    pub(super) const fn is_single_file(&self) -> bool {
        matches!(self, Self::SingleFile(_))
    }

    pub(super) const fn is_volatile(&self) -> bool {
        matches!(self, Self::Volatile(_))
    }

    pub(super) fn path(&self) -> Option<&Path> {
        match self {
            Self::Directory(backend) => Some(&backend.root),
            Self::SingleFile(backend) => Some(backend.container.path()),
            Self::Volatile(_) => None,
        }
    }

    pub(super) fn protection_profile(&self) -> crate::storage_encryption::StorageProtectionProfile {
        match self {
            Self::Directory(_) => crate::storage_encryption::StorageProtectionProfile::Unencrypted,
            Self::SingleFile(backend) => backend.container.protection_profile(),
            Self::Volatile(backend) => backend.protection_floor,
        }
    }

    pub(super) fn directory_root(&self) -> Result<&Path, DurabilityError> {
        match self {
            Self::Directory(backend) => Ok(&backend.root),
            Self::SingleFile(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "directory-only durability operation requested for single-file backend",
            }),
            Self::Volatile(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "directory-only durability operation requested for volatile backend",
            }),
        }
    }

    pub(super) fn single_file_container(
        &mut self,
    ) -> Result<&mut SingleFileContainer, DurabilityError> {
        match self {
            Self::SingleFile(backend) => Ok(&mut backend.container),
            Self::Directory(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file durability operation requested for directory backend",
            }),
            Self::Volatile(_) => Err(DurabilityError::Protocol {
                offset: 0,
                reason: "single-file durability operation requested for volatile backend",
            }),
        }
    }
    pub(super) fn persist_replication_frames(
        &mut self,
        wal: &mut RuntimeRevisionWal,
        frames: &[Vec<u8>],
    ) -> Result<(), DurabilityError> {
        match self {
            Self::Directory(_) => {
                if frames.is_empty() {
                    Ok(())
                } else {
                    Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "directory replication backend produced captured single-file frames",
                    })
                }
            }
            Self::SingleFile(_) => {
                for frame in frames {
                    wal.append_replication_authority_frame(frame)?;
                }
                if !frames.is_empty() {
                    wal.durability_barrier()?;
                }
                Ok(())
            }
            Self::Volatile(_) => {
                for frame in frames {
                    wal.append_replication_authority_frame(frame)?;
                }
                wal.durability_barrier()
            }
        }
    }

    pub(super) fn freshness_generation_material(
        &mut self,
        expected_generation: u64,
    ) -> Result<FreshnessGenerationMaterial, DurabilityError> {
        let material = match self {
            Self::Directory(backend) => directory_freshness_material(&backend.root)?.generation,
            Self::SingleFile(backend) => {
                single_file_freshness_material(&mut backend.container)?.generation
            }
            Self::Volatile(_) => {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness is not a volatile-backend operation",
                });
            }
        };
        if material.generation != expected_generation {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "freshness material generation does not match durable store generation",
            });
        }
        Ok(material)
    }

    pub(super) fn probe_directory_freshness(
        root: &Path,
    ) -> Result<FreshnessRecoveryMaterial, DurabilityError> {
        directory_freshness_material(root)
    }

    pub(super) fn probe_single_file_freshness_with_encryption(
        path: &Path,
        encryption: &StorageEncryption,
    ) -> Result<FreshnessRecoveryMaterial, DurabilityError> {
        let mut container = SingleFileContainer::open_with_encryption(path, encryption)?;
        single_file_freshness_material(&mut container)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    pub(super) fn compact_obsolete_generations(
        &mut self,
        wal: &mut RuntimeRevisionWal,
        generation: u64,
        checkpoint_revision: RevisionId,
        durable_head: RevisionId,
        pinned_historical_generations: &BTreeSet<u64>,
        hook: &mut impl StoreFaultHook,
        compaction_io: &mut impl SingleFileCompactionIo,
    ) -> Result<(), DurabilityError> {
        match self {
            Self::Directory(backend) => {
                for entry in fs::read_dir(&backend.root)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else {
                        continue;
                    };
                    let obsolete_generation =
                        super::generation_layout::parse_generation_name(name, "manifest-", ".cfmf")
                            .or_else(|| {
                                super::generation_layout::parse_generation_name(
                                    name,
                                    "checkpoint-",
                                    ".cfcp",
                                )
                            })
                            .or_else(|| {
                                super::generation_layout::parse_generation_name(
                                    name, "wal-", ".cfmw",
                                )
                            })
                            .or_else(|| {
                                super::generation_layout::parse_generation_name(
                                    name,
                                    "metadata-",
                                    ".cfdm",
                                )
                            })
                            .or_else(|| {
                                super::generation_layout::parse_generation_name(
                                    name,
                                    "prepared-",
                                    ".cfpc",
                                )
                            })
                            .or_else(|| {
                                super::generation_layout::parse_generation_name(
                                    name,
                                    "realization-",
                                    ".cfpr",
                                )
                            })
                            .or_else(|| {
                                super::generation_layout::parse_checkpoint_chunk_generation(name)
                            })
                            .or_else(|| {
                                super::generation_layout::parse_generation_name(
                                    name,
                                    "checkpoint-",
                                    "-stream.tmp",
                                )
                            });
                    let is_pending = super::generation_layout::parse_generation_name(
                        name,
                        "pending-manifest-",
                        ".tmp",
                    )
                    .is_some();
                    let removable_obsolete = obsolete_generation.is_some_and(|candidate| {
                        candidate != generation
                            && !pinned_historical_generations.contains(&candidate)
                    });
                    if is_pending || removable_obsolete {
                        hook.hit(StoreFaultPoint::BeforeCompactionRemove)?;
                        fs::remove_file(entry.path())?;
                        hook.hit(StoreFaultPoint::AfterCompactionRemove)?;
                    }
                }
                super::file_io::sync_directory(&backend.root)?;
                hook.hit(StoreFaultPoint::AfterCompactionDirectorySync)?;
                Ok(())
            }
            Self::SingleFile(backend) => {
                for &pinned in pinned_historical_generations {
                    if pinned != generation
                        && !backend.container.has_historical_epoch_archive(pinned)?
                    {
                        return Err(DurabilityError::Protocol {
                            offset: 0,
                            reason: "single-file compaction is missing archived historical epoch authority",
                        });
                    }
                }
                if backend.container.generation() != generation {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "single-file compaction generation does not match durable store",
                    });
                }
                let prepared = backend
                    .container
                    .read_section(SingleFileSectionKind::PreparedCapsule, 0)?
                    .map_or_else(
                        || Ok(PreparedCutCapsule::default()),
                        |bytes| decode_prepared_cut_capsule(&bytes),
                    )?;
                let seeds = prepared.scan_seeds();
                let mut single_file_fault =
                    |step| hook.hit(StoreFaultPoint::SingleFileCompaction(step));
                let _ = backend.container.compact_active_generation(
                    wal.file_mut()?,
                    checkpoint_revision,
                    &seeds,
                    durable_head,
                    &mut single_file_fault,
                    compaction_io,
                )?;
                Ok(())
            }
            Self::Volatile(_) => Ok(()),
        }
    }
}

fn directory_freshness_material(root: &Path) -> Result<FreshnessRecoveryMaterial, DurabilityError> {
    let manifest = read_current_manifest(root)?;
    let metadata = read_published_metadata(root, manifest)?;
    let binding = metadata
        .external_freshness
        .ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "published generation is missing external freshness binding",
        })?;
    let path = wal_path(root, manifest.generation);
    let end_offset = fs::metadata(&path)?.len();
    Ok(FreshnessRecoveryMaterial {
        generation: FreshnessGenerationMaterial {
            generation: manifest.generation,
            binding,
            generation_digest: generation_material_digest(root, manifest.generation)?,
        },
        wal: WalFreshnessSource {
            path,
            start_offset: 0,
            end_offset,
            first_lsn: manifest.wal_first_lsn,
        },
    })
}

fn single_file_freshness_material(
    container: &mut SingleFileContainer,
) -> Result<FreshnessRecoveryMaterial, DurabilityError> {
    let metadata = container
        .with_section_reader(SingleFileSectionKind::Metadata, 0, |reader, len| {
            metadata::decode_from_reader(reader, len)
                .map_err(|reason| DurabilityError::Corruption { offset: 0, reason })
        })?
        .ok_or(DurabilityError::Corruption {
            offset: 0,
            reason: "single-file durable metadata section is missing",
        })?;
    let binding = metadata
        .external_freshness
        .ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "published generation is missing external freshness binding",
        })?;
    let view = container.generation_view()?;
    let end_offset = view
        .journal_end
        .unwrap_or(fs::metadata(container.path())?.len());
    Ok(FreshnessRecoveryMaterial {
        generation: FreshnessGenerationMaterial {
            generation: view.generation,
            binding,
            generation_digest: AuthorityDigest(view.generation_digest),
        },
        wal: WalFreshnessSource {
            path: container.path().to_path_buf(),
            start_offset: view.journal_offset,
            end_offset,
            first_lsn: view.journal_first_lsn,
        },
    })
}
