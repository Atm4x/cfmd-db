use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use kernel_auth::AuthorityDigest;
use kernel_change::RevisionEffectId;
use kernel_lens::{
    ArchiveProofId, ComplementCapsule, ComplementRetention, LensSpecId, SemanticManifestId,
};
use kernel_model::Value;
use kernel_semantics::BuiltinSemanticModuleSpec;
use kernel_types::{
    ClientTransactionId, EntityId, MaterializationId, RevisionId, SemanticId, SemanticRevision,
};

mod checkpoint;
mod freshness_tcp;
mod metadata;
mod platform_assurance;
mod replication;
mod replication_transport;
mod store;

pub use freshness_tcp::{TcpExternalFreshnessAuthority, TcpExternalFreshnessAuthorityServer};

pub use platform_assurance::{
    DestructiveDurabilityCampaignEvidence, DestructiveDurabilityCut, DurabilityPlatformEvidence,
    SignedDestructiveDurabilityCampaignEvidence, SupportedDurabilityProfile,
    VerifiedDestructiveDurabilityCampaignEvidence, certify_supported_durability_platform,
    current_mount_durability_evidence, durability_platform_fingerprint,
    prepare_destructive_durability_case, probe_durability_primitives,
    verify_destructive_durability_case, verify_signed_destructive_durability_campaign,
    verify_supported_durability_platform,
};

pub use replication::{
    DurableSequencerOrder, ReplicaId, ReplicatedEffectEnvelope, ReplicationAuthenticationReceipt,
    ReplicationBranchHead, ReplicationBranchId, ReplicationClusterId, ReplicationDecisionLock,
    ReplicationDecisionVote, ReplicationEffectStage, ReplicationEffectVote,
    ReplicationIngestOutcome, ReplicationJointMembershipAck, ReplicationJointMembershipCertificate,
    ReplicationLeaderCertificate, ReplicationLeaderVote, ReplicationLockSummary,
    ReplicationMembership, ReplicationMembershipChange, ReplicationMembershipVote,
    ReplicationPeerAuthPolicy, ReplicationPeerEvidence, ReplicationQuorumAvailability,
    ReplicationQuorumCertificate, ReplicationQuorumLoss, ReplicationRecoveryAck,
    ReplicationRecoveryCertificate, ReplicationTermPromise, SignedReplicationPeerEvidence,
    replicated_effect_id, replicated_origin, replication_membership_digest,
    replication_peer_evidence_signing_message,
};

pub use replication_transport::{
    MAX_ANTI_ENTROPY_LOCKS, ReplicationAntiEntropyChunk, ReplicationAntiEntropyRelation,
    ReplicationAntiEntropyRequest, ReplicationAntiEntropySummary, ReplicationFailureDetector,
    ReplicationHeartbeat, ReplicationTransportFrame, ReplicationTransportIngress,
    ReplicationTransportPayload, SignedReplicationTransportFrame, compare_replication_anti_entropy,
    decode_signed_replication_transport_frame, encode_signed_replication_transport_frame,
    replication_anti_entropy_summary, replication_lock_frontier_digest,
    replication_transport_signing_message, validate_replication_anti_entropy_chunk,
};

pub use store::{
    DurableBatchEnqueueOutcome, DurableCommitBatchPolicy, DurableCommitBatcher,
    DurableGenerationReceipt, DurableRevisionStore, ExternalFreshnessAuthority,
    ExternalFreshnessConfig, PreparedCutCapsule, StreamingCheckpointProgress,
};

pub const MAGIC: [u8; 4] = *b"CFMW";
pub const FORMAT_VERSION: u16 = 1;
pub const MUTATION_CODEC_VERSION: u16 = 9;
pub const PHYSICAL_ARTIFACT_RECIPE_VERSION: u16 = 3;
pub const HEADER_LEN: usize = 36;
pub const MAX_PAYLOAD_LEN: usize = 64 * 1024 * 1024;
const MAX_COLLECTION_LEN: usize = 1_000_000;
const MAX_VALUE_DEPTH: usize = 128;

/// Durable namespace for exact client retry identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdempotencyEpoch(u64);

impl IdempotencyEpoch {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableExternalFreshnessBinding {
    pub store_id: [u8; 32],
    pub previous_generation_digest: Option<AuthorityDigest>,
    pub trust_root_epoch: u64,
    pub deployment_policy_epoch: u64,
}

/// Exact retry key. Raw client transaction ids may be reused only in a later
/// explicit idempotency epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurableTransactionKey {
    pub epoch: IdempotencyEpoch,
    pub transaction_id: ClientTransactionId,
}

impl DurableTransactionKey {
    #[must_use]
    pub const fn new(epoch: IdempotencyEpoch, transaction_id: ClientTransactionId) -> Self {
        Self {
            epoch,
            transaction_id,
        }
    }
}

/// Durable causal identity for one exact committed transition.
///
/// Event identity is deliberately independent from the client retry id: raw
/// transaction ids may be reused in a later idempotency epoch.  The canonical
/// intent is retained by the causal record itself so retry-payload GC cannot
/// tear a Γ-REIC ideal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRevisionEffectRecord {
    pub id: RevisionEffectId,
    pub prerequisites: BTreeSet<RevisionEffectId>,
    pub transaction_epoch: IdempotencyEpoch,
    pub transaction_id: ClientTransactionId,
    pub intent: DurableTransactionIntent,
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableEffectKind {
    RelationData,
    RelationRewrite,
    RelationResolution,
    FullRevision,
    SchemaMigration,
    LegacyTargetOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableEffectCoordinationClass {
    /// No durable confluence/coherence certificate is attached to this effect.
    /// It must therefore be ordered, explicitly resolved, or coordinated; it
    /// is never eligible for automatic REIC union merely because its payload
    /// happens to be a Rewrite.
    OpaqueNonConfluent,
}

impl DurableRevisionEffectRecord {
    #[must_use]
    pub const fn kind(&self) -> DurableEffectKind {
        self.intent.effect_kind()
    }

    #[must_use]
    pub const fn coordination_class(&self) -> DurableEffectCoordinationClass {
        DurableEffectCoordinationClass::OpaqueNonConfluent
    }

    pub fn validate_identity(&self) -> Result<(), &'static str> {
        if self.source_revision == self.target_revision {
            return Err("revision effect does not advance revision identity");
        }
        if self.intent.target_revision() != self.target_revision {
            return Err("revision effect canonical intent targets another revision");
        }
        if self
            .intent
            .source_revision()
            .is_some_and(|source| source != self.source_revision)
        {
            return Err("revision effect canonical intent starts from another revision");
        }
        if self.prerequisites.contains(&self.id) {
            return Err("revision effect depends on itself");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRelationMutation {
    pub relation: SemanticId,
    pub inserted: Vec<Vec<Value>>,
    pub removed: Vec<Vec<Value>>,
}

/// Durable identity of one semantic Rewrite family attached to one relation
/// mutation. Explicit user inputs/effects are not duplicated here: the exact
/// typed relation delta is persisted once in the same intent, while these IDs
/// preserve future merge/rebase meaning that endpoint equality cannot recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurableRelationRewriteIntent {
    pub relation: SemanticId,
    pub rewrite_spec: SemanticId,
    pub law_set: SemanticId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRelationResolution {
    pub relation_mutations: Vec<DurableRelationMutation>,
    pub rewrite_intents: Vec<DurableRelationRewriteIntent>,
    pub causal_parents: Vec<RevisionId>,
}

/// Durable migration-complement step. The chain metadata is retained even
/// after local payload release so historical reversibility loss is explicit.
/// `released=true` is irreversible: local complement bytes can never become
/// authority again merely because a later process happens to have a copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableMigrationComplement {
    pub source_schema: kernel_types::SchemaRevisionId,
    pub target_schema: kernel_types::SchemaRevisionId,
    pub lens_spec: LensSpecId,
    pub semantic_pins: SemanticManifestId,
    pub encoding_version: u32,
    pub retention: ComplementRetention,
    pub local_complement: Option<Value>,
    pub released: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalComplementError {
    PathNotFound {
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    },
    ExternalArchiveRequired(ArchiveProofId),
    ExplicitlyForgotten {
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    },
    LocalPayloadReleased {
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalHistoricalComplementChain {
    steps: Vec<DurableMigrationComplement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HistoricalLensImplementationKey {
    pub lens_spec: LensSpecId,
    pub semantic_pins: SemanticManifestId,
    pub encoding_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoricalLensImplementation {
    Identity,
    ProductField { field: SemanticId },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistoricalLensRegistry {
    implementations: BTreeMap<HistoricalLensImplementationKey, HistoricalLensImplementation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoricalRestoreError {
    MissingImplementation(HistoricalLensImplementationKey),
    DuplicateImplementation(HistoricalLensImplementationKey),
    ComplementShapeMismatch,
}

impl HistoricalLensRegistry {
    pub fn register(
        &mut self,
        key: HistoricalLensImplementationKey,
        implementation: HistoricalLensImplementation,
    ) -> Result<(), HistoricalRestoreError> {
        if self.implementations.insert(key, implementation).is_some() {
            return Err(HistoricalRestoreError::DuplicateImplementation(key));
        }
        Ok(())
    }

    fn restore_step(
        &self,
        step: &DurableMigrationComplement,
        target: &Value,
    ) -> Result<Value, HistoricalRestoreError> {
        let key = HistoricalLensImplementationKey {
            lens_spec: step.lens_spec,
            semantic_pins: step.semantic_pins,
            encoding_version: step.encoding_version,
        };
        let implementation = self
            .implementations
            .get(&key)
            .ok_or(HistoricalRestoreError::MissingImplementation(key))?;
        let complement = step
            .local_complement
            .as_ref()
            .ok_or(HistoricalRestoreError::ComplementShapeMismatch)?;
        match implementation {
            HistoricalLensImplementation::Identity => {
                if complement == &Value::Unit {
                    Ok(target.clone())
                } else {
                    Err(HistoricalRestoreError::ComplementShapeMismatch)
                }
            }
            HistoricalLensImplementation::ProductField { field } => {
                let Value::Product(remainder) = complement else {
                    return Err(HistoricalRestoreError::ComplementShapeMismatch);
                };
                if remainder.contains_key(field) {
                    return Err(HistoricalRestoreError::ComplementShapeMismatch);
                }
                let mut source = remainder.clone();
                source.insert(*field, target.clone());
                Ok(Value::Product(source))
            }
        }
    }
}

impl LocalHistoricalComplementChain {
    #[must_use]
    pub fn steps(&self) -> &[DurableMigrationComplement] {
        &self.steps
    }

    fn push(&mut self, step: DurableMigrationComplement) {
        self.steps.push(step);
    }

    pub fn restore_value(
        &self,
        target: &Value,
        registry: &HistoricalLensRegistry,
    ) -> Result<Value, HistoricalRestoreError> {
        let mut current = target.clone();
        for step in self.steps.iter().rev() {
            current = registry.restore_step(step, &current)?;
        }
        Ok(current)
    }
}

impl DurableMigrationComplement {
    #[must_use]
    pub fn from_capsule(capsule: ComplementCapsule, retention: ComplementRetention) -> Self {
        let local_required = retention.requires_local_storage();
        Self {
            source_schema: capsule.source_schema,
            target_schema: capsule.target_schema,
            lens_spec: capsule.lens_spec,
            semantic_pins: capsule.semantic_pins,
            encoding_version: capsule.encoding_version,
            retention,
            local_complement: local_required.then_some(capsule.complement),
            released: !local_required,
        }
    }

    #[must_use]
    pub fn local_capsule(&self) -> Option<ComplementCapsule> {
        let complement = (!self.released)
            .then(|| self.local_complement.clone())
            .flatten()?;
        Some(ComplementCapsule {
            source_schema: self.source_schema,
            target_schema: self.target_schema,
            lens_spec: self.lens_spec,
            semantic_pins: self.semantic_pins,
            encoding_version: self.encoding_version,
            complement,
        })
    }

    #[must_use]
    pub const fn archive_proof(&self) -> Option<ArchiveProofId> {
        match self.retention {
            ComplementRetention::ExternalArchive(proof) => Some(proof),
            ComplementRetention::Forever
            | ComplementRetention::UntilRevision(_)
            | ComplementRetention::UntilEpoch(_)
            | ComplementRetention::Forget => None,
        }
    }

    /// Releases local authority only when the declared retention boundary is
    /// explicitly reached. Revision deadlines are matched nominally rather
    /// than ordered numerically; revision IDs are identities, not timestamps.
    pub fn release_if_due(&mut self, revision: RevisionId, epoch: u64) -> bool {
        if self.released {
            return false;
        }
        let due = match self.retention {
            ComplementRetention::Forever => false,
            ComplementRetention::UntilRevision(deadline) => deadline == revision,
            ComplementRetention::UntilEpoch(deadline) => epoch >= deadline,
            ComplementRetention::ExternalArchive(_) | ComplementRetention::Forget => true,
        };
        if due {
            self.local_complement = None;
            self.released = true;
        }
        due
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        match self.retention {
            ComplementRetention::Forever if self.released || self.local_complement.is_none() => {
                Err("forever-retained complement lost local payload")
            }
            ComplementRetention::ExternalArchive(_) | ComplementRetention::Forget
                if !self.released || self.local_complement.is_some() =>
            {
                Err("nonlocal complement retention still carries local authority")
            }
            _ if self.released && self.local_complement.is_some() => {
                Err("released complement still carries local payload")
            }
            _ if !self.released && self.local_complement.is_none() => {
                Err("unreleased complement is missing local payload")
            }
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableRevisionChange {
    RelationData {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
    },
    FullRevision {
        encoded_target_revision: Vec<u8>,
    },
    FullRevisionAndMaterializations {
        encoded_target_revision: Vec<u8>,
        materializations: Vec<DurableMaterializationSpec>,
    },
}

/// Exact client-visible intent retained across WAL, checkpoint rotation and
/// restart. Relation-data transactions retain their immutable source/target
/// lineage plus the exact typed delta instead of duplicating the complete target
/// Revision. Full-revision replacements still retain canonical target bytes.
/// Legacy stores can still be decoded, but their old target-id-only entries are
/// never treated as exact idempotency matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableTransactionIntent {
    /// Exact relation-data request identity.  The immutable source revision plus
    /// the canonical relation mutations determine the target logical state, so
    /// retaining a second full target checkpoint is redundant.
    RelationDataExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    RelationRewriteExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    /// Exact resolution Rewrite whose causal parent revisions are part of the
    /// durable transaction identity. The store derives the prerequisite effect
    /// cut from those already-authoritative revision frontiers; callers never
    /// supply raw effect ids.
    RelationResolutionExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        causal_parents: Vec<RevisionId>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    /// Exact full-revision replacement.  Full payload bytes remain necessary
    /// because this transition is not derivable from a smaller typed delta.
    Exact {
        target_revision: RevisionId,
        encoded_target_revision: Vec<u8>,
        materializations: Option<Vec<DurableMaterializationSpec>>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    /// Exact schema migration whose inverse information is part of the same
    /// PREPARE/COMMIT identity as the target revision.  The complement must
    /// not be staged in a separate generation: recovery either observes both
    /// the committed schema transition and this authority or neither.
    SchemaMigrationExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        encoded_target_revision: Vec<u8>,
        migration_complement: DurableMigrationComplement,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    LegacyTargetOnly {
        target_revision: RevisionId,
    },
}

impl DurableTransactionIntent {
    #[must_use]
    pub const fn effect_kind(&self) -> DurableEffectKind {
        match self {
            Self::RelationDataExact { .. } => DurableEffectKind::RelationData,
            Self::RelationRewriteExact { .. } => DurableEffectKind::RelationRewrite,
            Self::RelationResolutionExact { .. } => DurableEffectKind::RelationResolution,
            Self::Exact { .. } => DurableEffectKind::FullRevision,
            Self::SchemaMigrationExact { .. } => DurableEffectKind::SchemaMigration,
            Self::LegacyTargetOnly { .. } => DurableEffectKind::LegacyTargetOnly,
        }
    }

    pub fn relation_data(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        mut relation_mutations: Vec<DurableRelationMutation>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        relation_mutations.sort_by_key(|mutation| mutation.relation);
        if relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::RelationDataExact {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations,
            semantic_modules,
        })
    }

    pub fn relation_rewrites(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        mut relation_mutations: Vec<DurableRelationMutation>,
        mut rewrite_intents: Vec<DurableRelationRewriteIntent>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        relation_mutations.sort_by_key(|mutation| mutation.relation);
        rewrite_intents.sort_by_key(|intent| intent.relation);
        if relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
            || rewrite_intents
                .windows(2)
                .any(|pair| pair[0].relation == pair[1].relation)
            || relation_mutations.len() != rewrite_intents.len()
            || relation_mutations
                .iter()
                .zip(&rewrite_intents)
                .any(|(mutation, intent)| mutation.relation != intent.relation)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::RelationRewriteExact {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            semantic_modules,
        })
    }

    pub fn relation_resolution(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        mut resolution: DurableRelationResolution,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        resolution
            .relation_mutations
            .sort_by_key(|mutation| mutation.relation);
        resolution
            .rewrite_intents
            .sort_by_key(|intent| intent.relation);
        resolution.causal_parents.sort();
        if resolution
            .relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
            || resolution
                .rewrite_intents
                .windows(2)
                .any(|pair| pair[0].relation == pair[1].relation)
            || resolution.relation_mutations.len() != resolution.rewrite_intents.len()
            || resolution
                .relation_mutations
                .iter()
                .zip(&resolution.rewrite_intents)
                .any(|(mutation, intent)| mutation.relation != intent.relation)
            || resolution.causal_parents.len() < 2
            || resolution
                .causal_parents
                .windows(2)
                .any(|pair| pair[0] == pair[1])
            || resolution
                .causal_parents
                .binary_search(&source_revision)
                .is_err()
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::RelationResolutionExact {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations: resolution.relation_mutations,
            rewrite_intents: resolution.rewrite_intents,
            causal_parents: resolution.causal_parents,
            semantic_modules,
        })
    }

    pub fn revision(
        target: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::Exact {
            target_revision: target.id(),
            encoded_target_revision: checkpoint::encode_revision(target)?,
            materializations: None,
            semantic_modules,
        })
    }

    pub fn revision_and_materializations(
        target: &kernel_revision::Revision,
        materializations: &[DurableMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let mut materializations = materializations.to_vec();
        materializations.sort_by_key(|spec| spec.id);
        if materializations
            .windows(2)
            .any(|pair| pair[0].id == pair[1].id)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::Exact {
            target_revision: target.id(),
            encoded_target_revision: checkpoint::encode_revision(target)?,
            materializations: Some(materializations),
            semantic_modules,
        })
    }

    pub fn schema_migration(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        migration_complement: DurableMigrationComplement,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        migration_complement
            .validate()
            .map_err(|_| CodecError::CollectionTooLarge)?;
        if migration_complement.target_schema != target.semantic_revision().schema {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::SchemaMigrationExact {
            source_revision,
            target_revision: target.id(),
            encoded_target_revision: checkpoint::encode_revision(target)?,
            migration_complement,
            semantic_modules,
        })
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        match self {
            Self::RelationDataExact {
                target_revision, ..
            }
            | Self::RelationRewriteExact {
                target_revision, ..
            }
            | Self::RelationResolutionExact {
                target_revision, ..
            }
            | Self::Exact {
                target_revision, ..
            }
            | Self::SchemaMigrationExact {
                target_revision, ..
            }
            | Self::LegacyTargetOnly { target_revision } => *target_revision,
        }
    }

    #[must_use]
    pub const fn is_exact(&self) -> bool {
        matches!(
            self,
            Self::RelationDataExact { .. }
                | Self::RelationRewriteExact { .. }
                | Self::RelationResolutionExact { .. }
                | Self::Exact { .. }
                | Self::SchemaMigrationExact { .. }
        )
    }

    #[must_use]
    pub const fn source_revision(&self) -> Option<RevisionId> {
        match self {
            Self::RelationDataExact {
                source_revision, ..
            }
            | Self::RelationRewriteExact {
                source_revision, ..
            }
            | Self::RelationResolutionExact {
                source_revision, ..
            }
            | Self::SchemaMigrationExact {
                source_revision, ..
            } => Some(*source_revision),
            Self::Exact { .. } | Self::LegacyTargetOnly { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRevisionDescriptor {
    pub idempotency_epoch: IdempotencyEpoch,
    pub revision_effect_id: Option<RevisionEffectId>,
    pub transaction_id: ClientTransactionId,
    pub source_revision: RevisionId,
    pub target_revision: RevisionId,
    pub intent: DurableTransactionIntent,
    pub change: DurableRevisionChange,
}

impl DurableRevisionDescriptor {
    pub fn relation_data(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::relation_data(
                source_revision,
                target,
                semantic_revision,
                relation_mutations.clone(),
                registry,
            )?,
            change: DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            },
        })
    }

    pub fn relation_rewrites(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::relation_rewrites(
                source_revision,
                target,
                semantic_revision,
                relation_mutations.clone(),
                rewrite_intents,
                registry,
            )?,
            change: DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            },
        })
    }

    pub fn relation_resolution(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        resolution: DurableRelationResolution,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let relation_mutations = resolution.relation_mutations.clone();
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::relation_resolution(
                source_revision,
                target,
                semantic_revision,
                resolution,
                registry,
            )?,
            change: DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            },
        })
    }

    pub fn full_revision(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent: DurableTransactionIntent::revision(target, registry)?,
            change: DurableRevisionChange::FullRevision {
                encoded_target_revision: checkpoint::encode_revision(target)?,
            },
        })
    }

    pub fn schema_migration(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        migration_complement: DurableMigrationComplement,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let intent = DurableTransactionIntent::schema_migration(
            source_revision,
            target,
            migration_complement,
            registry,
        )?;
        let DurableTransactionIntent::SchemaMigrationExact {
            encoded_target_revision,
            ..
        } = &intent
        else {
            unreachable!("schema migration intent has exact target bytes")
        };
        let encoded_target_revision = encoded_target_revision.clone();
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent,
            change: DurableRevisionChange::FullRevision {
                encoded_target_revision,
            },
        })
    }

    pub fn full_revision_and_materializations(
        transaction_id: ClientTransactionId,
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        materializations: &[DurableMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let intent = DurableTransactionIntent::revision_and_materializations(
            target,
            materializations,
            registry,
        )?;
        let DurableTransactionIntent::Exact {
            encoded_target_revision,
            materializations: Some(materializations),
            ..
        } = intent.clone()
        else {
            unreachable!("combined durable intent is exact and carries materializations")
        };
        Ok(Self {
            idempotency_epoch: IdempotencyEpoch::ZERO,
            revision_effect_id: None,
            transaction_id,
            source_revision,
            target_revision: target.id(),
            intent,
            change: DurableRevisionChange::FullRevisionAndMaterializations {
                encoded_target_revision,
                materializations,
            },
        })
    }

    pub fn decode_full_revision(
        &self,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<kernel_revision::Revision>, DurabilityError> {
        match &self.change {
            DurableRevisionChange::RelationData { .. } => Ok(None),
            DurableRevisionChange::FullRevision {
                encoded_target_revision,
            }
            | DurableRevisionChange::FullRevisionAndMaterializations {
                encoded_target_revision,
                ..
            } => {
                let revision = checkpoint::decode_revision(encoded_target_revision, registry)?;
                if revision.id() != self.target_revision {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "full revision payload target id mismatch",
                    });
                }
                Ok(Some(revision))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableMaterializationSpec {
    pub id: MaterializationId,
    pub query: kernel_query::RelExpr,
}

/// Layout-independent recipe for reconstructible physical state retained across
/// checkpoint/reopen. Payload row handles are intentionally not durable; recovery
/// rebuilds each artifact against the fresh runtime layout from authoritative data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurableSemanticKeyPart {
    pub column: usize,
    pub equivalence: SemanticId,
}

/// Reconstructible native representation for one active relation layout.
///
/// This is intentionally a logical lowering recipe, not a serialized physical
/// payload: row handles, dense local ids and allocator representation are rebuilt
/// against the recovered authoritative Revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurableRelationLayoutKind {
    RowStore,
    ValueColumnar,
    I64Columnar,
    TypedColumnar,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurablePhysicalArtifactSpec {
    RelationLayout {
        relation: SemanticId,
        layout_id: u128,
        kind: DurableRelationLayoutKind,
    },
    I64Index {
        relation: SemanticId,
        key_column: usize,
        equivalence: SemanticId,
        advisor_managed: bool,
    },
    SemanticIndex {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        advisor_managed: bool,
    },
    SemanticQuotientFactor {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        advisor_managed: bool,
    },
    SemanticStatistics {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        advisor_managed: bool,
    },
    ObservableAtom {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        advisor_managed: bool,
    },
}

/// Optional reconstructible semantic payload persisted beside a physical-artifact recipe.
///
/// Cores are never semantic authority. They are generation-local acceleration data and contain
/// no revision/process-local physical row handles. A stale or incompatible core is discarded and
/// the recipe falls back to a full exact rebuild.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurableArtifactCore {
    ObservableAtom {
        source_revision: RevisionId,
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
        /// One canonical-key tuple per durable relation occurrence ordinal.
        encoded_keys_by_ordinal: Vec<Vec<u8>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DurablePhysicalArtifactKey {
    RelationLayout {
        relation: SemanticId,
        layout_id: u128,
        kind: DurableRelationLayoutKind,
    },
    I64Index {
        relation: SemanticId,
        key_column: usize,
        equivalence: SemanticId,
    },
    Index {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
    },
    QuotientFactor {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
    },
    Statistics {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
    },
    ObservableAtom {
        relation: SemanticId,
        key_parts: Vec<DurableSemanticKeyPart>,
    },
}

fn physical_artifact_key(spec: &DurablePhysicalArtifactSpec) -> (DurablePhysicalArtifactKey, bool) {
    match spec {
        DurablePhysicalArtifactSpec::RelationLayout {
            relation,
            layout_id,
            kind,
        } => (
            DurablePhysicalArtifactKey::RelationLayout {
                relation: *relation,
                layout_id: *layout_id,
                kind: *kind,
            },
            false,
        ),
        DurablePhysicalArtifactSpec::I64Index {
            relation,
            key_column,
            equivalence,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::I64Index {
                relation: *relation,
                key_column: *key_column,
                equivalence: *equivalence,
            },
            *advisor_managed,
        ),
        DurablePhysicalArtifactSpec::SemanticIndex {
            relation,
            key_parts,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::Index {
                relation: *relation,
                key_parts: key_parts.clone(),
            },
            *advisor_managed,
        ),
        DurablePhysicalArtifactSpec::SemanticQuotientFactor {
            relation,
            key_parts,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::QuotientFactor {
                relation: *relation,
                key_parts: key_parts.clone(),
            },
            *advisor_managed,
        ),
        DurablePhysicalArtifactSpec::SemanticStatistics {
            relation,
            key_parts,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::Statistics {
                relation: *relation,
                key_parts: key_parts.clone(),
            },
            *advisor_managed,
        ),
        DurablePhysicalArtifactSpec::ObservableAtom {
            relation,
            key_parts,
            advisor_managed,
        } => (
            DurablePhysicalArtifactKey::ObservableAtom {
                relation: *relation,
                key_parts: key_parts.clone(),
            },
            *advisor_managed,
        ),
    }
}

fn physical_artifact_spec_from_key(
    key: DurablePhysicalArtifactKey,
    advisor_managed: bool,
) -> DurablePhysicalArtifactSpec {
    match key {
        DurablePhysicalArtifactKey::RelationLayout {
            relation,
            layout_id,
            kind,
        } => DurablePhysicalArtifactSpec::RelationLayout {
            relation,
            layout_id,
            kind,
        },
        DurablePhysicalArtifactKey::I64Index {
            relation,
            key_column,
            equivalence,
        } => DurablePhysicalArtifactSpec::I64Index {
            relation,
            key_column,
            equivalence,
            advisor_managed,
        },
        DurablePhysicalArtifactKey::Index {
            relation,
            key_parts,
        } => DurablePhysicalArtifactSpec::SemanticIndex {
            relation,
            key_parts,
            advisor_managed,
        },
        DurablePhysicalArtifactKey::QuotientFactor {
            relation,
            key_parts,
        } => DurablePhysicalArtifactSpec::SemanticQuotientFactor {
            relation,
            key_parts,
            advisor_managed,
        },
        DurablePhysicalArtifactKey::Statistics {
            relation,
            key_parts,
        } => DurablePhysicalArtifactSpec::SemanticStatistics {
            relation,
            key_parts,
            advisor_managed,
        },
        DurablePhysicalArtifactKey::ObservableAtom {
            relation,
            key_parts,
        } => DurablePhysicalArtifactSpec::ObservableAtom {
            relation,
            key_parts,
            advisor_managed,
        },
    }
}

fn canonical_physical_artifact_specs(
    specs: &[DurablePhysicalArtifactSpec],
) -> Vec<DurablePhysicalArtifactSpec> {
    let mut ownership = BTreeMap::<DurablePhysicalArtifactKey, bool>::new();
    for spec in specs {
        let (key, advisor_managed) = physical_artifact_key(spec);
        ownership
            .entry(key)
            .and_modify(|managed| *managed &= advisor_managed)
            .or_insert(advisor_managed);
    }
    ownership
        .into_iter()
        .map(|(key, advisor_managed)| physical_artifact_spec_from_key(key, advisor_managed))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableTransactionOutcome {
    Unknown,
    RetryHistoryExpired,
    Committed { target_revision: RevisionId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurablePrepareToken {
    transaction_id: ClientTransactionId,
    target_revision: RevisionId,
    prepare_lsn: u64,
    prepare_payload_crc32c: u32,
}

impl DurablePrepareToken {
    #[must_use]
    pub const fn transaction_id(self) -> ClientTransactionId {
        self.transaction_id
    }

    #[must_use]
    pub const fn target_revision(self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub const fn prepare_lsn(self) -> u64 {
        self.prepare_lsn
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableCommitReceipt {
    target_revision: RevisionId,
    prepare_lsn: u64,
    commit_lsn: u64,
}

impl DurableCommitReceipt {
    #[must_use]
    pub const fn target_revision(self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub const fn prepare_lsn(self) -> u64 {
        self.prepare_lsn
    }

    #[must_use]
    pub const fn commit_lsn(self) -> u64 {
        self.commit_lsn
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedRevision {
    pub descriptor: DurableRevisionDescriptor,
    pub prepare_lsn: u64,
    pub prepare_payload_crc32c: u32,
    pub commit_lsn: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailStatus {
    Clean,
    Truncated { offset: usize },
    Garbage { offset: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryScan {
    base_revision: RevisionId,
    committed: Vec<CommittedRevision>,
    last_good_offset: usize,
    next_lsn: u64,
    tail_status: TailStatus,
    committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
}

impl RecoveryScan {
    #[must_use]
    pub fn durable_revision(&self) -> RevisionId {
        self.committed
            .last()
            .map_or(self.base_revision, |revision| {
                revision.descriptor.target_revision
            })
    }

    #[must_use]
    pub const fn base_revision(&self) -> RevisionId {
        self.base_revision
    }

    #[must_use]
    pub fn committed(&self) -> &[CommittedRevision] {
        &self.committed
    }

    #[must_use]
    pub const fn last_good_offset(&self) -> usize {
        self.last_good_offset
    }

    #[must_use]
    pub const fn next_lsn(&self) -> u64 {
        self.next_lsn
    }

    #[must_use]
    pub const fn tail_status(&self) -> &TailStatus {
        &self.tail_status
    }

    #[must_use]
    pub fn transaction_outcome(
        &self,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        self.transaction_outcome_at(IdempotencyEpoch::ZERO, transaction_id)
    }

    #[must_use]
    pub fn transaction_intent(
        &self,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableTransactionIntent> {
        self.transaction_intent_at(IdempotencyEpoch::ZERO, transaction_id)
    }

    #[must_use]
    pub fn transaction_outcome_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> DurableTransactionOutcome {
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
            .map_or(DurableTransactionOutcome::Unknown, |intent| {
                DurableTransactionOutcome::Committed {
                    target_revision: intent.target_revision(),
                }
            })
    }

    #[must_use]
    pub const fn committed_transactions(
        &self,
    ) -> &BTreeMap<DurableTransactionKey, DurableTransactionIntent> {
        &self.committed_transactions
    }

    #[must_use]
    pub fn transaction_intent_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> Option<&DurableTransactionIntent> {
        self.committed_transactions
            .get(&DurableTransactionKey::new(epoch, transaction_id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableFormatComponent {
    Manifest,
    CheckpointFile,
    MetadataFile,
}

#[derive(Debug)]
pub enum DurabilityError {
    Io(io::Error),
    Poisoned,
    LsnExhausted,
    PayloadTooLarge,
    UnsupportedDurableFormat {
        component: DurableFormatComponent,
        version: u16,
    },
    Encode(CodecError),
    Corruption {
        offset: usize,
        reason: &'static str,
    },
    Protocol {
        offset: usize,
        reason: &'static str,
    },
}

impl PartialEq for DurabilityError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Io(left), Self::Io(right)) => left.kind() == right.kind(),
            (Self::Poisoned, Self::Poisoned)
            | (Self::LsnExhausted, Self::LsnExhausted)
            | (Self::PayloadTooLarge, Self::PayloadTooLarge) => true,
            (
                Self::UnsupportedDurableFormat {
                    component: left_component,
                    version: left_version,
                },
                Self::UnsupportedDurableFormat {
                    component: right_component,
                    version: right_version,
                },
            ) => left_component == right_component && left_version == right_version,
            (Self::Encode(left), Self::Encode(right)) => left == right,
            (
                Self::Corruption {
                    offset: left_offset,
                    reason: left_reason,
                },
                Self::Corruption {
                    offset: right_offset,
                    reason: right_reason,
                },
            )
            | (
                Self::Protocol {
                    offset: left_offset,
                    reason: left_reason,
                },
                Self::Protocol {
                    offset: right_offset,
                    reason: right_reason,
                },
            ) => left_offset == right_offset && left_reason == right_reason,
            _ => false,
        }
    }
}

impl Eq for DurabilityError {}

impl From<io::Error> for DurabilityError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<CodecError> for DurabilityError {
    fn from(value: CodecError) -> Self {
        Self::Encode(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecError {
    LengthOverflow,
    CollectionTooLarge,
    ValueNestingTooDeep,
    SemanticModuleUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum RecordKind {
    PrepareRevision = 1,
    CommitRevision = 2,
}

impl TryFrom<u8> for RecordKind {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::PrepareRevision),
            2 => Ok(Self::CommitRevision),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitRecord {
    target_revision: RevisionId,
    prepare_lsn: u64,
    prepare_payload_crc32c: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EncodedFrame {
    lsn: u64,
    payload_crc32c: u32,
    bytes: Vec<u8>,
}

pub trait RevisionDurability {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError>;

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError>;
}

#[derive(Debug)]
pub struct FileRevisionWal {
    path: PathBuf,
    file: File,
    next_lsn: u64,
    poisoned: bool,
}

impl FileRevisionWal {
    pub fn create(path: impl AsRef<Path>) -> Result<Self, DurabilityError> {
        Self::create_at_lsn(path, 1)
    }

    pub(crate) fn create_at_lsn(
        path: impl AsRef<Path>,
        next_lsn: u64,
    ) -> Result<Self, DurabilityError> {
        if next_lsn == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "WAL next LSN must be nonzero",
            });
        }
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
        file.lock()?;
        Ok(Self {
            path,
            file,
            next_lsn,
            poisoned: false,
        })
    }

    pub fn open_recovered(
        path: impl AsRef<Path>,
        base_revision: RevisionId,
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        Self::open_recovered_seeded(path, base_revision, 1, &[])
    }

    pub(crate) fn open_recovered_seeded(
        path: impl AsRef<Path>,
        base_revision: RevisionId,
        first_lsn: u64,
        seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
    ) -> Result<(Self, RecoveryScan), DurabilityError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)?;
        file.lock()?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let scan = scan_wal_seeded(&bytes, base_revision, first_lsn, seeded_prepares)?;
        if scan.last_good_offset() < bytes.len() {
            file.set_len(
                u64::try_from(scan.last_good_offset()).map_err(|_| CodecError::LengthOverflow)?,
            )?;
            file.sync_all()?;
        }
        file.seek(SeekFrom::End(0))?;
        Ok((
            Self {
                path,
                file,
                next_lsn: scan.next_lsn(),
                poisoned: false,
            },
            scan,
        ))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub(crate) const fn next_lsn(&self) -> u64 {
        self.next_lsn
    }

    #[must_use]
    pub(crate) const fn last_lsn(&self) -> u64 {
        self.next_lsn - 1
    }

    fn append_frame(
        &mut self,
        kind: RecordKind,
        revision: RevisionId,
        payload: &[u8],
    ) -> Result<EncodedFrame, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let lsn = self.next_lsn;
        let next_lsn = lsn.checked_add(1).ok_or(DurabilityError::LsnExhausted)?;
        let frame = encode_frame(lsn, kind, revision, payload)?;
        if let Err(error) = self.file.write_all(&frame.bytes) {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        self.next_lsn = next_lsn;
        Ok(frame)
    }

    pub(crate) fn append_exact_frame(
        &mut self,
        frame: &EncodedFrame,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if frame.lsn != self.next_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "shadow WAL exact frame LSN is not contiguous",
            });
        }
        if let Err(error) = self.file.write_all(&frame.bytes) {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        self.next_lsn = self
            .next_lsn
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?;
        Ok(())
    }

    fn barrier(&mut self) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if let Err(error) = self.file.sync_data() {
            self.poisoned = true;
            return Err(DurabilityError::Io(error));
        }
        Ok(())
    }

    pub(crate) fn append_prepare_unflushed(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        self.append_prepare_unflushed_with_frame(descriptor)
            .map(|(token, _)| token)
    }

    pub(crate) fn append_prepare_unflushed_with_frame(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<(DurablePrepareToken, EncodedFrame), DurabilityError> {
        let payload = encode_prepare_payload(descriptor)?;
        let frame = self.append_frame(
            RecordKind::PrepareRevision,
            descriptor.target_revision,
            &payload,
        )?;
        let token = DurablePrepareToken {
            transaction_id: descriptor.transaction_id,
            target_revision: descriptor.target_revision,
            prepare_lsn: frame.lsn,
            prepare_payload_crc32c: frame.payload_crc32c,
        };
        Ok((token, frame))
    }

    pub(crate) fn append_commit_unflushed(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        self.append_commit_unflushed_with_frame(prepared)
            .map(|(receipt, _)| receipt)
    }

    pub(crate) fn append_commit_unflushed_with_frame(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<(DurableCommitReceipt, EncodedFrame), DurabilityError> {
        let record = CommitRecord {
            target_revision: prepared.target_revision,
            prepare_lsn: prepared.prepare_lsn,
            prepare_payload_crc32c: prepared.prepare_payload_crc32c,
        };
        let payload = encode_commit_payload(&record);
        let frame = self.append_frame(
            RecordKind::CommitRevision,
            prepared.target_revision,
            &payload,
        )?;
        let receipt = DurableCommitReceipt {
            target_revision: prepared.target_revision,
            prepare_lsn: prepared.prepare_lsn,
            commit_lsn: frame.lsn,
        };
        Ok((receipt, frame))
    }

    pub(crate) fn durability_barrier(&mut self) -> Result<(), DurabilityError> {
        self.barrier()
    }
}

impl RevisionDurability for FileRevisionWal {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        let prepared = self.append_prepare_unflushed(descriptor)?;
        self.barrier()?;
        Ok(prepared)
    }

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        let receipt = self.append_commit_unflushed(prepared)?;
        self.barrier()?;
        Ok(receipt)
    }
}

#[derive(Debug, Default)]
pub struct SimulatedRevisionWal {
    bytes: Vec<u8>,
    durable_len: usize,
    next_lsn: u64,
}

impl SimulatedRevisionWal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            durable_len: 0,
            next_lsn: 1,
        }
    }

    #[must_use]
    pub fn crash_image(&self) -> &[u8] {
        &self.bytes[..self.durable_len]
    }

    #[must_use]
    pub fn volatile_image(&self) -> &[u8] {
        &self.bytes
    }

    fn append_frame(
        &mut self,
        kind: RecordKind,
        revision: RevisionId,
        payload: &[u8],
    ) -> Result<EncodedFrame, DurabilityError> {
        let lsn = self.next_lsn;
        self.next_lsn = lsn.checked_add(1).ok_or(DurabilityError::LsnExhausted)?;
        let frame = encode_frame(lsn, kind, revision, payload)?;
        self.bytes.extend_from_slice(&frame.bytes);
        Ok(frame)
    }

    fn barrier(&mut self) {
        self.durable_len = self.bytes.len();
    }
}

impl RevisionDurability for SimulatedRevisionWal {
    fn durably_prepare(
        &mut self,
        descriptor: &DurableRevisionDescriptor,
    ) -> Result<DurablePrepareToken, DurabilityError> {
        let payload = encode_prepare_payload(descriptor)?;
        let frame = self.append_frame(
            RecordKind::PrepareRevision,
            descriptor.target_revision,
            &payload,
        )?;
        self.barrier();
        Ok(DurablePrepareToken {
            transaction_id: descriptor.transaction_id,
            target_revision: descriptor.target_revision,
            prepare_lsn: frame.lsn,
            prepare_payload_crc32c: frame.payload_crc32c,
        })
    }

    fn durably_commit(
        &mut self,
        prepared: DurablePrepareToken,
    ) -> Result<DurableCommitReceipt, DurabilityError> {
        let record = CommitRecord {
            target_revision: prepared.target_revision,
            prepare_lsn: prepared.prepare_lsn,
            prepare_payload_crc32c: prepared.prepare_payload_crc32c,
        };
        let payload = encode_commit_payload(&record);
        let frame = self.append_frame(
            RecordKind::CommitRevision,
            prepared.target_revision,
            &payload,
        )?;
        self.barrier();
        Ok(DurableCommitReceipt {
            target_revision: prepared.target_revision,
            prepare_lsn: prepared.prepare_lsn,
            commit_lsn: frame.lsn,
        })
    }
}

pub fn scan_wal(bytes: &[u8], base_revision: RevisionId) -> Result<RecoveryScan, DurabilityError> {
    scan_wal_seeded(bytes, base_revision, 1, &[])
}

fn scan_wal_seeded(
    bytes: &[u8],
    base_revision: RevisionId,
    first_lsn: u64,
    seeded_prepares: &[(u64, DurableRevisionDescriptor, u32)],
) -> Result<RecoveryScan, DurabilityError> {
    if first_lsn == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "WAL first LSN must be nonzero",
        });
    }
    let mut state = ScanState::new(base_revision);
    for (lsn, descriptor, payload_crc) in seeded_prepares {
        state.seed_prepare(*lsn, descriptor.clone(), *payload_crc)?;
    }
    let mut offset = 0_usize;
    let mut expected_lsn = first_lsn;
    while offset < bytes.len() {
        match read_frame(bytes, offset, expected_lsn)? {
            FrameRead::Tail(tail_status) => {
                return Ok(state.finish(offset, expected_lsn, tail_status));
            }
            FrameRead::Complete(frame) => {
                let frame_len = frame.frame_len;
                state.accept(&frame)?;
                offset = offset
                    .checked_add(frame_len)
                    .ok_or(DurabilityError::Corruption {
                        offset,
                        reason: "scan offset overflow",
                    })?;
                expected_lsn = expected_lsn
                    .checked_add(1)
                    .ok_or(DurabilityError::LsnExhausted)?;
            }
        }
    }
    Ok(state.finish(offset, expected_lsn, TailStatus::Clean))
}

struct ScanState {
    base_revision: RevisionId,
    durable_head: RevisionId,
    prepares_by_lsn: BTreeMap<u64, (DurableRevisionDescriptor, u32)>,
    prepare_identity: BTreeMap<RevisionId, DurableRevisionDescriptor>,
    prepare_transaction_identity: BTreeMap<DurableTransactionKey, DurableRevisionDescriptor>,
    commits: BTreeMap<RevisionId, CommitRecord>,
    transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    committed: Vec<CommittedRevision>,
}

impl ScanState {
    fn new(base_revision: RevisionId) -> Self {
        Self {
            base_revision,
            durable_head: base_revision,
            prepares_by_lsn: BTreeMap::new(),
            prepare_identity: BTreeMap::new(),
            prepare_transaction_identity: BTreeMap::new(),
            commits: BTreeMap::new(),
            transactions: BTreeMap::new(),
            committed: Vec::new(),
        }
    }

    fn accept(&mut self, frame: &DecodedFrame<'_>) -> Result<(), DurabilityError> {
        match frame.kind {
            RecordKind::PrepareRevision => self.accept_prepare(frame),
            RecordKind::CommitRevision => self.accept_commit(frame),
        }
    }

    fn seed_prepare(
        &mut self,
        lsn: u64,
        descriptor: DurableRevisionDescriptor,
        payload_crc: u32,
    ) -> Result<(), DurabilityError> {
        let transaction_key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        if let Some(existing) = self.prepare_identity.get(&descriptor.target_revision)
            && existing != &descriptor
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "prepared cut capsule conflicts by target revision",
            });
        }
        if let Some(existing) = self.prepare_transaction_identity.get(&transaction_key)
            && existing != &descriptor
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "prepared cut capsule conflicts by transaction identity",
            });
        }
        self.prepare_identity
            .insert(descriptor.target_revision, descriptor.clone());
        self.prepare_transaction_identity
            .insert(transaction_key, descriptor.clone());
        self.prepares_by_lsn.insert(lsn, (descriptor, payload_crc));
        Ok(())
    }

    fn accept_prepare(&mut self, frame: &DecodedFrame<'_>) -> Result<(), DurabilityError> {
        let descriptor =
            decode_prepare_payload(frame.revision, frame.payload).map_err(|reason| {
                DurabilityError::Corruption {
                    offset: frame.offset,
                    reason,
                }
            })?;
        let transaction_key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        if let Some(existing_intent) = self.transactions.get(&transaction_key)
            && existing_intent != &descriptor.intent
        {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "transaction id already committed to another exact intent",
            });
        }
        if let Some(existing) = self.prepare_identity.get(&frame.revision) {
            if existing != &descriptor {
                return Err(DurabilityError::Protocol {
                    offset: frame.offset,
                    reason: "conflicting duplicate prepare",
                });
            }
        } else {
            self.prepare_identity
                .insert(frame.revision, descriptor.clone());
        }
        if let Some(existing) = self.prepare_transaction_identity.get(&transaction_key) {
            if existing != &descriptor {
                return Err(DurabilityError::Protocol {
                    offset: frame.offset,
                    reason: "transaction id reused by conflicting prepare",
                });
            }
        } else {
            self.prepare_transaction_identity
                .insert(transaction_key, descriptor.clone());
        }
        self.prepares_by_lsn
            .insert(frame.lsn, (descriptor, frame.payload_crc));
        Ok(())
    }

    fn accept_commit(&mut self, frame: &DecodedFrame<'_>) -> Result<(), DurabilityError> {
        let record = decode_commit_payload(frame.revision, frame.payload).map_err(|reason| {
            DurabilityError::Corruption {
                offset: frame.offset,
                reason,
            }
        })?;
        let Some((descriptor, prepare_crc)) = self.prepares_by_lsn.get(&record.prepare_lsn) else {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "commit references missing prepare",
            });
        };
        if descriptor.target_revision != frame.revision
            || *prepare_crc != record.prepare_payload_crc32c
        {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "commit does not bind the referenced prepare",
            });
        }
        if let Some(existing) = self.commits.get(&frame.revision) {
            if existing != &record {
                return Err(DurabilityError::Protocol {
                    offset: frame.offset,
                    reason: "conflicting duplicate commit",
                });
            }
            return Ok(());
        }
        if descriptor.source_revision != self.durable_head {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "commit source revision does not match durable head",
            });
        }
        if descriptor.target_revision == descriptor.source_revision {
            return Err(DurabilityError::Protocol {
                offset: frame.offset,
                reason: "revision transition does not advance identity",
            });
        }
        let transaction_key =
            DurableTransactionKey::new(descriptor.idempotency_epoch, descriptor.transaction_id);
        if let Some(existing_intent) = self.transactions.get(&transaction_key) {
            if existing_intent != &descriptor.intent {
                return Err(DurabilityError::Protocol {
                    offset: frame.offset,
                    reason: "transaction id conflicts with prior committed intent",
                });
            }
        } else {
            self.transactions
                .insert(transaction_key, descriptor.intent.clone());
        }
        self.commits.insert(frame.revision, record.clone());
        self.committed.push(CommittedRevision {
            descriptor: descriptor.clone(),
            prepare_lsn: record.prepare_lsn,
            prepare_payload_crc32c: record.prepare_payload_crc32c,
            commit_lsn: frame.lsn,
        });
        self.durable_head = descriptor.target_revision;
        Ok(())
    }

    fn finish(
        self,
        last_good_offset: usize,
        next_lsn: u64,
        tail_status: TailStatus,
    ) -> RecoveryScan {
        RecoveryScan {
            base_revision: self.base_revision,
            committed: self.committed,
            last_good_offset,
            next_lsn,
            tail_status,
            committed_transactions: self.transactions,
        }
    }
}

struct DecodedFrame<'a> {
    offset: usize,
    kind: RecordKind,
    lsn: u64,
    revision: RevisionId,
    payload_crc: u32,
    payload: &'a [u8],
    frame_len: usize,
}

enum FrameRead<'a> {
    Complete(DecodedFrame<'a>),
    Tail(TailStatus),
}

fn read_frame(
    bytes: &[u8],
    offset: usize,
    expected_lsn: u64,
) -> Result<FrameRead<'_>, DurabilityError> {
    let remaining = bytes.len() - offset;
    if remaining < HEADER_LEN {
        let tail = &bytes[offset..];
        let prefix_len = tail.len().min(MAGIC.len());
        let looks_torn = tail[..prefix_len] == MAGIC[..prefix_len];
        return Ok(FrameRead::Tail(if looks_torn {
            TailStatus::Truncated { offset }
        } else {
            TailStatus::Garbage { offset }
        }));
    }
    if bytes[offset..offset + 4] != MAGIC {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "non-frame bytes large enough to hide a complete frame",
        });
    }
    let header = &bytes[offset..offset + HEADER_LEN];
    validate_frame_header(header, offset, expected_lsn)?;
    let payload_len =
        usize::try_from(read_u32(&header[8..12])).map_err(|_| DurabilityError::Corruption {
            offset,
            reason: "payload length overflow",
        })?;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "payload length exceeds hard limit",
        });
    }
    let frame_len = HEADER_LEN
        .checked_add(payload_len)
        .ok_or(DurabilityError::Corruption {
            offset,
            reason: "frame length overflow",
        })?;
    if remaining < frame_len {
        return Ok(FrameRead::Tail(TailStatus::Truncated { offset }));
    }
    let payload = &bytes[offset + HEADER_LEN..offset + frame_len];
    let payload_crc = read_u32(&header[28..32]);
    if crc32c(payload) != payload_crc {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "payload checksum mismatch",
        });
    }
    Ok(FrameRead::Complete(DecodedFrame {
        offset,
        kind: RecordKind::try_from(header[6]).map_err(|()| DurabilityError::Corruption {
            offset,
            reason: "unknown frame kind",
        })?,
        lsn: read_u64(&header[12..20]),
        revision: RevisionId::new(read_u64(&header[20..28])),
        payload_crc,
        payload,
        frame_len,
    }))
}

fn validate_frame_header(
    header: &[u8],
    offset: usize,
    expected_lsn: u64,
) -> Result<(), DurabilityError> {
    if read_u16(&header[4..6]) != FORMAT_VERSION {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "unsupported/corrupt frame version",
        });
    }
    RecordKind::try_from(header[6]).map_err(|()| DurabilityError::Corruption {
        offset,
        reason: "unknown frame kind",
    })?;
    if header[7] != 0 {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "unknown frame flags",
        });
    }
    if crc32c(&header[..32]) != read_u32(&header[32..36]) {
        return Err(DurabilityError::Corruption {
            offset,
            reason: "header checksum mismatch",
        });
    }
    if read_u64(&header[12..20]) != expected_lsn {
        return Err(DurabilityError::Protocol {
            offset,
            reason: "non-monotone or gapped LSN",
        });
    }
    Ok(())
}

fn encode_frame(
    lsn: u64,
    kind: RecordKind,
    revision: RevisionId,
    payload: &[u8],
) -> Result<EncodedFrame, DurabilityError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    let payload_len = u32::try_from(payload.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
    let payload_crc32c = crc32c(payload);
    let mut header = [0_u8; HEADER_LEN];
    header[0..4].copy_from_slice(&MAGIC);
    header[4..6].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[6] = kind as u8;
    header[7] = 0;
    header[8..12].copy_from_slice(&payload_len.to_le_bytes());
    header[12..20].copy_from_slice(&lsn.to_le_bytes());
    header[20..28].copy_from_slice(&revision.raw().to_le_bytes());
    header[28..32].copy_from_slice(&payload_crc32c.to_le_bytes());
    let header_crc = crc32c(&header[..32]);
    header[32..36].copy_from_slice(&header_crc.to_le_bytes());
    let mut bytes = Vec::with_capacity(HEADER_LEN + payload.len());
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(payload);
    Ok(EncodedFrame {
        lsn,
        payload_crc32c,
        bytes,
    })
}

fn encode_relation_mutations(
    out: &mut Vec<u8>,
    relation_mutations: &[DurableRelationMutation],
) -> Result<(), CodecError> {
    push_len(out, relation_mutations.len())?;
    let mut previous = None;
    for mutation in relation_mutations {
        if previous.is_some_and(|id: SemanticId| id >= mutation.relation) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous = Some(mutation.relation);
        push_u128(out, mutation.relation.raw());
        encode_rows(out, &mutation.inserted)?;
        encode_rows(out, &mutation.removed)?;
    }
    Ok(())
}

fn encode_relation_rewrite_intents(
    out: &mut Vec<u8>,
    rewrite_intents: &[DurableRelationRewriteIntent],
) -> Result<(), CodecError> {
    push_len(out, rewrite_intents.len())?;
    let mut previous = None;
    for intent in rewrite_intents {
        if previous.is_some_and(|relation: SemanticId| relation >= intent.relation) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous = Some(intent.relation);
        push_u128(out, intent.relation.raw());
        push_u128(out, intent.rewrite_spec.raw());
        push_u128(out, intent.law_set.raw());
    }
    Ok(())
}

fn encode_schema_migration_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    encoded_target_revision: &[u8],
    migration_complement: &DurableMigrationComplement,
    semantic_modules: &[BuiltinSemanticModuleSpec],
) -> Result<(), CodecError> {
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
    {
        return Err(CodecError::CollectionTooLarge);
    }
    let DurableRevisionChange::FullRevision {
        encoded_target_revision: change_target,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if encoded_target_revision != change_target {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(4);
    push_bytes(out, encoded_target_revision)?;
    metadata::encode_migration_complements(out, std::slice::from_ref(migration_complement))?;
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    Ok(())
}

fn encode_relation_data_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    semantic_revision: SemanticRevision,
    relation_mutations: &[DurableRelationMutation],
    semantic_modules: &[BuiltinSemanticModuleSpec],
) -> Result<(), CodecError> {
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
        || semantic_revision != *change_semantics
        || relation_mutations != change_mutations
    {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(2);
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    push_u64(out, semantic_revision.schema.raw());
    push_u64(out, semantic_revision.environment.raw());
    encode_relation_mutations(out, relation_mutations)?;
    Ok(())
}

fn encode_relation_resolution_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    semantic_revision: SemanticRevision,
    resolution: &DurableRelationResolution,
    semantic_modules: &[BuiltinSemanticModuleSpec],
) -> Result<(), CodecError> {
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
        || semantic_revision != *change_semantics
        || resolution.relation_mutations != *change_mutations
    {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(5);
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    push_u64(out, semantic_revision.schema.raw());
    push_u64(out, semantic_revision.environment.raw());
    encode_relation_mutations(out, &resolution.relation_mutations)?;
    encode_relation_rewrite_intents(out, &resolution.rewrite_intents)?;
    push_len(out, resolution.causal_parents.len())?;
    for parent in &resolution.causal_parents {
        push_u64(out, parent.raw());
    }
    Ok(())
}

fn encode_relation_rewrite_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    intent: &DurableTransactionIntent,
) -> Result<(), CodecError> {
    let DurableTransactionIntent::RelationRewriteExact {
        source_revision,
        target_revision,
        semantic_revision,
        relation_mutations,
        rewrite_intents,
        semantic_modules,
    } = intent
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if *source_revision != descriptor.source_revision
        || *target_revision != descriptor.target_revision
        || *semantic_revision != *change_semantics
        || relation_mutations != change_mutations
    {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(3);
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    push_u64(out, semantic_revision.schema.raw());
    push_u64(out, semantic_revision.environment.raw());
    encode_relation_mutations(out, relation_mutations)?;
    encode_relation_rewrite_intents(out, rewrite_intents)?;
    Ok(())
}

fn encode_prepare_identity_prefix(out: &mut Vec<u8>, descriptor: &DurableRevisionDescriptor) {
    let identity_bound = descriptor.idempotency_epoch != IdempotencyEpoch::ZERO
        || descriptor.revision_effect_id.is_some();
    push_u16(
        out,
        if identity_bound {
            MUTATION_CODEC_VERSION
        } else {
            8
        },
    );
    push_u128(out, descriptor.transaction_id.raw());
    if identity_bound {
        push_u64(out, descriptor.idempotency_epoch.raw());
        match descriptor.revision_effect_id {
            Some(id) => {
                out.push(1);
                push_u128(out, id.0);
            }
            None => out.push(0),
        }
    }
    push_u64(out, descriptor.source_revision.raw());
}

fn encode_prepare_payload(descriptor: &DurableRevisionDescriptor) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    encode_prepare_identity_prefix(&mut out, descriptor);
    match &descriptor.intent {
        DurableTransactionIntent::RelationDataExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            semantic_modules,
        } => encode_relation_data_prepare(
            &mut out,
            descriptor,
            *source_revision,
            *target_revision,
            *semantic_revision,
            relation_mutations,
            semantic_modules,
        )?,
        intent @ DurableTransactionIntent::RelationRewriteExact { .. } => {
            encode_relation_rewrite_prepare(&mut out, descriptor, intent)?;
        }
        DurableTransactionIntent::RelationResolutionExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            causal_parents,
            semantic_modules,
        } => encode_relation_resolution_prepare(
            &mut out,
            descriptor,
            *source_revision,
            *target_revision,
            *semantic_revision,
            &DurableRelationResolution {
                relation_mutations: relation_mutations.clone(),
                rewrite_intents: rewrite_intents.clone(),
                causal_parents: causal_parents.clone(),
            },
            semantic_modules,
        )?,
        DurableTransactionIntent::Exact {
            target_revision,
            encoded_target_revision,
            materializations,
            semantic_modules,
        } => {
            if *target_revision != descriptor.target_revision {
                return Err(CodecError::CollectionTooLarge);
            }
            out.push(1);
            push_bytes(&mut out, encoded_target_revision)?;
            match materializations {
                None => out.push(0),
                Some(specs) => {
                    out.push(1);
                    metadata::encode_materialization_specs(&mut out, specs)?;
                }
            }
            metadata::encode_semantic_module_specs(&mut out, semantic_modules)?;
            match &descriptor.change {
                DurableRevisionChange::FullRevision { .. } => out.push(1),
                DurableRevisionChange::FullRevisionAndMaterializations { .. } => out.push(2),
                DurableRevisionChange::RelationData { .. } => {
                    return Err(CodecError::CollectionTooLarge);
                }
            }
        }
        DurableTransactionIntent::SchemaMigrationExact {
            source_revision,
            target_revision,
            encoded_target_revision,
            migration_complement,
            semantic_modules,
        } => encode_schema_migration_prepare(
            &mut out,
            descriptor,
            *source_revision,
            *target_revision,
            encoded_target_revision,
            migration_complement,
            semantic_modules,
        )?,
        DurableTransactionIntent::LegacyTargetOnly { .. } => {
            return Err(CodecError::CollectionTooLarge);
        }
    }
    Ok(out)
}

fn decode_prepare_payload(
    target_revision: RevisionId,
    payload: &[u8],
) -> Result<DurableRevisionDescriptor, &'static str> {
    let mut cursor = Cursor::new(payload);
    let version = cursor.u16()?;
    let transaction_id = ClientTransactionId::new(cursor.u128()?);
    let (idempotency_epoch, revision_effect_id) = if version >= 9 {
        let epoch = IdempotencyEpoch::new(cursor.u64()?);
        let effect = match cursor.u8()? {
            0 => None,
            1 => Some(RevisionEffectId(cursor.u128()?)),
            _ => return Err("invalid revision effect identity tag"),
        };
        (epoch, effect)
    } else {
        (IdempotencyEpoch::ZERO, None)
    };
    let source_revision = RevisionId::new(cursor.u64()?);
    let (intent, change) = match version {
        2 => (
            DurableTransactionIntent::LegacyTargetOnly { target_revision },
            DurableRevisionChange::RelationData {
                semantic_revision: SemanticRevision::new(
                    kernel_types::SchemaRevisionId::new(cursor.u64()?),
                    kernel_types::SemanticEnvId::new(cursor.u64()?),
                ),
                relation_mutations: decode_relation_mutations(&mut cursor)?,
            },
        ),
        3 => {
            let change = match cursor.u8()? {
                0 => DurableRevisionChange::RelationData {
                    semantic_revision: SemanticRevision::new(
                        kernel_types::SchemaRevisionId::new(cursor.u64()?),
                        kernel_types::SemanticEnvId::new(cursor.u64()?),
                    ),
                    relation_mutations: decode_relation_mutations(&mut cursor)?,
                },
                1 => {
                    let len = cursor.len()?;
                    DurableRevisionChange::FullRevision {
                        encoded_target_revision: cursor.take(len)?.to_vec(),
                    }
                }
                _ => return Err("unknown durable revision change tag"),
            };
            let intent = match &change {
                DurableRevisionChange::FullRevision {
                    encoded_target_revision,
                } => DurableTransactionIntent::Exact {
                    target_revision,
                    encoded_target_revision: encoded_target_revision.clone(),
                    materializations: None,
                    semantic_modules: Vec::new(),
                },
                DurableRevisionChange::RelationData { .. }
                | DurableRevisionChange::FullRevisionAndMaterializations { .. } => {
                    DurableTransactionIntent::LegacyTargetOnly { target_revision }
                }
            };
            (intent, change)
        }
        4 => decode_v4_prepare_payload(&mut cursor, target_revision)?,
        5 | 6 | 7 | 8 | MUTATION_CODEC_VERSION => {
            decode_current_prepare_payload(&mut cursor, source_revision, target_revision)?
        }
        _ => return Err("unsupported mutation codec version"),
    };
    cursor.finish()?;
    Ok(DurableRevisionDescriptor {
        idempotency_epoch,
        revision_effect_id,
        transaction_id,
        source_revision,
        target_revision,
        intent,
        change,
    })
}

fn decode_v4_prepare_payload(
    cursor: &mut Cursor<'_>,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    if cursor.u8()? != 1 {
        return Err("new durable transaction intent is not exact");
    }
    let encoded_target_revision = {
        let len = cursor.len()?;
        cursor.take(len)?.to_vec()
    };
    let materializations = match cursor.u8()? {
        0 => None,
        1 => Some(metadata::decode_materialization_specs(cursor)?),
        _ => return Err("invalid durable transaction materialization tag"),
    };
    let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
    let intent = DurableTransactionIntent::Exact {
        target_revision,
        encoded_target_revision: encoded_target_revision.clone(),
        materializations: materializations.clone(),
        semantic_modules,
    };
    let change = match cursor.u8()? {
        0 => DurableRevisionChange::RelationData {
            semantic_revision: SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            ),
            relation_mutations: decode_relation_mutations(cursor)?,
        },
        1 => {
            if materializations.is_some() {
                return Err("full revision record unexpectedly carries materializations");
            }
            DurableRevisionChange::FullRevision {
                encoded_target_revision,
            }
        }
        2 => DurableRevisionChange::FullRevisionAndMaterializations {
            encoded_target_revision,
            materializations: materializations
                .ok_or("combined revision record is missing materialization registry")?,
        },
        _ => return Err("unknown durable revision change tag"),
    };
    Ok((intent, change))
}

fn decode_current_prepare_payload(
    cursor: &mut Cursor<'_>,
    source_revision: RevisionId,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    match cursor.u8()? {
        5 => decode_relation_resolution_prepare(cursor, source_revision, target_revision),
        4 => decode_schema_migration_prepare(cursor, source_revision, target_revision),
        3 => {
            let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
            let semantic_revision = SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            );
            let relation_mutations = decode_relation_mutations(cursor)?;
            let rewrite_intents = decode_relation_rewrite_intents(cursor)?;
            if relation_mutations.len() != rewrite_intents.len()
                || relation_mutations
                    .iter()
                    .zip(&rewrite_intents)
                    .any(|(mutation, intent)| mutation.relation != intent.relation)
            {
                return Err("relation rewrite intents do not match relation mutations");
            }
            let intent = DurableTransactionIntent::RelationRewriteExact {
                source_revision,
                target_revision,
                semantic_revision,
                relation_mutations: relation_mutations.clone(),
                rewrite_intents,
                semantic_modules,
            };
            let change = DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            };
            Ok((intent, change))
        }
        2 => {
            let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
            let semantic_revision = SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            );
            let relation_mutations = decode_relation_mutations(cursor)?;
            let intent = DurableTransactionIntent::RelationDataExact {
                source_revision,
                target_revision,
                semantic_revision,
                relation_mutations: relation_mutations.clone(),
                semantic_modules,
            };
            let change = DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            };
            Ok((intent, change))
        }
        1 => {
            let encoded_target_revision = {
                let len = cursor.len()?;
                cursor.take(len)?.to_vec()
            };
            let materializations = match cursor.u8()? {
                0 => None,
                1 => Some(metadata::decode_materialization_specs(cursor)?),
                _ => return Err("invalid durable transaction materialization tag"),
            };
            let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
            let intent = DurableTransactionIntent::Exact {
                target_revision,
                encoded_target_revision: encoded_target_revision.clone(),
                materializations: materializations.clone(),
                semantic_modules,
            };
            let change = match cursor.u8()? {
                1 => {
                    if materializations.is_some() {
                        return Err("full revision record unexpectedly carries materializations");
                    }
                    DurableRevisionChange::FullRevision {
                        encoded_target_revision,
                    }
                }
                2 => DurableRevisionChange::FullRevisionAndMaterializations {
                    encoded_target_revision,
                    materializations: materializations
                        .ok_or("combined revision record is missing materialization registry")?,
                },
                _ => return Err("unknown durable revision change tag"),
            };
            Ok((intent, change))
        }
        _ => Err("new durable transaction intent tag is invalid"),
    }
}

fn decode_relation_resolution_prepare(
    cursor: &mut Cursor<'_>,
    source_revision: RevisionId,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
    let semantic_revision = SemanticRevision::new(
        kernel_types::SchemaRevisionId::new(cursor.u64()?),
        kernel_types::SemanticEnvId::new(cursor.u64()?),
    );
    let relation_mutations = decode_relation_mutations(cursor)?;
    let rewrite_intents = decode_relation_rewrite_intents(cursor)?;
    if relation_mutations.len() != rewrite_intents.len()
        || relation_mutations
            .iter()
            .zip(&rewrite_intents)
            .any(|(mutation, intent)| mutation.relation != intent.relation)
    {
        return Err("relation resolution intents do not match relation mutations");
    }
    let count = cursor.len()?;
    if count < 2 {
        return Err("relation resolution must have at least two causal parents");
    }
    let mut causal_parents = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let parent = RevisionId::new(cursor.u64()?);
        if previous.is_some_and(|prior| prior >= parent) {
            return Err("relation resolution causal parents are not strictly sorted");
        }
        previous = Some(parent);
        causal_parents.push(parent);
    }
    if causal_parents.binary_search(&source_revision).is_err() {
        return Err("relation resolution causal parents omit source revision");
    }
    Ok((
        DurableTransactionIntent::RelationResolutionExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations: relation_mutations.clone(),
            rewrite_intents,
            causal_parents,
            semantic_modules,
        },
        DurableRevisionChange::RelationData {
            semantic_revision,
            relation_mutations,
        },
    ))
}

fn decode_schema_migration_prepare(
    cursor: &mut Cursor<'_>,
    source_revision: RevisionId,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    let encoded_target_revision = {
        let len = cursor.len()?;
        cursor.take(len)?.to_vec()
    };
    let mut complements = metadata::decode_migration_complements(cursor)?;
    if complements.len() != 1 {
        return Err("schema migration prepare must carry exactly one complement");
    }
    let migration_complement = complements.remove(0);
    let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
    let intent = DurableTransactionIntent::SchemaMigrationExact {
        source_revision,
        target_revision,
        encoded_target_revision: encoded_target_revision.clone(),
        migration_complement,
        semantic_modules,
    };
    Ok((
        intent,
        DurableRevisionChange::FullRevision {
            encoded_target_revision,
        },
    ))
}

fn decode_relation_rewrite_intents(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DurableRelationRewriteIntent>, &'static str> {
    let count = cursor.len()?;
    let mut rewrite_intents = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let relation = SemanticId::new(cursor.u128()?);
        if previous.is_some_and(|id: SemanticId| id >= relation) {
            return Err("relation rewrite intents are not strictly sorted and unique");
        }
        previous = Some(relation);
        rewrite_intents.push(DurableRelationRewriteIntent {
            relation,
            rewrite_spec: SemanticId::new(cursor.u128()?),
            law_set: SemanticId::new(cursor.u128()?),
        });
    }
    Ok(rewrite_intents)
}

fn decode_relation_mutations(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DurableRelationMutation>, &'static str> {
    let count = cursor.len()?;
    let mut relation_mutations = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let relation = SemanticId::new(cursor.u128()?);
        if previous.is_some_and(|id: SemanticId| id >= relation) {
            return Err("relation mutations are not strictly sorted and unique");
        }
        previous = Some(relation);
        let inserted = cursor.rows(0)?;
        let removed = cursor.rows(0)?;
        relation_mutations.push(DurableRelationMutation {
            relation,
            inserted,
            removed,
        });
    }
    Ok(relation_mutations)
}

fn encode_commit_payload(record: &CommitRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    push_u64(&mut out, record.prepare_lsn);
    push_u32(&mut out, record.prepare_payload_crc32c);
    out
}

fn decode_commit_payload(
    target_revision: RevisionId,
    payload: &[u8],
) -> Result<CommitRecord, &'static str> {
    if payload.len() != 12 {
        return Err("commit payload length mismatch");
    }
    Ok(CommitRecord {
        target_revision,
        prepare_lsn: read_u64(&payload[0..8]),
        prepare_payload_crc32c: read_u32(&payload[8..12]),
    })
}

fn encode_rows(out: &mut Vec<u8>, rows: &[Vec<Value>]) -> Result<(), CodecError> {
    push_len(out, rows.len())?;
    for row in rows {
        push_len(out, row.len())?;
        for value in row {
            encode_value(out, value, 0)?;
        }
    }
    Ok(())
}

fn encode_value(out: &mut Vec<u8>, value: &Value, depth: usize) -> Result<(), CodecError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(CodecError::ValueNestingTooDeep);
    }
    match value {
        Value::Unit => out.push(0),
        Value::Bool(value) => {
            out.push(1);
            out.push(u8::from(*value));
        }
        Value::I64(value) => {
            out.push(2);
            out.extend_from_slice(&value.to_le_bytes());
        }
        Value::F64Bits(value) => {
            out.push(3);
            push_u64(out, *value);
        }
        Value::Text(value) => {
            out.push(4);
            push_bytes(out, value.as_bytes())?;
        }
        Value::LiveEntityRef { entity_type, id } => {
            out.push(5);
            push_u128(out, entity_type.raw());
            push_u128(out, id.raw());
        }
        Value::HistoricalEntityId { entity_type, id } => {
            out.push(6);
            push_u128(out, entity_type.raw());
            push_u128(out, id.raw());
        }
        Value::Product(fields) => {
            out.push(7);
            push_len(out, fields.len())?;
            for (field, child) in fields {
                push_u128(out, field.raw());
                encode_value(out, child, depth + 1)?;
            }
        }
        Value::Option(value) => {
            out.push(8);
            if let Some(value) = value {
                out.push(1);
                encode_value(out, value, depth + 1)?;
            } else {
                out.push(0);
            }
        }
        Value::Variant { tag, value } => {
            out.push(9);
            push_u128(out, tag.raw());
            encode_value(out, value, depth + 1)?;
        }
        Value::Seq(values) => {
            out.push(10);
            push_len(out, values.len())?;
            for value in values {
                encode_value(out, value, depth + 1)?;
            }
        }
        Value::Set {
            equivalence,
            elements,
        } => {
            out.push(11);
            push_u128(out, equivalence.raw());
            push_len(out, elements.len())?;
            for value in elements {
                encode_value(out, value, depth + 1)?;
            }
        }
        Value::Bag {
            equivalence,
            entries,
        } => {
            out.push(12);
            push_u128(out, equivalence.raw());
            push_len(out, entries.len())?;
            for (value, count) in entries {
                encode_value(out, value, depth + 1)?;
                push_u64(out, *count);
            }
        }
        Value::Map {
            key_equivalence,
            entries,
        } => {
            out.push(13);
            push_u128(out, key_equivalence.raw());
            push_len(out, entries.len())?;
            for (key, value) in entries {
                encode_value(out, key, depth + 1)?;
                encode_value(out, value, depth + 1)?;
            }
        }
    }
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], &'static str> {
        let end = self
            .position
            .checked_add(len)
            .ok_or("codec offset overflow")?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or("truncated mutation payload")?;
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, &'static str> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, &'static str> {
        Ok(read_u16(self.take(2)?))
    }

    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(read_u32(self.take(4)?))
    }

    fn u64(&mut self) -> Result<u64, &'static str> {
        Ok(read_u64(self.take(8)?))
    }

    fn u128(&mut self) -> Result<u128, &'static str> {
        Ok(u128::from_le_bytes(
            self.take(16)?.try_into().map_err(|_| "u128 decode")?,
        ))
    }

    fn len(&mut self) -> Result<usize, &'static str> {
        let value = usize::try_from(self.u32()?).map_err(|_| "collection length overflow")?;
        if value > MAX_COLLECTION_LEN {
            return Err("collection length exceeds hard limit");
        }
        Ok(value)
    }

    fn string(&mut self) -> Result<String, &'static str> {
        let len = self.len()?;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| "invalid utf-8 string")
    }

    fn rows(&mut self, depth: usize) -> Result<Vec<Vec<Value>>, &'static str> {
        let count = self.len()?;
        let mut rows = Vec::with_capacity(count);
        for _ in 0..count {
            let columns = self.len()?;
            let mut row = Vec::with_capacity(columns);
            for _ in 0..columns {
                row.push(self.value(depth)?);
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn value(&mut self, depth: usize) -> Result<Value, &'static str> {
        if depth > MAX_VALUE_DEPTH {
            return Err("value nesting exceeds hard limit");
        }
        match self.u8()? {
            0 => Ok(Value::Unit),
            1 => match self.u8()? {
                0 => Ok(Value::Bool(false)),
                1 => Ok(Value::Bool(true)),
                _ => Err("invalid bool encoding"),
            },
            2 => Ok(Value::I64(i64::from_le_bytes(
                self.take(8)?.try_into().map_err(|_| "i64 decode")?,
            ))),
            3 => Ok(Value::F64Bits(self.u64()?)),
            4 => {
                let len = self.len()?;
                let bytes = self.take(len)?;
                let text = std::str::from_utf8(bytes).map_err(|_| "invalid utf-8 text")?;
                Ok(Value::Text(text.to_owned()))
            }
            5 => Ok(Value::LiveEntityRef {
                entity_type: SemanticId::new(self.u128()?),
                id: EntityId::new(self.u128()?),
            }),
            6 => Ok(Value::HistoricalEntityId {
                entity_type: SemanticId::new(self.u128()?),
                id: EntityId::new(self.u128()?),
            }),
            7 => {
                let count = self.len()?;
                let mut fields = BTreeMap::new();
                let mut previous = None;
                for _ in 0..count {
                    let field = SemanticId::new(self.u128()?);
                    if previous.is_some_and(|id: SemanticId| id >= field) {
                        return Err("product fields are not strictly sorted and unique");
                    }
                    previous = Some(field);
                    fields.insert(field, self.value(depth + 1)?);
                }
                Ok(Value::Product(fields))
            }
            8 => match self.u8()? {
                0 => Ok(Value::Option(None)),
                1 => Ok(Value::Option(Some(Box::new(self.value(depth + 1)?)))),
                _ => Err("invalid option discriminant"),
            },
            9 => Ok(Value::Variant {
                tag: SemanticId::new(self.u128()?),
                value: Box::new(self.value(depth + 1)?),
            }),
            10 => {
                let count = self.len()?;
                let mut values = Vec::with_capacity(count);
                for _ in 0..count {
                    values.push(self.value(depth + 1)?);
                }
                Ok(Value::Seq(values))
            }
            11 => {
                let equivalence = SemanticId::new(self.u128()?);
                let count = self.len()?;
                let mut elements = Vec::with_capacity(count);
                for _ in 0..count {
                    elements.push(self.value(depth + 1)?);
                }
                Ok(Value::Set {
                    equivalence,
                    elements,
                })
            }
            12 => {
                let equivalence = SemanticId::new(self.u128()?);
                let count = self.len()?;
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push((self.value(depth + 1)?, self.u64()?));
                }
                Ok(Value::Bag {
                    equivalence,
                    entries,
                })
            }
            13 => {
                let key_equivalence = SemanticId::new(self.u128()?);
                let count = self.len()?;
                let mut entries = Vec::with_capacity(count);
                for _ in 0..count {
                    entries.push((self.value(depth + 1)?, self.value(depth + 1)?));
                }
                Ok(Value::Map {
                    key_equivalence,
                    entries,
                })
            }
            _ => Err("unknown value tag"),
        }
    }

    fn finish(self) -> Result<(), &'static str> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err("trailing bytes in mutation payload")
        }
    }
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), CodecError> {
    push_len(out, bytes.len())?;
    out.extend_from_slice(bytes);
    Ok(())
}

fn push_len(out: &mut Vec<u8>, value: usize) -> Result<(), CodecError> {
    if value > MAX_COLLECTION_LEN {
        return Err(CodecError::CollectionTooLarge);
    }
    push_u32(
        out,
        u32::try_from(value).map_err(|_| CodecError::LengthOverflow)?,
    );
    Ok(())
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u128(out: &mut Vec<u8>, value: u128) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes.try_into().expect("exact u16 slice"))
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("exact u32 slice"))
}

fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("exact u64 slice"))
}

#[must_use]
pub fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0x82F6_3B78 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_model::DatabaseState;
    use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
    use kernel_semantics::SemanticRegistry;
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    fn descriptor(source: u64, target: u64, value: Value) -> DurableRevisionDescriptor {
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
                inserted: vec![vec![value]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap()
    }

    #[test]
    fn crc32c_known_vector() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn value_codec_roundtrips_all_shapes() {
        let values = vec![
            Value::Unit,
            Value::Bool(true),
            Value::I64(-7),
            Value::F64Bits(f64::NAN.to_bits()),
            Value::Text("Aßz".into()),
            Value::LiveEntityRef {
                entity_type: SemanticId::new(2),
                id: EntityId::new(3),
            },
            Value::HistoricalEntityId {
                entity_type: SemanticId::new(4),
                id: EntityId::new(5),
            },
            Value::Product(BTreeMap::from([(
                SemanticId::new(6),
                Value::Option(Some(Box::new(Value::I64(8)))),
            )])),
            Value::Variant {
                tag: SemanticId::new(9),
                value: Box::new(Value::Seq(vec![Value::Bool(false)])),
            },
            Value::Set {
                equivalence: SemanticId::new(10),
                elements: vec![Value::Text("x".into())],
            },
            Value::Bag {
                equivalence: SemanticId::new(12),
                entries: vec![(Value::I64(4), 3)],
            },
            Value::Map {
                key_equivalence: SemanticId::new(13),
                entries: vec![(Value::Text("k".into()), Value::I64(1))],
            },
        ];
        for value in values {
            let descriptor = descriptor(1, 2, value);
            let payload = encode_prepare_payload(&descriptor).unwrap();
            assert_eq!(
                decode_prepare_payload(RevisionId::new(2), &payload).unwrap(),
                descriptor
            );
        }
    }

    #[test]
    fn relation_data_intent_and_prepare_scale_with_delta_not_target_snapshot() {
        let registry = SemanticRegistry::default();
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(70)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(80)),
        };
        let mut state = DatabaseState::default();
        for raw in 1..=5_000_u128 {
            let entity = EntityId::new(raw);
            state.lifecycle.entities.insert(entity);
            state.lifecycle.roots.insert(entity);
        }
        let target =
            kernel_revision::Revision::build(RevisionId::new(2), &context, &registry, state)
                .unwrap();
        let descriptor = DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(0x700),
            RevisionId::new(1),
            &target,
            target.semantic_revision(),
            vec![DurableRelationMutation {
                relation: SemanticId::new(11),
                inserted: vec![vec![Value::I64(7)]],
                removed: Vec::new(),
            }],
            &registry,
        )
        .unwrap();
        let full_revision = checkpoint::encode_revision(&target).unwrap();
        let prepare = encode_prepare_payload(&descriptor).unwrap();
        let metadata_bytes = metadata::encode(&metadata::DurableStoreMetadata {
            external_freshness: None,
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            materializations: Vec::new(),
            physical_artifacts: Vec::new(),
            artifact_cores: Vec::new(),
            migration_complements: Vec::new(),
            committed_transactions: BTreeMap::from([(
                DurableTransactionKey::new(IdempotencyEpoch::ZERO, descriptor.transaction_id),
                descriptor.intent.clone(),
            )]),
            semantic_modules: Vec::new(),
            causal_coverage_root: None,
            revision_effects: BTreeMap::new(),
            revision_effect_frontiers: BTreeMap::new(),
        })
        .unwrap();

        assert!(matches!(
            descriptor.intent,
            DurableTransactionIntent::RelationDataExact { .. }
        ));
        assert!(
            prepare.len() * 100 < full_revision.len(),
            "delta prepare={} full revision={}",
            prepare.len(),
            full_revision.len()
        );
        assert!(
            metadata_bytes.len() * 100 < full_revision.len(),
            "delta ledger={} full revision={}",
            metadata_bytes.len(),
            full_revision.len()
        );
    }

    #[test]
    fn relation_rewrite_prepare_roundtrips_and_v5_relation_data_remains_readable() {
        let registry = SemanticRegistry::default();
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(71)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(81)),
        };
        let target = kernel_revision::Revision::build(
            RevisionId::new(2),
            &context,
            &registry,
            DatabaseState::default(),
        )
        .unwrap();
        let mutation = DurableRelationMutation {
            relation: SemanticId::new(11),
            inserted: vec![vec![Value::I64(7)]],
            removed: Vec::new(),
        };
        let rewrite = DurableRevisionDescriptor::relation_rewrites(
            ClientTransactionId::new(0x701),
            RevisionId::new(1),
            &target,
            target.semantic_revision(),
            vec![mutation.clone()],
            vec![DurableRelationRewriteIntent {
                relation: mutation.relation,
                rewrite_spec: SemanticId::new(0xAA),
                law_set: SemanticId::new(0xBB),
            }],
            &registry,
        )
        .unwrap();
        let payload = encode_prepare_payload(&rewrite).unwrap();
        assert_eq!(
            decode_prepare_payload(target.id(), &payload).unwrap(),
            rewrite
        );

        let resolution = DurableRevisionDescriptor::relation_resolution(
            ClientTransactionId::new(0x703),
            RevisionId::new(1),
            &target,
            target.semantic_revision(),
            DurableRelationResolution {
                relation_mutations: vec![DurableRelationMutation {
                    relation: SemanticId::new(11),
                    inserted: vec![vec![Value::I64(8)]],
                    removed: Vec::new(),
                }],
                rewrite_intents: vec![DurableRelationRewriteIntent {
                    relation: SemanticId::new(11),
                    rewrite_spec: SemanticId::new(0xCC),
                    law_set: SemanticId::new(0xDD),
                }],
                causal_parents: vec![RevisionId::new(1), RevisionId::new(7)],
            },
            &registry,
        )
        .unwrap();
        let payload = encode_prepare_payload(&resolution).unwrap();
        assert_eq!(
            decode_prepare_payload(target.id(), &payload).unwrap(),
            resolution
        );

        let relation_data = DurableRevisionDescriptor::relation_data(
            ClientTransactionId::new(0x702),
            RevisionId::new(1),
            &target,
            target.semantic_revision(),
            vec![mutation],
            &registry,
        )
        .unwrap();
        let mut legacy_v5 = encode_prepare_payload(&relation_data).unwrap();
        legacy_v5[..2].copy_from_slice(&5_u16.to_le_bytes());
        assert_eq!(
            decode_prepare_payload(target.id(), &legacy_v5).unwrap(),
            relation_data
        );
    }

    #[test]
    fn full_revision_prepare_payload_roundtrips_and_revalidates_target() {
        let registry = SemanticRegistry::default();
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(50)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(60)),
        };
        let target = kernel_revision::Revision::build(
            RevisionId::new(2),
            &context,
            &registry,
            DatabaseState::default(),
        )
        .unwrap();
        let descriptor = DurableRevisionDescriptor::full_revision(
            ClientTransactionId::new(900),
            RevisionId::new(1),
            &target,
            &registry,
        )
        .unwrap();
        let payload = encode_prepare_payload(&descriptor).unwrap();
        let decoded = decode_prepare_payload(RevisionId::new(2), &payload).unwrap();
        assert_eq!(decoded, descriptor);
        assert_eq!(
            decoded.decode_full_revision(&registry).unwrap(),
            Some(target)
        );
    }

    #[test]
    fn mutation_codec_v2_relation_data_remains_readable() {
        let expected = descriptor(7, 8, Value::I64(9));
        let DurableRevisionChange::RelationData {
            semantic_revision,
            relation_mutations,
        } = &expected.change
        else {
            unreachable!();
        };
        let mut payload = Vec::new();
        push_u16(&mut payload, 2);
        push_u128(&mut payload, expected.transaction_id.raw());
        push_u64(&mut payload, expected.source_revision.raw());
        push_u64(&mut payload, semantic_revision.schema.raw());
        push_u64(&mut payload, semantic_revision.environment.raw());
        push_len(&mut payload, relation_mutations.len()).unwrap();
        for mutation in relation_mutations {
            push_u128(&mut payload, mutation.relation.raw());
            encode_rows(&mut payload, &mutation.inserted).unwrap();
            encode_rows(&mut payload, &mutation.removed).unwrap();
        }
        let decoded = decode_prepare_payload(expected.target_revision, &payload).unwrap();
        assert_eq!(decoded.transaction_id, expected.transaction_id);
        assert_eq!(decoded.source_revision, expected.source_revision);
        assert_eq!(decoded.target_revision, expected.target_revision);
        assert_eq!(decoded.change, expected.change);
        assert_eq!(
            decoded.intent,
            DurableTransactionIntent::LegacyTargetOnly {
                target_revision: expected.target_revision,
            }
        );
    }

    #[test]
    fn every_prefix_exposes_only_durable_commits() {
        let mut wal = SimulatedRevisionWal::new();
        let d1 = descriptor(10, 20, Value::I64(1));
        let p1 = wal.durably_prepare(&d1).unwrap();
        wal.durably_commit(p1).unwrap();
        let first_end = wal.crash_image().len();
        let d2 = descriptor(20, 30, Value::I64(2));
        let p2 = wal.durably_prepare(&d2).unwrap();
        wal.durably_commit(p2).unwrap();
        let second_end = wal.crash_image().len();
        let bytes = wal.crash_image().to_vec();
        for cut in 0..=bytes.len() {
            let scan = scan_wal(&bytes[..cut], RevisionId::new(10)).unwrap();
            let expected = if cut >= second_end {
                RevisionId::new(30)
            } else if cut >= first_end {
                RevisionId::new(20)
            } else {
                RevisionId::new(10)
            };
            assert_eq!(scan.durable_revision(), expected, "cut={cut}");
        }
    }

    #[test]
    fn durable_prepare_without_commit_is_not_visible() {
        let mut wal = SimulatedRevisionWal::new();
        wal.durably_prepare(&descriptor(1, 2, Value::I64(4)))
            .unwrap();
        let scan = scan_wal(wal.crash_image(), RevisionId::new(1)).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(1));
    }

    #[test]
    fn every_single_bit_corruption_of_committed_stream_is_rejected() {
        let mut wal = SimulatedRevisionWal::new();
        let prepared = wal
            .durably_prepare(&descriptor(1, 2, Value::Text("payload".into())))
            .unwrap();
        wal.durably_commit(prepared).unwrap();
        let bytes = wal.crash_image();
        for byte in 0..bytes.len() {
            for bit in 0..8 {
                let mut damaged = bytes.to_vec();
                damaged[byte] ^= 1_u8 << bit;
                assert!(
                    scan_wal(&damaged, RevisionId::new(1)).is_err(),
                    "accepted byte={byte} bit={bit}"
                );
            }
        }
    }

    #[test]
    fn scanner_rejects_commit_whose_source_is_not_durable_head() {
        let mut wal = SimulatedRevisionWal::new();
        let prepared = wal
            .durably_prepare(&descriptor(99, 100, Value::I64(1)))
            .unwrap();
        wal.durably_commit(prepared).unwrap();
        assert!(matches!(
            scan_wal(wal.crash_image(), RevisionId::new(1)),
            Err(DurabilityError::Protocol { .. })
        ));
    }

    #[test]
    fn duplicate_identical_prepare_and_commit_are_idempotent() {
        let mut wal = SimulatedRevisionWal::new();
        let descriptor = descriptor(1, 2, Value::I64(7));
        let first = wal.durably_prepare(&descriptor).unwrap();
        let _duplicate_prepare = wal.durably_prepare(&descriptor).unwrap();
        wal.durably_commit(first).unwrap();
        wal.durably_commit(first).unwrap();

        let scan = scan_wal(wal.crash_image(), RevisionId::new(1)).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(2));
        assert_eq!(scan.committed().len(), 1);
    }

    #[test]
    fn scanner_rejects_same_target_transaction_reuse_with_different_delta() {
        let mut wal = SimulatedRevisionWal::new();
        let first = descriptor(1, 2, Value::I64(7));
        let mut second = descriptor(1, 2, Value::I64(8));
        second.transaction_id = first.transaction_id;
        let prepared = wal.durably_prepare(&first).unwrap();
        wal.durably_commit(prepared).unwrap();
        wal.durably_prepare(&second).unwrap();

        assert!(matches!(
            scan_wal(wal.crash_image(), RevisionId::new(1)),
            Err(DurabilityError::Protocol {
                reason: "transaction id already committed to another exact intent",
                ..
            })
        ));
    }

    #[test]
    fn scanner_rejects_transaction_id_reuse_for_different_prepare() {
        let mut wal = SimulatedRevisionWal::new();
        let first = descriptor(1, 2, Value::I64(7));
        let mut second = descriptor(1, 3, Value::I64(8));
        second.transaction_id = first.transaction_id;
        wal.durably_prepare(&first).unwrap();
        wal.durably_prepare(&second).unwrap();

        assert!(matches!(
            scan_wal(wal.crash_image(), RevisionId::new(1)),
            Err(DurabilityError::Protocol {
                reason: "transaction id reused by conflicting prepare",
                ..
            })
        ));
    }

    #[test]
    fn scanner_rejects_conflicting_duplicate_prepare() {
        let mut wal = SimulatedRevisionWal::new();
        wal.durably_prepare(&descriptor(1, 2, Value::I64(7)))
            .unwrap();
        wal.durably_prepare(&descriptor(1, 2, Value::I64(8)))
            .unwrap();

        assert!(matches!(
            scan_wal(wal.crash_image(), RevisionId::new(1)),
            Err(DurabilityError::Protocol {
                reason: "conflicting duplicate prepare",
                ..
            })
        ));
    }

    #[test]
    fn scanner_rejects_conflicting_duplicate_commit() {
        let mut wal = SimulatedRevisionWal::new();
        let descriptor = descriptor(1, 2, Value::I64(7));
        let first = wal.durably_prepare(&descriptor).unwrap();
        let second = wal.durably_prepare(&descriptor).unwrap();
        wal.durably_commit(first).unwrap();
        wal.durably_commit(second).unwrap();

        assert!(matches!(
            scan_wal(wal.crash_image(), RevisionId::new(1)),
            Err(DurabilityError::Protocol {
                reason: "conflicting duplicate commit",
                ..
            })
        ));
    }

    #[test]
    fn scanner_distinguishes_short_garbage_tail_from_hidden_full_frame() {
        let mut wal = SimulatedRevisionWal::new();
        let prepared = wal
            .durably_prepare(&descriptor(1, 2, Value::I64(7)))
            .unwrap();
        wal.durably_commit(prepared).unwrap();

        let mut short_garbage = wal.crash_image().to_vec();
        short_garbage.extend_from_slice(b"junk");
        let scan = scan_wal(&short_garbage, RevisionId::new(1)).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(2));
        assert!(matches!(scan.tail_status(), TailStatus::Garbage { .. }));

        let mut full_garbage = wal.crash_image().to_vec();
        full_garbage.extend(std::iter::repeat_n(0xA5, HEADER_LEN));
        assert!(matches!(
            scan_wal(&full_garbage, RevisionId::new(1)),
            Err(DurabilityError::Corruption {
                reason: "non-frame bytes large enough to hide a complete frame",
                ..
            })
        ));
    }

    #[test]
    fn scanner_rejects_non_monotone_lsn_even_with_valid_checksums() {
        let descriptor = descriptor(1, 2, Value::I64(7));
        let payload = encode_prepare_payload(&descriptor).unwrap();
        let frame = encode_frame(
            2,
            RecordKind::PrepareRevision,
            descriptor.target_revision,
            &payload,
        )
        .unwrap();

        assert!(matches!(
            scan_wal(&frame.bytes, RevisionId::new(1)),
            Err(DurabilityError::Protocol {
                reason: "non-monotone or gapped LSN",
                ..
            })
        ));
    }

    #[test]
    fn create_never_truncates_an_existing_wal() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cfmd-pass32-create-{unique}.wal"));
        std::fs::write(&path, b"existing wal bytes").unwrap();

        assert!(FileRevisionWal::create(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing wal bytes");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn file_open_recovered_truncates_safe_torn_tail_and_continues_lsn() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("cfmd-pass32-{unique}.wal"));
        let mut wal = FileRevisionWal::create(&path).unwrap();
        let prepared = wal
            .durably_prepare(&descriptor(1, 2, Value::I64(5)))
            .unwrap();
        wal.durably_commit(prepared).unwrap();
        drop(wal);
        {
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(&MAGIC[..2]).unwrap();
            file.sync_all().unwrap();
        }
        let (mut reopened, scan) =
            FileRevisionWal::open_recovered(&path, RevisionId::new(1)).unwrap();
        assert_eq!(scan.durable_revision(), RevisionId::new(2));
        let prepared = reopened
            .durably_prepare(&descriptor(2, 7, Value::I64(6)))
            .unwrap();
        let receipt = reopened.durably_commit(prepared).unwrap();
        assert_eq!(receipt.commit_lsn(), 4);
        drop(reopened);
        let (_, final_scan) = FileRevisionWal::open_recovered(&path, RevisionId::new(1)).unwrap();
        assert_eq!(final_scan.durable_revision(), RevisionId::new(7));
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod historical_lens_registry_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn step(
        source: u64,
        target: u64,
        lens: u128,
        manifest: u128,
        complement: Value,
    ) -> DurableMigrationComplement {
        DurableMigrationComplement::from_capsule(
            ComplementCapsule {
                source_schema: kernel_types::SchemaRevisionId::new(source),
                target_schema: kernel_types::SchemaRevisionId::new(target),
                lens_spec: LensSpecId(SemanticId::new(lens)),
                semantic_pins: SemanticManifestId(SemanticId::new(manifest)),
                encoding_version: 1,
                complement,
            },
            ComplementRetention::Forever,
        )
    }

    #[test]
    fn historical_value_restore_runs_exact_lens_chain_in_reverse() {
        let outer = SemanticId::new(81_001);
        let inner = SemanticId::new(81_002);
        let mut chain = LocalHistoricalComplementChain::default();
        chain.push(step(
            1,
            2,
            81_101,
            81_201,
            Value::Product(BTreeMap::from([(outer, Value::I64(10))])),
        ));
        chain.push(step(
            2,
            3,
            81_102,
            81_202,
            Value::Product(BTreeMap::from([(inner, Value::I64(20))])),
        ));
        let mut registry = HistoricalLensRegistry::default();
        registry
            .register(
                HistoricalLensImplementationKey {
                    lens_spec: LensSpecId(SemanticId::new(81_101)),
                    semantic_pins: SemanticManifestId(SemanticId::new(81_201)),
                    encoding_version: 1,
                },
                HistoricalLensImplementation::ProductField {
                    field: SemanticId::new(81_011),
                },
            )
            .unwrap();
        registry
            .register(
                HistoricalLensImplementationKey {
                    lens_spec: LensSpecId(SemanticId::new(81_102)),
                    semantic_pins: SemanticManifestId(SemanticId::new(81_202)),
                    encoding_version: 1,
                },
                HistoricalLensImplementation::ProductField {
                    field: SemanticId::new(81_012),
                },
            )
            .unwrap();

        let restored = chain.restore_value(&Value::I64(99), &registry).unwrap();
        let Value::Product(first) = restored else {
            panic!("first migration must reconstruct product")
        };
        assert_eq!(first[&outer], Value::I64(10));
        let Value::Product(second) = &first[&SemanticId::new(81_011)] else {
            panic!("second migration must reconstruct nested product")
        };
        assert_eq!(second[&inner], Value::I64(20));
        assert_eq!(second[&SemanticId::new(81_012)], Value::I64(99));
    }

    #[test]
    fn historical_restore_requires_exact_manifest_and_encoding_binding() {
        let mut chain = LocalHistoricalComplementChain::default();
        chain.push(step(1, 2, 81_301, 81_401, Value::Unit));
        let mut registry = HistoricalLensRegistry::default();
        registry
            .register(
                HistoricalLensImplementationKey {
                    lens_spec: LensSpecId(SemanticId::new(81_301)),
                    semantic_pins: SemanticManifestId(SemanticId::new(81_999)),
                    encoding_version: 1,
                },
                HistoricalLensImplementation::Identity,
            )
            .unwrap();
        assert!(matches!(
            chain.restore_value(&Value::I64(1), &registry),
            Err(HistoricalRestoreError::MissingImplementation(_))
        ));
    }
}
