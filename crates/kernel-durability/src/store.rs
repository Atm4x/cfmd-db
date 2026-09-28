use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::fs;
use std::fs::File;
#[cfg(test)]
use std::fs::OpenOptions;
#[cfg(test)]
use std::io::Write;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
use kernel_auth::{AuthorityDigest, FreshnessCut};
use kernel_change::RevisionEffectId;
use kernel_revision::Revision;
use kernel_semantics::SemanticRegistry;
use kernel_types::RevisionId;

use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
};
use crate::domain::{
    DurableMigrationComplement, DurableRevisionEffectRecord, DurableTransactionIntent,
    DurableTransactionKey, IdempotencyEpoch,
};
use crate::replication::authority::ReplicationAuthorityJournal;
use crate::wal::FileRevisionWal;

#[cfg(test)]
use crate::binary_codec::crc32c;
#[cfg(test)]
use crate::{
    DurabilityError, DurableTransactionOutcome, ReplicationAuthenticationReceipt,
    RevisionDurability, checkpoint, metadata,
};
use migration_history::MigrationComplementIndex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableGenerationReceipt {
    pub generation: u64,
    pub base_revision: RevisionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingCheckpointProgress {
    pub generation: u64,
    pub chunks_written: u32,
    pub chunks_total: u32,
    pub mirrored_lsn: u64,
    pub durable_shadow_lsn: u64,
    pub ready_to_publish: bool,
}

#[derive(Debug)]
pub struct DurableRevisionStore {
    directory: PathBuf,
    _directory_lock: File,
    generation: u64,
    checkpoint: Revision,
    durable_head: RevisionId,
    wal: FileRevisionWal,
    semantic_registry: SemanticRegistry,
    materialization_specs: Vec<DurableMaterializationSpec>,
    physical_artifact_specs: Vec<DurablePhysicalArtifactSpec>,
    artifact_cores: Vec<DurableArtifactCore>,
    migration_complements: Vec<DurableMigrationComplement>,
    migration_complement_index: MigrationComplementIndex,
    current_idempotency_epoch: IdempotencyEpoch,
    minimum_retry_epoch: IdempotencyEpoch,
    committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    next_revision_effect_id: u128,
    causal_coverage_root: RevisionId,
    revision_effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    revision_effect_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    replication: ReplicationAuthorityJournal,
    prepared_transactions: prepared_lifecycle::PreparedTransactionLedger,
    streaming_checkpoint: Option<streaming_checkpoint::StreamingCheckpointJob>,
    external_freshness: Option<ExternalFreshnessState>,
    poisoned: bool,
}

mod bootstrap;
mod causal_ledger;
mod checkpoint_publication;
mod checkpoint_storage;
mod commit_authority;
mod commit_batcher;
mod file_io;
mod format_registry;
mod freshness;
mod generation_layout;
mod manifest;
mod metadata_storage;
mod migration_history;
mod observe;
mod prepare_validation;
mod prepared_capsule;
mod prepared_lifecycle;
mod publication_protocol;
mod recovery;
mod replication_facade;
mod retry_history;
mod semantic_deployment;
mod streaming_checkpoint;
mod transition;

pub use commit_batcher::{
    DurableBatchEnqueueOutcome, DurableCommitBatchPolicy, DurableCommitBatcher,
};
use freshness::ExternalFreshnessState;
pub use freshness::{ExternalFreshnessAuthority, ExternalFreshnessConfig};
pub use prepared_capsule::PreparedCutCapsule;
#[cfg(test)]
use publication_protocol::{StoreFaultHook, StoreFaultPoint};

#[cfg(test)]
use checkpoint_storage::{
    CHECKPOINT_HEADER_LEN, CHECKPOINT_MAGIC, MAX_CHECKPOINT_LEN, read_checkpoint_generation,
    read_checkpoint_root_bounded,
};
#[cfg(test)]
use format_registry::{CHECKPOINT_FORMAT_VERSION, METADATA_FILE_VERSION};
#[cfg(test)]
use generation_layout::{
    checkpoint_chunk_path, checkpoint_path, manifest_path, metadata_path,
    parse_checkpoint_chunk_generation, prepared_capsule_path, wal_path,
};
#[cfg(test)]
use manifest::{ManifestRecord, decode_manifest, encode_manifest};
#[cfg(test)]
use metadata_storage::{METADATA_HEADER_LEN, METADATA_MAGIC, read_metadata_bytes_bounded};
#[cfg(test)]
use prepared_capsule::{
    PREPARED_CAPSULE_HEADER_LEN, PREPARED_CAPSULE_MAGIC, PREPARED_CAPSULE_VERSION,
    decode_prepared_cut_capsule, read_prepared_cut_capsule_file,
};

#[cfg(test)]
mod publication_model;

#[cfg(test)]
mod tests;
