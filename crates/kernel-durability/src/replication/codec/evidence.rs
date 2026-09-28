use std::collections::{BTreeMap, BTreeSet};

use kernel_auth::{KeyId, Sha256Digest, sha256};
use sha2::{Digest as _, Sha256};

use super::super::{
    ReplicaId, ReplicationAuthenticationReceipt, ReplicationClusterId,
    ReplicationJointMembershipAck, ReplicationMembership, ReplicationMembershipVote,
    ReplicationPeerAuthPolicy, ReplicationPeerEvidence, ReplicationQuorumLoss,
    ReplicationRecoveryAck, ReplicationRecoveryCertificate, SignedReplicationPeerEvidence,
    protocol,
};
use super::effects::decode_replica_set;
use super::votes::{
    decode_decision_vote, decode_effect_vote, decode_leader_vote, decode_lock_summaries,
    decode_membership_vote, decode_term_promise, encode_decision_vote, encode_effect_vote,
    encode_leader_vote, encode_lock_summaries, encode_membership_vote, encode_term_promise,
    evidence_voter, validate_lock_summaries,
};
use crate::binary_codec::{Cursor, MAX_COLLECTION_LEN, push_len, push_u64};
use crate::runtime::{CodecError, DurabilityError};

const PEER_EVIDENCE_DOMAIN: &[u8] = b"CFMD-REPLICATION-PEER-EVIDENCE-v1\0";
const EVIDENCE_TERM_PROMISE: u8 = 1;
const EVIDENCE_LEADER_VOTE: u8 = 2;
const EVIDENCE_EFFECT_VOTE: u8 = 3;
const EVIDENCE_MEMBERSHIP_VOTE: u8 = 4;
const EVIDENCE_DECISION_VOTE: u8 = 5;
const EVIDENCE_JOINT_MEMBERSHIP_ACK: u8 = 6;
const EVIDENCE_RECOVERY_ACK: u8 = 7;

pub(in crate::replication) fn encode_peer_evidence(
    evidence: &ReplicationPeerEvidence,
) -> Result<(u8, Vec<u8>), DurabilityError> {
    let encoded = match evidence {
        ReplicationPeerEvidence::TermPromise(value) => {
            (EVIDENCE_TERM_PROMISE, encode_term_promise(value))
        }
        ReplicationPeerEvidence::LeaderVote(value) => {
            (EVIDENCE_LEADER_VOTE, encode_leader_vote(value))
        }
        ReplicationPeerEvidence::EffectVote(value) => {
            (EVIDENCE_EFFECT_VOTE, encode_effect_vote(value))
        }
        ReplicationPeerEvidence::MembershipVote(value) => {
            (EVIDENCE_MEMBERSHIP_VOTE, encode_membership_vote(value)?)
        }
        ReplicationPeerEvidence::DecisionVote(value) => {
            (EVIDENCE_DECISION_VOTE, encode_decision_vote(value))
        }
        ReplicationPeerEvidence::JointMembershipAck(value) => {
            let mut out = Vec::new();
            push_u64(&mut out, value.voter.raw());
            push_u64(&mut out, value.previous_membership_epoch);
            push_u64(&mut out, value.next_membership_epoch);
            push_u64(&mut out, value.term);
            push_u64(&mut out, value.leader.raw());
            out.extend_from_slice(&value.next_membership_digest.0);
            (EVIDENCE_JOINT_MEMBERSHIP_ACK, out)
        }
        ReplicationPeerEvidence::RecoveryAck(value) => {
            validate_lock_summaries(&value.locks)?;
            let mut out = Vec::new();
            push_u64(&mut out, value.voter.raw());
            push_u64(&mut out, value.membership_epoch);
            push_u64(&mut out, value.recovery_term);
            push_u64(&mut out, value.leader.raw());
            encode_lock_summaries(&mut out, &value.locks)?;
            (EVIDENCE_RECOVERY_ACK, out)
        }
    };
    Ok(encoded)
}

fn decode_peer_evidence(tag: u8, bytes: &[u8]) -> Result<ReplicationPeerEvidence, &'static str> {
    match tag {
        EVIDENCE_TERM_PROMISE => {
            decode_term_promise(bytes).map(ReplicationPeerEvidence::TermPromise)
        }
        EVIDENCE_LEADER_VOTE => decode_leader_vote(bytes).map(ReplicationPeerEvidence::LeaderVote),
        EVIDENCE_EFFECT_VOTE => decode_effect_vote(bytes).map(ReplicationPeerEvidence::EffectVote),
        EVIDENCE_MEMBERSHIP_VOTE => {
            decode_membership_vote(bytes).map(ReplicationPeerEvidence::MembershipVote)
        }
        EVIDENCE_DECISION_VOTE => {
            decode_decision_vote(bytes).map(ReplicationPeerEvidence::DecisionVote)
        }
        EVIDENCE_JOINT_MEMBERSHIP_ACK => {
            let mut cursor = Cursor::new(bytes);
            let voter = ReplicaId::new(cursor.u64()?);
            let previous_membership_epoch = cursor.u64()?;
            let next_membership_epoch = cursor.u64()?;
            let term = cursor.u64()?;
            let leader = ReplicaId::new(cursor.u64()?);
            let next_membership_digest = Sha256Digest(
                cursor
                    .take(32)?
                    .try_into()
                    .map_err(|_| "joint membership digest decode")?,
            );
            cursor.finish()?;
            Ok(ReplicationPeerEvidence::JointMembershipAck(
                ReplicationJointMembershipAck {
                    voter,
                    previous_membership_epoch,
                    next_membership_epoch,
                    term,
                    leader,
                    next_membership_digest,
                },
            ))
        }
        EVIDENCE_RECOVERY_ACK => {
            let mut cursor = Cursor::new(bytes);
            let value = ReplicationRecoveryAck {
                voter: ReplicaId::new(cursor.u64()?),
                membership_epoch: cursor.u64()?,
                recovery_term: cursor.u64()?,
                leader: ReplicaId::new(cursor.u64()?),
                locks: decode_lock_summaries(&mut cursor)?,
            };
            cursor.finish()?;
            Ok(ReplicationPeerEvidence::RecoveryAck(value))
        }
        _ => Err("unknown replication peer evidence kind"),
    }
}

fn recovery_ack_payload_len(ack: &ReplicationRecoveryAck) -> Result<usize, DurabilityError> {
    validate_lock_summaries(&ack.locks)?;
    if ack.locks.len() > MAX_COLLECTION_LEN {
        return Err(CodecError::CollectionTooLarge.into());
    }
    36_usize
        .checked_add(
            ack.locks
                .len()
                .checked_mul(32)
                .ok_or(CodecError::LengthOverflow)?,
        )
        .ok_or(CodecError::LengthOverflow.into())
}

fn membership_vote_payload_len(vote: &ReplicationMembershipVote) -> Result<usize, DurabilityError> {
    vote.next.validate()?;
    if vote.next.members.len() > MAX_COLLECTION_LEN || vote.next.quorum_size > MAX_COLLECTION_LEN {
        return Err(CodecError::CollectionTooLarge.into());
    }
    40_usize
        .checked_add(
            vote.next
                .members
                .len()
                .checked_mul(8)
                .ok_or(CodecError::LengthOverflow)?,
        )
        .ok_or(CodecError::LengthOverflow.into())
}

fn update_membership_vote_payload(hasher: &mut Sha256, vote: &ReplicationMembershipVote) {
    hasher.update(vote.voter.raw().to_le_bytes());
    hasher.update(vote.previous_membership_epoch.to_le_bytes());
    hasher.update(vote.term.to_le_bytes());
    hasher.update(vote.next.epoch.to_le_bytes());
    hasher.update(
        u32::try_from(vote.next.members.len())
            .expect("validated membership member count")
            .to_le_bytes(),
    );
    for member in &vote.next.members {
        hasher.update(member.raw().to_le_bytes());
    }
    hasher.update(
        u32::try_from(vote.next.quorum_size)
            .expect("validated membership quorum size")
            .to_le_bytes(),
    );
}

fn update_recovery_ack_payload(hasher: &mut Sha256, ack: &ReplicationRecoveryAck) {
    hasher.update(ack.voter.raw().to_le_bytes());
    hasher.update(ack.membership_epoch.to_le_bytes());
    hasher.update(ack.recovery_term.to_le_bytes());
    hasher.update(ack.leader.raw().to_le_bytes());
    hasher.update(
        u32::try_from(ack.locks.len())
            .expect("validated recovery lock count")
            .to_le_bytes(),
    );
    for lock in &ack.locks {
        hasher.update(lock.position.to_le_bytes());
        hasher.update(lock.term.to_le_bytes());
        hasher.update(lock.effect.0.to_le_bytes());
    }
}

pub(in crate::replication) fn peer_evidence_kind_and_payload_digest(
    evidence: &ReplicationPeerEvidence,
) -> Result<(u8, Sha256Digest), DurabilityError> {
    match evidence {
        ReplicationPeerEvidence::RecoveryAck(ack) => {
            recovery_ack_payload_len(ack)?;
            let mut hasher = Sha256::new();
            update_recovery_ack_payload(&mut hasher, ack);
            Ok((
                EVIDENCE_RECOVERY_ACK,
                Sha256Digest(hasher.finalize().into()),
            ))
        }
        ReplicationPeerEvidence::MembershipVote(vote) => {
            membership_vote_payload_len(vote)?;
            let mut hasher = Sha256::new();
            update_membership_vote_payload(&mut hasher, vote);
            Ok((
                EVIDENCE_MEMBERSHIP_VOTE,
                Sha256Digest(hasher.finalize().into()),
            ))
        }
        _ => {
            let (kind, payload) = encode_peer_evidence(evidence)?;
            Ok((kind, sha256(&payload)))
        }
    }
}

pub(crate) fn signed_peer_evidence_proof_digest(
    signed: &SignedReplicationPeerEvidence,
) -> Result<Sha256Digest, DurabilityError> {
    match &signed.evidence {
        ReplicationPeerEvidence::RecoveryAck(ack) => {
            let payload_len = recovery_ack_payload_len(ack)?;
            signed_peer_evidence_streaming_digest(
                signed,
                EVIDENCE_RECOVERY_ACK,
                payload_len,
                |hasher| update_recovery_ack_payload(hasher, ack),
            )
        }
        ReplicationPeerEvidence::MembershipVote(vote) => {
            let payload_len = membership_vote_payload_len(vote)?;
            signed_peer_evidence_streaming_digest(
                signed,
                EVIDENCE_MEMBERSHIP_VOTE,
                payload_len,
                |hasher| update_membership_vote_payload(hasher, vote),
            )
        }
        _ => Ok(sha256(&encode_signed_peer_evidence(signed)?)),
    }
}

fn signed_peer_evidence_streaming_digest(
    signed: &SignedReplicationPeerEvidence,
    kind: u8,
    payload_len: usize,
    update_payload: impl FnOnce(&mut Sha256),
) -> Result<Sha256Digest, DurabilityError> {
    if payload_len > MAX_COLLECTION_LEN {
        return Err(CodecError::CollectionTooLarge.into());
    }
    let payload_len = u32::try_from(payload_len).map_err(|_| CodecError::LengthOverflow)?;
    let mut hasher = Sha256::new();
    hasher.update(signed.trust_epoch.to_le_bytes());
    hasher.update(signed.signer.0);
    hasher.update(signed.signature);
    hasher.update([kind]);
    hasher.update(payload_len.to_le_bytes());
    update_payload(&mut hasher);
    Ok(Sha256Digest(hasher.finalize().into()))
}

pub fn replication_peer_evidence_signing_message(
    cluster: ReplicationClusterId,
    trust_epoch: u64,
    signer: KeyId,
    evidence: &ReplicationPeerEvidence,
) -> Result<Vec<u8>, DurabilityError> {
    if trust_epoch == 0 {
        return Err(protocol("replication peer evidence trust epoch is zero"));
    }
    let voter = evidence_voter(evidence);
    let (kind, payload_digest) = peer_evidence_kind_and_payload_digest(evidence)?;
    let mut message = Vec::with_capacity(PEER_EVIDENCE_DOMAIN.len() + 32 + 8 + 32 + 8 + 1 + 32);
    message.extend_from_slice(PEER_EVIDENCE_DOMAIN);
    message.extend_from_slice(&cluster.0);
    message.extend_from_slice(&trust_epoch.to_le_bytes());
    message.extend_from_slice(&signer.0);
    message.extend_from_slice(&voter.raw().to_le_bytes());
    message.push(kind);
    message.extend_from_slice(&payload_digest.0);
    Ok(message)
}

pub(crate) fn encode_signed_peer_evidence(
    signed: &SignedReplicationPeerEvidence,
) -> Result<Vec<u8>, DurabilityError> {
    let (kind, payload) = encode_peer_evidence(&signed.evidence)?;
    let mut out = Vec::new();
    push_u64(&mut out, signed.trust_epoch);
    out.extend_from_slice(&signed.signer.0);
    out.extend_from_slice(&signed.signature);
    out.push(kind);
    push_len(&mut out, payload.len())?;
    out.extend_from_slice(&payload);
    Ok(out)
}

pub(crate) fn decode_signed_peer_evidence(
    bytes: &[u8],
) -> Result<SignedReplicationPeerEvidence, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let trust_epoch = cursor.u64()?;
    let signer = KeyId(
        cursor
            .take(32)?
            .try_into()
            .map_err(|_| "peer evidence signer decode")?,
    );
    let signature = cursor
        .take(64)?
        .try_into()
        .map_err(|_| "peer evidence signature decode")?;
    let kind = cursor.u8()?;
    let payload_len = cursor.len()?;
    let payload = cursor.take(payload_len)?;
    let evidence = decode_peer_evidence(kind, payload)?;
    cursor.finish()?;
    Ok(SignedReplicationPeerEvidence {
        trust_epoch,
        signer,
        evidence,
        signature,
    })
}

pub(in crate::replication) fn authentication_receipt(
    signed: &SignedReplicationPeerEvidence,
) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
    let (_, payload_digest) = peer_evidence_kind_and_payload_digest(&signed.evidence)?;
    Ok(ReplicationAuthenticationReceipt {
        proof_digest: signed_peer_evidence_proof_digest(signed)?,
        payload_digest,
        trust_epoch: signed.trust_epoch,
        voter: evidence_voter(&signed.evidence),
        signer: signed.signer,
    })
}

pub(in crate::replication) fn encode_peer_auth_policy(
    policy: &ReplicationPeerAuthPolicy,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.extend_from_slice(&policy.cluster.0);
    push_u64(&mut out, policy.trust_epoch);
    push_len(&mut out, policy.peer_keys.len())?;
    for (replica, key) in &policy.peer_keys {
        push_u64(&mut out, replica.raw());
        out.extend_from_slice(&key.0);
    }
    Ok(out)
}

pub(in crate::replication) fn decode_peer_auth_policy(
    bytes: &[u8],
) -> Result<ReplicationPeerAuthPolicy, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let cluster = ReplicationClusterId(
        cursor
            .take(32)?
            .try_into()
            .map_err(|_| "replication cluster id decode")?,
    );
    let trust_epoch = cursor.u64()?;
    let count = cursor.len()?;
    let mut peer_keys = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        let replica = ReplicaId::new(cursor.u64()?);
        if previous.is_some_and(|prior| prior >= replica) {
            return Err("replication peer auth policy is not sorted and unique");
        }
        previous = Some(replica);
        let key = KeyId(
            cursor
                .take(32)?
                .try_into()
                .map_err(|_| "replication peer key id decode")?,
        );
        peer_keys.insert(replica, key);
    }
    cursor.finish()?;
    Ok(ReplicationPeerAuthPolicy {
        cluster,
        trust_epoch,
        peer_keys,
    })
}

pub(in crate::replication) fn encode_quorum_loss(loss: ReplicationQuorumLoss) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, loss.membership_epoch);
    push_u64(&mut out, loss.observed_term);
    out
}

pub(in crate::replication) fn decode_quorum_loss(
    bytes: &[u8],
) -> Result<ReplicationQuorumLoss, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let loss = ReplicationQuorumLoss {
        membership_epoch: cursor.u64()?,
        observed_term: cursor.u64()?,
    };
    cursor.finish()?;
    Ok(loss)
}

pub(in crate::replication) fn encode_recovery_certificate(
    certificate: &ReplicationRecoveryCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, certificate.membership_epoch);
    push_u64(&mut out, certificate.recovery_term);
    push_u64(&mut out, certificate.leader.raw());
    push_len(&mut out, certificate.acknowledged_by.len())?;
    for voter in &certificate.acknowledged_by {
        push_u64(&mut out, voter.raw());
    }
    encode_lock_summaries(&mut out, &certificate.reconciled_locks)?;
    Ok(out)
}

pub(in crate::replication) fn decode_recovery_certificate(
    bytes: &[u8],
) -> Result<ReplicationRecoveryCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let membership_epoch = cursor.u64()?;
    let recovery_term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication recovery acknowledgements are not sorted and unique",
    )?;
    let reconciled_locks = decode_lock_summaries(&mut cursor)?;
    cursor.finish()?;
    Ok(ReplicationRecoveryCertificate {
        membership_epoch,
        recovery_term,
        leader,
        acknowledged_by,
        reconciled_locks,
    })
}

pub(in crate::replication) fn validate_acknowledgements(
    membership: &ReplicationMembership,
    acknowledged_by: &BTreeSet<ReplicaId>,
    insufficient_reason: &'static str,
) -> Result<(), DurabilityError> {
    if !acknowledged_by.is_subset(&membership.members) {
        return Err(protocol(
            "replication acknowledgement references a non-member replica",
        ));
    }
    if acknowledged_by.len() < membership.quorum_size {
        return Err(protocol(insufficient_reason));
    }
    Ok(())
}
