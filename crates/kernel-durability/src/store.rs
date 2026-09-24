use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use kernel_auth::{
    AuthorityDigest, FreshnessCut, Sha256Digest, SignedFreshnessCut, TrustRootSet,
    VerifiedFreshnessCut, sha256, verify_freshness_cut,
};
use kernel_change::{RevisionEffect, RevisionEffectId, RevisionEffectIdeal};
use kernel_revision::Revision;
use kernel_semantics::{
    ArtifactAuthenticationSet, ImplementationArtifactDigest, SemanticContractIdentity,
    SemanticDeploymentRegistry, SemanticExecutionPolicy, SemanticRegistry,
};
use kernel_types::{ClientTransactionId, RevisionId, SchemaRevisionId};

use super::{
    DurabilityError, DurableArtifactCore, DurableCommitReceipt, DurableExternalFreshnessBinding,
    DurableMaterializationSpec, DurablePhysicalArtifactSpec, DurablePrepareToken,
    DurableRevisionChange, DurableRevisionDescriptor, DurableRevisionEffectRecord,
    DurableTransactionIntent, DurableTransactionKey, DurableTransactionOutcome, FileRevisionWal,
    IdempotencyEpoch, RecoveryScan, ReplicaId, ReplicatedEffectEnvelope,
    ReplicationAntiEntropyChunk, ReplicationAntiEntropyRequest, ReplicationAntiEntropySummary,
    ReplicationAuthenticationReceipt, ReplicationBranchHead, ReplicationBranchId,
    ReplicationDecisionLock, ReplicationDecisionVote, ReplicationEffectStage,
    ReplicationEffectVote, ReplicationFailureDetector, ReplicationIngestOutcome,
    ReplicationJointMembershipCertificate, ReplicationLeaderCertificate, ReplicationLeaderVote,
    ReplicationMembership, ReplicationMembershipChange, ReplicationMembershipVote,
    ReplicationPeerAuthPolicy, ReplicationQuorumAvailability, ReplicationQuorumCertificate,
    ReplicationQuorumLoss, ReplicationRecoveryCertificate, ReplicationTermPromise,
    ReplicationTransportIngress, ReplicationTransportPayload, RevisionDurability,
    SignedReplicationPeerEvidence, SignedReplicationTransportFrame, SupportedDurabilityProfile,
    VerifiedDestructiveDurabilityCampaignEvidence, canonical_physical_artifact_specs, checkpoint,
    crc32c, metadata, read_u16, read_u32, read_u64, replication_anti_entropy_summary,
};
use crate::replication::ReplicationAuthorityJournal;

const CHECKPOINT_MAGIC: [u8; 4] = *b"CFCP";
const LEGACY_CHECKPOINT_FORMAT_VERSION: u16 = 1;
const CHECKPOINT_FORMAT_VERSION: u16 = 2;
const CHECKPOINT_HEADER_LEN: usize = 32;
const MAX_CHECKPOINT_LEN: usize = 512 * 1024 * 1024;
const DEFAULT_CHECKPOINT_CHUNK_SIZE: usize = 1024 * 1024;
const CHECKPOINT_CHUNK_DESCRIPTOR_LEN: usize = 16;
const PREPARED_CAPSULE_MAGIC: [u8; 4] = *b"CFPC";
const PREPARED_CAPSULE_VERSION: u16 = 1;
const PREPARED_CAPSULE_HEADER_LEN: usize = 16;
const MANIFEST_MAGIC: [u8; 4] = *b"CFMF";
const LEGACY_MANIFEST_FORMAT_VERSION: u16 = 2;
const MANIFEST_FORMAT_VERSION: u16 = 3;
const LEGACY_MANIFEST_LEN: usize = 36;
const MANIFEST_LEN: usize = 64;
const METADATA_MAGIC: [u8; 4] = *b"CFDM";
const METADATA_FILE_VERSION: u16 = 1;
const METADATA_HEADER_LEN: usize = 20;
const MAX_METADATA_LEN: usize = 64 * 1024 * 1024;
const LOCK_FILE_NAME: &str = ".cfmd-durability.lock";

type RecoveredRevisionEffectState = (
    RevisionId,
    BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StoreFaultPoint {
    AfterCheckpointSync,
    AfterWalSync,
    AfterMetadataSync,
    AfterPrerequisiteDirectorySync,
    AfterPendingManifestSync,
    AfterManifestRename,
    AfterManifestDirectorySync,
    BeforeCompactionRemove,
    AfterCompactionRemove,
    AfterCompactionDirectorySync,
}

trait StoreFaultHook {
    fn hit(&mut self, point: StoreFaultPoint) -> Result<(), DurabilityError>;
}

struct NoStoreFault;

impl StoreFaultHook for NoStoreFault {
    fn hit(&mut self, _point: StoreFaultPoint) -> Result<(), DurabilityError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableGenerationReceipt {
    pub generation: u64,
    pub base_revision: RevisionId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFreshnessConfig {
    pub store_id: [u8; 32],
    pub trust_roots: TrustRootSet,
    pub deployment_policy_epoch: u64,
}

pub trait ExternalFreshnessAuthority: std::fmt::Debug + Send {
    fn read_signed(
        &mut self,
        store_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessCut>, DurabilityError>;

    fn compare_and_advance_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    ) -> Result<SignedFreshnessCut, DurabilityError>;
}

#[derive(Debug)]
struct ExternalFreshnessState {
    config: ExternalFreshnessConfig,
    current: Option<VerifiedFreshnessCut>,
    authority: Box<dyn ExternalFreshnessAuthority>,
}

impl ExternalFreshnessState {
    fn metadata_binding(&self) -> DurableExternalFreshnessBinding {
        DurableExternalFreshnessBinding {
            store_id: self.config.store_id,
            previous_generation_digest: self.current.map(|current| current.cut.generation_digest),
            trust_root_epoch: self.config.trust_roots.epoch(),
            deployment_policy_epoch: self.config.deployment_policy_epoch,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableCommitBatchPolicy {
    max_descriptors: usize,
}

impl DurableCommitBatchPolicy {
    pub fn new(max_descriptors: usize) -> Result<Self, DurabilityError> {
        if max_descriptors == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durability batch size must be nonzero",
            });
        }
        Ok(Self { max_descriptors })
    }

    #[must_use]
    pub const fn max_descriptors(self) -> usize {
        self.max_descriptors
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableBatchEnqueueOutcome {
    Queued,
    FlushRequired,
}

/// Non-authoritative scheduler state. Enqueue never acknowledges durability;
/// receipts only exist after `flush` crosses the shared COMMIT barrier.
#[derive(Debug)]
pub struct DurableCommitBatcher {
    policy: DurableCommitBatchPolicy,
    pending: Vec<DurableRevisionDescriptor>,
}

impl DurableCommitBatcher {
    #[must_use]
    pub const fn new(policy: DurableCommitBatchPolicy) -> Self {
        Self {
            policy,
            pending: Vec::new(),
        }
    }

    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn enqueue(
        &mut self,
        store: &DurableRevisionStore,
        descriptor: DurableRevisionDescriptor,
    ) -> Result<DurableBatchEnqueueOutcome, DurabilityError> {
        let expected_source = self
            .pending
            .last()
            .map_or(store.durable_head(), |tail| tail.target_revision);
        if descriptor.source_revision != expected_source {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "async durability batch is not a contiguous revision chain",
            });
        }
        if self
            .pending
            .iter()
            .any(|pending| pending.transaction_id == descriptor.transaction_id)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "async durability batch repeats a transaction id",
            });
        }
        self.pending.push(descriptor);
        Ok(if self.pending.len() >= self.policy.max_descriptors {
            DurableBatchEnqueueOutcome::FlushRequired
        } else {
            DurableBatchEnqueueOutcome::Queued
        })
    }

    pub fn flush(
        &mut self,
        store: &mut DurableRevisionStore,
    ) -> Result<Vec<DurableCommitReceipt>, DurabilityError> {
        if self.pending.is_empty() {
            return Ok(Vec::new());
        }
        let receipts = store.durably_commit_group(&self.pending)?;
        self.pending.clear();
        Ok(receipts)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct DurableFormatRegistry;

impl DurableFormatRegistry {
    fn require_supported(
        component: crate::DurableFormatComponent,
        version: u16,
        supported: &[u16],
    ) -> Result<(), DurabilityError> {
        if supported.contains(&version) {
            Ok(())
        } else {
            Err(DurabilityError::UnsupportedDurableFormat { component, version })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ManifestRecord {
    generation: u64,
    base_revision: RevisionId,
    published_head: RevisionId,
    wal_first_lsn: u64,
    published_tail_lsn: u64,
    checkpoint_crc32c: u32,
    metadata_crc32c: u32,
    prepared_capsule_crc32c: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedCutEntry {
    prepare_lsn: u64,
    payload_crc32c: u32,
    descriptor: DurableRevisionDescriptor,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PreparedCutCapsule {
    entries: Vec<PreparedCutEntry>,
}

impl PreparedCutCapsule {
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn scan_seeds(&self) -> Vec<(u64, DurableRevisionDescriptor, u32)> {
        self.entries
            .iter()
            .map(|entry| {
                (
                    entry.prepare_lsn,
                    entry.descriptor.clone(),
                    entry.payload_crc32c,
                )
            })
            .collect()
    }
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
struct StreamingCheckpointJob {
    generation: u64,
    cut_revision: Revision,
    payload: Vec<u8>,
    chunk_size: usize,
    chunk_crcs: Vec<u32>,
    next_chunk: usize,
    checkpoint_crc32c: Option<u32>,
    metadata_crc32c: u32,
    capsule_crc32c: u32,
    prepared_capsule: PreparedCutCapsule,
    shadow_wal: FileRevisionWal,
    wal_first_lsn: u64,
    mirrored_lsn: u64,
    durable_shadow_lsn: u64,
    failed: bool,
}

/// Canonical decoded authority of one published generation after its
/// checkpoint, metadata and WAL tail have been reconciled. File/codec versions
/// must disappear at this boundary; runtime publication consumes only this
/// semantic durable image.
#[derive(Debug)]
struct CanonicalDurableState {
    checkpoint: Revision,
    durable_head: RevisionId,
    semantic_registry: SemanticRegistry,
    materialization_specs: Vec<DurableMaterializationSpec>,
    physical_artifact_specs: Vec<DurablePhysicalArtifactSpec>,
    artifact_cores: Vec<DurableArtifactCore>,
    migration_complements: Vec<crate::DurableMigrationComplement>,
    current_idempotency_epoch: IdempotencyEpoch,
    minimum_retry_epoch: IdempotencyEpoch,
    committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    next_revision_effect_id: u128,
    causal_coverage_root: RevisionId,
    revision_effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    revision_effect_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
}

impl CanonicalDurableState {
    fn into_store(
        self,
        directory: PathBuf,
        directory_lock: File,
        generation: u64,
        wal: FileRevisionWal,
        replication: ReplicationAuthorityJournal,
    ) -> DurableRevisionStore {
        DurableRevisionStore {
            directory,
            _directory_lock: directory_lock,
            generation,
            checkpoint: self.checkpoint,
            durable_head: self.durable_head,
            wal,
            semantic_registry: self.semantic_registry,
            materialization_specs: self.materialization_specs,
            physical_artifact_specs: self.physical_artifact_specs,
            artifact_cores: self.artifact_cores,
            migration_complements: self.migration_complements,
            current_idempotency_epoch: self.current_idempotency_epoch,
            minimum_retry_epoch: self.minimum_retry_epoch,
            committed_transactions: self.committed_transactions,
            next_revision_effect_id: self.next_revision_effect_id,
            causal_coverage_root: self.causal_coverage_root,
            revision_effects: self.revision_effects,
            revision_effect_frontiers: self.revision_effect_frontiers,
            replication,
            prepared_transactions: BTreeMap::new(),
            streaming_checkpoint: None,
            external_freshness: None,
            poisoned: false,
        }
    }
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
    migration_complements: Vec<crate::DurableMigrationComplement>,
    current_idempotency_epoch: IdempotencyEpoch,
    minimum_retry_epoch: IdempotencyEpoch,
    committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    next_revision_effect_id: u128,
    causal_coverage_root: RevisionId,
    revision_effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    revision_effect_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    replication: ReplicationAuthorityJournal,
    prepared_transactions: BTreeMap<u64, DurableRevisionDescriptor>,
    streaming_checkpoint: Option<StreamingCheckpointJob>,
    external_freshness: Option<ExternalFreshnessState>,
    poisoned: bool,
}

fn append_revision_effect(
    effects: &mut BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &mut BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    record: DurableRevisionEffectRecord,
) -> Result<(), DurabilityError> {
    record
        .validate_identity()
        .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
    if record
        .prerequisites
        .iter()
        .any(|prerequisite| !effects.contains_key(prerequisite))
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "revision effect prerequisite is missing from durable causal ledger",
        });
    }
    if let Some(existing) = effects.get(&record.id) {
        if existing == &record {
            return Ok(());
        }
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "revision effect identity conflicts with durable causal ledger",
        });
    }
    if frontiers.contains_key(&record.target_revision) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "target revision already has a durable causal frontier",
        });
    }
    let target_revision = record.target_revision;
    let id = record.id;
    effects.insert(id, record);
    frontiers.insert(target_revision, BTreeSet::from([id]));
    Ok(())
}

fn causal_prerequisites_for_descriptor(
    frontiers: &BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    descriptor: &DurableRevisionDescriptor,
) -> Result<BTreeSet<RevisionEffectId>, DurabilityError> {
    match &descriptor.intent {
        DurableTransactionIntent::RelationResolutionExact { causal_parents, .. } => {
            if causal_parents.len() < 2
                || causal_parents.windows(2).any(|pair| pair[0] >= pair[1])
                || causal_parents
                    .binary_search(&descriptor.source_revision)
                    .is_err()
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "resolution causal parents are not canonical",
                });
            }
            let mut prerequisites = BTreeSet::new();
            for parent in causal_parents {
                let Some(frontier) = frontiers.get(parent) else {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "resolution causal parent is outside durable causal coverage",
                    });
                };
                prerequisites.extend(frontier.iter().copied());
            }
            Ok(prerequisites)
        }
        _ => frontiers
            .get(&descriptor.source_revision)
            .cloned()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "committed revision source is outside durable causal coverage",
            }),
    }
}

fn causal_frontier_for_revision(
    store: &DurableRevisionStore,
    revision: RevisionId,
) -> Option<&BTreeSet<RevisionEffectId>> {
    store
        .revision_effect_frontiers
        .get(&revision)
        .or_else(|| store.replication.revision_frontier(revision))
}

fn causal_prerequisites_for_replicated_effect(
    store: &DurableRevisionStore,
    record: &DurableRevisionEffectRecord,
) -> Result<BTreeSet<RevisionEffectId>, DurabilityError> {
    match &record.intent {
        DurableTransactionIntent::RelationResolutionExact { causal_parents, .. } => {
            if causal_parents.len() < 2
                || causal_parents.windows(2).any(|pair| pair[0] >= pair[1])
                || causal_parents
                    .binary_search(&record.source_revision)
                    .is_err()
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replicated resolution causal parents are not canonical",
                });
            }
            let mut prerequisites = BTreeSet::new();
            for parent in causal_parents {
                let Some(frontier) = causal_frontier_for_revision(store, *parent) else {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replicated resolution parent is outside durable causal coverage",
                    });
                };
                prerequisites.extend(frontier.iter().copied());
            }
            Ok(prerequisites)
        }
        _ => causal_frontier_for_revision(store, record.source_revision)
            .cloned()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect source is outside durable causal coverage",
            }),
    }
}

fn append_committed_revision_effect(
    effects: &mut BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &mut BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    descriptor: &DurableRevisionDescriptor,
) -> Result<(), DurabilityError> {
    let prerequisites = causal_prerequisites_for_descriptor(frontiers, descriptor)?;
    append_revision_effect(
        effects,
        frontiers,
        DurableRevisionEffectRecord {
            id: descriptor
                .revision_effect_id
                .unwrap_or(RevisionEffectId(descriptor.transaction_id.raw())),
            prerequisites,
            transaction_epoch: descriptor.idempotency_epoch,
            transaction_id: descriptor.transaction_id,
            intent: descriptor.intent.clone(),
            source_revision: descriptor.source_revision,
            target_revision: descriptor.target_revision,
        },
    )
}

fn allocate_local_revision_effect_id(next: &mut u128) -> Result<RevisionEffectId, DurabilityError> {
    if *next > u128::from(u64::MAX) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local revision effect namespace is exhausted",
        });
    }
    let id = RevisionEffectId(*next);
    *next = next.checked_add(1).ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "revision effect identity space is exhausted",
    })?;
    Ok(id)
}

fn recover_revision_effect_state(
    coverage_root: Option<RevisionId>,
    mut effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    mut frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    scan: &RecoveryScan,
) -> Result<RecoveredRevisionEffectState, DurabilityError> {
    let Some(coverage_root) = coverage_root else {
        let root = scan.durable_revision();
        effects.clear();
        frontiers.clear();
        frontiers.insert(root, BTreeSet::new());
        return Ok((root, effects, frontiers));
    };
    if !frontiers.contains_key(&coverage_root) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "durable causal coverage root has no frontier",
        });
    }
    for committed in scan.committed() {
        append_committed_revision_effect(&mut effects, &mut frontiers, &committed.descriptor)?;
    }
    Ok((coverage_root, effects, frontiers))
}

fn validate_revision_effect_state(
    coverage_root: RevisionId,
    effects: &BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
) -> Result<(), DurabilityError> {
    if frontiers.get(&coverage_root) != Some(&BTreeSet::new()) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "durable causal coverage root is not an empty frontier",
        });
    }
    for (&id, effect) in effects {
        effect
            .validate_identity()
            .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
        if id != effect.id {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal ledger map key disagrees with effect identity",
            });
        }
        let intent = &effect.intent;
        if intent.target_revision() != effect.target_revision {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal effect target disagrees with canonical effect intent",
            });
        }
        let expected_prerequisites = match intent {
            DurableTransactionIntent::RelationResolutionExact { causal_parents, .. } => {
                let mut cut = BTreeSet::new();
                for parent in causal_parents {
                    let Some(frontier) = frontiers.get(parent) else {
                        return Err(DurabilityError::Protocol {
                            offset: 0,
                            reason: "durable resolution parent frontier is missing",
                        });
                    };
                    cut.extend(frontier.iter().copied());
                }
                cut
            }
            _ => frontiers.get(&effect.source_revision).cloned().ok_or(
                DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable causal effect source frontier is missing",
                },
            )?,
        };
        if expected_prerequisites != effect.prerequisites {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal effect prerequisites disagree with exact causal cut",
            });
        }
        if frontiers.get(&effect.target_revision) != Some(&BTreeSet::from([id])) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable causal effect target frontier is not canonical",
            });
        }
    }
    if frontiers
        .values()
        .flatten()
        .any(|effect| !effects.contains_key(effect))
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "durable causal frontier references a missing effect",
        });
    }
    Ok(())
}

fn install_semantic_module_packages(
    registry: &mut SemanticRegistry,
    semantic_modules: &[kernel_semantics::BuiltinSemanticModuleSpec],
) -> Result<(), DurabilityError> {
    let deployment = SemanticDeploymentRegistry::from_builtin_specs(semantic_modules);
    let policy = SemanticExecutionPolicy::trusted_builtin_only();
    let authentications = ArtifactAuthenticationSet::trusted_builtins(semantic_modules);
    for spec in semantic_modules {
        let artifact = ImplementationArtifactDigest(spec.digest().0);
        let authorization = deployment
            .authorize_artifact(artifact, &policy, &authentications)
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "semantic implementation package is not execution-authorized",
            })?;
        if authorization.contract() != SemanticContractIdentity::Defined(spec.contract()) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "semantic implementation package contract changed during authorization",
            });
        }
        let installed =
            SemanticDeploymentRegistry::install_authorized_builtin(&authorization, registry)
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "authorized semantic package has no builtin executable artifact",
                })?;
        if installed != spec.digest() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "authorized semantic package installed a different artifact digest",
            });
        }
    }
    Ok(())
}

fn install_intent_semantic_modules(
    registry: &mut SemanticRegistry,
    intent: &DurableTransactionIntent,
) -> Result<(), DurabilityError> {
    let semantic_modules = match intent {
        DurableTransactionIntent::RelationDataExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::RelationRewriteExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::RelationResolutionExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::Exact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::SchemaMigrationExact {
            semantic_modules, ..
        } => semantic_modules,
        DurableTransactionIntent::LegacyTargetOnly { .. } => return Ok(()),
    };
    install_semantic_module_packages(registry, semantic_modules)
}

fn migration_step_same_identity(
    left: &crate::DurableMigrationComplement,
    right: &crate::DurableMigrationComplement,
) -> bool {
    left.source_schema == right.source_schema
        && left.target_schema == right.target_schema
        && left.lens_spec == right.lens_spec
        && left.semantic_pins == right.semantic_pins
        && left.encoding_version == right.encoding_version
        && left.retention == right.retention
}

fn append_migration_complement(
    complements: &mut Vec<crate::DurableMigrationComplement>,
    base_schema: kernel_types::SchemaRevisionId,
    complement: crate::DurableMigrationComplement,
) -> Result<(), DurabilityError> {
    if let Some(existing) = complements.iter().find(|existing| {
        existing.source_schema == complement.source_schema
            && existing.target_schema == complement.target_schema
    }) {
        if migration_step_same_identity(existing, &complement) {
            return Ok(());
        }
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement step identity conflicts with durable history",
        });
    }
    let expected_source = complements
        .last()
        .map_or(base_schema, |previous| previous.target_schema);
    if complement.source_schema != expected_source {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement chain is discontinuous",
        });
    }
    complements.push(complement);
    Ok(())
}

fn merge_wal_migration_complements(
    mut complements: Vec<crate::DurableMigrationComplement>,
    base_schema: kernel_types::SchemaRevisionId,
    scan: &RecoveryScan,
) -> Result<Vec<crate::DurableMigrationComplement>, DurabilityError> {
    for committed in scan.committed() {
        if let DurableTransactionIntent::SchemaMigrationExact {
            migration_complement,
            ..
        } = &committed.descriptor.intent
        {
            append_migration_complement(
                &mut complements,
                base_schema,
                migration_complement.clone(),
            )?;
        }
    }
    Ok(complements)
}

fn rebuild_semantic_registry(
    metadata: &metadata::DurableStoreMetadata,
    legacy_registry: Option<&SemanticRegistry>,
) -> Result<SemanticRegistry, DurabilityError> {
    let mut registry = if metadata.semantic_modules.is_empty() {
        legacy_registry.cloned().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "durable semantic deployment manifest is missing",
        })?
    } else {
        let mut registry = SemanticRegistry::default();
        install_semantic_module_packages(&mut registry, &metadata.semantic_modules)?;
        registry
    };
    for intent in metadata.committed_transactions.values() {
        install_intent_semantic_modules(&mut registry, intent)?;
    }
    Ok(registry)
}

fn merge_committed_transaction_intents(
    mut committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    scan: &RecoveryScan,
    registry: &mut SemanticRegistry,
) -> Result<BTreeMap<DurableTransactionKey, DurableTransactionIntent>, DurabilityError> {
    for (&transaction_id, intent) in scan.committed_transactions() {
        install_intent_semantic_modules(registry, intent)?;
        if let Some(existing) = committed_transactions.insert(transaction_id, intent.clone())
            && existing != *intent
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint transaction intent ledger conflicts with WAL tail",
            });
        }
    }
    Ok(committed_transactions)
}

fn read_published_metadata(
    directory: &Path,
    manifest: ManifestRecord,
) -> Result<metadata::DurableStoreMetadata, DurabilityError> {
    let metadata_file = metadata_path(directory, manifest.generation);
    let metadata_bytes = fs::read(&metadata_file).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            DurabilityError::Corruption {
                offset: 0,
                reason: "published durable metadata file is missing",
            }
        } else {
            DurabilityError::Io(error)
        }
    })?;
    if crc32c(&metadata_bytes) != manifest.metadata_crc32c {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published durable metadata file checksum mismatch",
        });
    }
    decode_metadata_file(&metadata_bytes)
}

fn open_published_generation(
    directory: &Path,
    manifest: ManifestRecord,
    registry: &SemanticRegistry,
) -> Result<(Revision, FileRevisionWal, RecoveryScan), DurabilityError> {
    let checkpoint = read_checkpoint_generation(directory, manifest, registry)?;
    if checkpoint.id() != manifest.base_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "manifest base revision does not match checkpoint",
        });
    }
    let prepared_capsule = if manifest.prepared_capsule_crc32c == 0 {
        PreparedCutCapsule::default()
    } else {
        let bytes = fs::read(prepared_capsule_path(directory, manifest.generation))?;
        if crc32c(&bytes) != manifest.prepared_capsule_crc32c {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "published prepared cut capsule checksum mismatch",
            });
        }
        decode_prepared_cut_capsule(&bytes)?
    };
    let wal_file = wal_path(directory, manifest.generation);
    if !wal_file.is_file() {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published WAL segment is missing",
        });
    }
    let seeds = prepared_capsule.scan_seeds();
    let (wal, scan) = FileRevisionWal::open_recovered_seeded(
        &wal_file,
        checkpoint.id(),
        manifest.wal_first_lsn,
        &seeds,
    )?;
    if scan.next_lsn() <= manifest.published_tail_lsn {
        return Err(DurabilityError::Corruption {
            offset: scan.last_good_offset(),
            reason: "published shadow WAL tail is shorter than manifest certificate",
        });
    }
    let certified_head = scan
        .committed()
        .iter()
        .take_while(|committed| committed.commit_lsn <= manifest.published_tail_lsn)
        .last()
        .map_or(checkpoint.id(), |committed| {
            committed.descriptor.target_revision
        });
    if certified_head != manifest.published_head {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "published WAL certificate does not reach manifest head exactly",
        });
    }
    Ok((checkpoint, wal, scan))
}

fn recover_retry_ledger(
    committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    current: IdempotencyEpoch,
    minimum: IdempotencyEpoch,
    scan: &RecoveryScan,
    registry: &mut SemanticRegistry,
) -> Result<
    (
        BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
        IdempotencyEpoch,
    ),
    DurabilityError,
> {
    let committed_transactions =
        merge_committed_transaction_intents(committed_transactions, scan, registry)?;
    let current = committed_transactions
        .keys()
        .map(|key| key.epoch)
        .max()
        .map_or(current, |epoch| epoch.max(current));
    if committed_transactions.keys().any(|key| key.epoch < minimum) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "WAL tail contains a transaction below the retry-history watermark",
        });
    }
    Ok((committed_transactions, current))
}

fn validate_relation_prepare_intent(
    descriptor: &DurableRevisionDescriptor,
    source_revision: kernel_types::RevisionId,
    target_revision: kernel_types::RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
    relation_mutations: &[crate::DurableRelationMutation],
    rewrite_intents: Option<&[crate::DurableRelationRewriteIntent]>,
) -> Result<(), DurabilityError> {
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "relation intent is paired with a non-delta change",
        });
    };
    let rewrite_shape_matches = rewrite_intents.is_none_or(|intents| {
        relation_mutations.len() == intents.len()
            && relation_mutations
                .iter()
                .zip(intents)
                .all(|(mutation, intent)| mutation.relation == intent.relation)
    });
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
        || semantic_revision != *change_semantics
        || relation_mutations != change_mutations
        || !rewrite_shape_matches
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "relation intent does not match descriptor delta",
        });
    }
    Ok(())
}

fn validate_full_revision_prepare_intent(
    descriptor: &DurableRevisionDescriptor,
    target_revision: kernel_types::RevisionId,
    encoded_target_revision: &[u8],
    registry: &SemanticRegistry,
) -> Result<(), DurabilityError> {
    if target_revision != descriptor.target_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction intent target does not match descriptor target",
        });
    }
    let target = checkpoint::decode_revision(encoded_target_revision, registry)?;
    if target.id() != descriptor.target_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction intent revision payload does not match target id",
        });
    }
    Ok(())
}

fn validate_schema_migration_prepare_intent(
    store: &mut DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
    source_revision: kernel_types::RevisionId,
    target_revision: kernel_types::RevisionId,
    encoded_target_revision: &[u8],
    migration_complement: &crate::DurableMigrationComplement,
    semantic_modules: &[kernel_semantics::BuiltinSemanticModuleSpec],
) -> Result<(), DurabilityError> {
    if source_revision != descriptor.source_revision {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "schema migration source revision does not match descriptor",
        });
    }
    install_semantic_module_packages(&mut store.semantic_registry, semantic_modules)?;
    validate_full_revision_prepare_intent(
        descriptor,
        target_revision,
        encoded_target_revision,
        &store.semantic_registry,
    )?;
    migration_complement
        .validate()
        .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
    let target = checkpoint::decode_revision(encoded_target_revision, &store.semantic_registry)?;
    if migration_complement.target_schema != target.semantic_revision().schema {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement target schema does not match target revision",
        });
    }
    let expected_source = store
        .migration_complements
        .last()
        .map_or(store.checkpoint.semantic_revision().schema, |previous| {
            previous.target_schema
        });
    if migration_complement.source_schema != expected_source {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "migration complement source schema does not match durable chain",
        });
    }
    Ok(())
}

fn validate_relation_resolution_prepare_intent(
    store: &mut DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    semantic_revision: kernel_types::SemanticRevision,
    resolution: &crate::DurableRelationResolution,
    semantic_modules: &[kernel_semantics::BuiltinSemanticModuleSpec],
) -> Result<(), DurabilityError> {
    validate_relation_prepare_intent(
        descriptor,
        source_revision,
        target_revision,
        semantic_revision,
        &resolution.relation_mutations,
        Some(&resolution.rewrite_intents),
    )?;
    if resolution.causal_parents.len() < 2
        || resolution
            .causal_parents
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || resolution
            .causal_parents
            .binary_search(&source_revision)
            .is_err()
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "relation resolution causal parents are not canonical",
        });
    }
    let _ = causal_prerequisites_for_descriptor(&store.revision_effect_frontiers, descriptor)?;
    install_semantic_module_packages(&mut store.semantic_registry, semantic_modules)?;
    Ok(())
}

fn validate_relation_rewrite_prepare_intent(
    store: &mut DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
    intent: &DurableTransactionIntent,
) -> Result<(), DurabilityError> {
    let DurableTransactionIntent::RelationRewriteExact {
        source_revision,
        target_revision,
        semantic_revision,
        relation_mutations,
        rewrite_intents,
        semantic_modules,
    } = intent
    else {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "rewrite prepare helper received non-rewrite intent",
        });
    };
    validate_relation_prepare_intent(
        descriptor,
        *source_revision,
        *target_revision,
        *semantic_revision,
        relation_mutations,
        Some(rewrite_intents),
    )?;
    install_semantic_module_packages(&mut store.semantic_registry, semantic_modules)?;
    Ok(())
}

fn validate_prepare_identity(
    store: &DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
) -> Result<(), DurabilityError> {
    if store.poisoned {
        return Err(DurabilityError::Poisoned);
    }
    let key = DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
    if descriptor.idempotency_epoch < store.minimum_retry_epoch {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction retry epoch has expired",
        });
    }
    if descriptor.idempotency_epoch != store.current_idempotency_epoch {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "transaction retry epoch does not match durable store epoch",
        });
    }
    if let Some(existing_intent) = store.committed_transactions.get(&key) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: if existing_intent == &descriptor.intent {
                "transaction is already committed"
            } else {
                "transaction retry key already committed to another exact intent"
            },
        });
    }
    if descriptor.source_revision != store.durable_head {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "prepare source revision does not match durable store head",
        });
    }
    Ok(())
}

impl DurableRevisionStore {
    /// Creates a store only after the target directory has passed the named
    /// supported-platform durability profile and live fsync/rename probe.
    pub fn create_on_supported_platform(
        directory: impl AsRef<Path>,
        profile: SupportedDurabilityProfile,
        campaign: &VerifiedDestructiveDurabilityCampaignEvidence,
        base_revision: &Revision,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        super::certify_supported_durability_platform(directory.as_ref(), profile, campaign)?;
        Self::create(directory, base_revision, registry)
    }

    /// Opens a store only after the target directory has passed the named
    /// supported-platform durability profile and live fsync/rename probe.
    pub fn open_on_supported_platform(
        directory: impl AsRef<Path>,
        profile: SupportedDurabilityProfile,
        campaign: &VerifiedDestructiveDurabilityCampaignEvidence,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        super::certify_supported_durability_platform(directory.as_ref(), profile, campaign)?;
        Self::open(directory)
    }

    pub fn create(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_materializations_and_physical_artifacts(
            directory,
            base_revision,
            &[],
            &[],
            registry,
        )
    }

    pub fn create_with_materializations(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_materializations_and_physical_artifacts(
            directory,
            base_revision,
            materialization_specs,
            &[],
            registry,
        )
    }

    pub fn create_with_materializations_and_physical_artifacts(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_materializations_physical_artifacts_and_cores(
            directory,
            base_revision,
            materialization_specs,
            physical_artifact_specs,
            &[],
            registry,
        )
    }

    pub fn create_with_materializations_physical_artifacts_and_cores(
        directory: impl AsRef<Path>,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        Self::create_with_hook(
            directory.as_ref(),
            base_revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            registry,
            &mut NoStoreFault,
        )
    }

    fn create_with_hook(
        directory: &Path,
        base_revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        registry: &SemanticRegistry,
        hook: &mut impl StoreFaultHook,
    ) -> Result<Self, DurabilityError> {
        let directory = directory.to_path_buf();
        fs::create_dir_all(&directory)?;
        let directory_lock = lock_directory(&directory)?;
        if highest_manifest_generation(&directory)?.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durable store already contains a published generation",
            });
        }
        let generation = 1;
        let checkpoint_path = checkpoint_path(&directory, generation);
        let checkpoint_crc32c = write_checkpoint_file(&checkpoint_path, base_revision)?;
        hook.hit(StoreFaultPoint::AfterCheckpointSync)?;
        let wal_path = wal_path(&directory, generation);
        let mut wal = FileRevisionWal::create(&wal_path)?;
        wal.barrier()?;
        hook.hit(StoreFaultPoint::AfterWalSync)?;
        let committed_transactions = BTreeMap::new();
        let semantic_modules = registry
            .builtin_modules_for_context(base_revision.semantic_context())
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "base revision requires unavailable semantic implementation",
            })?;
        let physical_artifact_specs = canonical_physical_artifact_specs(physical_artifact_specs);
        let causal_coverage_root = base_revision.id();
        let revision_effects = BTreeMap::new();
        let revision_effect_frontiers = BTreeMap::from([(causal_coverage_root, BTreeSet::new())]);
        let metadata_record = metadata::DurableStoreMetadata {
            external_freshness: None,
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            materializations: materialization_specs.to_vec(),
            physical_artifacts: physical_artifact_specs.clone(),
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: Vec::new(),
            committed_transactions: committed_transactions.clone(),
            semantic_modules,
            causal_coverage_root: Some(causal_coverage_root),
            revision_effects: revision_effects.clone(),
            revision_effect_frontiers: revision_effect_frontiers.clone(),
        };
        let metadata_file = metadata_path(&directory, generation);
        let metadata_crc32c = write_metadata_file(&metadata_file, &metadata_record)?;
        hook.hit(StoreFaultPoint::AfterMetadataSync)?;
        sync_directory(&directory)?;
        hook.hit(StoreFaultPoint::AfterPrerequisiteDirectorySync)?;
        let mut authority_uncertain = false;
        publish_manifest_with_hook(
            &directory,
            ManifestRecord {
                generation,
                base_revision: base_revision.id(),
                published_head: base_revision.id(),
                wal_first_lsn: 1,
                published_tail_lsn: 0,
                checkpoint_crc32c,
                metadata_crc32c,
                prepared_capsule_crc32c: 0,
            },
            hook,
            &mut authority_uncertain,
        )?;
        let replication =
            ReplicationAuthorityJournal::open_or_create(directory.join("replication.cfre"))?;
        Ok(Self {
            directory,
            _directory_lock: directory_lock,
            generation,
            checkpoint: base_revision.clone(),
            durable_head: base_revision.id(),
            wal,
            semantic_registry: registry.clone(),
            materialization_specs: materialization_specs.to_vec(),
            physical_artifact_specs,
            artifact_cores: artifact_cores.to_vec(),
            migration_complements: Vec::new(),
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            committed_transactions,
            next_revision_effect_id: 1,
            causal_coverage_root,
            revision_effects,
            revision_effect_frontiers,
            replication,
            prepared_transactions: BTreeMap::new(),
            streaming_checkpoint: None,
            external_freshness: None,
            poisoned: false,
        })
    }

    pub fn open(directory: impl AsRef<Path>) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_inner(directory.as_ref(), None, false)
    }

    pub fn open_with_legacy_registry(
        directory: impl AsRef<Path>,
        legacy_registry: &SemanticRegistry,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_inner(directory.as_ref(), Some(legacy_registry), false)
    }

    pub fn adopt_external_freshness(
        &mut self,
        config: ExternalFreshnessConfig,
        mut authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        if self.external_freshness.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority is already active",
            });
        }
        if authority.read_signed(config.store_id)?.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority already contains this store id",
            });
        }
        self.external_freshness = Some(ExternalFreshnessState {
            config,
            current: None,
            authority,
        });
        let result = self.rotate_checkpoint(&self.checkpoint.clone());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    pub fn open_with_external_freshness_on_supported_platform(
        directory: impl AsRef<Path>,
        profile: SupportedDurabilityProfile,
        campaign: &VerifiedDestructiveDurabilityCampaignEvidence,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        super::certify_supported_durability_platform(directory.as_ref(), profile, campaign)?;
        Self::open_with_external_freshness(directory, config, authority)
    }

    pub fn open_with_external_freshness(
        directory: impl AsRef<Path>,
        config: ExternalFreshnessConfig,
        mut authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let directory = directory.as_ref();
        let signed = authority
            .read_signed(config.store_id)?
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority has no record for durable store",
            })?;
        let current = verify_freshness_cut(&config.trust_roots, &signed).map_err(|_| {
            DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness record authentication failed",
            }
        })?;
        if current.cut.store_id != config.store_id
            || current.cut.deployment_policy_epoch != config.deployment_policy_epoch
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness policy identity mismatch",
            });
        }
        let manifest = read_current_manifest(directory)?;
        let metadata = read_published_metadata(directory, manifest)?;
        let next = preflight_external_freshness(directory, manifest, &metadata, &config, current)?;
        let current = if next == current.cut {
            current
        } else {
            let advanced =
                authority.compare_and_advance_signed(Some(current.record_digest), next)?;
            verify_returned_freshness_cut(&config, next, &advanced)?
        };
        let (mut store, scan) = Self::open_inner(directory, None, true)?;
        store.external_freshness = Some(ExternalFreshnessState {
            config,
            current: Some(current),
            authority,
        });
        Ok((store, scan))
    }

    fn open_inner(
        directory: &Path,
        legacy_registry: Option<&SemanticRegistry>,
        allow_external_freshness: bool,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let directory = directory.to_path_buf();
        let directory_lock = lock_directory(&directory)?;
        let manifest = read_current_manifest(&directory)?;
        let metadata = read_published_metadata(&directory, manifest)?;
        if metadata.external_freshness.is_some() && !allow_external_freshness {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "externally anchored store requires freshness-aware open",
            });
        }
        let mut registry = rebuild_semantic_registry(&metadata, legacy_registry)?;
        let (checkpoint, wal, scan) = open_published_generation(&directory, manifest, &registry)?;
        let minimum_retry_epoch = metadata.minimum_retry_epoch;
        let (committed_transactions, current_idempotency_epoch) = recover_retry_ledger(
            metadata.committed_transactions,
            metadata.current_idempotency_epoch,
            minimum_retry_epoch,
            &scan,
            &mut registry,
        )?;
        let migration_complements = merge_wal_migration_complements(
            metadata.migration_complements,
            checkpoint.semantic_revision().schema,
            &scan,
        )?;
        let (causal_coverage_root, revision_effects, revision_effect_frontiers) =
            recover_revision_effect_state(
                metadata.causal_coverage_root,
                metadata.revision_effects,
                metadata.revision_effect_frontiers,
                &scan,
            )?;
        validate_revision_effect_state(
            causal_coverage_root,
            &revision_effects,
            &revision_effect_frontiers,
        )?;
        let next_revision_effect_id = revision_effects
            .keys()
            .map(|id| id.0)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "revision effect identity space is exhausted",
            })?;
        let canonical = CanonicalDurableState {
            checkpoint,
            durable_head: scan.durable_revision(),
            semantic_registry: registry,
            materialization_specs: metadata.materializations,
            physical_artifact_specs: metadata.physical_artifacts,
            artifact_cores: metadata.artifact_cores,
            migration_complements,
            current_idempotency_epoch,
            minimum_retry_epoch,
            committed_transactions,
            next_revision_effect_id,
            causal_coverage_root,
            revision_effects,
            revision_effect_frontiers,
        };
        let replication =
            ReplicationAuthorityJournal::open_or_create(directory.join("replication.cfre"))?;
        let mut store = canonical.into_store(
            directory,
            directory_lock,
            manifest.generation,
            wal,
            replication,
        );
        store.validate_replicated_authority()?;
        Ok((store, scan))
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn checkpoint_revision(&self) -> &Revision {
        &self.checkpoint
    }

    #[must_use]
    pub const fn durable_head(&self) -> RevisionId {
        self.durable_head
    }

    #[must_use]
    pub const fn causal_coverage_root(&self) -> RevisionId {
        self.causal_coverage_root
    }

    #[must_use]
    pub fn revision_effect_record(
        &self,
        id: RevisionEffectId,
    ) -> Option<&DurableRevisionEffectRecord> {
        self.revision_effects.get(&id)
    }

    #[must_use]
    pub fn revision_effect_frontier(
        &self,
        revision: RevisionId,
    ) -> Option<&BTreeSet<RevisionEffectId>> {
        self.revision_effect_frontiers.get(&revision)
    }

    pub fn revision_effect_ideal(
        &self,
        revision: RevisionId,
    ) -> Result<Option<RevisionEffectIdeal<DurableTransactionIntent>>, DurabilityError> {
        let Some(frontier) = self.revision_effect_frontiers.get(&revision) else {
            return Ok(None);
        };
        let mut pending = frontier.iter().copied().collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        let mut events = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let record = self
                .revision_effects
                .get(&id)
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "durable revision frontier references a missing effect",
                })?;
            let payload = record.intent.clone();
            pending.extend(record.prerequisites.iter().copied());
            events.push(RevisionEffect {
                id,
                prerequisites: record.prerequisites.clone(),
                payload,
            });
        }
        RevisionEffectIdeal::new(events)
            .map(Some)
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "durable revision effect ledger is not a valid causal ideal",
            })
    }

    #[must_use]
    pub fn replication_branch_head(
        &self,
        branch: ReplicationBranchId,
    ) -> Option<ReplicationBranchHead> {
        self.replication.branch_head(branch)
    }

    #[must_use]
    pub fn replication_published_branch_head(
        &self,
        branch: ReplicationBranchId,
    ) -> Option<ReplicationBranchHead> {
        self.replication.published_branch_head(branch)
    }

    #[must_use]
    pub fn current_replication_membership(&self) -> Option<&ReplicationMembership> {
        self.replication.current_membership()
    }

    #[must_use]
    pub fn replication_effect_stage(
        &self,
        effect: RevisionEffectId,
    ) -> Option<ReplicationEffectStage> {
        self.replication.effect_stage(effect)
    }

    #[must_use]
    pub fn replicated_effect_record(
        &self,
        id: RevisionEffectId,
    ) -> Option<&DurableRevisionEffectRecord> {
        self.replication.effect(id)
    }

    #[must_use]
    pub fn replication_journal_path(&self) -> &Path {
        self.replication.path()
    }

    #[must_use]
    pub fn replication_peer_auth_policy(&self) -> Option<&ReplicationPeerAuthPolicy> {
        self.replication.peer_auth_policy()
    }

    #[must_use]
    pub const fn replication_quorum_availability(&self) -> Option<ReplicationQuorumAvailability> {
        self.replication.quorum_availability()
    }

    #[must_use]
    pub fn replication_authentication_receipt(
        &self,
        proof_digest: Sha256Digest,
    ) -> Option<ReplicationAuthenticationReceipt> {
        self.replication.authentication_receipt(proof_digest)
    }

    /// Compact non-authoritative summary used to decide whether peers must
    /// exchange decision-lock frontier chunks before quorum recovery.
    pub fn replication_anti_entropy_summary(
        &self,
    ) -> Result<Option<ReplicationAntiEntropySummary>, DurabilityError> {
        let Some(membership) = self.replication.current_membership() else {
            return Ok(None);
        };
        let locks = self.replication.decision_lock_summaries();
        replication_anti_entropy_summary(
            membership.epoch,
            self.replication.current_consensus_term(),
            &locks,
        )
        .map(Some)
    }

    /// Returns one bounded, ordered lock-frontier chunk. Chunks carry no
    /// authority by themselves; they only drive authenticated reconciliation.
    pub fn replication_anti_entropy_chunk(
        &self,
        request: ReplicationAntiEntropyRequest,
    ) -> Result<ReplicationAntiEntropyChunk, DurabilityError> {
        let current = self
            .replication
            .current_membership()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "replication anti-entropy request has no membership",
            })?;
        if request.membership_epoch != current.epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication anti-entropy request uses stale membership",
            });
        }
        let requested = usize::try_from(request.max_locks).unwrap_or(usize::MAX);
        let limit = requested.min(super::MAX_ANTI_ENTROPY_LOCKS);
        if limit == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication anti-entropy request has zero chunk bound",
            });
        }
        let all = self.replication.decision_lock_summaries();
        let mut matching = all
            .iter()
            .copied()
            .filter(|lock| lock.position >= request.from_position);
        let locks: Vec<_> = matching.by_ref().take(limit).collect();
        let complete = matching.next().is_none();
        Ok(ReplicationAntiEntropyChunk {
            membership_epoch: current.epoch,
            locks,
            complete,
        })
    }

    /// Activates or rotates authenticated peer evidence for replication.
    /// `TrustRootSet` remains the cryptographic authority owned by `kernel-auth`;
    /// this journal durably binds replica identities to key identities and a
    /// monotone trust epoch.
    pub fn durably_install_replication_peer_auth_policy(
        &mut self,
        policy: ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.install_peer_auth_policy(policy, trust)
    }

    /// Verifies and durably records signed peer evidence. For ordinary vote /
    /// promise evidence this also appends the corresponding semantic journal
    /// frame, so callers cannot accidentally downgrade a verified message into
    /// an unauthenticated assertion.
    pub fn durably_record_authenticated_replication_peer_evidence(
        &mut self,
        trust: &TrustRootSet,
        signed: SignedReplicationPeerEvidence,
    ) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication
            .record_authenticated_peer_evidence(trust, signed)
    }

    /// Authenticates one transport frame against the durable peer policy and
    /// routes authority-bearing peer evidence into the replication journal.
    /// Advisory heartbeat/anti-entropy payloads never create durable authority.
    pub fn durably_accept_replication_transport_frame(
        &mut self,
        ingress: &mut ReplicationTransportIngress,
        trust: &TrustRootSet,
        signed: SignedReplicationTransportFrame,
    ) -> Result<Option<ReplicationAuthenticationReceipt>, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let policy =
            self.replication
                .peer_auth_policy()
                .cloned()
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replication transport has no durable peer auth policy",
                })?;
        ingress.accept(&policy, trust, &signed)?;
        let current = self
            .replication
            .current_membership()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "replication transport has no durable membership",
            })?;
        if !current.members.contains(&signed.frame.sender) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication transport sender is not in current membership",
            });
        }
        match signed.frame.payload {
            ReplicationTransportPayload::PeerEvidence(peer) => self
                .replication
                .record_authenticated_peer_evidence(trust, peer)
                .map(Some),
            ReplicationTransportPayload::AntiEntropySummary(summary) => {
                if summary.membership_epoch != current.epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication anti-entropy summary uses stale membership",
                    });
                }
                Ok(None)
            }
            ReplicationTransportPayload::AntiEntropyRequest(request) => {
                if request.membership_epoch != current.epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication anti-entropy request uses stale membership",
                    });
                }
                Ok(None)
            }
            ReplicationTransportPayload::AntiEntropyChunk(chunk) => {
                if chunk.membership_epoch != current.epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication anti-entropy chunk uses stale membership",
                    });
                }
                super::validate_replication_anti_entropy_chunk(&chunk)?;
                Ok(None)
            }
            ReplicationTransportPayload::Heartbeat(heartbeat) => {
                if heartbeat.membership_epoch != current.epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication heartbeat uses stale membership",
                    });
                }
                Ok(None)
            }
        }
    }

    /// Converts a local failure-detector observation into the existing durable
    /// quorum-loss safety fence. False suspicions may reduce liveness but cannot
    /// create authority; recovery still requires a separate authenticated quorum.
    pub fn durably_fence_replication_if_quorum_unreachable(
        &mut self,
        detector: &ReplicationFailureDetector,
        observed_term: u64,
    ) -> Result<bool, DurabilityError> {
        let membership =
            self.replication
                .current_membership()
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replication failure detector has no durable membership",
                })?;
        let Some(loss) = detector.quorum_loss_observation(membership, observed_term) else {
            return Ok(false);
        };
        self.replication.mark_quorum_lost(loss)?;
        Ok(true)
    }

    /// Durably fences new consensus authority after local quorum-loss
    /// detection. Already-published state remains readable, while local effect
    /// durability may continue without becoming quorum/publication authority.
    pub fn durably_mark_replication_quorum_lost(
        &mut self,
        loss: ReplicationQuorumLoss,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.mark_quorum_lost(loss)
    }

    /// Completes recovery only after an authenticated membership quorum reports
    /// a lock frontier reconciled with the local durable journal and the
    /// recovery term advances every known safety floor.
    pub fn durably_recover_replication_quorum(
        &mut self,
        certificate: &ReplicationRecoveryCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.recover_quorum(certificate)
    }

    /// Durably ingests one already-ordered remote REIC effect without changing
    /// the single published revision head. Branch arrival is therefore never a
    /// second publication authority. The exact effect ontology is shared with
    /// local commits; only its lifecycle journal is separate from the linear
    /// transaction WAL.
    pub fn durably_ingest_replicated_effect(
        &mut self,
        envelope: ReplicatedEffectEnvelope,
    ) -> Result<ReplicationIngestOutcome, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.revision_effects.contains_key(&envelope.effect.id) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect identity collides with local causal authority",
            });
        }
        if self
            .revision_effect_frontiers
            .contains_key(&envelope.effect.target_revision)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated target revision already belongs to local causal authority",
            });
        }
        if !envelope.effect.intent.is_exact() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect does not carry exact executable intent",
            });
        }
        let expected = causal_prerequisites_for_replicated_effect(self, &envelope.effect)?;
        if expected != envelope.effect.prerequisites {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect prerequisites do not equal the authoritative causal cut",
            });
        }
        install_intent_semantic_modules(&mut self.semantic_registry, &envelope.effect.intent)?;
        self.replication.ingest(envelope)
    }

    pub fn durably_retire_replication_branch(
        &mut self,
        branch: ReplicationBranchId,
        expected_head: RevisionEffectId,
    ) -> Result<(), DurabilityError> {
        self.replication.retire_branch(branch, expected_head)
    }

    /// Durably installs a replication membership epoch. The first epoch is an
    /// explicit bootstrap; every later epoch must carry the previous epoch's
    /// configured quorum. Peer authentication is a caller/transport obligation,
    /// while this store enforces durable membership/threshold semantics.
    pub fn durably_install_replication_membership(
        &mut self,
        change: ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.install_membership(change)
    }

    /// Persists one already-authenticated peer vote for a replicated decision
    /// slot. The journal enforces vote-once for that membership epoch/position.
    pub fn durably_record_replicated_effect_vote(
        &mut self,
        vote: ReplicationEffectVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.record_effect_vote(vote)
    }

    /// Persists one already-authenticated vote for the successor membership.
    /// One voter cannot durably support conflicting successors of one epoch.
    pub fn durably_record_replication_membership_vote(
        &mut self,
        vote: ReplicationMembershipVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.record_membership_vote(vote)
    }

    #[must_use]
    pub fn replication_promised_term(
        &self,
        membership_epoch: u64,
        voter: ReplicaId,
    ) -> Option<u64> {
        self.replication.promised_term(membership_epoch, voter)
    }

    #[must_use]
    pub fn replication_leader_certificate(
        &self,
        membership_epoch: u64,
        term: u64,
    ) -> Option<&ReplicationLeaderCertificate> {
        self.replication.leader_certificate(membership_epoch, term)
    }

    #[must_use]
    pub fn replication_decision_lock(&self, position: u64) -> Option<&ReplicationDecisionLock> {
        self.replication.decision_lock(position)
    }

    pub fn durably_record_replication_term_promise(
        &mut self,
        promise: ReplicationTermPromise,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.record_term_promise(promise)
    }

    pub fn durably_record_replication_leader_vote(
        &mut self,
        vote: ReplicationLeaderVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.record_leader_vote(vote)
    }

    pub fn durably_certify_replication_leader(
        &mut self,
        certificate: ReplicationLeaderCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.certify_leader(certificate)
    }

    pub fn durably_certify_replication_joint_membership(
        &mut self,
        certificate: ReplicationJointMembershipCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.certify_joint_membership(certificate)
    }

    pub fn durably_record_replication_decision_vote(
        &mut self,
        vote: ReplicationDecisionVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.record_decision_vote(vote)
    }

    pub fn durably_lock_replication_decision(
        &mut self,
        lock: ReplicationDecisionLock,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.lock_decision(lock)
    }

    /// Advances one replicated effect from `LocalDurable` to `QuorumDurable`.
    pub fn durably_certify_replicated_effect_quorum(
        &mut self,
        certificate: ReplicationQuorumCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.certify_quorum(certificate)
    }

    /// Marks one quorum-durable effect reader-publishable in branch order.
    /// This does not mutate the store's single linear `durable_head`.
    pub fn durably_publish_replicated_effect(
        &mut self,
        effect: RevisionEffectId,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.replication.publish(effect)
    }

    pub fn replicated_branch_effect_ideal(
        &self,
        branch: ReplicationBranchId,
    ) -> Result<Option<RevisionEffectIdeal<DurableTransactionIntent>>, DurabilityError> {
        let Some(head) = self.replication.branch_head(branch) else {
            return Ok(None);
        };
        let mut pending = vec![head.head_effect];
        let mut seen = BTreeSet::new();
        let mut events = Vec::new();
        while let Some(id) = pending.pop() {
            if !seen.insert(id) {
                continue;
            }
            let record = self
                .revision_effects
                .get(&id)
                .or_else(|| self.replication.effect(id))
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replicated branch ideal references a missing causal effect",
                })?;
            pending.extend(record.prerequisites.iter().copied());
            events.push(RevisionEffect {
                id,
                prerequisites: record.prerequisites.clone(),
                payload: record.intent.clone(),
            });
        }
        RevisionEffectIdeal::new(events)
            .map(Some)
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "replicated branch effect ledger is not a valid causal ideal",
            })
    }

    fn validate_replicated_authority(&mut self) -> Result<(), DurabilityError> {
        let envelopes = self
            .replication
            .effects_iter()
            .map(|(_, envelope)| envelope.clone())
            .collect::<Vec<_>>();
        for envelope in envelopes {
            if self.revision_effects.contains_key(&envelope.effect.id)
                || self
                    .revision_effect_frontiers
                    .contains_key(&envelope.effect.target_revision)
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replicated causal authority collides with local authority",
                });
            }
            let expected = causal_prerequisites_for_replicated_effect(self, &envelope.effect)?;
            if expected != envelope.effect.prerequisites {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replayed replicated effect has a stale causal cut",
                });
            }
            install_intent_semantic_modules(&mut self.semantic_registry, &envelope.effect.intent)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn wal_path(&self) -> &Path {
        self.wal.path()
    }

    #[must_use]
    pub fn materialization_specs(&self) -> &[DurableMaterializationSpec] {
        &self.materialization_specs
    }

    #[must_use]
    pub fn physical_artifact_specs(&self) -> &[DurablePhysicalArtifactSpec] {
        &self.physical_artifact_specs
    }

    #[must_use]
    pub fn artifact_cores(&self) -> &[DurableArtifactCore] {
        &self.artifact_cores
    }

    #[must_use]
    pub fn migration_complements(&self) -> &[crate::DurableMigrationComplement] {
        &self.migration_complements
    }

    /// Resolves the exact locally-restorable complement chain between schema
    /// revisions. Retention policy is enforced here: released local payload,
    /// explicit Forget, and external-archive authority are never silently
    /// treated as locally reversible history.
    pub fn local_historical_complement_chain(
        &self,
        source: SchemaRevisionId,
        target: SchemaRevisionId,
    ) -> Result<crate::LocalHistoricalComplementChain, crate::HistoricalComplementError> {
        if source == target {
            return Ok(crate::LocalHistoricalComplementChain::default());
        }
        let mut cursor = source;
        let mut chain = crate::LocalHistoricalComplementChain::default();
        for durable in &self.migration_complements {
            if durable.source_schema != cursor {
                continue;
            }
            let Some(capsule) = durable.local_capsule() else {
                match durable.retention {
                    kernel_lens::ComplementRetention::ExternalArchive(proof) => {
                        return Err(crate::HistoricalComplementError::ExternalArchiveRequired(
                            proof,
                        ));
                    }
                    kernel_lens::ComplementRetention::Forget => {
                        return Err(crate::HistoricalComplementError::ExplicitlyForgotten {
                            source: durable.source_schema,
                            target: durable.target_schema,
                        });
                    }
                    kernel_lens::ComplementRetention::Forever
                    | kernel_lens::ComplementRetention::UntilRevision(_)
                    | kernel_lens::ComplementRetention::UntilEpoch(_) => {
                        return Err(crate::HistoricalComplementError::LocalPayloadReleased {
                            source: durable.source_schema,
                            target: durable.target_schema,
                        });
                    }
                }
            };
            debug_assert_eq!(capsule.source_schema, durable.source_schema);
            debug_assert_eq!(capsule.target_schema, durable.target_schema);
            chain.push(durable.clone());
            cursor = durable.target_schema;
            if cursor == target {
                return Ok(chain);
            }
        }
        Err(crate::HistoricalComplementError::PathNotFound { source, target })
    }

    #[must_use]
    pub const fn semantic_registry(&self) -> &SemanticRegistry {
        &self.semantic_registry
    }

    #[must_use]
    pub fn transaction_outcome(
        &self,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        self.transaction_outcome_at(self.current_idempotency_epoch, transaction_id)
    }

    #[must_use]
    pub fn transaction_outcome_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        if epoch < self.minimum_retry_epoch {
            return DurableTransactionOutcome::RetryHistoryExpired;
        }
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
            .map_or(DurableTransactionOutcome::Unknown, |intent| {
                DurableTransactionOutcome::Committed {
                    target_revision: intent.target_revision(),
                }
            })
    }

    #[must_use]
    pub fn transaction_intent(
        &self,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableTransactionIntent> {
        self.transaction_intent_at(self.current_idempotency_epoch, transaction_id)
    }

    #[must_use]
    pub fn transaction_intent_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableTransactionIntent> {
        if epoch < self.minimum_retry_epoch {
            return None;
        }
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
    }

    #[must_use]
    pub const fn current_idempotency_epoch(&self) -> IdempotencyEpoch {
        self.current_idempotency_epoch
    }

    #[must_use]
    pub const fn minimum_retry_epoch(&self) -> IdempotencyEpoch {
        self.minimum_retry_epoch
    }

    pub fn advance_idempotency_epoch(
        &mut self,
        next: IdempotencyEpoch,
    ) -> Result<(), DurabilityError> {
        if next <= self.current_idempotency_epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "idempotency epoch must advance monotonically",
            });
        }
        self.current_idempotency_epoch = next;
        Ok(())
    }

    pub fn expire_retry_history_before(
        &mut self,
        minimum: IdempotencyEpoch,
    ) -> Result<usize, DurabilityError> {
        if minimum < self.minimum_retry_epoch || minimum > self.current_idempotency_epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "retry-history watermark is outside the active epoch range",
            });
        }
        let before = self.committed_transactions.len();
        self.committed_transactions
            .retain(|key, _| key.epoch >= minimum);
        self.minimum_retry_epoch = minimum;
        Ok(before - self.committed_transactions.len())
    }

    fn bind_group_descriptors(
        &mut self,
        descriptors: &[DurableRevisionDescriptor],
    ) -> Result<Vec<DurableRevisionDescriptor>, DurabilityError> {
        let mut expected_source = self.durable_head;
        let mut transaction_ids = BTreeSet::new();
        let mut next_effect_id = self.next_revision_effect_id;
        let mut bound = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors {
            if descriptor.source_revision != expected_source {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "group commit descriptors are not a contiguous revision chain",
                });
            }
            if !transaction_ids.insert(descriptor.transaction_id) {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "group commit repeats a transaction id",
                });
            }
            let mut descriptor = descriptor.clone();
            descriptor.idempotency_epoch = self.current_idempotency_epoch;
            let key =
                DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
            if descriptor.idempotency_epoch < self.minimum_retry_epoch {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "transaction retry epoch has expired",
                });
            }
            if let Some(existing) = self.committed_transactions.get(&key) {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: if existing == &descriptor.intent {
                        "transaction is already committed"
                    } else {
                        "transaction retry key already committed to another exact intent"
                    },
                });
            }
            descriptor.revision_effect_id =
                Some(allocate_local_revision_effect_id(&mut next_effect_id)?);
            validate_bound_prepare_intent(self, &descriptor)?;
            expected_source = descriptor.target_revision;
            bound.push(descriptor);
        }
        self.next_revision_effect_id = next_effect_id;
        Ok(bound)
    }

    fn apply_group_commits(
        &mut self,
        descriptors: &[DurableRevisionDescriptor],
        receipts: &[DurableCommitReceipt],
    ) -> Result<(), DurabilityError> {
        for (descriptor, receipt) in descriptors.iter().zip(receipts) {
            self.durable_head = receipt.target_revision();
            if let DurableTransactionIntent::SchemaMigrationExact {
                migration_complement,
                ..
            } = &descriptor.intent
            {
                append_migration_complement(
                    &mut self.migration_complements,
                    self.checkpoint.semantic_revision().schema,
                    migration_complement.clone(),
                )?;
            }
            let key =
                DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
            if let Some(existing) = self
                .committed_transactions
                .insert(key, descriptor.intent.clone())
                && existing != descriptor.intent
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "committed transaction id changed exact intent",
                });
            }
            append_committed_revision_effect(
                &mut self.revision_effects,
                &mut self.revision_effect_frontiers,
                descriptor,
            )?;
        }
        Ok(())
    }

    /// Commits an ordered contiguous descriptor chain with one PREPARE
    /// durability barrier and one COMMIT durability barrier for the group.
    /// No receipt escapes before the final barrier succeeds.
    pub fn durably_commit_group(
        &mut self,
        descriptors: &[DurableRevisionDescriptor],
    ) -> Result<Vec<DurableCommitReceipt>, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if descriptors.is_empty() {
            return Ok(Vec::new());
        }
        let bound = self.bind_group_descriptors(descriptors)?;
        let mut prepared = Vec::with_capacity(bound.len());
        for descriptor in &bound {
            let (token, frame) = self
                .wal
                .append_prepare_unflushed_with_frame(descriptor)
                .inspect_err(|_| self.poisoned = true)?;
            self.mirror_streaming_frame(&frame);
            prepared.push(token);
        }
        self.wal.durability_barrier().inspect_err(|_| {
            self.poisoned = true;
        })?;
        self.barrier_streaming_shadow();
        self.advance_external_freshness_wal()?;

        let mut receipts = Vec::with_capacity(prepared.len());
        for token in prepared {
            let (receipt, frame) = self
                .wal
                .append_commit_unflushed_with_frame(token)
                .inspect_err(|_| self.poisoned = true)?;
            self.mirror_streaming_frame(&frame);
            receipts.push(receipt);
        }
        self.wal.durability_barrier().inspect_err(|_| {
            self.poisoned = true;
        })?;
        self.barrier_streaming_shadow();
        self.advance_external_freshness_wal()?;
        if let Err(error) = self.apply_group_commits(&bound, &receipts) {
            self.poisoned = true;
            return Err(error);
        }
        Ok(receipts)
    }

    pub fn rotate_checkpoint(
        &mut self,
        revision: &Revision,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let materialization_specs = self.materialization_specs.clone();
        let physical_artifact_specs = self.physical_artifact_specs.clone();
        self.rotate_checkpoint_with_materializations_and_physical_artifacts(
            revision,
            &materialization_specs,
            &physical_artifact_specs,
        )
    }

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
        let generation = next_generation(&self.directory)?;
        let payload = checkpoint::encode_revision(revision)?;
        if payload.len() > MAX_CHECKPOINT_LEN {
            return Err(DurabilityError::PayloadTooLarge);
        }
        let mut entries = Vec::with_capacity(self.prepared_transactions.len());
        for (&prepare_lsn, descriptor) in &self.prepared_transactions {
            let encoded = super::encode_prepare_payload(descriptor)?;
            entries.push(PreparedCutEntry {
                prepare_lsn,
                payload_crc32c: crc32c(&encoded),
                descriptor: descriptor.clone(),
            });
        }
        let prepared_capsule = PreparedCutCapsule { entries };
        let capsule_crc32c = write_prepared_cut_capsule(
            &prepared_capsule_path(&self.directory, generation),
            &prepared_capsule,
        )?;
        let metadata_record = metadata::DurableStoreMetadata {
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
        };
        let metadata_crc32c = write_metadata_file(
            &metadata_path(&self.directory, generation),
            &metadata_record,
        )?;
        let mut shadow_wal =
            FileRevisionWal::create_at_lsn(wal_path(&self.directory, generation), wal_first_lsn)?;
        shadow_wal.durability_barrier()?;
        sync_directory(&self.directory)?;
        self.streaming_checkpoint = Some(StreamingCheckpointJob {
            generation,
            cut_revision: revision.clone(),
            payload,
            chunk_size,
            chunk_crcs: Vec::new(),
            next_chunk: 0,
            checkpoint_crc32c: None,
            metadata_crc32c,
            capsule_crc32c,
            prepared_capsule,
            shadow_wal,
            wal_first_lsn,
            mirrored_lsn: wal_first_lsn - 1,
            durable_shadow_lsn: wal_first_lsn - 1,
            failed: false,
        });
        self.streaming_checkpoint_progress()
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
            let mut file =
                OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(checkpoint_chunk_path(
                        &self.directory,
                        job.generation,
                        ordinal,
                    ))?;
            file.write_all(chunk)?;
            file.sync_all()?;
            job.chunk_crcs.push(crc32c(chunk));
            job.next_chunk += 1;
        }
        if job.next_chunk == total && job.checkpoint_crc32c.is_none() {
            job.checkpoint_crc32c = Some(write_chunked_checkpoint_root(
                &self.directory,
                job.generation,
                job.cut_revision.id(),
                job.payload.len(),
                &job.chunk_crcs,
                job.chunk_size,
            )?);
            sync_directory(&self.directory)?;
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
        let job = self
            .streaming_checkpoint
            .take()
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
                reason: "shadow WAL is not durably caught up to active WAL",
            });
        }
        let shadow_bytes = fs::read(job.shadow_wal.path())?;
        let seeds = job.prepared_capsule.scan_seeds();
        let scan = super::scan_wal_seeded(
            &shadow_bytes,
            job.cut_revision.id(),
            job.wal_first_lsn,
            &seeds,
        )?;
        if scan.durable_revision() != self.durable_head || scan.next_lsn() <= job.mirrored_lsn {
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
            metadata_crc32c: job.metadata_crc32c,
            prepared_capsule_crc32c: job.capsule_crc32c,
        };
        let verified_cut = read_checkpoint_generation(
            &self.directory,
            candidate_manifest,
            &self.semantic_registry,
        )?;
        if verified_cut.id() != job.cut_revision.id() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "streaming checkpoint root does not decode to pinned cut",
            });
        }
        let metadata_bytes = fs::read(metadata_path(&self.directory, job.generation))?;
        if crc32c(&metadata_bytes) != job.metadata_crc32c {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "streaming checkpoint metadata changed before publication",
            });
        }
        let capsule_bytes = fs::read(prepared_capsule_path(&self.directory, job.generation))?;
        if crc32c(&capsule_bytes) != job.capsule_crc32c {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "prepared cut capsule changed before publication",
            });
        }
        let _ = decode_prepared_cut_capsule(&capsule_bytes)?;
        sync_directory(&self.directory)?;
        let mut authority_uncertain = false;
        if let Err(error) = publish_manifest_with_hook(
            &self.directory,
            candidate_manifest,
            &mut NoStoreFault,
            &mut authority_uncertain,
        ) {
            if authority_uncertain {
                self.poisoned = true;
            }
            return Err(error);
        }
        self.advance_external_freshness_generation(job.generation, job.mirrored_lsn)?;
        self.generation = job.generation;
        self.checkpoint = job.cut_revision;
        self.wal = job.shadow_wal;
        Ok(DurableGenerationReceipt {
            generation: self.generation,
            base_revision: self.checkpoint.id(),
        })
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

    fn mirror_streaming_frame(&mut self, frame: &super::EncodedFrame) {
        let Some(job) = &mut self.streaming_checkpoint else {
            return;
        };
        if job.failed {
            return;
        }
        if job.shadow_wal.append_exact_frame(frame).is_err() {
            job.failed = true;
            return;
        }
        job.mirrored_lsn = frame.lsn;
    }

    fn barrier_streaming_shadow(&mut self) {
        let Some(job) = &mut self.streaming_checkpoint else {
            return;
        };
        if job.failed || job.durable_shadow_lsn == job.mirrored_lsn {
            return;
        }
        if job.shadow_wal.durability_barrier().is_err() {
            job.failed = true;
            return;
        }
        job.durable_shadow_lsn = job.mirrored_lsn;
    }

    /// Publishes one logical migration-complement step in a fresh checkpoint
    /// generation before the corresponding schema transition is committed.
    /// An interrupted later transition may leave an orphan step, but can never
    /// leave a committed migration without its required complement authority.
    pub fn stage_migration_complement(
        &mut self,
        revision: &Revision,
        complement: crate::DurableMigrationComplement,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        complement
            .validate()
            .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })?;
        if complement.source_schema != revision.semantic_revision().schema {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "migration complement source schema does not match durable head schema",
            });
        }
        if let Some(previous) = self.migration_complements.last()
            && previous.target_schema != complement.source_schema
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "migration complement chain is discontinuous",
            });
        }
        self.migration_complements.push(complement);
        self.rotate_checkpoint(revision)
    }

    /// Irreversibly releases local complement payloads whose declared boundary
    /// is explicitly satisfied, then persists the tombstone-only chain in a
    /// new checkpoint generation. No-op releases do not rotate the generation.
    pub fn release_due_migration_complements(
        &mut self,
        revision: &Revision,
        epoch: u64,
    ) -> Result<Option<DurableGenerationReceipt>, DurabilityError> {
        if revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "complement release revision does not match durable WAL head",
            });
        }
        let mut changed = false;
        for complement in &mut self.migration_complements {
            changed |= complement.release_if_due(revision.id(), epoch);
        }
        if !changed {
            return Ok(None);
        }
        self.rotate_checkpoint(revision).map(Some)
    }

    pub fn rotate_checkpoint_with_materializations(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let physical_artifact_specs = self.physical_artifact_specs.clone();
        self.rotate_checkpoint_with_materializations_and_physical_artifacts(
            revision,
            materialization_specs,
            &physical_artifact_specs,
        )
    }

    pub fn rotate_checkpoint_with_materializations_and_physical_artifacts(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        self.rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
            revision,
            materialization_specs,
            physical_artifact_specs,
            &[],
        )
    }

    pub fn rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.streaming_checkpoint.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "synchronous checkpoint rotation is blocked by streaming checkpoint job",
            });
        }
        if revision.id() != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "checkpoint revision does not match durable WAL head",
            });
        }
        self.rotate_checkpoint_with_fault_policy(
            revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            &mut NoStoreFault,
        )
    }

    /// Re-encodes the fully recovered in-memory durable authority into the
    /// current writable component formats and publishes it as a fresh immutable
    /// generation. Historical source files are never modified in place.
    pub fn migrate_to_current_format(
        &mut self,
        revision: &Revision,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let materializations = self.materialization_specs.clone();
        let physical_artifacts = self.physical_artifact_specs.clone();
        let artifact_cores = self.artifact_cores.clone();
        self.rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
            revision,
            &materializations,
            &physical_artifacts,
            &artifact_cores,
        )
    }

    /// Returns whether the in-process store can no longer determine the
    /// authoritative durable generation without reopen/recovery.
    ///
    /// Failures that happen before manifest publication leave this false: the
    /// previous generation remains authoritative and serving may continue.
    #[must_use]
    pub const fn requires_recovery(&self) -> bool {
        self.poisoned
    }

    fn advance_external_freshness_wal(&mut self) -> Result<(), DurabilityError> {
        self.advance_external_freshness_generation(self.generation, self.wal.last_lsn())
    }

    fn advance_external_freshness_generation(
        &mut self,
        generation: u64,
        wal_lsn: u64,
    ) -> Result<(), DurabilityError> {
        let Some(mut freshness) = self.external_freshness.take() else {
            return Ok(());
        };
        let result =
            advance_external_freshness_state(&self.directory, generation, wal_lsn, &mut freshness);
        self.external_freshness = Some(freshness);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn rotate_checkpoint_with_fault_policy(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        hook: &mut impl StoreFaultHook,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let mut authority_uncertain = false;
        let result = self.rotate_checkpoint_with_hook(
            revision,
            materialization_specs,
            physical_artifact_specs,
            artifact_cores,
            hook,
            &mut authority_uncertain,
        );
        if result.is_err() && authority_uncertain {
            self.poisoned = true;
        }
        result
    }

    fn rotate_checkpoint_with_hook(
        &mut self,
        revision: &Revision,
        materialization_specs: &[DurableMaterializationSpec],
        physical_artifact_specs: &[DurablePhysicalArtifactSpec],
        artifact_cores: &[DurableArtifactCore],
        hook: &mut impl StoreFaultHook,
        authority_uncertain: &mut bool,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        let generation = next_generation(&self.directory)?;
        let checkpoint_file = checkpoint_path(&self.directory, generation);
        let checkpoint_crc32c = write_checkpoint_file(&checkpoint_file, revision)?;
        hook.hit(StoreFaultPoint::AfterCheckpointSync)?;
        let wal_file = wal_path(&self.directory, generation);
        let mut wal = FileRevisionWal::create(&wal_file)?;
        wal.barrier()?;
        hook.hit(StoreFaultPoint::AfterWalSync)?;
        let physical_artifact_specs = canonical_physical_artifact_specs(physical_artifact_specs);
        let metadata_record = metadata::DurableStoreMetadata {
            external_freshness: self
                .external_freshness
                .as_ref()
                .map(ExternalFreshnessState::metadata_binding),
            current_idempotency_epoch: self.current_idempotency_epoch,
            minimum_retry_epoch: self.minimum_retry_epoch,
            materializations: materialization_specs.to_vec(),
            physical_artifacts: physical_artifact_specs.clone(),
            artifact_cores: artifact_cores.to_vec(),
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
        };
        let metadata_file = metadata_path(&self.directory, generation);
        let metadata_crc32c = write_metadata_file(&metadata_file, &metadata_record)?;
        hook.hit(StoreFaultPoint::AfterMetadataSync)?;
        sync_directory(&self.directory)?;
        hook.hit(StoreFaultPoint::AfterPrerequisiteDirectorySync)?;
        publish_manifest_with_hook(
            &self.directory,
            ManifestRecord {
                generation,
                base_revision: revision.id(),
                published_head: revision.id(),
                wal_first_lsn: 1,
                published_tail_lsn: 0,
                checkpoint_crc32c,
                metadata_crc32c,
                prepared_capsule_crc32c: 0,
            },
            hook,
            authority_uncertain,
        )?;
        if let Some(mut freshness) = self.external_freshness.take() {
            if let Err(error) =
                advance_external_freshness_state(&self.directory, generation, 0, &mut freshness)
            {
                self.external_freshness = Some(freshness);
                *authority_uncertain = true;
                return Err(error);
            }
            self.external_freshness = Some(freshness);
        }
        self.generation = generation;
        self.checkpoint = revision.clone();
        self.wal = wal;
        self.materialization_specs = materialization_specs.to_vec();
        self.physical_artifact_specs = physical_artifact_specs;
        self.artifact_cores = artifact_cores.to_vec();
        self.prepared_transactions.clear();
        Ok(DurableGenerationReceipt {
            generation,
            base_revision: revision.id(),
        })
    }

    pub fn compact_obsolete_generations(&self) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.compact_obsolete_generations_with_hook(&mut NoStoreFault)
    }

    fn compact_obsolete_generations_with_hook(
        &self,
        hook: &mut impl StoreFaultHook,
    ) -> Result<(), DurabilityError> {
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let generation = parse_generation_name(name, "manifest-", ".cfmf")
                .or_else(|| parse_generation_name(name, "checkpoint-", ".cfcp"))
                .or_else(|| parse_generation_name(name, "wal-", ".cfmw"))
                .or_else(|| parse_generation_name(name, "metadata-", ".cfdm"))
                .or_else(|| parse_generation_name(name, "prepared-", ".cfpc"))
                .or_else(|| parse_checkpoint_chunk_generation(name));
            let is_pending = parse_generation_name(name, "pending-manifest-", ".tmp").is_some();
            if is_pending || generation.is_some_and(|generation| generation != self.generation) {
                hook.hit(StoreFaultPoint::BeforeCompactionRemove)?;
                fs::remove_file(entry.path())?;
                hook.hit(StoreFaultPoint::AfterCompactionRemove)?;
            }
        }
        sync_directory(&self.directory)?;
        hook.hit(StoreFaultPoint::AfterCompactionDirectorySync)?;
        Ok(())
    }
}

fn bind_prepare_descriptor(
    store: &mut DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
) -> Result<DurableRevisionDescriptor, DurabilityError> {
    let mut descriptor = descriptor.clone();
    descriptor.idempotency_epoch = store.current_idempotency_epoch;
    validate_prepare_identity(store, &descriptor)?;
    descriptor.revision_effect_id = Some(allocate_local_revision_effect_id(
        &mut store.next_revision_effect_id,
    )?);
    Ok(descriptor)
}

fn verify_returned_freshness_cut(
    config: &ExternalFreshnessConfig,
    expected: FreshnessCut,
    signed: &SignedFreshnessCut,
) -> Result<VerifiedFreshnessCut, DurabilityError> {
    let verified = verify_freshness_cut(&config.trust_roots, signed).map_err(|_| {
        DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority returned an invalid signature",
        }
    })?;
    if verified.cut != expected
        || verified.cut.store_id != config.store_id
        || verified.cut.deployment_policy_epoch != config.deployment_policy_epoch
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority signed another cut",
        });
    }
    Ok(verified)
}

fn preflight_external_freshness(
    directory: &Path,
    manifest: ManifestRecord,
    metadata: &metadata::DurableStoreMetadata,
    config: &ExternalFreshnessConfig,
    anchored: VerifiedFreshnessCut,
) -> Result<FreshnessCut, DurabilityError> {
    let binding = metadata
        .external_freshness
        .ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "published generation is missing external freshness binding",
        })?;
    if binding.store_id != config.store_id
        || binding.trust_root_epoch != config.trust_roots.epoch()
        || binding.deployment_policy_epoch != config.deployment_policy_epoch
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "published external freshness binding is stale or belongs to another store",
        });
    }
    if manifest.generation < anchored.cut.generation {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local durable generation was rolled back behind external freshness authority",
        });
    }
    if manifest.generation > anchored.cut.generation.saturating_add(1) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local durable generation jumped beyond external freshness authority",
        });
    }
    if manifest.generation == anchored.cut.generation.saturating_add(1)
        && binding.previous_generation_digest != Some(anchored.cut.generation_digest)
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local generation does not extend externally anchored predecessor",
        });
    }
    let generation_digest = generation_material_digest(directory, manifest.generation)?;
    if manifest.generation == anchored.cut.generation
        && generation_digest != anchored.cut.generation_digest
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "same-generation durable store fork detected by external freshness authority",
        });
    }
    let (wal_lsn, wal_digest) = wal_freshness_head(
        &wal_path(directory, manifest.generation),
        manifest.wal_first_lsn,
    )?;
    if manifest.generation == anchored.cut.generation {
        if wal_lsn < anchored.cut.wal_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "local WAL was truncated behind external freshness authority",
            });
        }
        let anchored_prefix = wal_freshness_digest_at_lsn(
            &wal_path(directory, manifest.generation),
            manifest.wal_first_lsn,
            anchored.cut.wal_lsn,
        )?;
        if anchored_prefix != anchored.cut.wal_digest {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "local WAL prefix forks from external freshness authority",
            });
        }
    }
    Ok(FreshnessCut {
        store_id: config.store_id,
        generation: manifest.generation,
        previous_generation: binding.previous_generation_digest,
        generation_digest,
        wal_lsn,
        wal_digest,
        trust_root_epoch: config.trust_roots.epoch(),
        deployment_policy_epoch: config.deployment_policy_epoch,
    })
}

fn advance_external_freshness_state(
    directory: &Path,
    generation: u64,
    wal_lsn: u64,
    state: &mut ExternalFreshnessState,
) -> Result<(), DurabilityError> {
    let manifest = read_manifest_generation(directory, generation)?;
    let metadata = read_published_metadata(directory, manifest)?;
    let binding = metadata
        .external_freshness
        .ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "externally anchored generation lost freshness binding",
        })?;
    let cut = FreshnessCut {
        store_id: state.config.store_id,
        generation,
        previous_generation: binding.previous_generation_digest,
        generation_digest: generation_material_digest(directory, generation)?,
        wal_lsn,
        wal_digest: wal_freshness_digest_at_lsn(
            &wal_path(directory, generation),
            manifest.wal_first_lsn,
            wal_lsn,
        )?,
        trust_root_epoch: state.config.trust_roots.epoch(),
        deployment_policy_epoch: state.config.deployment_policy_epoch,
    };
    let expected = state.current.map(|current| current.record_digest);
    let signed = state.authority.compare_and_advance_signed(expected, cut)?;
    state.current = Some(verify_returned_freshness_cut(&state.config, cut, &signed)?);
    Ok(())
}

fn generation_material_digest(
    directory: &Path,
    generation: u64,
) -> Result<AuthorityDigest, DurabilityError> {
    let mut material = Vec::new();
    material.extend_from_slice(b"CFMD-GENERATION-MATERIAL-v1\0");
    for path in [
        manifest_path(directory, generation),
        checkpoint_path(directory, generation),
        metadata_path(directory, generation),
    ] {
        material.extend_from_slice(&sha256(&fs::read(path)?).0);
    }
    let prepared = prepared_capsule_path(directory, generation);
    if prepared.is_file() {
        material.push(1);
        material.extend_from_slice(&sha256(&fs::read(prepared)?).0);
    } else {
        material.push(0);
    }
    let mut chunks = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (parse_checkpoint_chunk_generation(name) == Some(generation)).then_some(entry.path())
        })
        .collect::<Vec<_>>();
    chunks.sort();
    for path in chunks {
        material.extend_from_slice(&sha256(&fs::read(path)?).0);
    }
    Ok(AuthorityDigest(sha256(&material).0))
}

fn wal_freshness_head(
    path: &Path,
    first_lsn: u64,
) -> Result<(u64, AuthorityDigest), DurabilityError> {
    let bytes = fs::read(path)?;
    let mut offset = 0;
    let mut expected_lsn = first_lsn;
    while offset < bytes.len() {
        match super::read_frame(&bytes, offset, expected_lsn)? {
            super::FrameRead::Complete(frame) => {
                offset += frame.frame_len;
                expected_lsn = expected_lsn
                    .checked_add(1)
                    .ok_or(DurabilityError::LsnExhausted)?;
            }
            super::FrameRead::Tail(_) => break,
        }
    }
    let last_lsn = expected_lsn.saturating_sub(1);
    Ok((last_lsn, wal_prefix_digest(&bytes[..offset])))
}

fn wal_freshness_digest_at_lsn(
    path: &Path,
    first_lsn: u64,
    target_lsn: u64,
) -> Result<AuthorityDigest, DurabilityError> {
    let bytes = fs::read(path)?;
    if target_lsn < first_lsn {
        return Ok(wal_prefix_digest(&[]));
    }
    let mut offset = 0;
    let mut expected_lsn = first_lsn;
    loop {
        match super::read_frame(&bytes, offset, expected_lsn)? {
            super::FrameRead::Complete(frame) => {
                offset += frame.frame_len;
                if frame.lsn == target_lsn {
                    return Ok(wal_prefix_digest(&bytes[..offset]));
                }
                expected_lsn = expected_lsn
                    .checked_add(1)
                    .ok_or(DurabilityError::LsnExhausted)?;
            }
            super::FrameRead::Tail(_) => {
                return Err(DurabilityError::Protocol {
                    offset,
                    reason: "external freshness WAL head is beyond valid local WAL prefix",
                });
            }
        }
    }
}

fn wal_prefix_digest(bytes: &[u8]) -> AuthorityDigest {
    let mut material = Vec::with_capacity(32 + bytes.len());
    material.extend_from_slice(b"CFMD-WAL-FRESHNESS-PREFIX-v1\0");
    material.extend_from_slice(bytes);
    AuthorityDigest(sha256(&material).0)
}

fn read_manifest_generation(
    directory: &Path,
    generation: u64,
) -> Result<ManifestRecord, DurabilityError> {
    let bytes = fs::read(manifest_path(directory, generation))?;
    decode_manifest(&bytes)
}

fn validate_bound_prepare_intent(
    store: &mut DurableRevisionStore,
    descriptor: &DurableRevisionDescriptor,
) -> Result<(), DurabilityError> {
    match &descriptor.intent {
        DurableTransactionIntent::RelationResolutionExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            causal_parents,
            semantic_modules,
        } => validate_relation_resolution_prepare_intent(
            store,
            descriptor,
            *source_revision,
            *target_revision,
            *semantic_revision,
            &crate::DurableRelationResolution {
                relation_mutations: relation_mutations.clone(),
                rewrite_intents: rewrite_intents.clone(),
                causal_parents: causal_parents.clone(),
            },
            semantic_modules,
        ),
        intent @ DurableTransactionIntent::RelationRewriteExact { .. } => {
            validate_relation_rewrite_prepare_intent(store, descriptor, intent)
        }
        DurableTransactionIntent::RelationDataExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            semantic_modules,
        } => {
            validate_relation_prepare_intent(
                descriptor,
                *source_revision,
                *target_revision,
                *semantic_revision,
                relation_mutations,
                None,
            )?;
            install_semantic_module_packages(&mut store.semantic_registry, semantic_modules)?;
            Ok(())
        }
        DurableTransactionIntent::Exact {
            target_revision,
            encoded_target_revision,
            semantic_modules,
            ..
        } => {
            install_semantic_module_packages(&mut store.semantic_registry, semantic_modules)?;
            validate_full_revision_prepare_intent(
                descriptor,
                *target_revision,
                encoded_target_revision,
                &store.semantic_registry,
            )
        }
        DurableTransactionIntent::SchemaMigrationExact {
            source_revision,
            target_revision,
            encoded_target_revision,
            migration_complement,
            semantic_modules,
        } => validate_schema_migration_prepare_intent(
            store,
            descriptor,
            *source_revision,
            *target_revision,
            encoded_target_revision,
            migration_complement,
            semantic_modules,
        ),
        DurableTransactionIntent::LegacyTargetOnly { .. } => Err(DurabilityError::Protocol {
            offset: 0,
            reason: "new durable prepare does not carry exact transaction intent",
        }),
    }
}

impl RevisionDurability for DurableRevisionStore {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        let descriptor = bind_prepare_descriptor(self, descriptor)?;
        validate_bound_prepare_intent(self, &descriptor)?;
        let (token, frame) = self
            .wal
            .append_prepare_unflushed_with_frame(&descriptor)
            .inspect_err(|_| self.poisoned = true)?;
        self.mirror_streaming_frame(&frame);
        self.wal
            .durability_barrier()
            .inspect_err(|_| self.poisoned = true)?;
        self.barrier_streaming_shadow();
        if let Some(existing) = self
            .prepared_transactions
            .insert(token.prepare_lsn(), descriptor.clone())
            && existing != descriptor
        {
            self.poisoned = true;
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "prepare lsn was rebound to another descriptor",
            });
        }
        self.advance_external_freshness_wal()?;
        Ok(token)
    }

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let transaction_id = prepared.transaction_id();
        let descriptor = self
            .prepared_transactions
            .get(&prepared.prepare_lsn())
            .cloned()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "commit token has no prepared descriptor in this store",
            })?;
        if descriptor.transaction_id != transaction_id
            || descriptor.target_revision != prepared.target_revision()
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "commit token identity does not match prepared descriptor",
            });
        }
        let (receipt, frame) = self
            .wal
            .append_commit_unflushed_with_frame(prepared)
            .inspect_err(|_| self.poisoned = true)?;
        self.mirror_streaming_frame(&frame);
        self.wal
            .durability_barrier()
            .inspect_err(|_| self.poisoned = true)?;
        self.barrier_streaming_shadow();
        self.advance_external_freshness_wal()?;
        self.durable_head = receipt.target_revision();
        if let DurableTransactionIntent::SchemaMigrationExact {
            migration_complement,
            ..
        } = &descriptor.intent
            && let Err(error) = append_migration_complement(
                &mut self.migration_complements,
                self.checkpoint.semantic_revision().schema,
                migration_complement.clone(),
            )
        {
            self.poisoned = true;
            return Err(error);
        }
        let transaction_key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, transaction_id);
        if let Some(existing) = self
            .committed_transactions
            .insert(transaction_key, descriptor.intent.clone())
            && existing != descriptor.intent
        {
            self.poisoned = true;
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "committed transaction id changed exact intent",
            });
        }
        if let Err(error) = append_committed_revision_effect(
            &mut self.revision_effects,
            &mut self.revision_effect_frontiers,
            &descriptor,
        ) {
            self.poisoned = true;
            return Err(error);
        }
        Ok(receipt)
    }
}

fn lock_directory(directory: &Path) -> Result<File, DurabilityError> {
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(LOCK_FILE_NAME))?;
    lock.lock()?;
    Ok(lock)
}

fn checkpoint_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("checkpoint-{generation:020}.cfcp"))
}

fn checkpoint_chunk_path(directory: &Path, generation: u64, ordinal: usize) -> PathBuf {
    directory.join(format!(
        "checkpoint-{generation:020}-chunk-{ordinal:08}.cfck"
    ))
}

fn prepared_capsule_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("prepared-{generation:020}.cfpc"))
}

fn wal_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("wal-{generation:020}.cfmw"))
}

fn manifest_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("manifest-{generation:020}.cfmf"))
}

fn metadata_path(directory: &Path, generation: u64) -> PathBuf {
    directory.join(format!("metadata-{generation:020}.cfdm"))
}

fn encode_prepared_cut_capsule(capsule: &PreparedCutCapsule) -> Result<Vec<u8>, DurabilityError> {
    let mut payload = Vec::new();
    for entry in &capsule.entries {
        let encoded = super::encode_prepare_payload(&entry.descriptor)?;
        let encoded_len =
            u32::try_from(encoded.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
        payload.extend_from_slice(&entry.prepare_lsn.to_le_bytes());
        payload.extend_from_slice(&entry.payload_crc32c.to_le_bytes());
        payload.extend_from_slice(&entry.descriptor.target_revision.raw().to_le_bytes());
        payload.extend_from_slice(&encoded_len.to_le_bytes());
        payload.extend_from_slice(&encoded);
    }
    let count =
        u32::try_from(capsule.entries.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let mut out = Vec::with_capacity(PREPARED_CAPSULE_HEADER_LEN + payload.len());
    out.extend_from_slice(&PREPARED_CAPSULE_MAGIC);
    out.extend_from_slice(&PREPARED_CAPSULE_VERSION.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&crc32c(&payload).to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

fn decode_prepared_cut_capsule(bytes: &[u8]) -> Result<PreparedCutCapsule, DurabilityError> {
    if bytes.len() < PREPARED_CAPSULE_HEADER_LEN || bytes[..4] != PREPARED_CAPSULE_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "prepared cut capsule header mismatch",
        });
    }
    if read_u16(&bytes[4..6]) != PREPARED_CAPSULE_VERSION || read_u16(&bytes[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported prepared cut capsule format",
        });
    }
    let count =
        usize::try_from(read_u32(&bytes[8..12])).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let payload = &bytes[PREPARED_CAPSULE_HEADER_LEN..];
    if crc32c(payload) != read_u32(&bytes[12..16]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "prepared cut capsule checksum mismatch",
        });
    }
    let mut cursor = 0_usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if payload.len().saturating_sub(cursor) < 24 {
            return Err(DurabilityError::Corruption {
                offset: cursor,
                reason: "prepared cut capsule entry truncated",
            });
        }
        let prepare_lsn = read_u64(&payload[cursor..cursor + 8]);
        let payload_crc32c = read_u32(&payload[cursor + 8..cursor + 12]);
        let target_revision = RevisionId::new(read_u64(&payload[cursor + 12..cursor + 20]));
        let len = usize::try_from(read_u32(&payload[cursor + 20..cursor + 24]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        cursor += 24;
        let end = cursor
            .checked_add(len)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let encoded = payload
            .get(cursor..end)
            .ok_or(DurabilityError::Corruption {
                offset: cursor,
                reason: "prepared cut capsule payload truncated",
            })?;
        let descriptor =
            super::decode_prepare_payload(target_revision, encoded).map_err(|reason| {
                DurabilityError::Corruption {
                    offset: cursor,
                    reason,
                }
            })?;
        entries.push(PreparedCutEntry {
            prepare_lsn,
            payload_crc32c,
            descriptor,
        });
        cursor = end;
    }
    if cursor != payload.len() {
        return Err(DurabilityError::Corruption {
            offset: cursor,
            reason: "prepared cut capsule has trailing bytes",
        });
    }
    Ok(PreparedCutCapsule { entries })
}

fn write_prepared_cut_capsule(
    path: &Path,
    capsule: &PreparedCutCapsule,
) -> Result<u32, DurabilityError> {
    let bytes = encode_prepared_cut_capsule(capsule)?;
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(crc32c(&bytes))
}

fn write_chunked_checkpoint_root(
    directory: &Path,
    generation: u64,
    revision: RevisionId,
    logical_len: usize,
    chunk_crcs: &[u32],
    chunk_size: usize,
) -> Result<u32, DurabilityError> {
    let mut descriptors = Vec::with_capacity(chunk_crcs.len() * CHECKPOINT_CHUNK_DESCRIPTOR_LEN);
    for (ordinal, crc) in chunk_crcs.iter().copied().enumerate() {
        let start = ordinal.saturating_mul(chunk_size);
        let len = logical_len.saturating_sub(start).min(chunk_size);
        descriptors.extend_from_slice(
            &u32::try_from(ordinal)
                .map_err(|_| DurabilityError::PayloadTooLarge)?
                .to_le_bytes(),
        );
        descriptors.extend_from_slice(
            &u64::try_from(len)
                .map_err(|_| DurabilityError::PayloadTooLarge)?
                .to_le_bytes(),
        );
        descriptors.extend_from_slice(&crc.to_le_bytes());
    }
    let mut header = [0_u8; CHECKPOINT_HEADER_LEN];
    header[0..4].copy_from_slice(&CHECKPOINT_MAGIC);
    header[4..6].copy_from_slice(&CHECKPOINT_FORMAT_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(
        &u16::try_from(chunk_crcs.len())
            .map_err(|_| DurabilityError::PayloadTooLarge)?
            .to_le_bytes(),
    );
    header[8..16].copy_from_slice(
        &u64::try_from(logical_len)
            .map_err(|_| DurabilityError::PayloadTooLarge)?
            .to_le_bytes(),
    );
    header[16..24].copy_from_slice(&revision.raw().to_le_bytes());
    header[24..28].copy_from_slice(&crc32c(&descriptors).to_le_bytes());
    let header_crc = crc32c(&header[..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());
    let path = checkpoint_path(directory, generation);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    file.write_all(&header)?;
    file.write_all(&descriptors)?;
    file.sync_all()?;
    let mut complete = Vec::with_capacity(header.len() + descriptors.len());
    complete.extend_from_slice(&header);
    complete.extend_from_slice(&descriptors);
    Ok(crc32c(&complete))
}

fn read_checkpoint_generation(
    directory: &Path,
    manifest: ManifestRecord,
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    let root = fs::read(checkpoint_path(directory, manifest.generation))?;
    if crc32c(&root) != manifest.checkpoint_crc32c {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "published checkpoint file checksum mismatch",
        });
    }
    if root.len() < CHECKPOINT_HEADER_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header truncated",
        });
    }
    let version = read_u16(&root[4..6]);
    if version == LEGACY_CHECKPOINT_FORMAT_VERSION {
        return decode_checkpoint_file(&root, registry);
    }
    DurableFormatRegistry::require_supported(
        crate::DurableFormatComponent::CheckpointFile,
        version,
        &[CHECKPOINT_FORMAT_VERSION],
    )?;
    let header = &root[..CHECKPOINT_HEADER_LEN];
    if header[..4] != CHECKPOINT_MAGIC || crc32c(&header[..28]) != read_u32(&header[28..32]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root header mismatch",
        });
    }
    let chunk_count = usize::from(read_u16(&header[6..8]));
    let descriptor_len = chunk_count
        .checked_mul(CHECKPOINT_CHUNK_DESCRIPTOR_LEN)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    if root.len() != CHECKPOINT_HEADER_LEN + descriptor_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root descriptor length mismatch",
        });
    }
    let descriptors = &root[CHECKPOINT_HEADER_LEN..];
    if crc32c(descriptors) != read_u32(&header[24..28]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root descriptor checksum mismatch",
        });
    }
    let logical_len =
        usize::try_from(read_u64(&header[8..16])).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let mut logical = Vec::with_capacity(logical_len);
    for ordinal in 0..chunk_count {
        let off = ordinal * CHECKPOINT_CHUNK_DESCRIPTOR_LEN;
        if usize::try_from(read_u32(&descriptors[off..off + 4])).ok() != Some(ordinal) {
            return Err(DurabilityError::Corruption {
                offset: off,
                reason: "checkpoint chunk ordinal mismatch",
            });
        }
        let len = usize::try_from(read_u64(&descriptors[off + 4..off + 12]))
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        let expected_crc = read_u32(&descriptors[off + 12..off + 16]);
        let chunk = fs::read(checkpoint_chunk_path(
            directory,
            manifest.generation,
            ordinal,
        ))?;
        if chunk.len() != len || crc32c(&chunk) != expected_crc {
            return Err(DurabilityError::Corruption {
                offset: ordinal,
                reason: "checkpoint chunk integrity mismatch",
            });
        }
        logical.extend_from_slice(&chunk);
    }
    if logical.len() != logical_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint logical stream length mismatch",
        });
    }
    let revision = checkpoint::decode_revision(&logical, registry)?;
    if revision.id().raw() != read_u64(&header[16..24]) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint root cut revision mismatch",
        });
    }
    Ok(revision)
}

fn write_checkpoint_file(path: &Path, revision: &Revision) -> Result<u32, DurabilityError> {
    let payload = checkpoint::encode_revision(revision)?;
    if payload.len() > MAX_CHECKPOINT_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let payload_len = u64::try_from(payload.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let payload_crc = crc32c(&payload);
    let mut header = [0_u8; CHECKPOINT_HEADER_LEN];
    header[0..4].copy_from_slice(&CHECKPOINT_MAGIC);
    header[4..6].copy_from_slice(&LEGACY_CHECKPOINT_FORMAT_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(&0_u16.to_le_bytes());
    header[8..16].copy_from_slice(&payload_len.to_le_bytes());
    header[16..24].copy_from_slice(&revision.id().raw().to_le_bytes());
    header[24..28].copy_from_slice(&payload_crc.to_le_bytes());
    let header_crc = crc32c(&header[..28]);
    header[28..32].copy_from_slice(&header_crc.to_le_bytes());

    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&header)?;
    file.write_all(&payload)?;
    file.sync_all()?;
    let mut complete = Vec::with_capacity(header.len() + payload.len());
    complete.extend_from_slice(&header);
    complete.extend_from_slice(&payload);
    Ok(crc32c(&complete))
}

fn decode_checkpoint_file(
    bytes: &[u8],
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    if bytes.len() < CHECKPOINT_HEADER_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header truncated",
        });
    }
    let header = &bytes[..CHECKPOINT_HEADER_LEN];
    if header[..4] != CHECKPOINT_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint magic mismatch",
        });
    }
    let version = read_u16(&header[4..6]);
    DurableFormatRegistry::require_supported(
        crate::DurableFormatComponent::CheckpointFile,
        version,
        &[LEGACY_CHECKPOINT_FORMAT_VERSION],
    )?;
    if read_u16(&header[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported checkpoint file flags",
        });
    }
    if crc32c(&header[..28]) != read_u32(&header[28..32]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header checksum mismatch",
        });
    }
    let payload_len =
        usize::try_from(read_u64(&header[8..16])).map_err(|_| DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint payload length overflow",
        })?;
    if payload_len > MAX_CHECKPOINT_LEN
        || CHECKPOINT_HEADER_LEN.checked_add(payload_len) != Some(bytes.len())
    {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint payload length mismatch",
        });
    }
    let payload = &bytes[CHECKPOINT_HEADER_LEN..];
    if crc32c(payload) != read_u32(&header[24..28]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint payload checksum mismatch",
        });
    }
    let revision = checkpoint::decode_revision(payload, registry)?;
    if revision.id().raw() != read_u64(&header[16..24]) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "checkpoint header revision mismatch",
        });
    }
    Ok(revision)
}

fn write_metadata_file(
    path: &Path,
    metadata: &metadata::DurableStoreMetadata,
) -> Result<u32, DurabilityError> {
    let payload = metadata::encode(metadata)?;
    if payload.len() > MAX_METADATA_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let payload_len = u64::try_from(payload.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let payload_crc = crc32c(&payload);
    let mut header = [0_u8; METADATA_HEADER_LEN];
    header[0..4].copy_from_slice(&METADATA_MAGIC);
    header[4..6].copy_from_slice(&METADATA_FILE_VERSION.to_le_bytes());
    header[6..8].copy_from_slice(&0_u16.to_le_bytes());
    header[8..16].copy_from_slice(&payload_len.to_le_bytes());
    header[16..20].copy_from_slice(&payload_crc.to_le_bytes());
    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(&header)?;
    file.write_all(&payload)?;
    file.sync_all()?;
    let mut complete = Vec::with_capacity(header.len() + payload.len());
    complete.extend_from_slice(&header);
    complete.extend_from_slice(&payload);
    Ok(crc32c(&complete))
}

fn decode_metadata_file(bytes: &[u8]) -> Result<metadata::DurableStoreMetadata, DurabilityError> {
    if bytes.len() < METADATA_HEADER_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata header truncated",
        });
    }
    let header = &bytes[..METADATA_HEADER_LEN];
    if header[..4] != METADATA_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata magic mismatch",
        });
    }
    let version = read_u16(&header[4..6]);
    DurableFormatRegistry::require_supported(
        crate::DurableFormatComponent::MetadataFile,
        version,
        &[METADATA_FILE_VERSION],
    )?;
    if read_u16(&header[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported durable metadata flags",
        });
    }
    let payload_len =
        usize::try_from(read_u64(&header[8..16])).map_err(|_| DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload length overflow",
        })?;
    if payload_len > MAX_METADATA_LEN
        || METADATA_HEADER_LEN.checked_add(payload_len) != Some(bytes.len())
    {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload length mismatch",
        });
    }
    let payload = &bytes[METADATA_HEADER_LEN..];
    if crc32c(payload) != read_u32(&header[16..20]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "durable metadata payload checksum mismatch",
        });
    }
    metadata::decode(payload).map_err(|reason| DurabilityError::Corruption { offset: 0, reason })
}

fn publish_manifest_with_hook(
    directory: &Path,
    manifest: ManifestRecord,
    hook: &mut impl StoreFaultHook,
    authority_uncertain: &mut bool,
) -> Result<(), DurabilityError> {
    let final_path = manifest_path(directory, manifest.generation);
    let pending_path = directory.join(format!("pending-manifest-{:020}.tmp", manifest.generation));
    if pending_path.exists() {
        fs::remove_file(&pending_path)?;
    }
    let bytes = encode_manifest(manifest);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&pending_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    hook.hit(StoreFaultPoint::AfterPendingManifestSync)?;
    // From the rename attempt onward, an I/O error cannot be interpreted as
    // proof that the old manifest is still the only authoritative generation.
    *authority_uncertain = true;
    fs::rename(&pending_path, &final_path)?;
    hook.hit(StoreFaultPoint::AfterManifestRename)?;
    sync_directory(directory)?;
    hook.hit(StoreFaultPoint::AfterManifestDirectorySync)?;
    Ok(())
}

fn encode_manifest(manifest: ManifestRecord) -> [u8; MANIFEST_LEN] {
    let mut bytes = [0_u8; MANIFEST_LEN];
    bytes[0..4].copy_from_slice(&MANIFEST_MAGIC);
    bytes[4..6].copy_from_slice(&MANIFEST_FORMAT_VERSION.to_le_bytes());
    bytes[6..8].copy_from_slice(&0_u16.to_le_bytes());
    bytes[8..16].copy_from_slice(&manifest.generation.to_le_bytes());
    bytes[16..24].copy_from_slice(&manifest.base_revision.raw().to_le_bytes());
    bytes[24..32].copy_from_slice(&manifest.published_head.raw().to_le_bytes());
    bytes[32..40].copy_from_slice(&manifest.wal_first_lsn.to_le_bytes());
    bytes[40..48].copy_from_slice(&manifest.published_tail_lsn.to_le_bytes());
    bytes[48..52].copy_from_slice(&manifest.checkpoint_crc32c.to_le_bytes());
    bytes[52..56].copy_from_slice(&manifest.metadata_crc32c.to_le_bytes());
    bytes[56..60].copy_from_slice(&manifest.prepared_capsule_crc32c.to_le_bytes());
    let checksum = crc32c(&bytes[..60]);
    bytes[60..64].copy_from_slice(&checksum.to_le_bytes());
    bytes
}

fn decode_manifest(bytes: &[u8]) -> Result<ManifestRecord, DurabilityError> {
    if bytes.len() != MANIFEST_LEN && bytes.len() != LEGACY_MANIFEST_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "manifest length mismatch",
        });
    }
    if bytes[..4] != MANIFEST_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "manifest magic mismatch",
        });
    }
    let version = read_u16(&bytes[4..6]);
    DurableFormatRegistry::require_supported(
        crate::DurableFormatComponent::Manifest,
        version,
        &[LEGACY_MANIFEST_FORMAT_VERSION, MANIFEST_FORMAT_VERSION],
    )?;
    if read_u16(&bytes[6..8]) != 0 {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "unsupported manifest flags",
        });
    }
    if version == LEGACY_MANIFEST_FORMAT_VERSION {
        if bytes.len() != LEGACY_MANIFEST_LEN || crc32c(&bytes[..32]) != read_u32(&bytes[32..36]) {
            return Err(DurabilityError::Corruption {
                offset: 0,
                reason: "manifest checksum mismatch",
            });
        }
        let base_revision = RevisionId::new(read_u64(&bytes[16..24]));
        return Ok(ManifestRecord {
            generation: read_u64(&bytes[8..16]),
            base_revision,
            published_head: base_revision,
            wal_first_lsn: 1,
            published_tail_lsn: 0,
            checkpoint_crc32c: read_u32(&bytes[24..28]),
            metadata_crc32c: read_u32(&bytes[28..32]),
            prepared_capsule_crc32c: 0,
        });
    }
    if bytes.len() != MANIFEST_LEN || crc32c(&bytes[..60]) != read_u32(&bytes[60..64]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "manifest checksum mismatch",
        });
    }
    Ok(ManifestRecord {
        generation: read_u64(&bytes[8..16]),
        base_revision: RevisionId::new(read_u64(&bytes[16..24])),
        published_head: RevisionId::new(read_u64(&bytes[24..32])),
        wal_first_lsn: read_u64(&bytes[32..40]),
        published_tail_lsn: read_u64(&bytes[40..48]),
        checkpoint_crc32c: read_u32(&bytes[48..52]),
        metadata_crc32c: read_u32(&bytes[52..56]),
        prepared_capsule_crc32c: read_u32(&bytes[56..60]),
    })
}

fn read_current_manifest(directory: &Path) -> Result<ManifestRecord, DurabilityError> {
    let generation = highest_manifest_generation(directory)?.ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "durable store has no published manifest",
    })?;
    let bytes = fs::read(manifest_path(directory, generation))?;
    let manifest = decode_manifest(&bytes)?;
    if manifest.generation != generation {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "manifest filename generation mismatch",
        });
    }
    Ok(manifest)
}

fn highest_manifest_generation(directory: &Path) -> Result<Option<u64>, DurabilityError> {
    let mut highest = None;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if let Some(generation) = parse_generation_name(name, "manifest-", ".cfmf") {
            highest = Some(highest.map_or(generation, |current: u64| current.max(generation)));
        }
    }
    Ok(highest)
}

fn parse_generation_name(name: &str, prefix: &str, suffix: &str) -> Option<u64> {
    let raw = name.strip_prefix(prefix)?.strip_suffix(suffix)?;
    (raw.len() == 20).then(|| raw.parse().ok()).flatten()
}

fn parse_checkpoint_chunk_generation(name: &str) -> Option<u64> {
    let raw = name.strip_prefix("checkpoint-")?;
    let (generation, rest) = raw.split_once("-chunk-")?;
    if generation.len() != 20
        || !Path::new(rest)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cfck"))
    {
        return None;
    }
    generation.parse().ok()
}

fn next_generation(directory: &Path) -> Result<u64, DurabilityError> {
    let mut highest = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let generation = parse_generation_name(name, "manifest-", ".cfmf")
            .or_else(|| parse_generation_name(name, "checkpoint-", ".cfcp"))
            .or_else(|| parse_generation_name(name, "wal-", ".cfmw"))
            .or_else(|| parse_generation_name(name, "metadata-", ".cfdm"))
            .or_else(|| parse_generation_name(name, "prepared-", ".cfpc"))
            .or_else(|| parse_checkpoint_chunk_generation(name))
            .or_else(|| parse_generation_name(name, "pending-manifest-", ".tmp"));
        if let Some(generation) = generation {
            highest = highest.max(generation);
        }
    }
    highest.checked_add(1).ok_or(DurabilityError::LsnExhausted)
}

fn sync_directory(directory: &Path) -> Result<(), DurabilityError> {
    let directory = File::open(directory)?;
    directory.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod publication_model;

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use ed25519_dalek::{Signer, SigningKey};
    use kernel_auth::{
        SignedFreshnessCut, TrustRootSet, freshness_record_digest, key_id, sign_freshness_cut,
    };
    use kernel_model::{DatabaseState, Value};
    use kernel_revision::Revision;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{
        ClientTransactionId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId,
    };

    use super::*;
    use crate::{
        DurableRelationMutation, DurableRevisionDescriptor, DurableSequencerOrder, ReplicaId,
        ReplicatedEffectEnvelope, ReplicationAntiEntropyRequest, ReplicationBranchId,
        ReplicationDecisionLock, ReplicationDecisionVote, ReplicationEffectStage,
        ReplicationEffectVote, ReplicationFailureDetector, ReplicationHeartbeat,
        ReplicationIngestOutcome, ReplicationJointMembershipAck,
        ReplicationJointMembershipCertificate, ReplicationLeaderCertificate, ReplicationLeaderVote,
        ReplicationLockSummary, ReplicationMembership, ReplicationMembershipChange,
        ReplicationMembershipVote, ReplicationPeerAuthPolicy, ReplicationPeerEvidence,
        ReplicationQuorumAvailability, ReplicationQuorumCertificate, ReplicationQuorumLoss,
        ReplicationRecoveryAck, ReplicationRecoveryCertificate, ReplicationTermPromise,
        ReplicationTransportFrame, ReplicationTransportIngress, ReplicationTransportPayload,
        SignedReplicationPeerEvidence, SignedReplicationTransportFrame, replicated_effect_id,
        replication_membership_digest, replication_peer_evidence_signing_message,
        replication_transport_signing_message,
    };

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(1);
    const CRASH_WORKER_ENV: &str = "CFMD_CRASH_WORKER";
    const CRASH_DIR_ENV: &str = "CFMD_CRASH_DIR";
    const CRASH_POINT_ENV: &str = "CFMD_CRASH_POINT";
    const CRASH_READY_FILE: &str = ".cfmd-crash-ready";

    #[derive(Debug)]
    struct TestFreshnessAuthority {
        signing: SigningKey,
        current: Option<SignedFreshnessCut>,
    }

    impl ExternalFreshnessAuthority for TestFreshnessAuthority {
        fn read_signed(
            &mut self,
            _store_id: [u8; 32],
        ) -> Result<Option<SignedFreshnessCut>, DurabilityError> {
            Ok(self.current.clone())
        }

        fn compare_and_advance_signed(
            &mut self,
            expected_record: Option<AuthorityDigest>,
            next: FreshnessCut,
        ) -> Result<SignedFreshnessCut, DurabilityError> {
            let current = self.current.as_ref().map(freshness_record_digest);
            if current != expected_record {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "test freshness CAS mismatch",
                });
            }
            let signed = sign_freshness_cut(&self.signing, next);
            self.current = Some(signed.clone());
            Ok(signed)
        }
    }

    const FRESHNESS_OK: u8 = 0;
    const FRESHNESS_FAIL_READ: u8 = 1;
    const FRESHNESS_FAIL_BEFORE_APPLY: u8 = 2;
    const FRESHNESS_FAIL_AFTER_APPLY: u8 = 3;

    #[derive(Debug, Clone)]
    struct SharedFreshnessAuthority {
        signing: SigningKey,
        current: Arc<Mutex<Option<SignedFreshnessCut>>>,
        failure_mode: Arc<AtomicU8>,
    }

    impl SharedFreshnessAuthority {
        fn new(signing: SigningKey) -> Self {
            Self {
                signing,
                current: Arc::new(Mutex::new(None)),
                failure_mode: Arc::new(AtomicU8::new(FRESHNESS_OK)),
            }
        }

        fn boxed(&self) -> Box<dyn ExternalFreshnessAuthority> {
            Box::new(self.clone())
        }

        fn fail_once(&self, mode: u8) {
            self.failure_mode.store(mode, Ordering::SeqCst);
        }
    }

    impl ExternalFreshnessAuthority for SharedFreshnessAuthority {
        fn read_signed(
            &mut self,
            _store_id: [u8; 32],
        ) -> Result<Option<SignedFreshnessCut>, DurabilityError> {
            if self.failure_mode.swap(FRESHNESS_OK, Ordering::SeqCst) == FRESHNESS_FAIL_READ {
                return Err(DurabilityError::Io(std::io::Error::other(
                    "external freshness authority unavailable",
                )));
            }
            Ok(self.current.lock().unwrap().clone())
        }

        fn compare_and_advance_signed(
            &mut self,
            expected_record: Option<AuthorityDigest>,
            next: FreshnessCut,
        ) -> Result<SignedFreshnessCut, DurabilityError> {
            let mode = self.failure_mode.swap(FRESHNESS_OK, Ordering::SeqCst);
            if mode == FRESHNESS_FAIL_BEFORE_APPLY {
                return Err(DurabilityError::Io(std::io::Error::other(
                    "external freshness authority failed before CAS apply",
                )));
            }
            let mut current = self.current.lock().unwrap();
            if current.as_ref().map(freshness_record_digest) != expected_record {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "shared freshness CAS mismatch",
                });
            }
            let signed = sign_freshness_cut(&self.signing, next);
            *current = Some(signed.clone());
            drop(current);
            if mode == FRESHNESS_FAIL_AFTER_APPLY {
                return Err(DurabilityError::Io(std::io::Error::other(
                    "external freshness response lost after CAS apply",
                )));
            }
            Ok(signed)
        }
    }

    fn external_freshness_fixture(
        store_id: [u8; 32],
    ) -> (ExternalFreshnessConfig, SharedFreshnessAuthority) {
        let signing = SigningKey::from_bytes(&[92; 32]);
        let trust = TrustRootSet::bootstrap(13, &[signing.verifying_key().to_bytes()]).unwrap();
        (
            ExternalFreshnessConfig {
                store_id,
                trust_roots: trust,
                deployment_policy_epoch: 8,
            },
            SharedFreshnessAuthority::new(signing),
        )
    }

    struct BlockingKillFault {
        target: StoreFaultPoint,
        directory: PathBuf,
    }

    impl StoreFaultHook for BlockingKillFault {
        fn hit(&mut self, point: StoreFaultPoint) -> Result<(), DurabilityError> {
            if point == self.target {
                signal_crash_ready(&self.directory);
            }
            Ok(())
        }
    }

    struct ErrorFault {
        target: StoreFaultPoint,
    }

    impl StoreFaultHook for ErrorFault {
        fn hit(&mut self, point: StoreFaultPoint) -> Result<(), DurabilityError> {
            if point == self.target {
                return Err(DurabilityError::Io(std::io::Error::other(
                    "injected checkpoint publication failure",
                )));
            }
            Ok(())
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "cfmd-durability-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn setup_revision(id: u64, values: &[i64]) -> (Revision, SemanticRegistry, SemanticId) {
        let relation = SemanticId::new(1);
        let equivalence = SemanticId::new(2);
        let mut registry = SemanticRegistry::default();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(
            equivalence,
            registry.install_equivalence(EquivalenceModule::I64Exact),
        );
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.model.relations.insert(
            relation,
            values
                .iter()
                .map(|value| vec![Value::I64(*value)])
                .collect(),
        );
        (
            Revision::build(RevisionId::new(id), &context, &registry, state).unwrap(),
            registry,
            relation,
        )
    }

    #[test]
    fn external_freshness_blocks_plain_open_and_detects_generation_rollback() {
        let dir = test_dir("external-freshness-rollback");
        let (revision, registry, _) = setup_revision(990, &[1]);
        let signing = SigningKey::from_bytes(&[91; 32]);
        let trust = TrustRootSet::bootstrap(7, &[signing.verifying_key().to_bytes()]).unwrap();
        let config = ExternalFreshnessConfig {
            store_id: [17; 32],
            trust_roots: trust,
            deployment_policy_epoch: 4,
        };
        let authority = Box::new(TestFreshnessAuthority {
            signing,
            current: None,
        });
        let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
        let receipt = store
            .adopt_external_freshness(config.clone(), authority)
            .unwrap();
        assert_eq!(receipt.generation, 2);
        drop(store);

        assert!(matches!(
            DurableRevisionStore::open(&dir),
            Err(DurabilityError::Protocol {
                reason: "externally anchored store requires freshness-aware open",
                ..
            })
        ));
    }

    #[test]
    fn external_freshness_recovers_cas_response_loss_before_and_after_apply() {
        let before_dir = test_dir("external-freshness-response-loss-before");
        let (revision, registry, relation) = setup_revision(1_100, &[1]);
        let (config, authority) = external_freshness_fixture([0x41; 32]);
        let mut store = DurableRevisionStore::create(&before_dir, &revision, &registry).unwrap();
        store
            .adopt_external_freshness(config.clone(), authority.boxed())
            .unwrap();
        let descriptor = committed_descriptor(&revision, &registry, relation, 1_101, 2);
        authority.fail_once(FRESHNESS_FAIL_BEFORE_APPLY);
        assert!(store.durably_prepare(&descriptor).is_err());
        assert!(store.requires_recovery());
        drop(store);
        let (recovered, _) = DurableRevisionStore::open_with_external_freshness(
            &before_dir,
            config,
            authority.boxed(),
        )
        .unwrap();
        assert!(!recovered.requires_recovery());
        drop(recovered);
        fs::remove_dir_all(before_dir).unwrap();

        let after_dir = test_dir("external-freshness-response-loss-after");
        let (revision, registry, relation) = setup_revision(1_200, &[1]);
        let (config, authority) = external_freshness_fixture([0x46; 32]);
        let mut store = DurableRevisionStore::create(&after_dir, &revision, &registry).unwrap();
        store
            .adopt_external_freshness(config.clone(), authority.boxed())
            .unwrap();
        let descriptor = committed_descriptor(&revision, &registry, relation, 1_201, 3);
        authority.fail_once(FRESHNESS_FAIL_AFTER_APPLY);
        assert!(store.durably_prepare(&descriptor).is_err());
        assert!(store.requires_recovery());
        drop(store);
        let (recovered, _) = DurableRevisionStore::open_with_external_freshness(
            &after_dir,
            config,
            authority.boxed(),
        )
        .unwrap();
        assert!(!recovered.requires_recovery());
        drop(recovered);
        fs::remove_dir_all(after_dir).unwrap();
    }

    #[test]
    fn external_freshness_rejects_unavailable_stale_and_rolled_back_state_before_recovery() {
        let dir = test_dir("external-freshness-unavailable-rollback");
        let (revision, registry, _) = setup_revision(1_200, &[1]);
        let (config, authority) = external_freshness_fixture([0x42; 32]);
        let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
        store
            .adopt_external_freshness(config.clone(), authority.boxed())
            .unwrap();
        store.rotate_checkpoint(&revision).unwrap();
        drop(store);

        authority.fail_once(FRESHNESS_FAIL_READ);
        assert!(matches!(
            DurableRevisionStore::open_with_external_freshness(
                &dir,
                config.clone(),
                authority.boxed()
            ),
            Err(DurabilityError::Io(_))
        ));

        let mut stale = config.clone();
        stale.deployment_policy_epoch += 1;
        assert!(matches!(
            DurableRevisionStore::open_with_external_freshness(&dir, stale, authority.boxed()),
            Err(DurabilityError::Protocol {
                reason: "external freshness policy identity mismatch",
                ..
            })
        ));

        let generation = 3_u64;
        for path in [
            manifest_path(&dir, generation),
            checkpoint_path(&dir, generation),
            metadata_path(&dir, generation),
            wal_path(&dir, generation),
            prepared_capsule_path(&dir, generation),
        ] {
            let _ = fs::remove_file(path);
        }
        assert!(matches!(
            DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed()),
            Err(DurabilityError::Protocol {
                reason: "local durable generation was rolled back behind external freshness authority",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn external_freshness_rejects_same_generation_fork_and_wal_truncation() {
        let fork_dir = test_dir("external-freshness-generation-fork");
        let (revision, registry, _) = setup_revision(1_300, &[1]);
        let (config, authority) = external_freshness_fixture([0x43; 32]);
        let mut store = DurableRevisionStore::create(&fork_dir, &revision, &registry).unwrap();
        store
            .adopt_external_freshness(config.clone(), authority.boxed())
            .unwrap();
        drop(store);
        let checkpoint = checkpoint_path(&fork_dir, 2);
        let mut bytes = fs::read(&checkpoint).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x01;
        fs::write(&checkpoint, bytes).unwrap();
        assert!(matches!(
            DurableRevisionStore::open_with_external_freshness(
                &fork_dir,
                config,
                authority.boxed()
            ),
            Err(DurabilityError::Protocol {
                reason: "same-generation durable store fork detected by external freshness authority",
                ..
            })
        ));
        fs::remove_dir_all(fork_dir).unwrap();

        let wal_dir = test_dir("external-freshness-wal-truncation");
        let (revision, registry, relation) = setup_revision(1_400, &[1]);
        let (config, authority) = external_freshness_fixture([0x44; 32]);
        let mut store = DurableRevisionStore::create(&wal_dir, &revision, &registry).unwrap();
        store
            .adopt_external_freshness(config.clone(), authority.boxed())
            .unwrap();
        let descriptor = committed_descriptor(&revision, &registry, relation, 1_401, 2);
        store.durably_prepare(&descriptor).unwrap();
        drop(store);
        File::create(wal_path(&wal_dir, 2))
            .unwrap()
            .sync_all()
            .unwrap();
        assert!(matches!(
            DurableRevisionStore::open_with_external_freshness(&wal_dir, config, authority.boxed()),
            Err(DurabilityError::Protocol {
                reason: "local WAL was truncated behind external freshness authority",
                ..
            })
        ));
        fs::remove_dir_all(wal_dir).unwrap();
    }

    #[test]
    fn external_freshness_rejects_authenticated_wal_prefix_fork() {
        let dir = test_dir("external-freshness-wal-prefix-fork");
        let (revision, registry, relation) = setup_revision(1_500, &[1]);
        let (config, authority) = external_freshness_fixture([0x45; 32]);
        let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
        store
            .adopt_external_freshness(config.clone(), authority.boxed())
            .unwrap();
        let descriptor = committed_descriptor(&revision, &registry, relation, 1_501, 2);
        store.durably_prepare(&descriptor).unwrap();
        drop(store);

        {
            let mut current = authority.current.lock().unwrap();
            let mut cut = current.as_ref().unwrap().cut;
            cut.wal_digest = AuthorityDigest([0xEE; 32]);
            *current = Some(sign_freshness_cut(&authority.signing, cut));
        }
        assert!(matches!(
            DurableRevisionStore::open_with_external_freshness(&dir, config, authority.boxed()),
            Err(DurabilityError::Protocol {
                reason: "local WAL prefix forks from external freshness authority",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_complement_payload_survives_restart_and_release_is_persisted() {
        let dir = test_dir("migration-complement-retention");
        let (revision, registry, _) = setup_revision(901, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
        let durable = crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: revision.semantic_revision().schema,
                target_schema: SchemaRevisionId::new(2),
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_001)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_002)),
                encoding_version: 1,
                complement: Value::I64(77),
            },
            kernel_lens::ComplementRetention::UntilEpoch(10),
        );
        store
            .stage_migration_complement(&revision, durable)
            .unwrap();
        drop(store);

        let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.migration_complements().len(), 1);
        assert_eq!(
            reopened.migration_complements()[0]
                .local_capsule()
                .unwrap()
                .complement,
            Value::I64(77)
        );
        assert!(
            reopened
                .release_due_migration_complements(&revision, 9)
                .unwrap()
                .is_none()
        );
        assert!(
            reopened
                .release_due_migration_complements(&revision, 10)
                .unwrap()
                .is_some()
        );
        drop(reopened);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert!(reopened.migration_complements()[0].released);
        assert!(
            reopened.migration_complements()[0]
                .local_capsule()
                .is_none()
        );
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn external_archive_and_forget_never_persist_local_complement_authority() {
        let capsule = kernel_lens::ComplementCapsule {
            source_schema: SchemaRevisionId::new(1),
            target_schema: SchemaRevisionId::new(2),
            lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_101)),
            semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_102)),
            encoding_version: 1,
            complement: Value::I64(88),
        };
        let archive = crate::DurableMigrationComplement::from_capsule(
            capsule.clone(),
            kernel_lens::ComplementRetention::ExternalArchive(kernel_lens::ArchiveProofId(
                SemanticId::new(9_103),
            )),
        );
        let forgotten = crate::DurableMigrationComplement::from_capsule(
            capsule,
            kernel_lens::ComplementRetention::Forget,
        );
        assert!(archive.released && archive.local_complement.is_none());
        assert!(archive.archive_proof().is_some());
        assert!(forgotten.released && forgotten.local_complement.is_none());
    }

    fn durable_complement(
        source: u64,
        target: u64,
        payload: i64,
        retention: kernel_lens::ComplementRetention,
    ) -> crate::DurableMigrationComplement {
        crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: SchemaRevisionId::new(source),
                target_schema: SchemaRevisionId::new(target),
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(u128::from(9_300 + target))),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(u128::from(
                    9_400 + target,
                ))),
                encoding_version: 1,
                complement: Value::I64(payload),
            },
            retention,
        )
    }

    #[test]
    fn historical_chain_enforces_forget_archive_and_released_payload_boundaries() {
        let dir = test_dir("historical-chain-retention-boundary");
        let (revision, registry, _) = setup_revision(905, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
        store.migration_complements = vec![
            durable_complement(1, 2, 10, kernel_lens::ComplementRetention::Forever),
            durable_complement(2, 3, 20, kernel_lens::ComplementRetention::UntilEpoch(10)),
        ];
        let chain = store
            .local_historical_complement_chain(SchemaRevisionId::new(1), SchemaRevisionId::new(3))
            .unwrap();
        assert_eq!(chain.steps().len(), 2);
        assert_eq!(
            chain.steps()[0].local_capsule().unwrap().complement,
            Value::I64(10)
        );
        assert_eq!(
            chain.steps()[1].local_capsule().unwrap().complement,
            Value::I64(20)
        );

        assert!(store.migration_complements[1].release_if_due(revision.id(), 10));
        assert_eq!(
            store.local_historical_complement_chain(
                SchemaRevisionId::new(1),
                SchemaRevisionId::new(3),
            ),
            Err(crate::HistoricalComplementError::LocalPayloadReleased {
                source: SchemaRevisionId::new(2),
                target: SchemaRevisionId::new(3),
            })
        );

        let proof = kernel_lens::ArchiveProofId(SemanticId::new(9_999));
        store.migration_complements[1] = durable_complement(
            2,
            3,
            20,
            kernel_lens::ComplementRetention::ExternalArchive(proof),
        );
        assert_eq!(
            store.local_historical_complement_chain(
                SchemaRevisionId::new(1),
                SchemaRevisionId::new(3),
            ),
            Err(crate::HistoricalComplementError::ExternalArchiveRequired(
                proof
            ))
        );

        store.migration_complements[1] =
            durable_complement(2, 3, 20, kernel_lens::ComplementRetention::Forget);
        assert_eq!(
            store.local_historical_complement_chain(
                SchemaRevisionId::new(1),
                SchemaRevisionId::new(3),
            ),
            Err(crate::HistoricalComplementError::ExplicitlyForgotten {
                source: SchemaRevisionId::new(2),
                target: SchemaRevisionId::new(3),
            })
        );
        assert_eq!(
            store.local_historical_complement_chain(
                SchemaRevisionId::new(3),
                SchemaRevisionId::new(4),
            ),
            Err(crate::HistoricalComplementError::PathNotFound {
                source: SchemaRevisionId::new(3),
                target: SchemaRevisionId::new(4),
            })
        );
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn schema_migration_wal_atomically_recovers_complement_authority() {
        let dir = test_dir("schema-migration-atomic-complement");
        let (base, registry, _) = setup_revision(911, &[1]);
        let mut target_context = base.semantic_context().clone();
        target_context.schema.revision = SchemaRevisionId::new(2);
        let target = Revision::build(
            RevisionId::new(912),
            &target_context,
            &registry,
            base.state().clone(),
        )
        .unwrap();
        let complement = crate::DurableMigrationComplement::from_capsule(
            kernel_lens::ComplementCapsule {
                source_schema: base.semantic_revision().schema,
                target_schema: target.semantic_revision().schema,
                lens_spec: kernel_lens::LensSpecId(SemanticId::new(9_201)),
                semantic_pins: kernel_lens::SemanticManifestId(SemanticId::new(9_202)),
                encoding_version: 1,
                complement: Value::I64(99),
            },
            kernel_lens::ComplementRetention::Forever,
        );
        let descriptor = DurableRevisionDescriptor::schema_migration(
            kernel_types::ClientTransactionId::new(9_203),
            base.id(),
            &target,
            complement.clone(),
            &registry,
        )
        .unwrap();

        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        assert_eq!(
            store.migration_complements(),
            std::slice::from_ref(&complement)
        );
        drop(store);

        // No checkpoint rotation occurred after COMMIT. Reopen must recover
        // both the schema transition and complement from the same WAL tail.
        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), target.id());
        assert_eq!(
            reopened.migration_complements(),
            std::slice::from_ref(&complement)
        );
        assert!(matches!(
            scan.transaction_intent(kernel_types::ClientTransactionId::new(9_203)),
            Some(crate::DurableTransactionIntent::SchemaMigrationExact { .. })
        ));
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    fn crash_point_name(point: StoreFaultPoint) -> &'static str {
        match point {
            StoreFaultPoint::AfterCheckpointSync => "after-checkpoint-sync",
            StoreFaultPoint::AfterWalSync => "after-wal-sync",
            StoreFaultPoint::AfterMetadataSync => "after-metadata-sync",
            StoreFaultPoint::AfterPrerequisiteDirectorySync => "after-prerequisite-directory-sync",
            StoreFaultPoint::AfterPendingManifestSync => "after-pending-manifest-sync",
            StoreFaultPoint::AfterManifestRename => "after-manifest-rename",
            StoreFaultPoint::AfterManifestDirectorySync => "after-manifest-directory-sync",
            StoreFaultPoint::BeforeCompactionRemove => "before-compaction-remove",
            StoreFaultPoint::AfterCompactionRemove => "after-compaction-remove",
            StoreFaultPoint::AfterCompactionDirectorySync => "after-compaction-directory-sync",
        }
    }

    fn parse_crash_point(raw: &str) -> StoreFaultPoint {
        [
            StoreFaultPoint::AfterCheckpointSync,
            StoreFaultPoint::AfterWalSync,
            StoreFaultPoint::AfterMetadataSync,
            StoreFaultPoint::AfterPrerequisiteDirectorySync,
            StoreFaultPoint::AfterPendingManifestSync,
            StoreFaultPoint::AfterManifestRename,
            StoreFaultPoint::AfterManifestDirectorySync,
            StoreFaultPoint::BeforeCompactionRemove,
            StoreFaultPoint::AfterCompactionRemove,
            StoreFaultPoint::AfterCompactionDirectorySync,
        ]
        .into_iter()
        .find(|point| crash_point_name(*point) == raw)
        .unwrap_or_else(|| panic!("unknown crash point {raw}"))
    }

    fn signal_crash_ready(dir: &Path) -> ! {
        let marker = dir.join(CRASH_READY_FILE);
        let file = File::create(marker).unwrap();
        file.sync_all().unwrap();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }

    fn run_crash_worker(test_name: &str, dir: &Path, point: &str) {
        let marker = dir.join(CRASH_READY_FILE);
        let _ = fs::remove_file(&marker);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(test_name)
            .arg("--nocapture")
            .env(CRASH_WORKER_ENV, "1")
            .env(CRASH_DIR_ENV, dir)
            .env(CRASH_POINT_ENV, point)
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !marker.is_file() {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("crash worker exited before killpoint: {status}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "crash worker did not reach killpoint {point}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        child.kill().unwrap();
        let _ = child.wait().unwrap();
        let _ = fs::remove_file(marker);
    }

    fn committed_descriptor(
        base: &Revision,
        registry: &SemanticRegistry,
        relation: SemanticId,
        target_revision: u64,
        inserted: i64,
    ) -> DurableRevisionDescriptor {
        let mut state = base.state().clone();
        state
            .model
            .relations
            .entry(relation)
            .or_default()
            .push(vec![Value::I64(inserted)]);
        let target = Revision::build(
            RevisionId::new(target_revision),
            base.semantic_context(),
            registry,
            state,
        )
        .unwrap();
        DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(u128::from(target_revision)),
            base.id(),
            &target,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(inserted)]],
                removed: Vec::new(),
            }],
            registry,
        )
        .unwrap()
    }

    fn transition_from(
        base: &Revision,
        registry: &SemanticRegistry,
        relation: SemanticId,
        target_revision: u64,
        inserted: i64,
    ) -> (Revision, DurableRevisionDescriptor) {
        let mut state = base.state().clone();
        state
            .model
            .relations
            .entry(relation)
            .or_default()
            .push(vec![Value::I64(inserted)]);
        let target = Revision::build(
            RevisionId::new(target_revision),
            base.semantic_context(),
            registry,
            state,
        )
        .unwrap();
        let descriptor = DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(u128::from(target_revision)),
            base.id(),
            &target,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(inserted)]],
                removed: Vec::new(),
            }],
            registry,
        )
        .unwrap();
        (target, descriptor)
    }

    #[test]
    fn streaming_checkpoint_capsule_recovers_prepare_before_cut_commit_after_cut() {
        let dir = test_dir("streaming-cross-cut-prepare");
        let (base, registry, relation) = setup_revision(1, &[1]);
        let (_target, descriptor) = transition_from(&base, &registry, relation, 2, 2);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();

        let started = store
            .begin_streaming_checkpoint_with_chunk_size(&base, 32)
            .unwrap();
        assert_eq!(started.mirrored_lsn, prepared.prepare_lsn());
        assert!(store.has_streaming_checkpoint());
        store.write_streaming_checkpoint_chunks(1).unwrap();
        store.durably_commit(prepared).unwrap();
        let progress = store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
        assert!(progress.ready_to_publish);
        let receipt = store.finalize_streaming_checkpoint().unwrap();
        assert_eq!(receipt.base_revision, base.id());
        assert_eq!(store.durable_head(), RevisionId::new(2));
        drop(store);

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.checkpoint_revision().id(), base.id());
        assert_eq!(scan.durable_revision(), RevisionId::new(2));
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn streaming_checkpoint_allows_commits_during_build_and_after_publication() {
        let dir = test_dir("streaming-live-tail");
        let (base, registry, relation) = setup_revision(10, &[1]);
        let (r11, d11) = transition_from(&base, &registry, relation, 11, 2);
        let (r12, d12) = transition_from(&r11, &registry, relation, 12, 3);
        let (_r13, d13) = transition_from(&r12, &registry, relation, 13, 4);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .begin_streaming_checkpoint_with_chunk_size(&base, 24)
            .unwrap();
        store.write_streaming_checkpoint_chunks(1).unwrap();
        let p11 = store.durably_prepare(&d11).unwrap();
        store.durably_commit(p11).unwrap();
        store.write_streaming_checkpoint_chunks(1).unwrap();
        let p12 = store.durably_prepare(&d12).unwrap();
        store.durably_commit(p12).unwrap();
        store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
        store.finalize_streaming_checkpoint().unwrap();
        assert_eq!(store.durable_head(), r12.id());

        // The manifest certifies publication at R12, but the same shadow WAL
        // becomes active and may legally grow beyond that certificate.
        let p13 = store.durably_prepare(&d13).unwrap();
        store.durably_commit(p13).unwrap();
        drop(store);
        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.checkpoint_revision().id(), base.id());
        assert_eq!(scan.durable_revision(), RevisionId::new(13));
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn streaming_checkpoint_chunk_corruption_blocks_publish_and_old_authority_survives() {
        let dir = test_dir("streaming-corrupt-chunk");
        let (base, registry, _) = setup_revision(20, &[1, 2, 3]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let progress = store
            .begin_streaming_checkpoint_with_chunk_size(&base, 16)
            .unwrap();
        store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
        let chunk = checkpoint_chunk_path(&dir, progress.generation, 0);
        let mut bytes = fs::read(&chunk).unwrap();
        bytes[0] ^= 0x5a;
        fs::write(&chunk, bytes).unwrap();
        assert!(matches!(
            store.finalize_streaming_checkpoint(),
            Err(DurabilityError::Corruption { .. })
        ));
        assert!(!store.requires_recovery());
        drop(store);
        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), base.id());
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_unpublished_shadow_job_does_not_poison_published_store() {
        let dir = test_dir("streaming-shadow-abort");
        let (base, registry, relation) = setup_revision(30, &[1]);
        let (_target, descriptor) = transition_from(&base, &registry, relation, 31, 2);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store.begin_streaming_checkpoint(&base).unwrap();
        store.write_streaming_checkpoint_chunks(usize::MAX).unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        store.streaming_checkpoint.as_mut().unwrap().failed = true;
        assert!(matches!(
            store.finalize_streaming_checkpoint(),
            Err(DurabilityError::Protocol { .. })
        ));
        assert!(!store.requires_recovery());
        drop(store);
        let (_reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(31));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn group_commit_publishes_contiguous_chain_only_after_shared_barriers() {
        let dir = test_dir("group-commit-chain");
        let (base, registry, relation) = setup_revision(20, &[1]);
        let (middle, _, _) = setup_revision(21, &[1, 2]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let first = committed_descriptor(&base, &registry, relation, 21, 2);
        let second = committed_descriptor(&middle, &registry, relation, 22, 3);

        let receipts = store
            .durably_commit_group(&[first.clone(), second.clone()])
            .unwrap();
        assert_eq!(receipts.len(), 2);
        assert_eq!(store.durable_head(), RevisionId::new(22));
        assert_eq!(
            store.transaction_outcome(ClientTransactionId::new(21)),
            DurableTransactionOutcome::Committed {
                target_revision: RevisionId::new(21)
            }
        );
        assert_eq!(
            store.transaction_outcome(ClientTransactionId::new(22)),
            DurableTransactionOutcome::Committed {
                target_revision: RevisionId::new(22)
            }
        );
        drop(store);

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(22));
        assert_eq!(scan.committed().len(), 2);
        assert_eq!(
            reopened.transaction_outcome(ClientTransactionId::new(21)),
            DurableTransactionOutcome::Committed {
                target_revision: RevisionId::new(21)
            }
        );
        assert_eq!(
            reopened.transaction_outcome(ClientTransactionId::new(22)),
            DurableTransactionOutcome::Committed {
                target_revision: RevisionId::new(22)
            }
        );
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn group_commit_rejects_noncontiguous_or_duplicate_transaction_chain() {
        let dir = test_dir("group-commit-invalid");
        let (base, registry, relation) = setup_revision(30, &[1]);
        let (middle, _, _) = setup_revision(31, &[1, 2]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let first = committed_descriptor(&base, &registry, relation, 31, 2);
        let noncontiguous = committed_descriptor(&base, &registry, relation, 33, 3);
        assert!(matches!(
            store.durably_commit_group(&[first.clone(), noncontiguous]),
            Err(DurabilityError::Protocol {
                reason: "group commit descriptors are not a contiguous revision chain",
                ..
            })
        ));
        assert_eq!(store.durable_head(), RevisionId::new(30));

        let mut duplicate = committed_descriptor(&middle, &registry, relation, 32, 3);
        duplicate.transaction_id = first.transaction_id;
        assert!(matches!(
            store.durably_commit_group(&[first, duplicate]),
            Err(DurabilityError::Protocol {
                reason: "group commit repeats a transaction id",
                ..
            })
        ));
        assert_eq!(store.durable_head(), RevisionId::new(30));
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn async_commit_batcher_never_acknowledges_before_flush_and_retains_on_failure() {
        let dir = test_dir("group-commit-batcher");
        let (base, registry, relation) = setup_revision(40, &[1]);
        let (middle, _, _) = setup_revision(41, &[1, 2]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let first = committed_descriptor(&base, &registry, relation, 41, 2);
        let second = committed_descriptor(&middle, &registry, relation, 42, 3);
        let mut batcher = DurableCommitBatcher::new(DurableCommitBatchPolicy::new(2).unwrap());

        assert_eq!(
            batcher.enqueue(&store, first.clone()).unwrap(),
            DurableBatchEnqueueOutcome::Queued
        );
        assert_eq!(
            store.transaction_outcome(first.transaction_id),
            DurableTransactionOutcome::Unknown
        );
        assert_eq!(
            batcher.enqueue(&store, second.clone()).unwrap(),
            DurableBatchEnqueueOutcome::FlushRequired
        );
        assert_eq!(batcher.pending_len(), 2);

        // A failed flush does not discard exact pending descriptors.
        store.poisoned = true;
        assert!(matches!(
            batcher.flush(&mut store),
            Err(DurabilityError::Poisoned)
        ));
        assert_eq!(batcher.pending_len(), 2);
        store.poisoned = false;

        let receipts = batcher.flush(&mut store).unwrap();
        assert_eq!(receipts.len(), 2);
        assert!(batcher.is_empty());
        assert_eq!(store.durable_head(), RevisionId::new(42));
        drop(store);

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(42));
        assert_eq!(scan.committed().len(), 2);
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn store_reopens_exact_checkpoint_and_committed_wal_tail() {
        let dir = test_dir("reopen");
        let (base, registry, relation) = setup_revision(10, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let (target, _, _) = setup_revision(11, &[1, 2]);
        let descriptor = DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(11),
            RevisionId::new(10),
            &target,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(2)]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        assert_eq!(store.durable_head(), RevisionId::new(11));
        assert_eq!(store.causal_coverage_root(), RevisionId::new(10));
        let effect_11 = *store
            .revision_effect_frontier(RevisionId::new(11))
            .unwrap()
            .iter()
            .next()
            .unwrap();
        assert_eq!(
            store.revision_effect_frontier(RevisionId::new(11)),
            Some(&BTreeSet::from([effect_11]))
        );
        let ideal = store
            .revision_effect_ideal(RevisionId::new(11))
            .unwrap()
            .unwrap();
        assert_eq!(ideal.events().len(), 1);
        assert_eq!(ideal.events()[&effect_11].payload, descriptor.intent);
        drop(store);

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.checkpoint_revision(), &base);
        assert_eq!(reopened.durable_head(), RevisionId::new(11));
        assert_eq!(scan.durable_revision(), RevisionId::new(11));
        assert_eq!(
            reopened.revision_effect_frontier(RevisionId::new(11)),
            Some(&BTreeSet::from([effect_11]))
        );
        assert_eq!(
            reopened
                .revision_effect_record(effect_11)
                .unwrap()
                .transaction_id,
            ClientTransactionId::new(11)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn multi_parent_resolution_derives_exact_causal_cut_and_recovers_it() {
        let dir = test_dir("multi-parent-resolution-cut");
        let (base, registry, relation) = setup_revision(30, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

        for (source, target, inserted) in [(30, 31, 2), (31, 32, 3)] {
            let (target_revision, _, _) = setup_revision(target, &[1, 2, 3]);
            let descriptor = DurableRevisionDescriptor::relation_data(
                ClientTransactionId::new(u128::from(target)),
                RevisionId::new(source),
                &target_revision,
                base.semantic_revision(),
                vec![DurableRelationMutation {
                    relation,
                    inserted: vec![vec![Value::I64(inserted)]],
                    removed: Vec::new(),
                }],
                &registry,
            )
            .unwrap();
            let prepared = store.durably_prepare(&descriptor).unwrap();
            store.durably_commit(prepared).unwrap();
        }

        let (resolved, _, _) = setup_revision(33, &[1, 2, 3, 4]);
        let descriptor = DurableRevisionDescriptor::relation_resolution(
            ClientTransactionId::new(0x3033),
            RevisionId::new(32),
            &resolved,
            base.semantic_revision(),
            crate::DurableRelationResolution {
                relation_mutations: vec![DurableRelationMutation {
                    relation,
                    inserted: vec![vec![Value::I64(4)]],
                    removed: Vec::new(),
                }],
                rewrite_intents: vec![crate::DurableRelationRewriteIntent {
                    relation,
                    rewrite_spec: SemanticId::new(0x301),
                    law_set: SemanticId::new(0x302),
                }],
                causal_parents: vec![RevisionId::new(31), RevisionId::new(32)],
            },
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();

        let effect_31 = *store
            .revision_effect_frontier(RevisionId::new(31))
            .unwrap()
            .iter()
            .next()
            .unwrap();
        let effect_32 = *store
            .revision_effect_frontier(RevisionId::new(32))
            .unwrap()
            .iter()
            .next()
            .unwrap();
        let resolution_effect = *store
            .revision_effect_frontier(RevisionId::new(33))
            .unwrap()
            .iter()
            .next()
            .unwrap();
        let effect = store.revision_effect_record(resolution_effect).unwrap();
        assert_eq!(effect.kind(), crate::DurableEffectKind::RelationResolution);
        assert_eq!(
            effect.coordination_class(),
            crate::DurableEffectCoordinationClass::OpaqueNonConfluent
        );
        assert_eq!(effect.prerequisites, BTreeSet::from([effect_31, effect_32]));
        drop(store);

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(33));
        assert!(matches!(
            reopened.transaction_intent(ClientTransactionId::new(0x3033)),
            Some(crate::DurableTransactionIntent::RelationResolutionExact {
                causal_parents,
                ..
            }) if causal_parents == &vec![RevisionId::new(31), RevisionId::new(32)]
        ));
        assert_eq!(
            reopened
                .revision_effect_record(resolution_effect)
                .unwrap()
                .prerequisites,
            BTreeSet::from([effect_31, effect_32])
        );
        drop(reopened);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unpublished_checkpoint_failure_keeps_previous_authority_usable() {
        let safe_points = [
            StoreFaultPoint::AfterCheckpointSync,
            StoreFaultPoint::AfterWalSync,
            StoreFaultPoint::AfterMetadataSync,
            StoreFaultPoint::AfterPrerequisiteDirectorySync,
            StoreFaultPoint::AfterPendingManifestSync,
        ];

        for point in safe_points {
            let dir = test_dir(&format!("checkpoint-unpublished-{point:?}"));
            let (base, registry, relation) = setup_revision(20, &[1]);
            let (next, _, _) = setup_revision(21, &[1, 2]);
            let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
            let descriptor = committed_descriptor(&base, &registry, relation, 21, 2);
            let prepared = store.durably_prepare(&descriptor).unwrap();
            store.durably_commit(prepared).unwrap();
            let specs = store.materialization_specs().to_vec();
            let mut hook = ErrorFault { target: point };

            assert!(
                store
                    .rotate_checkpoint_with_fault_policy(&next, &specs, &[], &[], &mut hook)
                    .is_err(),
                "{point:?}"
            );
            assert!(!store.requires_recovery(), "{point:?}");
            assert_eq!(store.durable_head(), RevisionId::new(21), "{point:?}");
            assert_eq!(
                store.checkpoint_revision().id(),
                RevisionId::new(20),
                "{point:?}"
            );

            // The failed generation was never authoritative. A fresh rotation
            // may skip its orphan generation number and still publish normally.
            let receipt = store.rotate_checkpoint(&next).unwrap();
            assert!(receipt.generation >= 2, "{point:?}");
            assert_eq!(store.checkpoint_revision().id(), RevisionId::new(21));
            drop(store);
            let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
            assert_eq!(reopened.durable_head(), RevisionId::new(21));
            assert!(scan.committed().is_empty());
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn manifest_publication_failure_requires_recovery() {
        let uncertain_points = [
            StoreFaultPoint::AfterManifestRename,
            StoreFaultPoint::AfterManifestDirectorySync,
        ];

        for point in uncertain_points {
            let dir = test_dir(&format!("checkpoint-uncertain-{point:?}"));
            let (base, registry, relation) = setup_revision(30, &[1]);
            let (next, _, _) = setup_revision(31, &[1, 2]);
            let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
            let descriptor = committed_descriptor(&base, &registry, relation, 31, 2);
            let prepared = store.durably_prepare(&descriptor).unwrap();
            store.durably_commit(prepared).unwrap();
            let specs = store.materialization_specs().to_vec();
            let mut hook = ErrorFault { target: point };

            assert!(
                store
                    .rotate_checkpoint_with_fault_policy(&next, &specs, &[], &[], &mut hook)
                    .is_err(),
                "{point:?}"
            );
            assert!(store.requires_recovery(), "{point:?}");
            assert!(matches!(
                store.rotate_checkpoint(&next),
                Err(DurabilityError::Poisoned)
            ));

            // Reopen is the only authority-resolution path. Depending on where
            // the injected acknowledgement failure occurred, generation 2 is
            // already visible and must be accepted as authoritative.
            drop(store);
            let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
            assert_eq!(reopened.durable_head(), RevisionId::new(31), "{point:?}");
            assert_eq!(
                reopened.checkpoint_revision().id(),
                RevisionId::new(31),
                "{point:?}"
            );
            assert!(scan.committed().is_empty(), "{point:?}");
            drop(reopened);
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn checkpoint_rotation_publishes_new_generation_and_resets_wal_base() {
        let dir = test_dir("rotate");
        let (base, registry, relation) = setup_revision(20, &[1]);
        let (next, _, _) = setup_revision(21, &[1, 2]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let descriptor = DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(21),
            RevisionId::new(20),
            &next,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(2)]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&descriptor).unwrap();
        store.durably_commit(prepared).unwrap();
        let receipt = store.rotate_checkpoint(&next).unwrap();
        assert_eq!(receipt.generation, 2);
        assert_eq!(store.checkpoint_revision(), &next);
        drop(store);

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.generation(), 2);
        assert_eq!(reopened.checkpoint_revision(), &next);
        assert_eq!(scan.base_revision(), RevisionId::new(21));
        assert!(scan.committed().is_empty());
        assert_eq!(reopened.causal_coverage_root(), RevisionId::new(20));
        let effect_21 = reopened
            .revision_effect_frontier(RevisionId::new(21))
            .unwrap();
        assert_eq!(effect_21.len(), 1);
        assert!(
            reopened
                .revision_effect_ideal(RevisionId::new(21))
                .unwrap()
                .is_some_and(|ideal| ideal.events().len() == 1)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unpublished_orphan_generation_is_ignored_and_next_rotation_skips_it() {
        let dir = test_dir("orphan");
        let (base, registry, _) = setup_revision(30, &[1]);
        let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        drop(store);
        fs::copy(checkpoint_path(&dir, 1), checkpoint_path(&dir, 2)).unwrap();
        File::create(wal_path(&dir, 2)).unwrap().sync_all().unwrap();
        fs::write(
            dir.join("pending-manifest-00000000000000000002.tmp"),
            b"torn",
        )
        .unwrap();

        let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.generation(), 1);
        assert_eq!(scan.durable_revision(), RevisionId::new(30));
        let receipt = reopened.rotate_checkpoint(&base).unwrap();
        assert_eq!(receipt.generation, 3);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn published_manifest_does_not_fallback_when_checkpoint_is_corrupt() {
        let dir = test_dir("corrupt");
        let (base, registry, _) = setup_revision(40, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store.rotate_checkpoint(&base).unwrap();
        drop(store);
        let checkpoint = checkpoint_path(&dir, 2);
        let mut bytes = fs::read(&checkpoint).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0x80;
        fs::write(checkpoint, bytes).unwrap();

        assert!(matches!(
            DurableRevisionStore::open(&dir),
            Err(DurabilityError::Corruption { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unsupported_highest_manifest_format_never_falls_back() {
        let dir = test_dir("unsupported-highest-format");
        let (base, registry, _) = setup_revision(41, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store.rotate_checkpoint(&base).unwrap();
        drop(store);

        let path = manifest_path(&dir, 2);
        let mut bytes = fs::read(&path).unwrap();
        bytes[4..6].copy_from_slice(&99_u16.to_le_bytes());
        let checksum = crc32c(&bytes[..32]);
        bytes[32..36].copy_from_slice(&checksum.to_le_bytes());
        fs::write(path, bytes).unwrap();

        assert!(matches!(
            DurableRevisionStore::open(&dir),
            Err(DurabilityError::UnsupportedDurableFormat {
                component: crate::DurableFormatComponent::Manifest,
                version: 99,
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn canonical_format_migration_republishes_historical_metadata_without_loss() {
        let dir = test_dir("canonical-format-migration");
        let (base, registry, _) = setup_revision(42, &[1]);
        let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        drop(store);

        // Re-express generation 1 with the historical metadata-v6 semantics.
        // V6 predates migration complements, causal state, artifact cores and
        // idempotency epochs; their unique historical meaning is empty/zero.
        let mut payload = Vec::new();
        payload.extend_from_slice(&6_u16.to_le_bytes());
        metadata::encode_materialization_specs(&mut payload, &[]).unwrap();
        payload.extend_from_slice(&crate::PHYSICAL_ARTIFACT_RECIPE_VERSION.to_le_bytes());
        crate::push_len(&mut payload, 0).unwrap();
        crate::push_len(&mut payload, 0).unwrap();
        let modules = registry
            .builtin_modules_for_context(base.semantic_context())
            .unwrap();
        metadata::encode_semantic_module_specs(&mut payload, &modules).unwrap();

        let payload_crc = crc32c(&payload);
        let mut header = [0_u8; METADATA_HEADER_LEN];
        header[0..4].copy_from_slice(&METADATA_MAGIC);
        header[4..6].copy_from_slice(&METADATA_FILE_VERSION.to_le_bytes());
        header[6..8].copy_from_slice(&0_u16.to_le_bytes());
        header[8..16].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        header[16..20].copy_from_slice(&payload_crc.to_le_bytes());
        let mut historical_file = Vec::new();
        historical_file.extend_from_slice(&header);
        historical_file.extend_from_slice(&payload);
        fs::write(metadata_path(&dir, 1), &historical_file).unwrap();

        let manifest_path = manifest_path(&dir, 1);
        let mut manifest = decode_manifest(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest.metadata_crc32c = crc32c(&historical_file);
        fs::write(&manifest_path, encode_manifest(manifest)).unwrap();

        let (mut reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), base.id());
        assert_eq!(reopened.current_idempotency_epoch, IdempotencyEpoch::ZERO);
        assert_eq!(reopened.minimum_retry_epoch, IdempotencyEpoch::ZERO);
        assert!(reopened.migration_complements.is_empty());
        assert!(reopened.revision_effects.is_empty());

        let receipt = reopened.migrate_to_current_format(&base).unwrap();
        assert_eq!(receipt.generation, 2);
        drop(reopened);

        let current_metadata = fs::read(metadata_path(&dir, 2)).unwrap();
        assert_ne!(
            read_u16(&current_metadata[METADATA_HEADER_LEN..METADATA_HEADER_LEN + 2]),
            6
        );
        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.generation(), 2);
        assert_eq!(scan.durable_revision(), base.id());
        assert_eq!(reopened.current_idempotency_epoch, IdempotencyEpoch::ZERO);
        assert_eq!(reopened.minimum_retry_epoch, IdempotencyEpoch::ZERO);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn compaction_keeps_only_active_generation_artifacts() {
        let dir = test_dir("compact");
        let (base, registry, _) = setup_revision(50, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store.rotate_checkpoint(&base).unwrap();
        store.compact_obsolete_generations().unwrap();
        assert!(!checkpoint_path(&dir, 1).exists());
        assert!(!wal_path(&dir, 1).exists());
        assert!(!metadata_path(&dir, 1).exists());
        assert!(!manifest_path(&dir, 1).exists());
        assert!(checkpoint_path(&dir, 2).exists());
        assert!(wal_path(&dir, 2).exists());
        assert!(metadata_path(&dir, 2).exists());
        assert!(manifest_path(&dir, 2).exists());
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn published_manifest_requires_exact_durable_metadata_sidecar() {
        let dir = test_dir("metadata-authority");
        let (base, registry, _) = setup_revision(59, &[1]);
        let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        drop(store);
        let path = metadata_path(&dir, 1);
        let original = fs::read(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(matches!(
            DurableRevisionStore::open(&dir),
            Err(DurabilityError::Corruption {
                reason: "published durable metadata file is missing",
                ..
            })
        ));
        fs::write(&path, &original).unwrap();
        let mut corrupt = original;
        let last = corrupt.len() - 1;
        corrupt[last] ^= 0x40;
        fs::write(&path, corrupt).unwrap();
        assert!(matches!(
            DurableRevisionStore::open(&dir),
            Err(DurabilityError::Corruption { .. })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn published_manifest_with_missing_wal_is_corruption_not_empty_segment() {
        let dir = test_dir("missing-wal");
        let (base, registry, _) = setup_revision(60, &[1]);
        let store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        drop(store);
        fs::remove_file(wal_path(&dir, 1)).unwrap();
        assert!(matches!(
            DurableRevisionStore::open(&dir),
            Err(DurabilityError::Corruption {
                reason: "published WAL segment is missing",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn subprocess_kill_after_prepare_recovers_previous_committed_head() {
        let dir = test_dir("kill-after-prepare");
        let (base, registry, _) = setup_revision(80, &[1]);
        drop(DurableRevisionStore::create(&dir, &base, &registry).unwrap());

        run_crash_worker(
            "store::tests::crash_worker_wal_boundary",
            &dir,
            "after-prepare",
        );

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.durable_head(), RevisionId::new(80));
        assert!(scan.committed().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn subprocess_kill_after_commit_recovers_committed_head_without_ack() {
        let dir = test_dir("kill-after-commit");
        let (base, registry, _) = setup_revision(80, &[1]);
        drop(DurableRevisionStore::create(&dir, &base, &registry).unwrap());

        run_crash_worker(
            "store::tests::crash_worker_wal_boundary",
            &dir,
            "after-commit",
        );

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.durable_head(), RevisionId::new(81));
        assert_eq!(scan.committed().len(), 1);
        assert_eq!(
            scan.committed()[0].descriptor.target_revision,
            RevisionId::new(81)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn subprocess_kill_checkpoint_manifest_matrix_preserves_authority_boundary() {
        let points = [
            (StoreFaultPoint::AfterCheckpointSync, 1),
            (StoreFaultPoint::AfterWalSync, 1),
            (StoreFaultPoint::AfterMetadataSync, 1),
            (StoreFaultPoint::AfterPrerequisiteDirectorySync, 1),
            (StoreFaultPoint::AfterPendingManifestSync, 1),
            (StoreFaultPoint::AfterManifestRename, 2),
            (StoreFaultPoint::AfterManifestDirectorySync, 2),
        ];

        for (point, expected_generation) in points {
            let dir = test_dir(crash_point_name(point));
            let (base, registry, relation) = setup_revision(90, &[1]);
            let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
            let descriptor = committed_descriptor(&base, &registry, relation, 91, 2);
            let prepared = store.durably_prepare(&descriptor).unwrap();
            store.durably_commit(prepared).unwrap();
            drop(store);

            run_crash_worker(
                "store::tests::crash_worker_checkpoint_rotation",
                &dir,
                crash_point_name(point),
            );

            let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
            assert_eq!(reopened.generation(), expected_generation, "{point:?}");
            assert_eq!(reopened.durable_head(), RevisionId::new(91), "{point:?}");
            if expected_generation == 1 {
                assert_eq!(reopened.checkpoint_revision().id(), RevisionId::new(90));
                assert_eq!(scan.committed().len(), 1);
            } else {
                assert_eq!(reopened.checkpoint_revision().id(), RevisionId::new(91));
                assert!(scan.committed().is_empty());
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn subprocess_kill_during_compaction_never_removes_active_generation() {
        for point in [
            StoreFaultPoint::BeforeCompactionRemove,
            StoreFaultPoint::AfterCompactionRemove,
            StoreFaultPoint::AfterCompactionDirectorySync,
        ] {
            let dir = test_dir(crash_point_name(point));
            let (base, registry, _) = setup_revision(100, &[1]);
            let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
            store.rotate_checkpoint(&base).unwrap();
            drop(store);

            run_crash_worker(
                "store::tests::crash_worker_compaction",
                &dir,
                crash_point_name(point),
            );

            let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
            assert_eq!(reopened.generation(), 2, "{point:?}");
            assert_eq!(reopened.durable_head(), RevisionId::new(100), "{point:?}");
            assert!(scan.committed().is_empty());
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn crash_worker_wal_boundary() {
        if std::env::var_os(CRASH_WORKER_ENV).is_none() {
            return;
        }
        let dir = PathBuf::from(std::env::var_os(CRASH_DIR_ENV).unwrap());
        let point = std::env::var(CRASH_POINT_ENV).unwrap();
        let (base, registry, relation) = setup_revision(80, &[1]);
        let (mut store, _) = DurableRevisionStore::open(&dir).unwrap();
        let descriptor = committed_descriptor(&base, &registry, relation, 81, 2);
        let prepared = store.durably_prepare(&descriptor).unwrap();
        if point == "after-prepare" {
            signal_crash_ready(&dir);
        }
        store.durably_commit(prepared).unwrap();
        if point == "after-commit" {
            signal_crash_ready(&dir);
        }
        panic!("unknown WAL crash point {point}");
    }

    #[test]
    fn crash_worker_checkpoint_rotation() {
        if std::env::var_os(CRASH_WORKER_ENV).is_none() {
            return;
        }
        let dir = PathBuf::from(std::env::var_os(CRASH_DIR_ENV).unwrap());
        let point = parse_crash_point(&std::env::var(CRASH_POINT_ENV).unwrap());
        let (next, _, _) = setup_revision(91, &[1, 2]);
        let (mut store, _) = DurableRevisionStore::open(&dir).unwrap();
        let mut hook = BlockingKillFault {
            target: point,
            directory: dir.clone(),
        };
        let specs = store.materialization_specs().to_vec();
        store
            .rotate_checkpoint_with_fault_policy(&next, &specs, &[], &[], &mut hook)
            .unwrap();
        panic!("checkpoint crash point was not reached: {point:?}");
    }

    #[test]
    fn crash_worker_compaction() {
        if std::env::var_os(CRASH_WORKER_ENV).is_none() {
            return;
        }
        let dir = PathBuf::from(std::env::var_os(CRASH_DIR_ENV).unwrap());
        let point = parse_crash_point(&std::env::var(CRASH_POINT_ENV).unwrap());
        let (store, _) = DurableRevisionStore::open(&dir).unwrap();
        let mut hook = BlockingKillFault {
            target: point,
            directory: dir.clone(),
        };
        store
            .compact_obsolete_generations_with_hook(&mut hook)
            .unwrap();
        panic!("compaction crash point was not reached: {point:?}");
    }

    fn assert_committed_at(
        store: &DurableRevisionStore,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
        target_revision: u64,
    ) {
        assert_eq!(
            store.transaction_outcome_at(epoch, transaction_id),
            DurableTransactionOutcome::Committed {
                target_revision: RevisionId::new(target_revision),
            }
        );
    }

    #[test]
    fn idempotency_epoch_reuse_survives_crash_before_checkpoint_without_causal_alias() {
        let dir = test_dir("epoch-reuse-before-checkpoint");
        let (base, registry, relation) = setup_revision(300, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let transaction_id = ClientTransactionId::new(0xDEAD);

        let (rev_301, _, _) = setup_revision(301, &[1, 2]);
        let first = DurableRevisionDescriptor::relation_data(
            transaction_id,
            RevisionId::new(300),
            &rev_301,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(2)]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&first).unwrap();
        store.durably_commit(prepared).unwrap();
        let first_effect = *store
            .revision_effect_frontier(RevisionId::new(301))
            .unwrap()
            .iter()
            .next()
            .unwrap();

        store
            .advance_idempotency_epoch(IdempotencyEpoch::new(1))
            .unwrap();
        let (rev_302, _, _) = setup_revision(302, &[1, 2, 3]);
        let second = DurableRevisionDescriptor::relation_data(
            transaction_id,
            RevisionId::new(301),
            &rev_302,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(3)]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&second).unwrap();
        store.durably_commit(prepared).unwrap();
        let second_effect = *store
            .revision_effect_frontier(RevisionId::new(302))
            .unwrap()
            .iter()
            .next()
            .unwrap();
        assert_ne!(first_effect, second_effect);
        assert_committed_at(&store, IdempotencyEpoch::ZERO, transaction_id, 301);
        assert_committed_at(&store, IdempotencyEpoch::new(1), transaction_id, 302);
        drop(store); // no checkpoint after epoch advance/reuse

        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(302));
        assert_eq!(
            reopened.current_idempotency_epoch(),
            IdempotencyEpoch::new(1)
        );
        assert_committed_at(&reopened, IdempotencyEpoch::ZERO, transaction_id, 301);
        assert_committed_at(&reopened, IdempotencyEpoch::new(1), transaction_id, 302);
        assert_eq!(
            reopened
                .revision_effect_record(first_effect)
                .unwrap()
                .transaction_epoch,
            IdempotencyEpoch::ZERO
        );
        assert_eq!(
            reopened
                .revision_effect_record(second_effect)
                .unwrap()
                .transaction_epoch,
            IdempotencyEpoch::new(1)
        );
        assert!(
            reopened
                .revision_effect_ideal(RevisionId::new(302))
                .unwrap()
                .is_some()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn retry_gc_persists_watermark_and_keeps_causal_history_self_contained() {
        let dir = test_dir("retry-gc-causal-independence");
        let (base, registry, relation) = setup_revision(400, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let old_id = ClientTransactionId::new(0xA1);
        let new_id = ClientTransactionId::new(0xA2);

        let (rev_401, _, _) = setup_revision(401, &[1, 2]);
        let old = DurableRevisionDescriptor::relation_data(
            old_id,
            RevisionId::new(400),
            &rev_401,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(2)]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&old).unwrap();
        store.durably_commit(prepared).unwrap();
        let old_effect = *store
            .revision_effect_frontier(RevisionId::new(401))
            .unwrap()
            .iter()
            .next()
            .unwrap();

        store
            .advance_idempotency_epoch(IdempotencyEpoch::new(1))
            .unwrap();
        let (rev_402, _, _) = setup_revision(402, &[1, 2, 3]);
        let new = DurableRevisionDescriptor::relation_data(
            new_id,
            RevisionId::new(401),
            &rev_402,
            base.semantic_revision(),
            vec![DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(3)]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap();
        let prepared = store.durably_prepare(&new).unwrap();
        store.durably_commit(prepared).unwrap();

        assert_eq!(
            store
                .expire_retry_history_before(IdempotencyEpoch::new(1))
                .unwrap(),
            1
        );
        assert_eq!(
            store.transaction_outcome_at(IdempotencyEpoch::ZERO, old_id),
            DurableTransactionOutcome::RetryHistoryExpired
        );
        assert!(
            store
                .transaction_intent_at(IdempotencyEpoch::ZERO, old_id)
                .is_none()
        );
        let old_ideal = store
            .revision_effect_ideal(RevisionId::new(401))
            .unwrap()
            .unwrap();
        assert_eq!(old_ideal.events()[&old_effect].payload, old.intent);

        store.rotate_checkpoint(&rev_402).unwrap();
        drop(store);
        let (reopened, scan) = DurableRevisionStore::open(&dir).unwrap();
        assert!(scan.committed().is_empty());
        assert_eq!(
            reopened.current_idempotency_epoch(),
            IdempotencyEpoch::new(1)
        );
        assert_eq!(reopened.minimum_retry_epoch(), IdempotencyEpoch::new(1));
        assert_eq!(
            reopened.transaction_outcome_at(IdempotencyEpoch::ZERO, old_id),
            DurableTransactionOutcome::RetryHistoryExpired
        );
        assert_eq!(
            reopened.transaction_outcome_at(IdempotencyEpoch::new(1), new_id),
            DurableTransactionOutcome::Committed {
                target_revision: RevisionId::new(402)
            }
        );
        assert_eq!(
            reopened
                .revision_effect_ideal(RevisionId::new(401))
                .unwrap()
                .unwrap()
                .events()[&old_effect]
                .payload,
            old.intent
        );
        fs::remove_dir_all(dir).unwrap();
    }

    macro_rules! replicated_relation_effect {
        ($origin:expr, $sequence:expr, $branch:expr, $source:expr, $target:expr, $deps:expr, $semantic:expr, $position:expr) => {{
            let origin = ReplicaId::new($origin);
            let id = replicated_effect_id(origin, $sequence);
            ReplicatedEffectEnvelope {
                origin,
                origin_sequence: $sequence,
                branch: ReplicationBranchId::new($branch),
                effect: DurableRevisionEffectRecord {
                    id,
                    prerequisites: $deps,
                    transaction_epoch: IdempotencyEpoch::ZERO,
                    transaction_id: ClientTransactionId::new(id.0),
                    intent: DurableTransactionIntent::RelationDataExact {
                        source_revision: $source,
                        target_revision: $target,
                        semantic_revision: $semantic,
                        relation_mutations: Vec::new(),
                        semantic_modules: Vec::new(),
                    },
                    source_revision: $source,
                    target_revision: $target,
                },
                ordered_by: DurableSequencerOrder {
                    sequencer: ReplicaId::new(99),
                    epoch: 7,
                    position: $position,
                },
            }
        }};
    }

    fn membership_change(
        epoch: u64,
        members: &[u64],
        quorum_size: usize,
        acknowledged_by_previous: &[u64],
    ) -> ReplicationMembershipChange {
        ReplicationMembershipChange {
            next: ReplicationMembership {
                epoch,
                members: members.iter().copied().map(ReplicaId::new).collect(),
                quorum_size,
            },
            acknowledged_by_previous: acknowledged_by_previous
                .iter()
                .copied()
                .map(ReplicaId::new)
                .collect(),
        }
    }

    fn quorum_certificate(
        effect: RevisionEffectId,
        membership_epoch: u64,
        acknowledged_by: &[u64],
    ) -> ReplicationQuorumCertificate {
        ReplicationQuorumCertificate {
            effect,
            membership_epoch,
            acknowledged_by: acknowledged_by
                .iter()
                .copied()
                .map(ReplicaId::new)
                .collect(),
        }
    }

    fn replica_signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn replication_auth_policy(
        trust_epoch: u64,
        entries: &[(u64, &SigningKey)],
    ) -> ReplicationPeerAuthPolicy {
        ReplicationPeerAuthPolicy {
            cluster: crate::ReplicationClusterId([0xA5; 32]),
            trust_epoch,
            peer_keys: entries
                .iter()
                .map(|(replica, key)| {
                    (
                        ReplicaId::new(*replica),
                        key_id(key.verifying_key().as_bytes()),
                    )
                })
                .collect(),
        }
    }

    fn replication_trust(epoch: u64, keys: &[&SigningKey]) -> TrustRootSet {
        let verifying_keys: Vec<_> = keys
            .iter()
            .map(|key| *key.verifying_key().as_bytes())
            .collect();
        TrustRootSet::bootstrap(epoch, &verifying_keys).unwrap()
    }

    fn sign_replication_evidence(
        policy: &ReplicationPeerAuthPolicy,
        key: &SigningKey,
        evidence: ReplicationPeerEvidence,
    ) -> SignedReplicationPeerEvidence {
        let signer = key_id(key.verifying_key().as_bytes());
        let message = replication_peer_evidence_signing_message(
            policy.cluster,
            policy.trust_epoch,
            signer,
            &evidence,
        )
        .unwrap();
        SignedReplicationPeerEvidence {
            trust_epoch: policy.trust_epoch,
            signer,
            evidence,
            signature: key.sign(&message).to_bytes(),
        }
    }

    fn record_signed_replication_evidence(
        store: &mut DurableRevisionStore,
        trust: &TrustRootSet,
        policy: &ReplicationPeerAuthPolicy,
        key: &SigningKey,
        evidence: ReplicationPeerEvidence,
    ) -> ReplicationAuthenticationReceipt {
        store
            .durably_record_authenticated_replication_peer_evidence(
                trust,
                sign_replication_evidence(policy, key, evidence),
            )
            .unwrap()
    }

    fn certify_authenticated_replication_leader(
        store: &mut DurableRevisionStore,
        trust: &TrustRootSet,
        policy: &ReplicationPeerAuthPolicy,
        term: u64,
        leader: u64,
        voters: &[(u64, &SigningKey)],
    ) {
        for (voter, key) in voters {
            record_signed_replication_evidence(
                store,
                trust,
                policy,
                key,
                ReplicationPeerEvidence::LeaderVote(ReplicationLeaderVote {
                    voter: ReplicaId::new(*voter),
                    membership_epoch: 1,
                    term,
                    candidate: ReplicaId::new(leader),
                }),
            );
        }
        store
            .durably_certify_replication_leader(ReplicationLeaderCertificate {
                membership_epoch: 1,
                term,
                leader: ReplicaId::new(leader),
                acknowledged_by: voters
                    .iter()
                    .map(|(voter, _)| ReplicaId::new(*voter))
                    .collect(),
            })
            .unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn authenticated_replication_evidence_is_cluster_key_and_epoch_bound_across_restart() {
        let dir = test_dir("replication-auth-evidence");
        let (base, registry, _) = setup_revision(700, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        let key1 = replica_signing_key(11);
        let key2 = replica_signing_key(12);
        let key3 = replica_signing_key(13);
        let trust1 = replication_trust(1, &[&key1, &key2, &key3]);
        let policy1 = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
        store
            .durably_install_replication_peer_auth_policy(policy1.clone(), &trust1)
            .unwrap();

        let vote = ReplicationLeaderVote {
            voter: ReplicaId::new(1),
            membership_epoch: 1,
            term: 5,
            candidate: ReplicaId::new(1),
        };
        assert_eq!(
            store.durably_record_replication_leader_vote(vote),
            Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication peer evidence is not authenticated in current trust epoch",
            })
        );

        let receipt = record_signed_replication_evidence(
            &mut store,
            &trust1,
            &policy1,
            &key1,
            ReplicationPeerEvidence::LeaderVote(vote),
        );
        assert_eq!(
            store.replication_authentication_receipt(receipt.proof_digest),
            Some(receipt)
        );

        let mut forged = sign_replication_evidence(
            &policy1,
            &key2,
            ReplicationPeerEvidence::LeaderVote(ReplicationLeaderVote {
                voter: ReplicaId::new(2),
                membership_epoch: 1,
                term: 5,
                candidate: ReplicaId::new(1),
            }),
        );
        forged.signature[0] ^= 0x80;
        assert_eq!(
            store.durably_record_authenticated_replication_peer_evidence(&trust1, forged),
            Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication peer evidence signature verification failed",
            })
        );

        let key1v2 = replica_signing_key(21);
        let key2v2 = replica_signing_key(22);
        let key3v2 = replica_signing_key(23);
        let trust2 = replication_trust(2, &[&key1v2, &key2v2, &key3v2]);
        let policy2 = replication_auth_policy(2, &[(1, &key1v2), (2, &key2v2), (3, &key3v2)]);
        store
            .durably_install_replication_peer_auth_policy(policy2.clone(), &trust2)
            .unwrap();

        let stale = sign_replication_evidence(
            &policy1,
            &key2,
            ReplicationPeerEvidence::TermPromise(ReplicationTermPromise {
                voter: ReplicaId::new(2),
                membership_epoch: 1,
                term: 6,
            }),
        );
        assert_eq!(
            store.durably_record_authenticated_replication_peer_evidence(&trust2, stale),
            Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication peer evidence uses a stale trust epoch",
            })
        );

        record_signed_replication_evidence(
            &mut store,
            &trust2,
            &policy2,
            &key2v2,
            ReplicationPeerEvidence::TermPromise(ReplicationTermPromise {
                voter: ReplicaId::new(2),
                membership_epoch: 1,
                term: 6,
            }),
        );
        drop(store);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.replication_peer_auth_policy(), Some(&policy2));
        assert_eq!(
            reopened.replication_authentication_receipt(receipt.proof_digest),
            Some(receipt)
        );
        assert_eq!(
            reopened.replication_promised_term(1, ReplicaId::new(2)),
            Some(6)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn quorum_loss_fences_authority_until_authenticated_lock_recovery_and_survives_restart() {
        let dir = test_dir("replication-quorum-loss-recovery");
        let (base, registry, _) = setup_revision(710, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        let key1 = replica_signing_key(31);
        let key2 = replica_signing_key(32);
        let key3 = replica_signing_key(33);
        let trust = replication_trust(1, &[&key1, &key2, &key3]);
        let policy = replication_auth_policy(1, &[(1, &key1), (2, &key2), (3, &key3)]);
        store
            .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
            .unwrap();

        let effect = replicated_relation_effect!(
            211,
            1,
            211,
            base.id(),
            RevisionId::new(711),
            BTreeSet::new(),
            semantic_revision,
            80
        );
        let effect_id = effect.effect.id;
        store.durably_ingest_replicated_effect(effect).unwrap();

        certify_authenticated_replication_leader(
            &mut store,
            &trust,
            &policy,
            5,
            1,
            &[(1, &key1), (2, &key2)],
        );
        assert_eq!(
            store.replication_quorum_availability(),
            Some(ReplicationQuorumAvailability::Available {
                membership_epoch: 1,
                term: 5,
            })
        );
        for (voter, key) in [(1, &key1), (2, &key2)] {
            record_signed_replication_evidence(
                &mut store,
                &trust,
                &policy,
                key,
                ReplicationPeerEvidence::DecisionVote(ReplicationDecisionVote {
                    voter: ReplicaId::new(voter),
                    membership_epoch: 1,
                    term: 5,
                    leader: ReplicaId::new(1),
                    position: 80,
                    effect: effect_id,
                    carried_from_term: None,
                }),
            );
        }
        store
            .durably_lock_replication_decision(ReplicationDecisionLock {
                membership_epoch: 1,
                term: 5,
                leader: ReplicaId::new(1),
                position: 80,
                effect: effect_id,
                acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
                carried_from_term: None,
            })
            .unwrap();

        store
            .durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
                membership_epoch: 1,
                observed_term: 5,
            })
            .unwrap();
        assert_eq!(
            store.replication_quorum_availability(),
            Some(ReplicationQuorumAvailability::Lost(ReplicationQuorumLoss {
                membership_epoch: 1,
                observed_term: 5,
            }))
        );

        for (voter, key) in [(1, &key1), (2, &key2)] {
            record_signed_replication_evidence(
                &mut store,
                &trust,
                &policy,
                key,
                ReplicationPeerEvidence::LeaderVote(ReplicationLeaderVote {
                    voter: ReplicaId::new(voter),
                    membership_epoch: 1,
                    term: 6,
                    candidate: ReplicaId::new(2),
                }),
            );
        }
        assert_eq!(
            store.durably_certify_replication_leader(ReplicationLeaderCertificate {
                membership_epoch: 1,
                term: 6,
                leader: ReplicaId::new(2),
                acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            }),
            Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication consensus authority is fenced by quorum loss",
            })
        );

        let local_lock = ReplicationLockSummary {
            position: 80,
            term: 5,
            effect: effect_id,
        };
        for (voter, key) in [(1, &key1), (2, &key2)] {
            record_signed_replication_evidence(
                &mut store,
                &trust,
                &policy,
                key,
                ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
                    voter: ReplicaId::new(voter),
                    membership_epoch: 1,
                    recovery_term: 7,
                    leader: ReplicaId::new(2),
                    locks: vec![local_lock],
                }),
            );
        }
        store
            .durably_recover_replication_quorum(&ReplicationRecoveryCertificate {
                membership_epoch: 1,
                recovery_term: 7,
                leader: ReplicaId::new(2),
                acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
                reconciled_locks: vec![local_lock],
            })
            .unwrap();
        assert_eq!(
            store.replication_quorum_availability(),
            Some(ReplicationQuorumAvailability::Available {
                membership_epoch: 1,
                term: 7,
            })
        );
        assert_eq!(
            store.replication_leader_certificate(1, 7).unwrap().leader,
            ReplicaId::new(2)
        );
        drop(store);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(
            reopened.replication_quorum_availability(),
            Some(ReplicationQuorumAvailability::Available {
                membership_epoch: 1,
                term: 7,
            })
        );
        assert_eq!(
            reopened.replication_decision_lock(80).unwrap().effect,
            effect_id
        );
        assert_eq!(
            reopened
                .replication_leader_certificate(1, 7)
                .unwrap()
                .leader,
            ReplicaId::new(2)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn quorum_loss_observed_term_cannot_regress() {
        let dir = test_dir("replication-quorum-loss-term-monotone");
        let (base, registry, _) = setup_revision(712, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();
        store
            .durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
                membership_epoch: 1,
                observed_term: 5,
            })
            .unwrap();
        assert_eq!(
            store.durably_mark_replication_quorum_lost(ReplicationQuorumLoss {
                membership_epoch: 1,
                observed_term: 4,
            }),
            Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication quorum loss observed term regressed",
            })
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn joint_membership_requires_authenticated_successor_quorum_when_peer_auth_is_active() {
        let dir = test_dir("replication-auth-joint-membership");
        let (base, registry, _) = setup_revision(720, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        let key1 = replica_signing_key(41);
        let key2 = replica_signing_key(42);
        let key3 = replica_signing_key(43);
        let key4 = replica_signing_key(44);
        let key5 = replica_signing_key(45);
        let key6 = replica_signing_key(46);
        let trust = replication_trust(1, &[&key1, &key2, &key3, &key4, &key5, &key6]);
        let policy = replication_auth_policy(
            1,
            &[
                (1, &key1),
                (2, &key2),
                (3, &key3),
                (4, &key4),
                (5, &key5),
                (6, &key6),
            ],
        );
        store
            .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
            .unwrap();
        certify_authenticated_replication_leader(
            &mut store,
            &trust,
            &policy,
            10,
            1,
            &[(1, &key1), (2, &key2)],
        );

        let next = ReplicationMembership {
            epoch: 2,
            members: [ReplicaId::new(4), ReplicaId::new(5), ReplicaId::new(6)]
                .into_iter()
                .collect(),
            quorum_size: 2,
        };
        for (voter, key) in [(1, &key1), (2, &key2)] {
            record_signed_replication_evidence(
                &mut store,
                &trust,
                &policy,
                key,
                ReplicationPeerEvidence::MembershipVote(ReplicationMembershipVote {
                    voter: ReplicaId::new(voter),
                    previous_membership_epoch: 1,
                    term: 10,
                    next: next.clone(),
                }),
            );
        }
        let certificate = ReplicationJointMembershipCertificate {
            previous_membership_epoch: 1,
            term: 10,
            leader: ReplicaId::new(1),
            next: next.clone(),
            acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            acknowledged_by_next: [ReplicaId::new(4), ReplicaId::new(5)].into_iter().collect(),
        };
        assert_eq!(
            store.durably_certify_replication_joint_membership(certificate.clone()),
            Err(DurabilityError::Protocol {
                offset: 0,
                reason: "joint membership certificate lacks authenticated successor evidence",
            })
        );

        let next_digest = replication_membership_digest(&next).unwrap();
        for (voter, key) in [(4, &key4), (5, &key5)] {
            record_signed_replication_evidence(
                &mut store,
                &trust,
                &policy,
                key,
                ReplicationPeerEvidence::JointMembershipAck(ReplicationJointMembershipAck {
                    voter: ReplicaId::new(voter),
                    previous_membership_epoch: 1,
                    next_membership_epoch: 2,
                    term: 10,
                    leader: ReplicaId::new(1),
                    next_membership_digest: next_digest,
                }),
            );
        }
        store
            .durably_certify_replication_joint_membership(certificate)
            .unwrap();
        store
            .durably_install_replication_membership(ReplicationMembershipChange {
                next,
                acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)]
                    .into_iter()
                    .collect(),
            })
            .unwrap();
        assert_eq!(store.current_replication_membership().unwrap().epoch, 2);
        drop(store);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.current_replication_membership().unwrap().epoch, 2);
        assert_eq!(reopened.replication_peer_auth_policy(), Some(&policy));
        fs::remove_dir_all(dir).unwrap();
    }

    fn certify_replication_leader(
        store: &mut DurableRevisionStore,
        membership_epoch: u64,
        term: u64,
        leader: u64,
        voters: &[u64],
    ) {
        for voter in voters {
            store
                .durably_record_replication_leader_vote(ReplicationLeaderVote {
                    voter: ReplicaId::new(*voter),
                    membership_epoch,
                    term,
                    candidate: ReplicaId::new(leader),
                })
                .unwrap();
        }
        store
            .durably_certify_replication_leader(ReplicationLeaderCertificate {
                membership_epoch,
                term,
                leader: ReplicaId::new(leader),
                acknowledged_by: voters.iter().copied().map(ReplicaId::new).collect(),
            })
            .unwrap();
    }

    fn record_consensus_decision_votes(
        store: &mut DurableRevisionStore,
        template: ReplicationDecisionVote,
        voters: &[u64],
    ) {
        for voter in voters {
            store
                .durably_record_replication_decision_vote(ReplicationDecisionVote {
                    voter: ReplicaId::new(*voter),
                    ..template
                })
                .unwrap();
        }
    }

    fn record_effect_votes(
        store: &mut DurableRevisionStore,
        effect: RevisionEffectId,
        membership_epoch: u64,
        voters: &[u64],
    ) {
        for voter in voters {
            store
                .durably_record_replicated_effect_vote(ReplicationEffectVote {
                    voter: ReplicaId::new(*voter),
                    effect,
                    membership_epoch,
                })
                .unwrap();
        }
    }

    fn record_membership_votes(
        store: &mut DurableRevisionStore,
        previous_membership_epoch: u64,
        term: u64,
        next: &ReplicationMembership,
        voters: &[u64],
    ) {
        for voter in voters {
            store
                .durably_record_replication_membership_vote(ReplicationMembershipVote {
                    voter: ReplicaId::new(*voter),
                    previous_membership_epoch,
                    term,
                    next: next.clone(),
                })
                .unwrap();
        }
    }

    #[test]
    fn replication_effect_lifecycle_requires_quorum_before_publication_and_survives_restart() {
        let dir = test_dir("replication-quorum-publication-restart");
        let (base, registry, _) = setup_revision(545, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        let branch = ReplicationBranchId::new(50);
        let envelope = replicated_relation_effect!(
            50,
            1,
            50,
            base.id(),
            RevisionId::new(546),
            BTreeSet::new(),
            semantic_revision,
            1
        );
        let effect = envelope.effect.id;
        store.durably_ingest_replicated_effect(envelope).unwrap();
        assert_eq!(
            store.replication_effect_stage(effect),
            Some(ReplicationEffectStage::LocalDurable)
        );
        assert!(store.replication_published_branch_head(branch).is_none());
        assert!(matches!(
            store.durably_publish_replicated_effect(effect),
            Err(DurabilityError::Protocol {
                reason: "replicated effect cannot publish before quorum durability",
                ..
            })
        ));
        assert!(matches!(
            store.durably_certify_replicated_effect_quorum(quorum_certificate(effect, 1, &[1])),
            Err(DurabilityError::Protocol {
                reason: "replication quorum certificate lacks configured quorum",
                ..
            })
        ));
        record_effect_votes(&mut store, effect, 1, &[1, 2]);
        store
            .durably_certify_replicated_effect_quorum(quorum_certificate(effect, 1, &[1, 2]))
            .unwrap();
        assert_eq!(
            store.replication_effect_stage(effect),
            Some(ReplicationEffectStage::QuorumDurable)
        );
        store.durably_publish_replicated_effect(effect).unwrap();
        assert_eq!(
            store.replication_effect_stage(effect),
            Some(ReplicationEffectStage::Published)
        );
        assert_eq!(
            store
                .replication_published_branch_head(branch)
                .unwrap()
                .head_revision,
            RevisionId::new(546)
        );
        assert_eq!(store.durable_head(), base.id());

        drop(store);
        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.current_replication_membership().unwrap().epoch, 1);
        assert_eq!(
            reopened.replication_effect_stage(effect),
            Some(ReplicationEffectStage::Published)
        );
        assert_eq!(
            reopened
                .replication_published_branch_head(branch)
                .unwrap()
                .head_effect,
            effect
        );
        assert_eq!(reopened.durable_head(), base.id());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replication_membership_rotation_requires_previous_quorum_and_fences_stale_certificates() {
        let dir = test_dir("replication-membership-rotation");
        let (base, registry, _) = setup_revision(550, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();
        assert!(matches!(
            store.durably_install_replication_membership(membership_change(2, &[2, 3, 4], 2, &[1])),
            Err(DurabilityError::Protocol {
                reason: "replication membership change lacks previous-epoch quorum",
                ..
            })
        ));
        let epoch2 = membership_change(2, &[2, 3, 4], 2, &[1, 2]);
        record_membership_votes(&mut store, 1, 10, &epoch2.next, &[1, 2]);
        store
            .durably_install_replication_membership(epoch2)
            .unwrap();

        let envelope = replicated_relation_effect!(
            51,
            1,
            51,
            base.id(),
            RevisionId::new(551),
            BTreeSet::new(),
            semantic_revision,
            2
        );
        let effect = envelope.effect.id;
        store.durably_ingest_replicated_effect(envelope).unwrap();
        assert!(matches!(
            store.durably_certify_replicated_effect_quorum(quorum_certificate(effect, 1, &[1, 2])),
            Err(DurabilityError::Protocol {
                reason: "replication quorum certificate uses a stale membership epoch",
                ..
            })
        ));
        record_effect_votes(&mut store, effect, 2, &[2, 4]);
        store
            .durably_certify_replicated_effect_quorum(quorum_certificate(effect, 2, &[2, 4]))
            .unwrap();
        assert!(matches!(
            store.durably_install_replication_membership(membership_change(
                3,
                &[3, 4, 5],
                2,
                &[1, 2]
            )),
            Err(DurabilityError::Protocol {
                reason: "replication acknowledgement references a non-member replica",
                ..
            })
        ));
        let epoch3 = membership_change(3, &[3, 4, 5], 2, &[2, 3]);
        record_membership_votes(&mut store, 2, 11, &epoch3.next, &[2, 3]);
        store
            .durably_install_replication_membership(epoch3)
            .unwrap();
        drop(store);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.current_replication_membership().unwrap().epoch, 3);
        assert_eq!(
            reopened.replication_effect_stage(effect),
            Some(ReplicationEffectStage::QuorumDurable)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replication_vote_once_rejects_conflicting_effects_across_leaders_and_survives_restart() {
        let dir = test_dir("replication-effect-vote-once");
        let (base, registry, _) = setup_revision(552, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        let first = replicated_relation_effect!(
            61,
            1,
            61,
            base.id(),
            RevisionId::new(553),
            BTreeSet::new(),
            semantic_revision,
            40
        );
        let first_id = first.effect.id;
        store.durably_ingest_replicated_effect(first).unwrap();

        let mut second = replicated_relation_effect!(
            62,
            1,
            62,
            base.id(),
            RevisionId::new(554),
            BTreeSet::new(),
            semantic_revision,
            40
        );
        second.ordered_by.sequencer = ReplicaId::new(100);
        second.ordered_by.epoch = 8;
        let second_id = second.effect.id;
        store.durably_ingest_replicated_effect(second).unwrap();

        store
            .durably_record_replicated_effect_vote(ReplicationEffectVote {
                voter: ReplicaId::new(1),
                effect: first_id,
                membership_epoch: 1,
            })
            .unwrap();
        assert!(matches!(
            store.durably_record_replicated_effect_vote(ReplicationEffectVote {
                voter: ReplicaId::new(1),
                effect: second_id,
                membership_epoch: 1,
            }),
            Err(DurabilityError::Protocol {
                reason: "replication voter already voted for another effect in this decision slot",
                ..
            })
        ));
        drop(store);

        let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert!(matches!(
            reopened.durably_record_replicated_effect_vote(ReplicationEffectVote {
                voter: ReplicaId::new(1),
                effect: second_id,
                membership_epoch: 1,
            }),
            Err(DurabilityError::Protocol {
                reason: "replication voter already voted for another effect in this decision slot",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replication_term_and_leader_authority_fences_stale_leader_and_survives_restart() {
        let dir = test_dir("replication-term-leader-authority");
        let (base, registry, _) = setup_revision(2_601, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        certify_replication_leader(&mut store, 1, 5, 1, &[1, 2]);
        assert!(store.replication_leader_certificate(1, 5).is_some());

        assert!(matches!(
            store.durably_record_replication_leader_vote(ReplicationLeaderVote {
                voter: ReplicaId::new(1),
                membership_epoch: 1,
                term: 5,
                candidate: ReplicaId::new(2),
            }),
            Err(DurabilityError::Protocol {
                reason: "replication voter already voted for another leader in this term",
                ..
            })
        ));
        store
            .durably_record_replication_term_promise(ReplicationTermPromise {
                voter: ReplicaId::new(2),
                membership_epoch: 1,
                term: 6,
            })
            .unwrap();
        assert!(matches!(
            store.durably_record_replication_leader_vote(ReplicationLeaderVote {
                voter: ReplicaId::new(2),
                membership_epoch: 1,
                term: 5,
                candidate: ReplicaId::new(1),
            }),
            Err(DurabilityError::Protocol {
                reason: "replication leader vote uses a stale term",
                ..
            })
        ));
        assert!(matches!(
            store.durably_record_replication_term_promise(ReplicationTermPromise {
                voter: ReplicaId::new(2),
                membership_epoch: 1,
                term: 5,
            }),
            Err(DurabilityError::Protocol {
                reason: "replication term promise regressed",
                ..
            })
        ));
        assert!(matches!(
            store.durably_certify_replication_leader(ReplicationLeaderCertificate {
                membership_epoch: 1,
                term: 5,
                leader: ReplicaId::new(1),
                acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
            }),
            Ok(())
        ));
        drop(store);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(
            reopened.replication_promised_term(1, ReplicaId::new(2)),
            Some(6)
        );
        assert_eq!(
            reopened
                .replication_leader_certificate(1, 5)
                .unwrap()
                .leader,
            ReplicaId::new(1)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn replication_decision_lock_requires_leader_quorum_and_safe_carry_forward() {
        let dir = test_dir("replication-decision-lock-carry-forward");
        let (base, registry, _) = setup_revision(2_610, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        let first = replicated_relation_effect!(
            201,
            1,
            201,
            base.id(),
            RevisionId::new(2_611),
            BTreeSet::new(),
            semantic_revision,
            70
        );
        let first_id = first.effect.id;
        store.durably_ingest_replicated_effect(first).unwrap();
        let mut conflicting = replicated_relation_effect!(
            202,
            1,
            202,
            base.id(),
            RevisionId::new(2_612),
            BTreeSet::new(),
            semantic_revision,
            70
        );
        conflicting.ordered_by.sequencer = ReplicaId::new(100);
        conflicting.ordered_by.epoch = 8;
        let conflicting_id = conflicting.effect.id;
        store.durably_ingest_replicated_effect(conflicting).unwrap();

        certify_replication_leader(&mut store, 1, 5, 1, &[1, 2]);
        record_consensus_decision_votes(
            &mut store,
            ReplicationDecisionVote {
                voter: ReplicaId::new(1),
                membership_epoch: 1,
                term: 5,
                leader: ReplicaId::new(1),
                position: 70,
                effect: first_id,
                carried_from_term: None,
            },
            &[1, 2],
        );
        record_effect_votes(&mut store, first_id, 1, &[1, 2]);
        assert!(matches!(
            store.durably_certify_replicated_effect_quorum(quorum_certificate(
                first_id,
                1,
                &[1, 2]
            )),
            Err(DurabilityError::Protocol {
                reason: "replication quorum lacks consensus decision lock",
                ..
            })
        ));
        store
            .durably_lock_replication_decision(ReplicationDecisionLock {
                membership_epoch: 1,
                term: 5,
                leader: ReplicaId::new(1),
                position: 70,
                effect: first_id,
                acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
                carried_from_term: None,
            })
            .unwrap();
        store
            .durably_certify_replicated_effect_quorum(quorum_certificate(first_id, 1, &[1, 2]))
            .unwrap();

        certify_replication_leader(&mut store, 1, 6, 2, &[1, 2]);

        assert!(matches!(
            store.durably_record_replication_decision_vote(ReplicationDecisionVote {
                voter: ReplicaId::new(1),
                membership_epoch: 1,
                term: 6,
                leader: ReplicaId::new(2),
                position: 70,
                effect: conflicting_id,
                carried_from_term: Some(5),
            }),
            Err(DurabilityError::Protocol {
                reason: "replication decision conflicts with a durable locked value",
                ..
            })
        ));
        assert!(matches!(
            store.durably_record_replication_decision_vote(ReplicationDecisionVote {
                voter: ReplicaId::new(1),
                membership_epoch: 1,
                term: 6,
                leader: ReplicaId::new(2),
                position: 70,
                effect: first_id,
                carried_from_term: None,
            }),
            Err(DurabilityError::Protocol {
                reason: "replication later-term decision did not carry forward the durable lock",
                ..
            })
        ));
        record_consensus_decision_votes(
            &mut store,
            ReplicationDecisionVote {
                voter: ReplicaId::new(1),
                membership_epoch: 1,
                term: 6,
                leader: ReplicaId::new(2),
                position: 70,
                effect: first_id,
                carried_from_term: Some(5),
            },
            &[1, 2],
        );
        store
            .durably_lock_replication_decision(ReplicationDecisionLock {
                membership_epoch: 1,
                term: 6,
                leader: ReplicaId::new(2),
                position: 70,
                effect: first_id,
                acknowledged_by: [ReplicaId::new(1), ReplicaId::new(2)].into_iter().collect(),
                carried_from_term: Some(5),
            })
            .unwrap();
        assert_eq!(store.replication_decision_lock(70).unwrap().term, 6);
        drop(store);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        let lock = reopened.replication_decision_lock(70).unwrap();
        assert_eq!(lock.effect, first_id);
        assert_eq!(lock.term, 6);
        assert_eq!(lock.carried_from_term, Some(5));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replication_joint_membership_requires_old_and_new_quorums_in_certified_term() {
        let dir = test_dir("replication-joint-membership");
        let (base, registry, _) = setup_revision(2_620, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();
        certify_replication_leader(&mut store, 1, 10, 1, &[1, 2]);

        let next = membership_change(2, &[4, 5, 6], 2, &[1, 2]);
        record_membership_votes(&mut store, 1, 10, &next.next, &[1, 2]);
        assert!(matches!(
            store.durably_install_replication_membership(next.clone()),
            Err(DurabilityError::Protocol {
                reason: "replication membership change lacks joint quorum certificate",
                ..
            })
        ));
        assert!(matches!(
            store.durably_certify_replication_joint_membership(
                ReplicationJointMembershipCertificate {
                    previous_membership_epoch: 1,
                    term: 10,
                    leader: ReplicaId::new(1),
                    next: next.next.clone(),
                    acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)]
                        .into_iter()
                        .collect(),
                    acknowledged_by_next: [ReplicaId::new(4)].into_iter().collect(),
                }
            ),
            Err(DurabilityError::Protocol {
                reason: "joint membership certificate lacks successor-membership quorum",
                ..
            })
        ));
        store
            .durably_certify_replication_joint_membership(ReplicationJointMembershipCertificate {
                previous_membership_epoch: 1,
                term: 10,
                leader: ReplicaId::new(1),
                next: next.next.clone(),
                acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)]
                    .into_iter()
                    .collect(),
                acknowledged_by_next: [ReplicaId::new(4), ReplicaId::new(5)].into_iter().collect(),
            })
            .unwrap();
        store.durably_install_replication_membership(next).unwrap();
        assert_eq!(store.current_replication_membership().unwrap().epoch, 2);
        drop(store);

        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.current_replication_membership().unwrap().epoch, 2);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replication_joint_membership_is_fenced_by_later_term_promise() {
        let dir = test_dir("replication-joint-membership-stale-term");
        let (base, registry, _) = setup_revision(2_630, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();
        certify_replication_leader(&mut store, 1, 10, 1, &[1, 2]);
        let next = membership_change(2, &[4, 5, 6], 2, &[1, 2]);
        record_membership_votes(&mut store, 1, 10, &next.next, &[1, 2]);
        store
            .durably_certify_replication_joint_membership(ReplicationJointMembershipCertificate {
                previous_membership_epoch: 1,
                term: 10,
                leader: ReplicaId::new(1),
                next: next.next.clone(),
                acknowledged_by_previous: [ReplicaId::new(1), ReplicaId::new(2)]
                    .into_iter()
                    .collect(),
                acknowledged_by_next: [ReplicaId::new(4), ReplicaId::new(5)].into_iter().collect(),
            })
            .unwrap();
        store
            .durably_record_replication_term_promise(ReplicationTermPromise {
                voter: ReplicaId::new(3),
                membership_epoch: 1,
                term: 11,
            })
            .unwrap();
        assert!(matches!(
            store.durably_install_replication_membership(next),
            Err(DurabilityError::Protocol {
                reason: "replication membership change uses a stale consensus term",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn authenticated_transport_routes_peer_evidence_but_keeps_heartbeat_advisory() {
        let dir = test_dir("replication-authenticated-transport-route");
        let (revision, registry, _) = setup_revision(1, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[2], 1, &[]))
            .unwrap();

        let signing = SigningKey::from_bytes(&[42_u8; 32]);
        let verifying = signing.verifying_key().to_bytes();
        let signer_id = key_id(&verifying);
        let trust = TrustRootSet::bootstrap(1, &[verifying]).unwrap();
        let policy = ReplicationPeerAuthPolicy {
            cluster: crate::ReplicationClusterId([3; 32]),
            trust_epoch: 1,
            peer_keys: BTreeMap::from([(ReplicaId::new(2), signer_id)]),
        };
        store
            .durably_install_replication_peer_auth_policy(policy.clone(), &trust)
            .unwrap();

        let promise = ReplicationPeerEvidence::TermPromise(ReplicationTermPromise {
            voter: ReplicaId::new(2),
            membership_epoch: 1,
            term: 4,
        });
        let inner = sign_replication_evidence(&policy, &signing, promise);
        let frame = ReplicationTransportFrame {
            cluster: policy.cluster,
            trust_epoch: 1,
            sender: ReplicaId::new(2),
            sequence: 1,
            payload: ReplicationTransportPayload::PeerEvidence(inner),
        };
        let transport_signed = SignedReplicationTransportFrame {
            signature: signing
                .sign(&replication_transport_signing_message(&frame).unwrap())
                .to_bytes(),
            frame,
        };
        let mut ingress = ReplicationTransportIngress::new();
        let receipt = store
            .durably_accept_replication_transport_frame(&mut ingress, &trust, transport_signed)
            .unwrap()
            .expect("authority evidence receipt");
        assert_eq!(receipt.voter, ReplicaId::new(2));
        assert_eq!(
            store.replication_promised_term(1, ReplicaId::new(2)),
            Some(4)
        );

        let before = store.replication_quorum_availability();
        let heartbeat = ReplicationTransportFrame {
            cluster: policy.cluster,
            trust_epoch: 1,
            sender: ReplicaId::new(2),
            sequence: 2,
            payload: ReplicationTransportPayload::Heartbeat(ReplicationHeartbeat {
                membership_epoch: 1,
                term: 4,
                logical_tick: 10,
            }),
        };
        let signed_heartbeat = SignedReplicationTransportFrame {
            signature: signing
                .sign(&replication_transport_signing_message(&heartbeat).unwrap())
                .to_bytes(),
            frame: heartbeat,
        };
        assert!(
            store
                .durably_accept_replication_transport_frame(&mut ingress, &trust, signed_heartbeat,)
                .unwrap()
                .is_none()
        );
        assert_eq!(store.replication_quorum_availability(), before);
    }

    #[test]
    fn anti_entropy_summary_chunk_and_failure_detector_are_non_authoritative_until_fenced() {
        let dir = test_dir("replication-anti-entropy-failure-detector");
        let (revision, registry, _) = setup_revision(1, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &revision, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();

        let summary = store
            .replication_anti_entropy_summary()
            .unwrap()
            .expect("summary");
        assert_eq!(summary.membership_epoch, 1);
        assert_eq!(summary.lock_count, 0);
        let chunk = store
            .replication_anti_entropy_chunk(ReplicationAntiEntropyRequest {
                membership_epoch: 1,
                from_position: 0,
                max_locks: 8,
            })
            .unwrap();
        assert!(chunk.complete);
        assert!(chunk.locks.is_empty());
        let before_advisory = store.replication_quorum_availability();
        assert_eq!(
            before_advisory,
            Some(ReplicationQuorumAvailability::Available {
                membership_epoch: 1,
                term: 0,
            })
        );
        assert_eq!(store.replication_quorum_availability(), before_advisory);

        let mut detector = ReplicationFailureDetector::new(ReplicaId::new(1), 2).unwrap();
        detector.observe_authenticated(ReplicaId::new(2));
        assert!(
            !store
                .durably_fence_replication_if_quorum_unreachable(&detector, 1)
                .unwrap()
        );
        detector.advance_to(3).unwrap();
        assert!(
            store
                .durably_fence_replication_if_quorum_unreachable(&detector, 1)
                .unwrap()
        );
        assert!(matches!(
            store.replication_quorum_availability(),
            Some(ReplicationQuorumAvailability::Lost(_))
        ));
    }

    #[test]
    fn replication_membership_vote_once_blocks_conflicting_successors_across_restart() {
        let dir = test_dir("replication-membership-vote-once");
        let (base, registry, _) = setup_revision(558, &[1]);
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();
        let first = membership_change(2, &[2, 3, 4], 2, &[1, 2]).next;
        let conflicting = membership_change(2, &[1, 3, 4], 2, &[1, 3]).next;
        store
            .durably_record_replication_membership_vote(ReplicationMembershipVote {
                voter: ReplicaId::new(1),
                previous_membership_epoch: 1,
                term: 10,
                next: first,
            })
            .unwrap();
        assert!(matches!(
            store.durably_record_replication_membership_vote(ReplicationMembershipVote {
                voter: ReplicaId::new(1),
                previous_membership_epoch: 1,
                term: 11,
                next: conflicting.clone(),
            }),
            Err(DurabilityError::Protocol {
                reason: "replication voter already voted for another successor membership",
                ..
            })
        ));
        drop(store);

        let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert!(matches!(
            reopened.durably_record_replication_membership_vote(ReplicationMembershipVote {
                voter: ReplicaId::new(1),
                previous_membership_epoch: 1,
                term: 12,
                next: conflicting,
            }),
            Err(DurabilityError::Protocol {
                reason: "replication voter already voted for another successor membership",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replicated_branch_publication_is_contiguous_even_when_later_effect_is_quorum_durable() {
        let dir = test_dir("replication-publication-contiguous");
        let (base, registry, _) = setup_revision(555, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();
        let first = replicated_relation_effect!(
            52,
            1,
            52,
            base.id(),
            RevisionId::new(556),
            BTreeSet::new(),
            semantic_revision,
            3
        );
        let first_id = first.effect.id;
        store.durably_ingest_replicated_effect(first).unwrap();
        let second = replicated_relation_effect!(
            52,
            2,
            52,
            RevisionId::new(556),
            RevisionId::new(557),
            BTreeSet::from([first_id]),
            semantic_revision,
            4
        );
        let second_id = second.effect.id;
        store.durably_ingest_replicated_effect(second).unwrap();
        record_effect_votes(&mut store, first_id, 1, &[1, 2]);
        store
            .durably_certify_replicated_effect_quorum(quorum_certificate(first_id, 1, &[1, 2]))
            .unwrap();
        record_effect_votes(&mut store, second_id, 1, &[2, 3]);
        store
            .durably_certify_replicated_effect_quorum(quorum_certificate(second_id, 1, &[2, 3]))
            .unwrap();
        assert!(matches!(
            store.durably_publish_replicated_effect(second_id),
            Err(DurabilityError::Protocol {
                reason: "replicated branch publication skipped an earlier local-durable effect",
                ..
            })
        ));
        store.durably_publish_replicated_effect(first_id).unwrap();
        store.durably_publish_replicated_effect(second_id).unwrap();
        assert_eq!(
            store
                .replication_published_branch_head(ReplicationBranchId::new(52))
                .unwrap()
                .head_effect,
            second_id
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replicated_publication_cannot_outrun_quorum_durability_of_remote_causal_prefix() {
        let dir = test_dir("replication-publication-causal-quorum");
        let (base, registry, _) = setup_revision(560, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        store
            .durably_install_replication_membership(membership_change(1, &[1, 2, 3], 2, &[]))
            .unwrap();
        let prerequisite = replicated_relation_effect!(
            53,
            1,
            53,
            base.id(),
            RevisionId::new(561),
            BTreeSet::new(),
            semantic_revision,
            5
        );
        let prerequisite_id = prerequisite.effect.id;
        store
            .durably_ingest_replicated_effect(prerequisite)
            .unwrap();
        let dependent = replicated_relation_effect!(
            54,
            1,
            54,
            RevisionId::new(561),
            RevisionId::new(562),
            BTreeSet::from([prerequisite_id]),
            semantic_revision,
            6
        );
        let dependent_id = dependent.effect.id;
        store.durably_ingest_replicated_effect(dependent).unwrap();
        record_effect_votes(&mut store, dependent_id, 1, &[1, 2]);
        store
            .durably_certify_replicated_effect_quorum(quorum_certificate(dependent_id, 1, &[1, 2]))
            .unwrap();
        assert!(matches!(
            store.durably_publish_replicated_effect(dependent_id),
            Err(DurabilityError::Protocol {
                reason: "replicated effect cannot publish before replicated prerequisites are quorum durable",
                ..
            })
        ));
        record_effect_votes(&mut store, prerequisite_id, 1, &[2, 3]);
        store
            .durably_certify_replicated_effect_quorum(quorum_certificate(
                prerequisite_id,
                1,
                &[2, 3],
            ))
            .unwrap();
        store
            .durably_publish_replicated_effect(dependent_id)
            .unwrap();
        assert_eq!(
            store.replication_effect_stage(dependent_id),
            Some(ReplicationEffectStage::Published)
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn independent_replication_branches_survive_restart_and_retirement() {
        let dir = test_dir("replication-branches-restart");
        let (base, registry, _) = setup_revision(500, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();

        let branch_a = ReplicationBranchId::new(1);
        let branch_b = ReplicationBranchId::new(2);
        let a1 = replicated_relation_effect!(
            10,
            1,
            1,
            base.id(),
            RevisionId::new(501),
            BTreeSet::new(),
            semantic_revision,
            1
        );
        let a1_id = a1.effect.id;
        assert_eq!(
            store.durably_ingest_replicated_effect(a1).unwrap(),
            ReplicationIngestOutcome::Inserted
        );
        let a2 = replicated_relation_effect!(
            10,
            2,
            1,
            RevisionId::new(501),
            RevisionId::new(502),
            BTreeSet::from([a1_id]),
            semantic_revision,
            2
        );
        let a2_id = a2.effect.id;
        store.durably_ingest_replicated_effect(a2).unwrap();
        let b1 = replicated_relation_effect!(
            11,
            1,
            2,
            base.id(),
            RevisionId::new(510),
            BTreeSet::new(),
            semantic_revision,
            3
        );
        let b1_id = b1.effect.id;
        store.durably_ingest_replicated_effect(b1).unwrap();

        assert_eq!(store.durable_head(), base.id());
        let head_a = store.replication_branch_head(branch_a).unwrap();
        assert_eq!(head_a.head_revision, RevisionId::new(502));
        assert_eq!(
            store.replication_branch_head(branch_b).unwrap().head_effect,
            b1_id
        );
        let ideal_a = store
            .replicated_branch_effect_ideal(branch_a)
            .unwrap()
            .unwrap();
        assert_eq!(ideal_a.events().len(), 2);

        drop(store);
        let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(reopened.durable_head(), base.id());
        assert_eq!(
            reopened
                .replication_branch_head(branch_a)
                .unwrap()
                .head_effect,
            a2_id
        );
        let ideal_b = reopened
            .replicated_branch_effect_ideal(branch_b)
            .unwrap()
            .unwrap();
        assert_eq!(ideal_b.events().len(), 1);
        reopened
            .durably_retire_replication_branch(branch_a, a2_id)
            .unwrap();
        drop(reopened);

        let (mut reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert!(reopened.replication_branch_head(branch_a).unwrap().retired);
        let after_retirement = replicated_relation_effect!(
            10,
            3,
            1,
            RevisionId::new(502),
            RevisionId::new(503),
            BTreeSet::from([a2_id]),
            semantic_revision,
            4
        );
        assert!(matches!(
            reopened.durably_ingest_replicated_effect(after_retirement),
            Err(DurabilityError::Protocol {
                reason: "retired replication branch cannot be advanced",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replicated_admission_rejects_stale_causal_cut_and_conflicting_order_slot() {
        let dir = test_dir("replication-admission-hostile");
        let (base, registry, _) = setup_revision(520, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let bad = replicated_relation_effect!(
            20,
            1,
            20,
            base.id(),
            RevisionId::new(521),
            BTreeSet::from([RevisionEffectId(77)]),
            semantic_revision,
            1
        );
        let before = fs::metadata(store.replication_journal_path())
            .unwrap()
            .len();
        assert!(matches!(
            store.durably_ingest_replicated_effect(bad),
            Err(DurabilityError::Protocol {
                reason: "replicated effect prerequisites do not equal the authoritative causal cut",
                ..
            })
        ));
        assert_eq!(
            fs::metadata(store.replication_journal_path())
                .unwrap()
                .len(),
            before
        );

        let first = replicated_relation_effect!(
            20,
            1,
            20,
            base.id(),
            RevisionId::new(521),
            BTreeSet::new(),
            semantic_revision,
            1
        );
        store.durably_ingest_replicated_effect(first).unwrap();
        let second = replicated_relation_effect!(
            21,
            1,
            21,
            base.id(),
            RevisionId::new(522),
            BTreeSet::new(),
            semantic_revision,
            1
        );
        assert!(matches!(
            store.durably_ingest_replicated_effect(second),
            Err(DurabilityError::Protocol {
                reason: "sequencer order slot is already bound to another effect",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replicated_admission_fences_stale_sequencer_epochs() {
        let dir = test_dir("replication-epoch-fence");
        let (base, registry, _) = setup_revision(525, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let newer_epoch = ReplicatedEffectEnvelope {
            ordered_by: DurableSequencerOrder {
                sequencer: ReplicaId::new(99),
                epoch: 8,
                position: 1,
            },
            ..replicated_relation_effect!(
                22,
                1,
                22,
                base.id(),
                RevisionId::new(526),
                BTreeSet::new(),
                semantic_revision,
                2
            )
        };
        store.durably_ingest_replicated_effect(newer_epoch).unwrap();
        let stale_epoch = ReplicatedEffectEnvelope {
            ordered_by: DurableSequencerOrder {
                sequencer: ReplicaId::new(99),
                epoch: 7,
                position: 9,
            },
            ..replicated_relation_effect!(
                23,
                1,
                23,
                base.id(),
                RevisionId::new(527),
                BTreeSet::new(),
                semantic_revision,
                9
            )
        };
        assert!(matches!(
            store.durably_ingest_replicated_effect(stale_epoch),
            Err(DurabilityError::Protocol {
                reason: "replicated effect uses a stale sequencer epoch",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replication_journal_truncates_only_an_incomplete_tail_on_reopen() {
        let dir = test_dir("replication-truncated-tail");
        let (base, registry, _) = setup_revision(530, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let branch = ReplicationBranchId::new(30);
        let effect = replicated_relation_effect!(
            30,
            1,
            30,
            base.id(),
            RevisionId::new(531),
            BTreeSet::new(),
            semantic_revision,
            1
        );
        let effect_id = effect.effect.id;
        store.durably_ingest_replicated_effect(effect).unwrap();
        let journal = store.replication_journal_path().to_path_buf();
        let good_len = fs::metadata(&journal).unwrap().len();
        drop(store);

        OpenOptions::new()
            .append(true)
            .open(&journal)
            .unwrap()
            .write_all(b"CFRP\x01")
            .unwrap();
        let (reopened, _) = DurableRevisionStore::open(&dir).unwrap();
        assert_eq!(fs::metadata(&journal).unwrap().len(), good_len);
        assert_eq!(
            reopened
                .replication_branch_head(branch)
                .unwrap()
                .head_effect,
            effect_id
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn replication_journal_checksum_corruption_is_not_silently_truncated() {
        let dir = test_dir("replication-checksum-corruption");
        let (base, registry, _) = setup_revision(540, &[1]);
        let semantic_revision = base.semantic_revision();
        let mut store = DurableRevisionStore::create(&dir, &base, &registry).unwrap();
        let effect = replicated_relation_effect!(
            40,
            1,
            40,
            base.id(),
            RevisionId::new(541),
            BTreeSet::new(),
            semantic_revision,
            1
        );
        let same = effect.clone();
        assert_eq!(
            store.durably_ingest_replicated_effect(effect).unwrap(),
            ReplicationIngestOutcome::Inserted
        );
        let size = fs::metadata(store.replication_journal_path())
            .unwrap()
            .len();
        assert_eq!(
            store.durably_ingest_replicated_effect(same).unwrap(),
            ReplicationIngestOutcome::AlreadyPresent
        );
        assert_eq!(
            fs::metadata(store.replication_journal_path())
                .unwrap()
                .len(),
            size
        );
        let journal = store.replication_journal_path().to_path_buf();
        drop(store);

        let mut bytes = fs::read(&journal).unwrap();
        *bytes.last_mut().unwrap() ^= 0x55;
        fs::write(&journal, bytes).unwrap();
        assert!(matches!(
            DurableRevisionStore::open(&dir),
            Err(DurabilityError::Corruption {
                reason: "replication journal payload checksum mismatch",
                ..
            })
        ));
        fs::remove_dir_all(dir).unwrap();
    }
}
