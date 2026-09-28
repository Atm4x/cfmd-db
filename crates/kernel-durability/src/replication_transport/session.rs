use std::collections::{BTreeMap, BTreeSet};

use kernel_auth::{Sha256Digest, TrustRootSet};

use crate::binary_codec::Cursor;
use crate::replication::codec::{
    transport_decode_membership_successor_owner as decode_membership_successor_owner,
    transport_decode_recovery_lock_frontier_owner as decode_recovery_lock_frontier_owner,
    transport_decode_signed_membership_vote_ref as decode_signed_membership_vote_ref,
    transport_decode_signed_recovery_ack_ref as decode_signed_recovery_ack_ref,
    transport_encode_membership_successor_owner as encode_membership_successor_owner,
    transport_encode_recovery_lock_frontier_owner as encode_recovery_lock_frontier_owner,
    transport_encode_signed_membership_vote_ref as encode_signed_membership_vote_ref,
    transport_encode_signed_recovery_ack_ref as encode_signed_recovery_ack_ref,
    transport_evidence_voter as evidence_voter,
    transport_recovery_lock_frontier_digest as recovery_lock_frontier_digest,
    transport_validate_lock_summaries as validate_lock_summaries,
};
use crate::replication::{
    ReplicaId, ReplicationClusterId, ReplicationLockSummary, ReplicationMembership,
    ReplicationPeerAuthPolicy, ReplicationPeerEvidence, ReplicationQuorumLoss,
    SignedReplicationPeerEvidence, replication_membership_digest,
};
use crate::runtime::DurabilityError;

use super::codec::{
    DecodedPhysicalTransportFrame, PAYLOAD_MEMBERSHIP_VOTE_OWNER_REF, PAYLOAD_MEMBERSHIP_VOTE_REF,
    PAYLOAD_RECOVERY_ACK_OWNER_REF, PAYLOAD_RECOVERY_ACK_REF, codec, decode_payload,
    decode_physical_transport_frame, encode_signed_replication_transport_frame,
    encode_signed_replication_transport_physical, membership_vote_payload_from_reference, protocol,
    recovery_ack_payload_from_reference, replication_transport_signing_message,
};

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
pub struct ReplicationTransportEgress {
    recovery_frontiers: BTreeSet<Sha256Digest>,
    membership_successors: BTreeSet<Sha256Digest>,
}

impl Default for ReplicationTransportEgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplicationTransportEgress {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            recovery_frontiers: BTreeSet::new(),
            membership_successors: BTreeSet::new(),
        }
    }

    pub fn encode(
        &mut self,
        signed: &SignedReplicationTransportFrame,
    ) -> Result<Vec<u8>, DurabilityError> {
        let ReplicationTransportPayload::PeerEvidence(peer) = &signed.frame.payload else {
            return encode_signed_replication_transport_frame(signed);
        };
        match &peer.evidence {
            ReplicationPeerEvidence::RecoveryAck(ack) => {
                let digest = recovery_lock_frontier_digest(&ack.locks)?;
                let reference = encode_signed_recovery_ack_ref(peer, digest)?;
                let first_use = !self.recovery_frontiers.contains(&digest);
                let (kind, payload) = if first_use {
                    let owner =
                        encode_recovery_lock_frontier_owner(&ack.locks, digest).map_err(|_| {
                            protocol("replication recovery frontier owner encode failed")
                        })?;
                    let owner_len = u32::try_from(owner.len())
                        .map_err(|_| protocol("replication recovery frontier owner too large"))?;
                    let mut payload = Vec::with_capacity(4 + owner.len() + reference.len());
                    payload.extend_from_slice(&owner_len.to_le_bytes());
                    payload.extend_from_slice(&owner);
                    payload.extend_from_slice(&reference);
                    (PAYLOAD_RECOVERY_ACK_OWNER_REF, payload)
                } else {
                    (PAYLOAD_RECOVERY_ACK_REF, reference)
                };
                let bytes = encode_signed_replication_transport_physical(signed, kind, &payload)?;
                if first_use {
                    self.recovery_frontiers.insert(digest);
                }
                Ok(bytes)
            }
            ReplicationPeerEvidence::MembershipVote(vote) => {
                let digest = replication_membership_digest(&vote.next)?;
                let reference = encode_signed_membership_vote_ref(peer, digest)?;
                let first_use = !self.membership_successors.contains(&digest);
                let (kind, payload) = if first_use {
                    let owner =
                        encode_membership_successor_owner(&vote.next, digest).map_err(|_| {
                            protocol("replication membership successor owner encode failed")
                        })?;
                    let owner_len = u32::try_from(owner.len()).map_err(|_| {
                        protocol("replication membership successor owner too large")
                    })?;
                    let mut payload = Vec::with_capacity(4 + owner.len() + reference.len());
                    payload.extend_from_slice(&owner_len.to_le_bytes());
                    payload.extend_from_slice(&owner);
                    payload.extend_from_slice(&reference);
                    (PAYLOAD_MEMBERSHIP_VOTE_OWNER_REF, payload)
                } else {
                    (PAYLOAD_MEMBERSHIP_VOTE_REF, reference)
                };
                let bytes = encode_signed_replication_transport_physical(signed, kind, &payload)?;
                if first_use {
                    self.membership_successors.insert(digest);
                }
                Ok(bytes)
            }
            _ => encode_signed_replication_transport_frame(signed),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationTransportIngress {
    highest_sequence: BTreeMap<ReplicaId, u64>,
    recovery_frontiers: BTreeMap<Sha256Digest, Vec<ReplicationLockSummary>>,
    membership_successors: BTreeMap<Sha256Digest, ReplicationMembership>,
}

enum PendingTransportOwner {
    Recovery(Sha256Digest, Vec<ReplicationLockSummary>),
    Membership(Sha256Digest, ReplicationMembership),
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
            recovery_frontiers: BTreeMap::new(),
            membership_successors: BTreeMap::new(),
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

    pub fn accept_wire(
        &mut self,
        policy: &ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
        bytes: &[u8],
    ) -> Result<SignedReplicationTransportFrame, DurabilityError> {
        let DecodedPhysicalTransportFrame {
            header,
            kind,
            payload,
            signature,
        } = decode_physical_transport_frame(bytes)?;
        let (payload, pending_owner) = self.decode_session_payload(kind, payload)?;
        self.validate_pending_owner(pending_owner.as_ref())?;
        let signed = SignedReplicationTransportFrame {
            frame: ReplicationTransportFrame {
                cluster: header.cluster,
                trust_epoch: header.trust_epoch,
                sender: header.sender,
                sequence: header.sequence,
                payload,
            },
            signature,
        };
        self.accept(policy, trust, &signed)?;
        self.publish_pending_owner(pending_owner);
        Ok(signed)
    }

    fn decode_session_payload(
        &self,
        kind: u8,
        payload: &[u8],
    ) -> Result<(ReplicationTransportPayload, Option<PendingTransportOwner>), DurabilityError> {
        match kind {
            PAYLOAD_RECOVERY_ACK_OWNER_REF => Self::decode_recovery_owner_ref(payload),
            PAYLOAD_RECOVERY_ACK_REF => {
                let reference = decode_signed_recovery_ack_ref(payload).map_err(codec)?;
                let locks = self
                    .recovery_frontiers
                    .get(&reference.lock_frontier_digest)
                    .ok_or_else(|| protocol("replication recovery frontier owner is missing"))?;
                Ok((recovery_ack_payload_from_reference(reference, locks), None))
            }
            PAYLOAD_MEMBERSHIP_VOTE_OWNER_REF => Self::decode_membership_owner_ref(payload),
            PAYLOAD_MEMBERSHIP_VOTE_REF => {
                let reference = decode_signed_membership_vote_ref(payload).map_err(codec)?;
                let successor = self
                    .membership_successors
                    .get(&reference.successor_digest)
                    .ok_or_else(|| protocol("replication membership successor owner is missing"))?;
                Ok((
                    membership_vote_payload_from_reference(reference, successor),
                    None,
                ))
            }
            _ => Ok((decode_payload(kind, payload)?, None)),
        }
    }

    fn decode_recovery_owner_ref(
        payload: &[u8],
    ) -> Result<(ReplicationTransportPayload, Option<PendingTransportOwner>), DurabilityError> {
        let mut cursor = Cursor::new(payload);
        let owner_len = cursor.u32().map_err(codec)? as usize;
        let (digest, locks) =
            decode_recovery_lock_frontier_owner(cursor.take(owner_len).map_err(codec)?)
                .map_err(codec)?;
        validate_lock_summaries(&locks)?;
        if recovery_lock_frontier_digest(&locks)? != digest {
            return Err(protocol(
                "replication recovery frontier owner digest mismatch",
            ));
        }
        let reference_len = payload
            .len()
            .checked_sub(4 + owner_len)
            .ok_or_else(|| protocol("replication recovery reference length underflow"))?;
        let reference = decode_signed_recovery_ack_ref(cursor.take(reference_len).map_err(codec)?)
            .map_err(codec)?;
        cursor.finish().map_err(codec)?;
        if reference.lock_frontier_digest != digest {
            return Err(protocol("replication recovery reference owner mismatch"));
        }
        Ok((
            recovery_ack_payload_from_reference(reference, &locks),
            Some(PendingTransportOwner::Recovery(digest, locks)),
        ))
    }

    fn decode_membership_owner_ref(
        payload: &[u8],
    ) -> Result<(ReplicationTransportPayload, Option<PendingTransportOwner>), DurabilityError> {
        let mut cursor = Cursor::new(payload);
        let owner_len = cursor.u32().map_err(codec)? as usize;
        let (digest, successor) =
            decode_membership_successor_owner(cursor.take(owner_len).map_err(codec)?)
                .map_err(codec)?;
        if replication_membership_digest(&successor)? != digest {
            return Err(protocol(
                "replication membership successor owner digest mismatch",
            ));
        }
        let reference_len = payload
            .len()
            .checked_sub(4 + owner_len)
            .ok_or_else(|| protocol("replication membership reference length underflow"))?;
        let reference =
            decode_signed_membership_vote_ref(cursor.take(reference_len).map_err(codec)?)
                .map_err(codec)?;
        cursor.finish().map_err(codec)?;
        if reference.successor_digest != digest {
            return Err(protocol("replication membership reference owner mismatch"));
        }
        Ok((
            membership_vote_payload_from_reference(reference, &successor),
            Some(PendingTransportOwner::Membership(digest, successor)),
        ))
    }

    fn validate_pending_owner(
        &self,
        pending: Option<&PendingTransportOwner>,
    ) -> Result<(), DurabilityError> {
        match pending {
            Some(PendingTransportOwner::Recovery(digest, locks))
                if self
                    .recovery_frontiers
                    .get(digest)
                    .is_some_and(|existing| existing != locks) =>
            {
                Err(protocol("replication recovery frontier digest collision"))
            }
            Some(PendingTransportOwner::Membership(digest, successor))
                if self
                    .membership_successors
                    .get(digest)
                    .is_some_and(|existing| existing != successor) =>
            {
                Err(protocol(
                    "replication membership successor digest collision",
                ))
            }
            _ => Ok(()),
        }
    }

    fn publish_pending_owner(&mut self, pending: Option<PendingTransportOwner>) {
        match pending {
            Some(PendingTransportOwner::Recovery(digest, locks)) => {
                self.recovery_frontiers.entry(digest).or_insert(locks);
            }
            Some(PendingTransportOwner::Membership(digest, successor)) => {
                self.membership_successors
                    .entry(digest)
                    .or_insert(successor);
            }
            None => {}
        }
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
