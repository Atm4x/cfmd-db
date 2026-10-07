use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::PathBuf;

use kernel_auth::{KeyId, Sha256Digest, sha256};
use kernel_change::RevisionEffectId;
use kernel_types::RevisionId;
use sha2::Sha256;

use super::codec::encode_membership_vote;
use super::{
    DurableSequencerOrder, ReplicaId, ReplicatedEffectEnvelope, ReplicationAuthenticationReceipt,
    ReplicationBranchHead, ReplicationBranchId, ReplicationDecisionLock, ReplicationDecisionVote,
    ReplicationJointMembershipAck, ReplicationJointMembershipCertificate,
    ReplicationLeaderCertificate, ReplicationLockSummary, ReplicationMembership,
    ReplicationMembershipVote, ReplicationPeerAuthPolicy, ReplicationQuorumAvailability,
    ReplicationQuorumCertificate, replication_membership_digest,
};
use crate::runtime::DurabilityError;

fn corruption(offset: usize, reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset, reason }
}

fn membership_vote_index(
    vote: &ReplicationMembershipVote,
) -> Result<MembershipVoteIndex, DurabilityError> {
    let successor_digest = replication_membership_digest(&vote.next)?;
    let payload_digest = sha256(&encode_membership_vote(vote)?);
    Ok(MembershipVoteIndex {
        term: vote.term,
        successor_digest,
        payload_digest,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuthenticatedRecoveryAckIndex {
    leader: ReplicaId,
    trust_epoch: u64,
    payload_digest: Sha256Digest,
    lock_frontier_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MembershipVoteIndex {
    term: u64,
    successor_digest: Sha256Digest,
    payload_digest: Sha256Digest,
}

#[derive(Debug)]
struct SemanticAuthorityBaseReplay {
    expected_records: usize,
    record_count: usize,
    canonical_len: u64,
    hasher: Sha256,
    decoder: semantic_snapshot::SemanticAuthorityRecordDecoder,
}

#[derive(Debug)]
pub(crate) struct ReplicationAuthorityJournal {
    path: PathBuf,
    file: Option<File>,
    single_file_capture: bool,
    pending_single_file_frames: Vec<Vec<u8>>,
    live_single_file_frames: Vec<Vec<u8>>,
    semantic_base_replay: Option<SemanticAuthorityBaseReplay>,
    effects: BTreeMap<RevisionEffectId, ReplicatedEffectEnvelope>,
    branches: BTreeMap<ReplicationBranchId, ReplicationBranchHead>,
    revision_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
    ordered_slots: BTreeMap<DurableSequencerOrder, RevisionEffectId>,
    sequencer_epochs: BTreeMap<ReplicaId, u64>,
    memberships: BTreeMap<u64, ReplicationMembership>,
    current_membership_epoch: Option<u64>,
    quorum_certificates: BTreeMap<RevisionEffectId, ReplicationQuorumCertificate>,
    effect_votes: BTreeMap<(u64, u64, ReplicaId), RevisionEffectId>,
    membership_votes: BTreeMap<(u64, ReplicaId), MembershipVoteIndex>,
    membership_vote_successors: BTreeMap<Sha256Digest, ReplicationMembership>,
    promised_terms: BTreeMap<(u64, ReplicaId), u64>,
    highest_promised_terms: BTreeMap<u64, u64>,
    leader_votes: BTreeMap<(u64, u64, ReplicaId), ReplicaId>,
    leader_certificates: BTreeMap<(u64, u64), ReplicationLeaderCertificate>,
    decision_votes: BTreeMap<(u64, u64, u64, ReplicaId), ReplicationDecisionVote>,
    decision_locks: BTreeMap<u64, ReplicationDecisionLock>,
    highest_decision_lock_term: u64,
    joint_membership_certificates: BTreeMap<u64, ReplicationJointMembershipCertificate>,
    peer_auth_policy: Option<ReplicationPeerAuthPolicy>,
    authenticated_evidence: BTreeMap<Sha256Digest, ReplicationAuthenticationReceipt>,
    authenticated_evidence_index: BTreeMap<(u64, ReplicaId, Sha256Digest), KeyId>,
    joint_membership_acks: BTreeMap<(u64, u64, ReplicaId), ReplicationJointMembershipAck>,
    recovery_acks: BTreeMap<(u64, u64, ReplicaId), AuthenticatedRecoveryAckIndex>,
    recovery_lock_frontiers: BTreeMap<Sha256Digest, Vec<ReplicationLockSummary>>,
    quorum_availability: Option<ReplicationQuorumAvailability>,
    published_effects: BTreeSet<RevisionEffectId>,
    published_branches: BTreeMap<ReplicationBranchId, ReplicationBranchHead>,
    poisoned: bool,
}

mod authentication;
mod availability;
mod decision;
mod effects;
mod election;
mod journal_io;
mod lifecycle;
mod membership;
mod replay;
#[cfg_attr(not(test), allow(dead_code))]
mod segments;
mod semantic_snapshot;
pub(crate) use semantic_snapshot::ReplicationAuthoritySemanticSnapshot;

#[cfg(test)]
pub(crate) use segments::collect_indexed_segment_object_chain_frames;
pub(crate) use segments::{
    ReplicationAuthorityFrameSlice, ReplicationAuthorityFrameSource,
    ReplicationAuthorityLocatorRoot, ReplicationAuthoritySegmentExtent,
    ReplicationAuthoritySegmentId, ReplicationAuthoritySegmentPlan, locator_stored_len,
    recover_locator_chain, replay_indexed_segment_object_chain, write_locator_node,
    write_segment_object,
};
