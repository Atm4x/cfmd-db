pub(super) const REPLICATION_MAGIC: [u8; 4] = *b"CFRP";
pub(super) const REPLICATION_VERSION: u16 = 2;
pub(super) const FRAME_HEADER_LEN: usize = 16;
pub(super) const KIND_INGEST: u8 = 1;
pub(super) const KIND_RETIRE: u8 = 2;
pub(super) const KIND_MEMBERSHIP: u8 = 3;
pub(super) const KIND_QUORUM: u8 = 4;
pub(super) const KIND_PUBLISH: u8 = 5;
pub(super) const KIND_EFFECT_VOTE: u8 = 6;
pub(super) const KIND_MEMBERSHIP_VOTE: u8 = 7;
pub(super) const KIND_TERM_PROMISE: u8 = 8;
pub(super) const KIND_LEADER_VOTE: u8 = 9;
pub(super) const KIND_LEADER_CERTIFICATE: u8 = 10;
pub(super) const KIND_DECISION_VOTE: u8 = 11;
pub(super) const KIND_DECISION_LOCK: u8 = 12;
pub(super) const KIND_JOINT_MEMBERSHIP_CERTIFICATE: u8 = 13;
pub(super) const KIND_PEER_AUTH_POLICY: u8 = 14;
pub(super) const KIND_AUTHENTICATED_PEER_EVIDENCE: u8 = 15;
pub(super) const KIND_QUORUM_LOSS: u8 = 16;
pub(super) const KIND_QUORUM_RECOVERY: u8 = 17;
pub(super) const KIND_MEMBERSHIP_SUCCESSOR_OWNER: u8 = 18;
pub(super) const KIND_MEMBERSHIP_VOTE_REF: u8 = 19;
pub(super) const KIND_AUTHENTICATED_MEMBERSHIP_VOTE_REF: u8 = 20;
pub(super) const KIND_RECOVERY_LOCK_FRONTIER_OWNER: u8 = 21;
pub(super) const KIND_AUTHENTICATED_RECOVERY_ACK_REF: u8 = 22;

mod effects;
mod evidence;
mod votes;

pub use effects::{replicated_effect_id, replicated_origin};
pub use evidence::replication_peer_evidence_signing_message;
pub use votes::replication_membership_digest;
pub(crate) use votes::{SignedMembershipVoteRef, SignedRecoveryAckRef};

// Authority journal wire contract: physical encode/decode only.
pub(super) use effects::{
    decode_ingest, decode_membership_change, decode_publish, decode_quorum_certificate,
    decode_retire, encode_ingest, encode_membership_change, encode_quorum_certificate,
};
pub(super) use evidence::{
    authentication_receipt, decode_peer_auth_policy, decode_quorum_loss,
    decode_recovery_certificate, encode_peer_auth_policy, encode_peer_evidence, encode_quorum_loss,
    encode_recovery_certificate, validate_acknowledgements,
};
pub(super) use votes::{
    decode_decision_lock, decode_decision_vote, decode_effect_vote,
    decode_joint_membership_certificate, decode_leader_certificate, decode_leader_vote,
    decode_membership_successor_owner, decode_membership_vote, decode_membership_vote_ref,
    decode_recovery_lock_frontier_owner, decode_signed_membership_vote_ref,
    decode_signed_recovery_ack_ref, decode_term_promise, encode_decision_lock,
    encode_decision_vote, encode_effect_vote, encode_joint_membership_certificate,
    encode_leader_certificate, encode_leader_vote, encode_membership_successor_owner,
    encode_membership_vote, encode_membership_vote_ref, encode_recovery_lock_frontier_owner,
    encode_signed_membership_vote_ref, encode_signed_recovery_ack_ref, encode_term_promise,
    evidence_voter, recovery_lock_frontier_digest, validate_lock_summaries,
};

// Transport-facing wire contract. Keep this narrower than the authority codec surface.
#[cfg(test)]
pub(super) use evidence::peer_evidence_kind_and_payload_digest;
pub(crate) use evidence::{
    decode_signed_peer_evidence, encode_signed_peer_evidence, signed_peer_evidence_proof_digest,
};
pub(crate) use votes::{
    decode_membership_successor_owner as transport_decode_membership_successor_owner,
    decode_recovery_lock_frontier_owner as transport_decode_recovery_lock_frontier_owner,
    decode_signed_membership_vote_ref as transport_decode_signed_membership_vote_ref,
    decode_signed_recovery_ack_ref as transport_decode_signed_recovery_ack_ref,
    encode_membership_successor_owner as transport_encode_membership_successor_owner,
    encode_recovery_lock_frontier_owner as transport_encode_recovery_lock_frontier_owner,
    encode_signed_membership_vote_ref as transport_encode_signed_membership_vote_ref,
    encode_signed_recovery_ack_ref as transport_encode_signed_recovery_ack_ref,
    evidence_voter as transport_evidence_voter,
    recovery_lock_frontier_digest as transport_recovery_lock_frontier_digest,
    validate_lock_summaries as transport_validate_lock_summaries,
};
