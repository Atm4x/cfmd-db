mod binary_codec;
mod checkpoint;
mod descriptor;
mod domain;
mod durable_format;
mod freshness_tcp;
mod metadata;
mod platform_assurance;
mod realization;
mod replication;
mod replication_transport;
mod runtime;
mod single_file;
mod storage_encryption;
mod store;
mod wal;
mod wal_frame;
mod wal_payload;

pub use binary_codec::crc32c;
pub use descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    DurableRelationLayoutKind, DurableRevisionDescriptor, DurableSemanticKeyPart,
    PHYSICAL_ARTIFACT_RECIPE_TAG,
};
pub use domain::{
    ClientIntentGuardDigest, DurableCarrierPatch, DurableCausalObservationCoordinate,
    DurableCausalObservationGroup, DurableClientIntent, DurableCommittedTransaction,
    DurableEffectCoordinationClass, DurableEffectKind, DurableExternalFreshnessBinding,
    DurableFieldPatch, DurableIntentPrefix, DurableIntentPrefixNode, DurableKeepsAlivePatch,
    DurableMigrationComplement, DurableModelDelta, DurableObjectFieldWrite, DurableObservedScalar,
    DurableRelationAuthorization, DurableRelationMutation, DurableRelationResolution,
    DurableRelationRewriteIntent, DurableRelationalCausalObservation, DurableRevisionChange,
    DurableRevisionEffectRecord, DurableTransactionIntent, DurableTransactionKey,
    HistoricalBoundaryAuthority, HistoricalComplementError, HistoricalEpochAnchor,
    HistoricalLensImplementation, HistoricalLensImplementationKey, HistoricalLensRegistry,
    HistoricalRestoreError, IdempotencyEpoch, LocalHistoricalComplementChain,
    MigrationPhysicalAuthority, SchemaMigrationPhysicalState, SemanticChangeEvent,
};
pub use durable_format::{
    DurableFormatCompatibility, DurableFormatSupport, FORMAT_COMPATIBILITY,
    FORMAT_DOWNGRADE_TARGETS, FORMAT_UPGRADE_SOURCES, FORMAT_VERSION, durable_format_support,
};
pub use freshness_tcp::{TcpExternalFreshnessAuthority, TcpExternalFreshnessAuthorityServer};
pub use wal_frame::{HEADER_LEN, MAGIC, MAX_PAYLOAD_LEN};
pub use wal_payload::MUTATION_FORMAT_TAG;

/// Canonical structural identity bytes for a relational expression.
/// This is reconstructible metadata, not a persisted capsule identifier.
pub fn canonical_rel_expr_identity(
    expr: &kernel_query::RelExpr,
) -> Result<Vec<u8>, runtime::CodecError> {
    let mut out = Vec::new();
    metadata::query_codec::encode_rel_expr(&mut out, expr, 0)?;
    Ok(out)
}

/// Canonical durable identity bytes for one schema migration program.
///
/// This is the same codec used by durable revision metadata. Callers may hash
/// these bytes together with the source revision when they need a sealed
/// approval/certification identity; no separate migration serialization is
/// permitted to define security meaning.
pub fn canonical_schema_migration_program_identity(
    program: &kernel_transport::SchemaMigrationProgram,
) -> Result<Vec<u8>, runtime::CodecError> {
    let mut out = Vec::new();
    metadata::encode_schema_migration_program(&mut out, program)?;
    Ok(out)
}

/// Canonical durable identity bytes for one exact relation-effect prefix.
/// Used only for reconstructible in-memory interning; this is not a public semantic id.
pub fn canonical_relation_mutations_identity(
    mutations: &[DurableRelationMutation],
) -> Result<Vec<u8>, runtime::CodecError> {
    metadata::canonical_relation_mutations_identity(mutations)
}

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

pub use realization::{
    DurableFactorizedReadSnapshot, DurableFactorizedRealization, DurableHistoricalRealizationRoot,
    decode_factorized_realization, encode_factorized_realization,
};
pub use replication_transport::{
    MAX_ANTI_ENTROPY_LOCKS, ReplicationAntiEntropyChunk, ReplicationAntiEntropyRelation,
    ReplicationAntiEntropyRequest, ReplicationAntiEntropySummary, ReplicationFailureDetector,
    ReplicationHeartbeat, ReplicationTransportEgress, ReplicationTransportFrame,
    ReplicationTransportIngress, ReplicationTransportPayload, SignedReplicationTransportFrame,
    compare_replication_anti_entropy, decode_signed_replication_transport_frame,
    encode_signed_replication_transport_frame, replication_anti_entropy_summary,
    replication_lock_frontier_digest, replication_transport_signing_message,
    validate_replication_anti_entropy_chunk,
};

pub use runtime::{
    CodecError, CommittedRevision, DurabilityError, DurableCommitReceipt, DurableFormatComponent,
    DurablePrepareToken, DurableTransactionOutcome, RecoveryScan, RevisionDurability, TailStatus,
};
pub use single_file::{
    SingleFileContainer, SingleFileGenerationView, SingleFileSectionDescriptor,
    SingleFileSectionInput, SingleFileSectionKind,
};
pub use storage_encryption::{
    PersistentSecretStateProtection, SecretStateInitializationProtection, StorageAeadAlgorithm,
    StorageAeadBackendCapabilities, StorageAeadCodec, StorageEncryption, StorageEncryptionDomain,
    StorageEncryptionKey, StorageEncryptionKeyInitError,
};
pub use store::{
    CanonicalPersistenceImage, DurableBatchEnqueueOutcome, DurableCommitBatchPolicy,
    DurableCommitBatcher, DurableGenerationReceipt, DurableRevisionStore,
    DurableSatisfiedIntentSealOutcome, ExternalFreshnessAuthority, ExternalFreshnessConfig,
    ExternalFreshnessHandoff, ForkPersistenceImage, HistoricalEpochMaterial, PreparedCutCapsule,
    StreamingCheckpointProgress,
};
pub use wal::{FileRevisionWal, SimulatedRevisionWal, scan_wal};

#[cfg(test)]
mod tests;
