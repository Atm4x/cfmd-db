mod binary_codec;
mod checkpoint;
mod descriptor;
mod domain;
mod freshness_tcp;
mod metadata;
mod platform_assurance;
mod replication;
mod replication_transport;
mod runtime;
mod store;
mod wal;
mod wal_frame;
mod wal_payload;

pub use binary_codec::crc32c;
pub use descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
    DurableRelationLayoutKind, DurableRevisionDescriptor, DurableSemanticKeyPart,
    PHYSICAL_ARTIFACT_RECIPE_VERSION,
};
pub use domain::{
    DurableEffectCoordinationClass, DurableEffectKind, DurableExternalFreshnessBinding,
    DurableMigrationComplement, DurableRelationMutation, DurableRelationResolution,
    DurableRelationRewriteIntent, DurableRevisionChange, DurableRevisionEffectRecord,
    DurableTransactionIntent, DurableTransactionKey, HistoricalComplementError,
    HistoricalLensImplementation, HistoricalLensImplementationKey, HistoricalLensRegistry,
    HistoricalRestoreError, IdempotencyEpoch, LocalHistoricalComplementChain,
};
pub use freshness_tcp::{TcpExternalFreshnessAuthority, TcpExternalFreshnessAuthorityServer};
pub use wal_frame::{FORMAT_VERSION, HEADER_LEN, MAGIC, MAX_PAYLOAD_LEN};
pub use wal_payload::MUTATION_CODEC_VERSION;

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
pub use store::{
    DurableBatchEnqueueOutcome, DurableCommitBatchPolicy, DurableCommitBatcher,
    DurableGenerationReceipt, DurableRevisionStore, ExternalFreshnessAuthority,
    ExternalFreshnessConfig, PreparedCutCapsule, StreamingCheckpointProgress,
};
pub use wal::{FileRevisionWal, SimulatedRevisionWal, scan_wal};

#[cfg(test)]
mod tests;
