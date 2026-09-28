use kernel_auth::{KeyId, Sha256Digest, sha256};
use kernel_change::RevisionEffectId;

use super::super::{
    ReplicaId, ReplicationDecisionLock, ReplicationDecisionVote, ReplicationEffectVote,
    ReplicationJointMembershipCertificate, ReplicationLeaderCertificate, ReplicationLeaderVote,
    ReplicationLockSummary, ReplicationMembership, ReplicationMembershipVote,
    ReplicationPeerEvidence, ReplicationTermPromise, SignedReplicationPeerEvidence, protocol,
};
use super::effects::decode_replica_set;
use crate::binary_codec::{Cursor, push_len, push_u64, push_u128};
use crate::runtime::{CodecError, DurabilityError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::replication) struct MembershipVoteRef {
    pub(in crate::replication) voter: ReplicaId,
    pub(in crate::replication) previous_membership_epoch: u64,
    pub(in crate::replication) term: u64,
    pub(in crate::replication) successor_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SignedMembershipVoteRef {
    pub(crate) trust_epoch: u64,
    pub(crate) signer: KeyId,
    pub(crate) signature: [u8; 64],
    pub(crate) voter: ReplicaId,
    pub(crate) previous_membership_epoch: u64,
    pub(crate) term: u64,
    pub(crate) successor_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SignedRecoveryAckRef {
    pub(crate) trust_epoch: u64,
    pub(crate) signer: KeyId,
    pub(crate) signature: [u8; 64],
    pub(crate) voter: ReplicaId,
    pub(crate) membership_epoch: u64,
    pub(crate) recovery_term: u64,
    pub(crate) leader: ReplicaId,
    pub(crate) lock_frontier_digest: Sha256Digest,
}

pub(in crate::replication) fn encode_effect_vote(vote: &ReplicationEffectVote) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u128(&mut out, vote.effect.0);
    push_u64(&mut out, vote.membership_epoch);
    out
}

pub(in crate::replication) fn decode_effect_vote(
    bytes: &[u8],
) -> Result<ReplicationEffectVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let vote = ReplicationEffectVote {
        voter: ReplicaId::new(cursor.u64()?),
        effect: RevisionEffectId(cursor.u128()?),
        membership_epoch: cursor.u64()?,
    };
    cursor.finish()?;
    Ok(vote)
}

pub(in crate::replication) fn encode_membership_vote(
    vote: &ReplicationMembershipVote,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.previous_membership_epoch);
    push_u64(&mut out, vote.term);
    push_u64(&mut out, vote.next.epoch);
    push_len(&mut out, vote.next.members.len())?;
    for member in &vote.next.members {
        push_u64(&mut out, member.raw());
    }
    push_len(&mut out, vote.next.quorum_size)?;
    Ok(out)
}

pub(crate) fn encode_membership_successor_owner(
    membership: &ReplicationMembership,
    digest: Sha256Digest,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.extend_from_slice(&digest.0);
    encode_membership_value(&mut out, membership)?;
    Ok(out)
}

pub(crate) fn decode_membership_successor_owner(
    bytes: &[u8],
) -> Result<(Sha256Digest, ReplicationMembership), &'static str> {
    let mut cursor = Cursor::new(bytes);
    let digest = Sha256Digest(
        cursor
            .take(32)?
            .try_into()
            .map_err(|_| "membership successor digest decode")?,
    );
    let membership = decode_membership_value(&mut cursor)?;
    cursor.finish()?;
    Ok((digest, membership))
}

pub(in crate::replication) fn encode_membership_vote_ref(
    vote: &ReplicationMembershipVote,
    successor_digest: Sha256Digest,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(56);
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.previous_membership_epoch);
    push_u64(&mut out, vote.term);
    out.extend_from_slice(&successor_digest.0);
    out
}

pub(in crate::replication) fn decode_membership_vote_ref(
    bytes: &[u8],
) -> Result<MembershipVoteRef, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let reference = MembershipVoteRef {
        voter: ReplicaId::new(cursor.u64()?),
        previous_membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
        successor_digest: Sha256Digest(
            cursor
                .take(32)?
                .try_into()
                .map_err(|_| "membership vote reference digest decode")?,
        ),
    };
    cursor.finish()?;
    Ok(reference)
}

pub(crate) fn encode_signed_membership_vote_ref(
    signed: &SignedReplicationPeerEvidence,
    successor_digest: Sha256Digest,
) -> Result<Vec<u8>, DurabilityError> {
    let ReplicationPeerEvidence::MembershipVote(vote) = &signed.evidence else {
        return Err(protocol(
            "signed membership vote reference requires membership vote evidence",
        ));
    };
    let mut out = Vec::with_capacity(160);
    push_u64(&mut out, signed.trust_epoch);
    out.extend_from_slice(&signed.signer.0);
    out.extend_from_slice(&signed.signature);
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.previous_membership_epoch);
    push_u64(&mut out, vote.term);
    out.extend_from_slice(&successor_digest.0);
    Ok(out)
}

pub(crate) fn decode_signed_membership_vote_ref(
    bytes: &[u8],
) -> Result<SignedMembershipVoteRef, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let reference = SignedMembershipVoteRef {
        trust_epoch: cursor.u64()?,
        signer: KeyId(
            cursor
                .take(32)?
                .try_into()
                .map_err(|_| "signed membership vote signer decode")?,
        ),
        signature: cursor
            .take(64)?
            .try_into()
            .map_err(|_| "signed membership vote signature decode")?,
        voter: ReplicaId::new(cursor.u64()?),
        previous_membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
        successor_digest: Sha256Digest(
            cursor
                .take(32)?
                .try_into()
                .map_err(|_| "signed membership vote successor digest decode")?,
        ),
    };
    cursor.finish()?;
    Ok(reference)
}

pub(in crate::replication) fn decode_membership_vote(
    bytes: &[u8],
) -> Result<ReplicationMembershipVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let voter = ReplicaId::new(cursor.u64()?);
    let previous_membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let epoch = cursor.u64()?;
    let members = decode_replica_set(
        &mut cursor,
        "membership vote members are not sorted and unique",
    )?;
    let quorum_size = cursor.len()?;
    cursor.finish()?;
    Ok(ReplicationMembershipVote {
        voter,
        previous_membership_epoch,
        term,
        next: ReplicationMembership {
            epoch,
            members,
            quorum_size,
        },
    })
}

pub(in crate::replication) fn encode_term_promise(promise: &ReplicationTermPromise) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, promise.voter.raw());
    push_u64(&mut out, promise.membership_epoch);
    push_u64(&mut out, promise.term);
    out
}

pub(in crate::replication) fn decode_term_promise(
    bytes: &[u8],
) -> Result<ReplicationTermPromise, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let promise = ReplicationTermPromise {
        voter: ReplicaId::new(cursor.u64()?),
        membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
    };
    cursor.finish()?;
    Ok(promise)
}

pub(in crate::replication) fn encode_leader_vote(vote: &ReplicationLeaderVote) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.membership_epoch);
    push_u64(&mut out, vote.term);
    push_u64(&mut out, vote.candidate.raw());
    out
}

pub(in crate::replication) fn decode_leader_vote(
    bytes: &[u8],
) -> Result<ReplicationLeaderVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let vote = ReplicationLeaderVote {
        voter: ReplicaId::new(cursor.u64()?),
        membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
        candidate: ReplicaId::new(cursor.u64()?),
    };
    cursor.finish()?;
    Ok(vote)
}

pub(in crate::replication) fn encode_leader_certificate(
    certificate: &ReplicationLeaderCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, certificate.membership_epoch);
    push_u64(&mut out, certificate.term);
    push_u64(&mut out, certificate.leader.raw());
    push_len(&mut out, certificate.acknowledged_by.len())?;
    for voter in &certificate.acknowledged_by {
        push_u64(&mut out, voter.raw());
    }
    Ok(out)
}

pub(in crate::replication) fn decode_leader_certificate(
    bytes: &[u8],
) -> Result<ReplicationLeaderCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication leader certificate acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationLeaderCertificate {
        membership_epoch,
        term,
        leader,
        acknowledged_by,
    })
}

fn encode_optional_term(out: &mut Vec<u8>, term: Option<u64>) {
    match term {
        None => push_u64(out, 0),
        Some(term) => push_u64(out, term),
    }
}

fn decode_optional_term(cursor: &mut Cursor<'_>) -> Result<Option<u64>, &'static str> {
    match cursor.u64()? {
        0 => Ok(None),
        term => Ok(Some(term)),
    }
}

pub(in crate::replication) fn encode_decision_vote(vote: &ReplicationDecisionVote) -> Vec<u8> {
    let mut out = Vec::new();
    push_u64(&mut out, vote.voter.raw());
    push_u64(&mut out, vote.membership_epoch);
    push_u64(&mut out, vote.term);
    push_u64(&mut out, vote.leader.raw());
    push_u64(&mut out, vote.position);
    push_u128(&mut out, vote.effect.0);
    encode_optional_term(&mut out, vote.carried_from_term);
    out
}

pub(in crate::replication) fn decode_decision_vote(
    bytes: &[u8],
) -> Result<ReplicationDecisionVote, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let vote = ReplicationDecisionVote {
        voter: ReplicaId::new(cursor.u64()?),
        membership_epoch: cursor.u64()?,
        term: cursor.u64()?,
        leader: ReplicaId::new(cursor.u64()?),
        position: cursor.u64()?,
        effect: RevisionEffectId(cursor.u128()?),
        carried_from_term: decode_optional_term(&mut cursor)?,
    };
    cursor.finish()?;
    Ok(vote)
}

pub(in crate::replication) fn encode_decision_lock(
    lock: &ReplicationDecisionLock,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, lock.membership_epoch);
    push_u64(&mut out, lock.term);
    push_u64(&mut out, lock.leader.raw());
    push_u64(&mut out, lock.position);
    push_u128(&mut out, lock.effect.0);
    encode_optional_term(&mut out, lock.carried_from_term);
    push_len(&mut out, lock.acknowledged_by.len())?;
    for voter in &lock.acknowledged_by {
        push_u64(&mut out, voter.raw());
    }
    Ok(out)
}

pub(in crate::replication) fn decode_decision_lock(
    bytes: &[u8],
) -> Result<ReplicationDecisionLock, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let position = cursor.u64()?;
    let effect = RevisionEffectId(cursor.u128()?);
    let carried_from_term = decode_optional_term(&mut cursor)?;
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication decision lock acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationDecisionLock {
        membership_epoch,
        term,
        leader,
        position,
        effect,
        acknowledged_by,
        carried_from_term,
    })
}

fn encode_membership_value(
    out: &mut Vec<u8>,
    membership: &ReplicationMembership,
) -> Result<(), CodecError> {
    push_u64(out, membership.epoch);
    push_len(out, membership.members.len())?;
    for member in &membership.members {
        push_u64(out, member.raw());
    }
    push_len(out, membership.quorum_size)?;
    Ok(())
}

fn decode_membership_value(cursor: &mut Cursor<'_>) -> Result<ReplicationMembership, &'static str> {
    let epoch = cursor.u64()?;
    let members = decode_replica_set(
        cursor,
        "joint membership certificate members are not sorted and unique",
    )?;
    let quorum_size = cursor.len()?;
    Ok(ReplicationMembership {
        epoch,
        members,
        quorum_size,
    })
}

pub(in crate::replication) fn encode_joint_membership_certificate(
    certificate: &ReplicationJointMembershipCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, certificate.previous_membership_epoch);
    push_u64(&mut out, certificate.term);
    push_u64(&mut out, certificate.leader.raw());
    encode_membership_value(&mut out, &certificate.next)?;
    push_len(&mut out, certificate.acknowledged_by_previous.len())?;
    for voter in &certificate.acknowledged_by_previous {
        push_u64(&mut out, voter.raw());
    }
    push_len(&mut out, certificate.acknowledged_by_next.len())?;
    for voter in &certificate.acknowledged_by_next {
        push_u64(&mut out, voter.raw());
    }
    Ok(out)
}

pub(in crate::replication) fn decode_joint_membership_certificate(
    bytes: &[u8],
) -> Result<ReplicationJointMembershipCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let previous_membership_epoch = cursor.u64()?;
    let term = cursor.u64()?;
    let leader = ReplicaId::new(cursor.u64()?);
    let next = decode_membership_value(&mut cursor)?;
    let acknowledged_by_previous = decode_replica_set(
        &mut cursor,
        "joint membership previous acknowledgements are not sorted and unique",
    )?;
    let acknowledged_by_next = decode_replica_set(
        &mut cursor,
        "joint membership next acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationJointMembershipCertificate {
        previous_membership_epoch,
        term,
        leader,
        next,
        acknowledged_by_previous,
        acknowledged_by_next,
    })
}

pub(super) fn encode_lock_summaries(
    out: &mut Vec<u8>,
    locks: &[ReplicationLockSummary],
) -> Result<(), CodecError> {
    push_len(out, locks.len())?;
    for lock in locks {
        push_u64(out, lock.position);
        push_u64(out, lock.term);
        push_u128(out, lock.effect.0);
    }
    Ok(())
}

pub(super) fn decode_lock_summaries(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<ReplicationLockSummary>, &'static str> {
    let count = cursor.len()?;
    let mut locks = Vec::with_capacity(cursor.bounded_capacity(count));
    let mut previous = None;
    for _ in 0..count {
        let lock = ReplicationLockSummary {
            position: cursor.u64()?,
            term: cursor.u64()?,
            effect: RevisionEffectId(cursor.u128()?),
        };
        if previous.is_some_and(|position| position >= lock.position) {
            return Err("replication lock summaries are not sorted and unique");
        }
        previous = Some(lock.position);
        locks.push(lock);
    }
    Ok(locks)
}

pub(crate) fn recovery_lock_frontier_digest(
    locks: &[ReplicationLockSummary],
) -> Result<Sha256Digest, DurabilityError> {
    let mut encoded = Vec::new();
    encode_lock_summaries(&mut encoded, locks)?;
    Ok(sha256(&encoded))
}

pub(crate) fn validate_lock_summaries(
    locks: &[ReplicationLockSummary],
) -> Result<(), DurabilityError> {
    if locks
        .windows(2)
        .any(|pair| pair[0].position >= pair[1].position)
    {
        return Err(protocol(
            "replication lock summaries are not sorted and unique",
        ));
    }
    if locks.iter().any(|lock| lock.term == 0) {
        return Err(protocol("replication lock summary has zero term"));
    }
    Ok(())
}

pub fn replication_membership_digest(
    membership: &ReplicationMembership,
) -> Result<Sha256Digest, DurabilityError> {
    membership.validate()?;
    let mut encoded = Vec::new();
    encode_membership_value(&mut encoded, membership)?;
    Ok(sha256(&encoded))
}

pub(crate) fn evidence_voter(evidence: &ReplicationPeerEvidence) -> ReplicaId {
    match evidence {
        ReplicationPeerEvidence::TermPromise(value) => value.voter,
        ReplicationPeerEvidence::LeaderVote(value) => value.voter,
        ReplicationPeerEvidence::EffectVote(value) => value.voter,
        ReplicationPeerEvidence::MembershipVote(value) => value.voter,
        ReplicationPeerEvidence::DecisionVote(value) => value.voter,
        ReplicationPeerEvidence::JointMembershipAck(value) => value.voter,
        ReplicationPeerEvidence::RecoveryAck(value) => value.voter,
    }
}

pub(crate) fn encode_recovery_lock_frontier_owner(
    locks: &[ReplicationLockSummary],
    digest: Sha256Digest,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.extend_from_slice(&digest.0);
    encode_lock_summaries(&mut out, locks)?;
    Ok(out)
}

pub(crate) fn decode_recovery_lock_frontier_owner(
    bytes: &[u8],
) -> Result<(Sha256Digest, Vec<ReplicationLockSummary>), &'static str> {
    let mut cursor = Cursor::new(bytes);
    let digest = Sha256Digest(
        cursor
            .take(32)?
            .try_into()
            .map_err(|_| "recovery lock frontier owner digest decode")?,
    );
    let locks = decode_lock_summaries(&mut cursor)?;
    cursor.finish()?;
    Ok((digest, locks))
}

pub(crate) fn encode_signed_recovery_ack_ref(
    signed: &SignedReplicationPeerEvidence,
    lock_frontier_digest: Sha256Digest,
) -> Result<Vec<u8>, DurabilityError> {
    let ReplicationPeerEvidence::RecoveryAck(ack) = &signed.evidence else {
        return Err(protocol(
            "signed recovery acknowledgement reference requires recovery evidence",
        ));
    };
    let mut out = Vec::with_capacity(168);
    push_u64(&mut out, signed.trust_epoch);
    out.extend_from_slice(&signed.signer.0);
    out.extend_from_slice(&signed.signature);
    push_u64(&mut out, ack.voter.raw());
    push_u64(&mut out, ack.membership_epoch);
    push_u64(&mut out, ack.recovery_term);
    push_u64(&mut out, ack.leader.raw());
    out.extend_from_slice(&lock_frontier_digest.0);
    Ok(out)
}

pub(crate) fn decode_signed_recovery_ack_ref(
    bytes: &[u8],
) -> Result<SignedRecoveryAckRef, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let reference = SignedRecoveryAckRef {
        trust_epoch: cursor.u64()?,
        signer: KeyId(
            cursor
                .take(32)?
                .try_into()
                .map_err(|_| "signed recovery acknowledgement signer decode")?,
        ),
        signature: cursor
            .take(64)?
            .try_into()
            .map_err(|_| "signed recovery acknowledgement signature decode")?,
        voter: ReplicaId::new(cursor.u64()?),
        membership_epoch: cursor.u64()?,
        recovery_term: cursor.u64()?,
        leader: ReplicaId::new(cursor.u64()?),
        lock_frontier_digest: Sha256Digest(
            cursor
                .take(32)?
                .try_into()
                .map_err(|_| "signed recovery lock frontier digest decode")?,
        ),
    };
    cursor.finish()?;
    Ok(reference)
}
