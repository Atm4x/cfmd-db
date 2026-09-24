use std::collections::{BTreeMap, BTreeSet};

use kernel_auth::{Sha256Digest, TrustRootSet, sha256};

use super::replication::{
    ReplicaId, ReplicationClusterId, ReplicationLockSummary, ReplicationMembership,
    ReplicationPeerAuthPolicy, ReplicationQuorumLoss, SignedReplicationPeerEvidence,
    decode_signed_peer_evidence, encode_signed_peer_evidence, evidence_voter,
};
use super::{Cursor, DurabilityError};

const TRANSPORT_DOMAIN: &[u8] = b"CFMD-REPLICATION-TRANSPORT-v1\0";
const TRANSPORT_MAGIC: [u8; 4] = *b"CFTR";
const TRANSPORT_VERSION: u16 = 1;
const PAYLOAD_PEER_EVIDENCE: u8 = 1;
const PAYLOAD_ANTI_ENTROPY_SUMMARY: u8 = 2;
const PAYLOAD_ANTI_ENTROPY_REQUEST: u8 = 3;
const PAYLOAD_ANTI_ENTROPY_CHUNK: u8 = 4;
const PAYLOAD_HEARTBEAT: u8 = 5;
pub const MAX_ANTI_ENTROPY_LOCKS: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationAntiEntropySummary {
    pub membership_epoch: u64,
    pub term: u64,
    pub lock_count: u64,
    pub highest_position: Option<u64>,
    pub lock_digest: Sha256Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationAntiEntropyRequest {
    pub membership_epoch: u64,
    pub from_position: u64,
    pub max_locks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationAntiEntropyChunk {
    pub membership_epoch: u64,
    pub locks: Vec<ReplicationLockSummary>,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationHeartbeat {
    pub membership_epoch: u64,
    pub term: u64,
    pub logical_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplicationTransportPayload {
    PeerEvidence(SignedReplicationPeerEvidence),
    AntiEntropySummary(ReplicationAntiEntropySummary),
    AntiEntropyRequest(ReplicationAntiEntropyRequest),
    AntiEntropyChunk(ReplicationAntiEntropyChunk),
    Heartbeat(ReplicationHeartbeat),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationTransportFrame {
    pub cluster: ReplicationClusterId,
    pub trust_epoch: u64,
    pub sender: ReplicaId,
    pub sequence: u64,
    pub payload: ReplicationTransportPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedReplicationTransportFrame {
    pub frame: ReplicationTransportFrame,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicationAntiEntropyRelation {
    InSync,
    ExchangeRequired,
    MembershipMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationTransportIngress {
    highest_sequence: BTreeMap<ReplicaId, u64>,
}

impl Default for ReplicationTransportIngress {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplicationTransportIngress {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            highest_sequence: BTreeMap::new(),
        }
    }

    pub fn accept(
        &mut self,
        policy: &ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
        signed: &SignedReplicationTransportFrame,
    ) -> Result<(), DurabilityError> {
        let frame = &signed.frame;
        if frame.sender.raw() == 0 || frame.sequence == 0 {
            return Err(protocol("replication transport sender/sequence is zero"));
        }
        if frame.cluster != policy.cluster || frame.trust_epoch != policy.trust_epoch {
            return Err(protocol(
                "replication transport cluster/trust epoch mismatch",
            ));
        }
        if trust.epoch() != policy.trust_epoch {
            return Err(protocol("replication transport trust root epoch mismatch"));
        }
        let expected_key = *policy
            .peer_keys
            .get(&frame.sender)
            .ok_or_else(|| protocol("replication transport sender is not authorized"))?;
        if let ReplicationTransportPayload::PeerEvidence(peer) = &frame.payload {
            if evidence_voter(&peer.evidence) != frame.sender {
                return Err(protocol(
                    "replication transport peer evidence sender mismatch",
                ));
            }
            if peer.trust_epoch != frame.trust_epoch || peer.signer != expected_key {
                return Err(protocol(
                    "replication transport nested peer evidence trust mismatch",
                ));
            }
        }
        if self
            .highest_sequence
            .get(&frame.sender)
            .is_some_and(|seen| frame.sequence <= *seen)
        {
            return Err(protocol(
                "replication transport replay/non-monotone sequence",
            ));
        }
        let message = replication_transport_signing_message(frame)?;
        trust
            .verify_message(expected_key, &message, &signed.signature)
            .map_err(|_| protocol("replication transport signature verification failed"))?;
        self.highest_sequence.insert(frame.sender, frame.sequence);
        Ok(())
    }

    /// Verifies a frame before allowing it to refresh failure-detector liveness.
    /// This prevents unauthenticated traffic from suppressing quorum-loss fences.
    pub fn accept_and_observe(
        &mut self,
        policy: &ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
        signed: &SignedReplicationTransportFrame,
        detector: &mut ReplicationFailureDetector,
    ) -> Result<(), DurabilityError> {
        self.accept(policy, trust, signed)?;
        detector.observe_authenticated(signed.frame.sender);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationFailureDetector {
    local: ReplicaId,
    timeout_ticks: u64,
    now: u64,
    last_seen: BTreeMap<ReplicaId, u64>,
}

impl ReplicationFailureDetector {
    pub fn new(local: ReplicaId, timeout_ticks: u64) -> Result<Self, DurabilityError> {
        if local.raw() == 0 || timeout_ticks == 0 {
            return Err(protocol(
                "replication failure detector configuration is invalid",
            ));
        }
        let mut last_seen = BTreeMap::new();
        last_seen.insert(local, 0);
        Ok(Self {
            local,
            timeout_ticks,
            now: 0,
            last_seen,
        })
    }

    pub fn advance_to(&mut self, tick: u64) -> Result<(), DurabilityError> {
        if tick < self.now {
            return Err(protocol("replication failure detector clock regressed"));
        }
        self.now = tick;
        self.last_seen.insert(self.local, tick);
        Ok(())
    }

    pub fn observe_authenticated(&mut self, sender: ReplicaId) {
        self.last_seen.insert(sender, self.now);
    }

    #[must_use]
    pub fn reachable_members(&self, membership: &ReplicationMembership) -> BTreeSet<ReplicaId> {
        membership
            .members
            .iter()
            .copied()
            .filter(|member| {
                self.last_seen
                    .get(member)
                    .is_some_and(|seen| self.now.saturating_sub(*seen) <= self.timeout_ticks)
            })
            .collect()
    }

    #[must_use]
    pub fn quorum_reachable(&self, membership: &ReplicationMembership) -> bool {
        self.reachable_members(membership).len() >= membership.quorum_size
    }

    #[must_use]
    pub fn quorum_loss_observation(
        &self,
        membership: &ReplicationMembership,
        observed_term: u64,
    ) -> Option<ReplicationQuorumLoss> {
        (!self.quorum_reachable(membership)).then_some(ReplicationQuorumLoss {
            membership_epoch: membership.epoch,
            observed_term,
        })
    }
}

pub fn replication_transport_signing_message(
    frame: &ReplicationTransportFrame,
) -> Result<Vec<u8>, DurabilityError> {
    if frame.trust_epoch == 0 || frame.sender.raw() == 0 || frame.sequence == 0 {
        return Err(protocol(
            "replication transport frame has zero authority coordinate",
        ));
    }
    let payload = encode_payload(&frame.payload)?;
    let digest = sha256(&payload);
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
    let (kind, payload) = encode_payload_with_kind(&signed.frame.payload)?;
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
    out.extend_from_slice(&payload);
    out.extend_from_slice(&signed.signature);
    Ok(out)
}

pub fn decode_signed_replication_transport_frame(
    bytes: &[u8],
) -> Result<SignedReplicationTransportFrame, DurabilityError> {
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
    let payload = decode_payload(kind, cursor.take(payload_len).map_err(codec)?)?;
    let signature = cursor
        .take(64)
        .map_err(codec)?
        .try_into()
        .map_err(|_| protocol("replication transport signature decode"))?;
    cursor.finish().map_err(codec)?;
    Ok(SignedReplicationTransportFrame {
        frame: ReplicationTransportFrame {
            cluster,
            trust_epoch,
            sender,
            sequence,
            payload,
        },
        signature,
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

fn decode_payload(kind: u8, bytes: &[u8]) -> Result<ReplicationTransportPayload, DurabilityError> {
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
            let mut locks = Vec::with_capacity(count);
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

fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

fn codec(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use ed25519_dalek::{Signer, SigningKey};
    use kernel_auth::{TrustRootSet, key_id};
    use kernel_change::RevisionEffectId;

    use super::*;

    fn signed_heartbeat() -> (
        ReplicationPeerAuthPolicy,
        TrustRootSet,
        SignedReplicationTransportFrame,
    ) {
        let signing = SigningKey::from_bytes(&[7_u8; 32]);
        let verifying = signing.verifying_key().to_bytes();
        let id = key_id(&verifying);
        let trust = TrustRootSet::bootstrap(3, &[verifying]).unwrap();
        let peer = ReplicaId::new(2);
        let policy = ReplicationPeerAuthPolicy {
            cluster: ReplicationClusterId([9; 32]),
            trust_epoch: 3,
            peer_keys: BTreeMap::from([(peer, id)]),
        };
        let frame = ReplicationTransportFrame {
            cluster: policy.cluster,
            trust_epoch: policy.trust_epoch,
            sender: peer,
            sequence: 1,
            payload: ReplicationTransportPayload::Heartbeat(ReplicationHeartbeat {
                membership_epoch: 4,
                term: 11,
                logical_tick: 7,
            }),
        };
        let signature = signing
            .sign(&replication_transport_signing_message(&frame).unwrap())
            .to_bytes();
        (
            policy,
            trust,
            SignedReplicationTransportFrame { frame, signature },
        )
    }

    #[test]
    fn transport_wire_is_canonical_authenticated_and_session_replay_safe() {
        let (policy, trust, signed) = signed_heartbeat();
        let bytes = encode_signed_replication_transport_frame(&signed).unwrap();
        let decoded = decode_signed_replication_transport_frame(&bytes).unwrap();
        assert_eq!(decoded, signed);
        let mut ingress = ReplicationTransportIngress::new();
        ingress.accept(&policy, &trust, &decoded).unwrap();
        assert!(ingress.accept(&policy, &trust, &decoded).is_err());

        let mut tampered = bytes;
        tampered[60] ^= 1;
        let decoded = decode_signed_replication_transport_frame(&tampered).unwrap();
        let mut ingress = ReplicationTransportIngress::new();
        assert!(ingress.accept(&policy, &trust, &decoded).is_err());
    }

    #[test]
    fn anti_entropy_detects_divergence_without_treating_digest_as_authority() {
        let locks = vec![
            ReplicationLockSummary {
                position: 1,
                term: 3,
                effect: RevisionEffectId(10),
            },
            ReplicationLockSummary {
                position: 2,
                term: 3,
                effect: RevisionEffectId(11),
            },
        ];
        let local = replication_anti_entropy_summary(4, 3, &locks).unwrap();
        let remote = replication_anti_entropy_summary(4, 3, &locks[..1]).unwrap();
        assert_eq!(
            compare_replication_anti_entropy(&local, &local),
            ReplicationAntiEntropyRelation::InSync
        );
        assert_eq!(
            compare_replication_anti_entropy(&local, &remote),
            ReplicationAntiEntropyRelation::ExchangeRequired
        );
        let other = replication_anti_entropy_summary(5, 3, &locks).unwrap();
        assert_eq!(
            compare_replication_anti_entropy(&local, &other),
            ReplicationAntiEntropyRelation::MembershipMismatch
        );
    }

    #[test]
    fn failure_detector_only_reports_loss_and_never_creates_recovery_authority() {
        let membership = ReplicationMembership {
            epoch: 4,
            members: BTreeSet::from([ReplicaId::new(1), ReplicaId::new(2), ReplicaId::new(3)]),
            quorum_size: 2,
        };
        let mut detector = ReplicationFailureDetector::new(ReplicaId::new(1), 5).unwrap();
        detector.observe_authenticated(ReplicaId::new(2));
        assert!(detector.quorum_reachable(&membership));
        detector.advance_to(6).unwrap();
        assert!(!detector.quorum_reachable(&membership));
        assert_eq!(
            detector.quorum_loss_observation(&membership, 9),
            Some(ReplicationQuorumLoss {
                membership_epoch: 4,
                observed_term: 9
            })
        );
        assert!(detector.advance_to(5).is_err());
    }
}
