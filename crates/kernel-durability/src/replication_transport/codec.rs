use crate::binary_codec::Cursor;
use crate::replication::codec::{
    SignedMembershipVoteRef, SignedRecoveryAckRef, decode_signed_peer_evidence,
    encode_signed_peer_evidence, signed_peer_evidence_proof_digest,
};
use crate::replication::{
    ReplicaId, ReplicationClusterId, ReplicationLockSummary, ReplicationMembership,
    ReplicationMembershipVote, ReplicationPeerEvidence, ReplicationRecoveryAck,
    SignedReplicationPeerEvidence,
};
use crate::runtime::DurabilityError;
use crate::wal_frame::MAX_PAYLOAD_LEN;
use kernel_auth::{Sha256Digest, sha256};

use super::session::{
    MAX_ANTI_ENTROPY_LOCKS, ReplicationAntiEntropyChunk, ReplicationAntiEntropyRelation,
    ReplicationAntiEntropyRequest, ReplicationAntiEntropySummary, ReplicationHeartbeat,
    ReplicationTransportFrame, ReplicationTransportPayload, SignedReplicationTransportFrame,
};

const TRANSPORT_DOMAIN: &[u8] = b"CFMD-REPLICATION-TRANSPORT-v1\0";
const TRANSPORT_MAGIC: [u8; 4] = *b"CFTR";
const TRANSPORT_VERSION: u16 = 1;
const PAYLOAD_PEER_EVIDENCE: u8 = 1;
const PAYLOAD_ANTI_ENTROPY_SUMMARY: u8 = 2;
const PAYLOAD_ANTI_ENTROPY_REQUEST: u8 = 3;
const PAYLOAD_ANTI_ENTROPY_CHUNK: u8 = 4;
const PAYLOAD_HEARTBEAT: u8 = 5;
pub(super) const PAYLOAD_RECOVERY_ACK_OWNER_REF: u8 = 6;
pub(super) const PAYLOAD_RECOVERY_ACK_REF: u8 = 7;
pub(super) const PAYLOAD_MEMBERSHIP_VOTE_OWNER_REF: u8 = 8;
pub(super) const PAYLOAD_MEMBERSHIP_VOTE_REF: u8 = 9;

pub fn replication_transport_signing_message(
    frame: &ReplicationTransportFrame,
) -> Result<Vec<u8>, DurabilityError> {
    if frame.trust_epoch == 0 || frame.sender.raw() == 0 || frame.sequence == 0 {
        return Err(protocol(
            "replication transport frame has zero authority coordinate",
        ));
    }
    let digest = match &frame.payload {
        ReplicationTransportPayload::PeerEvidence(peer) => signed_peer_evidence_proof_digest(peer)?,
        payload => sha256(&encode_payload(payload)?),
    };
    let mut out = Vec::with_capacity(TRANSPORT_DOMAIN.len() + 32 + 8 + 8 + 8 + 32);
    out.extend_from_slice(TRANSPORT_DOMAIN);
    out.extend_from_slice(&frame.cluster.0);
    out.extend_from_slice(&frame.trust_epoch.to_le_bytes());
    out.extend_from_slice(&frame.sender.raw().to_le_bytes());
    out.extend_from_slice(&frame.sequence.to_le_bytes());
    out.extend_from_slice(&digest.0);
    Ok(out)
}

pub fn encode_signed_replication_transport_frame(
    signed: &SignedReplicationTransportFrame,
) -> Result<Vec<u8>, DurabilityError> {
    if let ReplicationTransportPayload::PeerEvidence(SignedReplicationPeerEvidence {
        evidence,
        ..
    }) = &signed.frame.payload
    {
        match evidence {
            ReplicationPeerEvidence::RecoveryAck(_) => {
                return Err(protocol(
                    "recovery acknowledgement wire encoding requires stateful transport egress",
                ));
            }
            ReplicationPeerEvidence::MembershipVote(_) => {
                return Err(protocol(
                    "membership vote wire encoding requires stateful transport egress",
                ));
            }
            _ => {}
        }
    }
    let (kind, payload) = encode_payload_with_kind(&signed.frame.payload)?;
    encode_signed_replication_transport_physical(signed, kind, &payload)
}

pub(super) fn encode_signed_replication_transport_physical(
    signed: &SignedReplicationTransportFrame,
    kind: u8,
    payload: &[u8],
) -> Result<Vec<u8>, DurabilityError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(protocol("replication transport payload exceeds hard bound"));
    }
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| protocol("replication transport payload is too large"))?;
    let mut out = Vec::with_capacity(4 + 2 + 32 + 8 + 8 + 8 + 1 + 4 + payload.len() + 64);
    out.extend_from_slice(&TRANSPORT_MAGIC);
    out.extend_from_slice(&TRANSPORT_VERSION.to_le_bytes());
    out.extend_from_slice(&signed.frame.cluster.0);
    out.extend_from_slice(&signed.frame.trust_epoch.to_le_bytes());
    out.extend_from_slice(&signed.frame.sender.raw().to_le_bytes());
    out.extend_from_slice(&signed.frame.sequence.to_le_bytes());
    out.push(kind);
    out.extend_from_slice(&payload_len.to_le_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&signed.signature);
    Ok(out)
}

pub fn decode_signed_replication_transport_frame(
    bytes: &[u8],
) -> Result<SignedReplicationTransportFrame, DurabilityError> {
    let DecodedPhysicalTransportFrame {
        header,
        kind,
        payload,
        signature,
    } = decode_physical_transport_frame(bytes)?;
    if matches!(
        kind,
        PAYLOAD_RECOVERY_ACK_OWNER_REF
            | PAYLOAD_RECOVERY_ACK_REF
            | PAYLOAD_MEMBERSHIP_VOTE_OWNER_REF
            | PAYLOAD_MEMBERSHIP_VOTE_REF
    ) {
        return Err(protocol(
            "stateful replication transport frame requires ingress decoder",
        ));
    }
    Ok(SignedReplicationTransportFrame {
        frame: ReplicationTransportFrame {
            cluster: header.cluster,
            trust_epoch: header.trust_epoch,
            sender: header.sender,
            sequence: header.sequence,
            payload: decode_payload(kind, payload)?,
        },
        signature,
    })
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PhysicalTransportHeader {
    pub(super) cluster: ReplicationClusterId,
    pub(super) trust_epoch: u64,
    pub(super) sender: ReplicaId,
    pub(super) sequence: u64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DecodedPhysicalTransportFrame<'a> {
    pub(super) header: PhysicalTransportHeader,
    pub(super) kind: u8,
    pub(super) payload: &'a [u8],
    pub(super) signature: [u8; 64],
}

pub(super) fn decode_physical_transport_frame(
    bytes: &[u8],
) -> Result<DecodedPhysicalTransportFrame<'_>, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(4).map_err(codec)? != TRANSPORT_MAGIC {
        return Err(protocol("replication transport magic mismatch"));
    }
    let version = u16::from_le_bytes(
        cursor
            .take(2)
            .map_err(codec)?
            .try_into()
            .map_err(|_| protocol("replication transport version decode"))?,
    );
    if version != TRANSPORT_VERSION {
        return Err(protocol("replication transport version mismatch"));
    }
    let cluster = ReplicationClusterId(
        cursor
            .take(32)
            .map_err(codec)?
            .try_into()
            .map_err(|_| protocol("replication transport cluster decode"))?,
    );
    let trust_epoch = cursor.u64().map_err(codec)?;
    let sender = ReplicaId::new(cursor.u64().map_err(codec)?);
    let sequence = cursor.u64().map_err(codec)?;
    let kind = cursor.u8().map_err(codec)?;
    let payload_len = cursor.u32().map_err(codec)? as usize;
    if payload_len > MAX_PAYLOAD_LEN {
        return Err(protocol("replication transport payload exceeds hard bound"));
    }
    let payload = cursor.take(payload_len).map_err(codec)?;
    let signature = cursor
        .take(64)
        .map_err(codec)?
        .try_into()
        .map_err(|_| protocol("replication transport signature decode"))?;
    cursor.finish().map_err(codec)?;
    Ok(DecodedPhysicalTransportFrame {
        header: PhysicalTransportHeader {
            cluster,
            trust_epoch,
            sender,
            sequence,
        },
        kind,
        payload,
        signature,
    })
}

pub(super) fn recovery_ack_payload_from_reference(
    reference: SignedRecoveryAckRef,
    locks: &[ReplicationLockSummary],
) -> ReplicationTransportPayload {
    ReplicationTransportPayload::PeerEvidence(SignedReplicationPeerEvidence {
        trust_epoch: reference.trust_epoch,
        signer: reference.signer,
        evidence: ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
            voter: reference.voter,
            membership_epoch: reference.membership_epoch,
            recovery_term: reference.recovery_term,
            leader: reference.leader,
            locks: locks.to_vec(),
        }),
        signature: reference.signature,
    })
}

pub(super) fn membership_vote_payload_from_reference(
    reference: SignedMembershipVoteRef,
    successor: &ReplicationMembership,
) -> ReplicationTransportPayload {
    ReplicationTransportPayload::PeerEvidence(SignedReplicationPeerEvidence {
        trust_epoch: reference.trust_epoch,
        signer: reference.signer,
        evidence: ReplicationPeerEvidence::MembershipVote(ReplicationMembershipVote {
            voter: reference.voter,
            previous_membership_epoch: reference.previous_membership_epoch,
            term: reference.term,
            next: successor.clone(),
        }),
        signature: reference.signature,
    })
}

#[must_use]
pub fn replication_lock_frontier_digest(locks: &[ReplicationLockSummary]) -> Sha256Digest {
    let mut bytes = Vec::with_capacity(locks.len() * 40);
    for lock in locks {
        bytes.extend_from_slice(&lock.position.to_le_bytes());
        bytes.extend_from_slice(&lock.term.to_le_bytes());
        bytes.extend_from_slice(&lock.effect.0.to_le_bytes());
    }
    sha256(&bytes)
}

pub fn replication_anti_entropy_summary(
    membership_epoch: u64,
    term: u64,
    locks: &[ReplicationLockSummary],
) -> Result<ReplicationAntiEntropySummary, DurabilityError> {
    validate_locks(locks)?;
    Ok(ReplicationAntiEntropySummary {
        membership_epoch,
        term,
        lock_count: locks.len() as u64,
        highest_position: locks.last().map(|lock| lock.position),
        lock_digest: replication_lock_frontier_digest(locks),
    })
}

#[must_use]
pub fn compare_replication_anti_entropy(
    local: &ReplicationAntiEntropySummary,
    remote: &ReplicationAntiEntropySummary,
) -> ReplicationAntiEntropyRelation {
    if local.membership_epoch != remote.membership_epoch {
        ReplicationAntiEntropyRelation::MembershipMismatch
    } else if local.lock_count == remote.lock_count
        && local.highest_position == remote.highest_position
        && local.lock_digest == remote.lock_digest
    {
        ReplicationAntiEntropyRelation::InSync
    } else {
        ReplicationAntiEntropyRelation::ExchangeRequired
    }
}

pub fn validate_replication_anti_entropy_chunk(
    chunk: &ReplicationAntiEntropyChunk,
) -> Result<(), DurabilityError> {
    if chunk.locks.len() > MAX_ANTI_ENTROPY_LOCKS {
        return Err(protocol("replication anti-entropy chunk exceeds bound"));
    }
    validate_locks(&chunk.locks)
}

fn validate_locks(locks: &[ReplicationLockSummary]) -> Result<(), DurabilityError> {
    if locks
        .windows(2)
        .any(|pair| pair[0].position >= pair[1].position)
    {
        return Err(protocol(
            "replication lock frontier is not strictly ordered",
        ));
    }
    Ok(())
}

fn encode_payload(payload: &ReplicationTransportPayload) -> Result<Vec<u8>, DurabilityError> {
    encode_payload_with_kind(payload).map(|(_, bytes)| bytes)
}

fn encode_payload_with_kind(
    payload: &ReplicationTransportPayload,
) -> Result<(u8, Vec<u8>), DurabilityError> {
    match payload {
        ReplicationTransportPayload::PeerEvidence(value) => {
            Ok((PAYLOAD_PEER_EVIDENCE, encode_signed_peer_evidence(value)?))
        }
        ReplicationTransportPayload::AntiEntropySummary(value) => {
            let mut out = Vec::with_capacity(89);
            out.extend_from_slice(&value.membership_epoch.to_le_bytes());
            out.extend_from_slice(&value.term.to_le_bytes());
            out.extend_from_slice(&value.lock_count.to_le_bytes());
            encode_option_u64(&mut out, value.highest_position);
            out.extend_from_slice(&value.lock_digest.0);
            Ok((PAYLOAD_ANTI_ENTROPY_SUMMARY, out))
        }
        ReplicationTransportPayload::AntiEntropyRequest(value) => {
            let mut out = Vec::with_capacity(20);
            out.extend_from_slice(&value.membership_epoch.to_le_bytes());
            out.extend_from_slice(&value.from_position.to_le_bytes());
            out.extend_from_slice(&value.max_locks.to_le_bytes());
            Ok((PAYLOAD_ANTI_ENTROPY_REQUEST, out))
        }
        ReplicationTransportPayload::AntiEntropyChunk(value) => {
            validate_replication_anti_entropy_chunk(value)?;
            let count = u32::try_from(value.locks.len())
                .map_err(|_| protocol("replication anti-entropy lock count overflow"))?;
            let mut out = Vec::with_capacity(13 + value.locks.len() * 32);
            out.extend_from_slice(&value.membership_epoch.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            for lock in &value.locks {
                out.extend_from_slice(&lock.position.to_le_bytes());
                out.extend_from_slice(&lock.term.to_le_bytes());
                out.extend_from_slice(&lock.effect.0.to_le_bytes());
            }
            out.push(u8::from(value.complete));
            Ok((PAYLOAD_ANTI_ENTROPY_CHUNK, out))
        }
        ReplicationTransportPayload::Heartbeat(value) => {
            let mut out = Vec::with_capacity(24);
            out.extend_from_slice(&value.membership_epoch.to_le_bytes());
            out.extend_from_slice(&value.term.to_le_bytes());
            out.extend_from_slice(&value.logical_tick.to_le_bytes());
            Ok((PAYLOAD_HEARTBEAT, out))
        }
    }
}

pub(super) fn decode_payload(
    kind: u8,
    bytes: &[u8],
) -> Result<ReplicationTransportPayload, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    let result = match kind {
        PAYLOAD_PEER_EVIDENCE => {
            return decode_signed_peer_evidence(bytes)
                .map(ReplicationTransportPayload::PeerEvidence)
                .map_err(codec);
        }
        PAYLOAD_ANTI_ENTROPY_SUMMARY => {
            let membership_epoch = cursor.u64().map_err(codec)?;
            let term = cursor.u64().map_err(codec)?;
            let lock_count = cursor.u64().map_err(codec)?;
            let highest_position = decode_option_u64(&mut cursor)?;
            let lock_digest = Sha256Digest(
                cursor
                    .take(32)
                    .map_err(codec)?
                    .try_into()
                    .map_err(|_| protocol("replication anti-entropy digest decode"))?,
            );
            ReplicationTransportPayload::AntiEntropySummary(ReplicationAntiEntropySummary {
                membership_epoch,
                term,
                lock_count,
                highest_position,
                lock_digest,
            })
        }
        PAYLOAD_ANTI_ENTROPY_REQUEST => {
            ReplicationTransportPayload::AntiEntropyRequest(ReplicationAntiEntropyRequest {
                membership_epoch: cursor.u64().map_err(codec)?,
                from_position: cursor.u64().map_err(codec)?,
                max_locks: cursor.u32().map_err(codec)?,
            })
        }
        PAYLOAD_ANTI_ENTROPY_CHUNK => {
            let membership_epoch = cursor.u64().map_err(codec)?;
            let count = cursor.u32().map_err(codec)? as usize;
            if count > MAX_ANTI_ENTROPY_LOCKS {
                return Err(protocol("replication anti-entropy chunk exceeds bound"));
            }
            let mut locks = Vec::with_capacity(cursor.bounded_capacity(count));
            for _ in 0..count {
                locks.push(ReplicationLockSummary {
                    position: cursor.u64().map_err(codec)?,
                    term: cursor.u64().map_err(codec)?,
                    effect: kernel_change::RevisionEffectId(cursor.u128().map_err(codec)?),
                });
            }
            let complete = match cursor.u8().map_err(codec)? {
                0 => false,
                1 => true,
                _ => return Err(protocol("replication anti-entropy completion flag invalid")),
            };
            let chunk = ReplicationAntiEntropyChunk {
                membership_epoch,
                locks,
                complete,
            };
            validate_replication_anti_entropy_chunk(&chunk)?;
            ReplicationTransportPayload::AntiEntropyChunk(chunk)
        }
        PAYLOAD_HEARTBEAT => ReplicationTransportPayload::Heartbeat(ReplicationHeartbeat {
            membership_epoch: cursor.u64().map_err(codec)?,
            term: cursor.u64().map_err(codec)?,
            logical_tick: cursor.u64().map_err(codec)?,
        }),
        _ => return Err(protocol("unknown replication transport payload kind")),
    };
    cursor.finish().map_err(codec)?;
    Ok(result)
}

fn encode_option_u64(out: &mut Vec<u8>, value: Option<u64>) {
    match value {
        Some(value) => {
            out.push(1);
            out.extend_from_slice(&value.to_le_bytes());
        }
        None => out.push(0),
    }
}

fn decode_option_u64(cursor: &mut Cursor<'_>) -> Result<Option<u64>, DurabilityError> {
    match cursor.u8().map_err(codec)? {
        0 => Ok(None),
        1 => Ok(Some(cursor.u64().map_err(codec)?)),
        _ => Err(protocol("replication transport option flag invalid")),
    }
}

pub(super) fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

pub(super) fn codec(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}
