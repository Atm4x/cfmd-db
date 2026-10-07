use std::collections::{BTreeMap, BTreeSet};

use kernel_auth::{KeyId, Sha256Digest};
use kernel_change::RevisionEffectId;
use kernel_types::RevisionId;
use sha2::{Digest, Sha256};

use super::ReplicationAuthoritySemanticSnapshot;
use crate::binary_codec::{Cursor, push_len, push_u64, push_u128};
use crate::replication::codec::{
    KIND_SEMANTIC_AUTHORITY_BASE_BEGIN, KIND_SEMANTIC_AUTHORITY_BASE_END,
    KIND_SEMANTIC_AUTHORITY_BASE_RECORD, decode_decision_lock, decode_decision_vote, decode_ingest,
    decode_joint_membership_certificate, decode_leader_certificate,
    decode_membership_successor_owner, decode_peer_auth_policy, decode_quorum_certificate,
    decode_recovery_lock_frontier_owner, encode_decision_lock, encode_decision_vote, encode_ingest,
    encode_joint_membership_certificate, encode_leader_certificate,
    encode_membership_successor_owner, encode_peer_auth_policy, encode_quorum_certificate,
    encode_recovery_lock_frontier_owner,
};
use crate::replication::{
    DurableSequencerOrder, ReplicaId, ReplicationAuthenticationReceipt, ReplicationBranchHead,
    ReplicationBranchId, ReplicationJointMembershipAck, ReplicationQuorumAvailability,
    ReplicationQuorumLoss, replication_membership_digest,
};
use crate::runtime::{CodecError, DurabilityError};

pub(super) const CARRIER_VERSION: u8 = 2;

const RECORD_EFFECT: u8 = 1;
const RECORD_BRANCH: u8 = 2;
const RECORD_REVISION_FRONTIER: u8 = 3;
const RECORD_ORDERED_SLOT: u8 = 4;
const RECORD_SEQUENCER_EPOCH: u8 = 5;
const RECORD_MEMBERSHIP: u8 = 6;
const RECORD_CURRENT_MEMBERSHIP: u8 = 7;
const RECORD_QUORUM_CERTIFICATE: u8 = 8;
const RECORD_EFFECT_VOTE: u8 = 9;
const RECORD_MEMBERSHIP_VOTE: u8 = 10;
const RECORD_MEMBERSHIP_SUCCESSOR: u8 = 11;
const RECORD_PROMISED_TERM: u8 = 12;
const RECORD_HIGHEST_PROMISED_TERM: u8 = 13;
const RECORD_LEADER_VOTE: u8 = 14;
const RECORD_LEADER_CERTIFICATE: u8 = 15;
const RECORD_DECISION_VOTE: u8 = 16;
const RECORD_DECISION_LOCK: u8 = 17;
const RECORD_HIGHEST_DECISION_LOCK_TERM: u8 = 18;
const RECORD_JOINT_MEMBERSHIP_CERTIFICATE: u8 = 19;
const RECORD_PEER_AUTH_POLICY: u8 = 20;
const RECORD_AUTHENTICATED_EVIDENCE: u8 = 21;
const RECORD_AUTHENTICATED_EVIDENCE_INDEX: u8 = 22;
const RECORD_JOINT_MEMBERSHIP_ACK: u8 = 23;
const RECORD_RECOVERY_ACK: u8 = 24;
const RECORD_RECOVERY_LOCK_FRONTIER: u8 = 25;
const RECORD_QUORUM_AVAILABILITY: u8 = 26;
const RECORD_PUBLISHED_EFFECT: u8 = 27;
const RECORD_PUBLISHED_BRANCH: u8 = 28;

const SINGLETON_CURRENT_MEMBERSHIP: u8 = 1 << 0;
const SINGLETON_HIGHEST_DECISION_LOCK_TERM: u8 = 1 << 1;
const SINGLETON_PEER_AUTH_POLICY: u8 = 1 << 2;
const SINGLETON_QUORUM_AVAILABILITY: u8 = 1 << 3;
const REQUIRED_SINGLETONS: u8 = SINGLETON_CURRENT_MEMBERSHIP
    | SINGLETON_HIGHEST_DECISION_LOCK_TERM
    | SINGLETON_PEER_AUTH_POLICY
    | SINGLETON_QUORUM_AVAILABILITY;

fn push_digest(out: &mut Vec<u8>, digest: Sha256Digest) {
    out.extend_from_slice(&digest.0);
}

fn take_digest(cursor: &mut Cursor<'_>) -> Result<Sha256Digest, &'static str> {
    Ok(Sha256Digest(
        cursor
            .take(32)?
            .try_into()
            .map_err(|_| "sha256 digest decode")?,
    ))
}

fn push_key(out: &mut Vec<u8>, key: KeyId) {
    out.extend_from_slice(&key.0);
}

fn take_key(cursor: &mut Cursor<'_>) -> Result<KeyId, &'static str> {
    Ok(KeyId(
        cursor.take(32)?.try_into().map_err(|_| "key id decode")?,
    ))
}

fn insert_unique<K: Ord, V>(
    map: &mut BTreeMap<K, V>,
    key: K,
    value: V,
    reason: &'static str,
) -> Result<(), &'static str> {
    if map.insert(key, value).is_some() {
        Err(reason)
    } else {
        Ok(())
    }
}

fn decode_bool(cursor: &mut Cursor<'_>, reason: &'static str) -> Result<bool, &'static str> {
    match cursor.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(reason),
    }
}

fn singleton_once(seen: &mut u8, bit: u8, reason: &'static str) -> Result<(), &'static str> {
    if *seen & bit != 0 {
        Err(reason)
    } else {
        *seen |= bit;
        Ok(())
    }
}

fn emit_record(
    emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    record: &[u8],
) -> Result<(), DurabilityError> {
    if record.len() > crate::MAX_PAYLOAD_LEN {
        return Err(DurabilityError::PayloadTooLarge);
    }
    emit(record)
}

fn emit_blob_record(
    emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    tag: u8,
    blob: Vec<u8>,
) -> Result<(), DurabilityError> {
    let mut record = Vec::new();
    record
        .try_reserve_exact(1_usize.saturating_add(blob.len()))
        .map_err(|_| DurabilityError::PayloadTooLarge)?;
    record.push(tag);
    record.extend(blob);
    emit_record(emit, &record)
}

impl ReplicationAuthoritySemanticSnapshot {
    pub(super) fn record_count(&self) -> Result<usize, DurabilityError> {
        let mut count = 4_usize;
        for len in [
            self.effects.len(),
            self.branches.len(),
            self.revision_frontiers.len(),
            self.ordered_slots.len(),
            self.sequencer_epochs.len(),
            self.memberships.len(),
            self.quorum_certificates.len(),
            self.effect_votes.len(),
            self.membership_votes.len(),
            self.membership_vote_successors.len(),
            self.promised_terms.len(),
            self.highest_promised_terms.len(),
            self.leader_votes.len(),
            self.leader_certificates.len(),
            self.decision_votes.len(),
            self.decision_locks.len(),
            self.joint_membership_certificates.len(),
            self.authenticated_evidence.len(),
            self.authenticated_evidence_index.len(),
            self.joint_membership_acks.len(),
            self.recovery_acks.len(),
            self.recovery_lock_frontiers.len(),
            self.published_effects.len(),
            self.published_branches.len(),
        ] {
            count = count
                .checked_add(len)
                .ok_or(DurabilityError::PayloadTooLarge)?;
        }
        Ok(count)
    }

    pub(super) fn for_each_record(
        &self,
        mut emit: impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        self.emit_causal_records(&mut emit)?;
        self.emit_membership_records(&mut emit)?;
        self.emit_election_records(&mut emit)?;
        self.emit_decision_records(&mut emit)?;
        self.emit_security_records(&mut emit)?;
        self.emit_recovery_records(&mut emit)?;
        self.emit_publication_records(&mut emit)
    }

    fn emit_causal_records(
        &self,
        emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        for (id, envelope) in &self.effects {
            if envelope.effect.id != *id {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replication semantic effect key disagrees with envelope",
                });
            }
            emit_blob_record(emit, RECORD_EFFECT, encode_ingest(envelope)?)?;
        }
        for (id, head) in &self.branches {
            if head.branch != *id {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replication semantic branch key disagrees with head",
                });
            }
            let mut record = vec![RECORD_BRANCH];
            push_u128(&mut record, id.raw());
            push_u64(&mut record, head.head_revision.raw());
            push_u128(&mut record, head.head_effect.0);
            record.push(u8::from(head.retired));
            emit_record(emit, &record)?;
        }
        for (revision, frontier) in &self.revision_frontiers {
            let mut record = vec![RECORD_REVISION_FRONTIER];
            push_u64(&mut record, revision.raw());
            push_len(&mut record, frontier.len())?;
            for effect in frontier {
                push_u128(&mut record, effect.0);
            }
            emit_record(emit, &record)?;
        }
        Ok(())
    }

    fn emit_membership_records(
        &self,
        emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        for (order, effect) in &self.ordered_slots {
            let mut record = vec![RECORD_ORDERED_SLOT];
            push_u64(&mut record, order.sequencer.raw());
            push_u64(&mut record, order.epoch);
            push_u64(&mut record, order.position);
            push_u128(&mut record, effect.0);
            emit_record(emit, &record)?;
        }
        for (replica, epoch) in &self.sequencer_epochs {
            let mut record = vec![RECORD_SEQUENCER_EPOCH];
            push_u64(&mut record, replica.raw());
            push_u64(&mut record, *epoch);
            emit_record(emit, &record)?;
        }
        for membership in self.memberships.values() {
            let digest = replication_membership_digest(membership)?;
            emit_blob_record(
                emit,
                RECORD_MEMBERSHIP,
                encode_membership_successor_owner(membership, digest)?,
            )?;
        }
        let mut current = vec![RECORD_CURRENT_MEMBERSHIP];
        match self.current_membership_epoch {
            Some(epoch) => {
                current.push(1);
                push_u64(&mut current, epoch);
            }
            None => current.push(0),
        }
        emit_record(emit, &current)?;
        for certificate in self.quorum_certificates.values() {
            emit_blob_record(
                emit,
                RECORD_QUORUM_CERTIFICATE,
                encode_quorum_certificate(certificate)?,
            )?;
        }
        for ((epoch, position, voter), effect) in &self.effect_votes {
            let mut record = vec![RECORD_EFFECT_VOTE];
            push_u64(&mut record, *epoch);
            push_u64(&mut record, *position);
            push_u64(&mut record, voter.raw());
            push_u128(&mut record, effect.0);
            emit_record(emit, &record)?;
        }
        for ((epoch, voter), index) in &self.membership_votes {
            let mut record = vec![RECORD_MEMBERSHIP_VOTE];
            push_u64(&mut record, *epoch);
            push_u64(&mut record, voter.raw());
            push_u64(&mut record, index.term);
            push_digest(&mut record, index.successor_digest);
            push_digest(&mut record, index.payload_digest);
            emit_record(emit, &record)?;
        }
        for (digest, membership) in &self.membership_vote_successors {
            emit_blob_record(
                emit,
                RECORD_MEMBERSHIP_SUCCESSOR,
                encode_membership_successor_owner(membership, *digest)?,
            )?;
        }
        Ok(())
    }

    fn emit_election_records(
        &self,
        emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        for ((epoch, voter), term) in &self.promised_terms {
            let mut record = vec![RECORD_PROMISED_TERM];
            push_u64(&mut record, *epoch);
            push_u64(&mut record, voter.raw());
            push_u64(&mut record, *term);
            emit_record(emit, &record)?;
        }
        for (epoch, term) in &self.highest_promised_terms {
            let mut record = vec![RECORD_HIGHEST_PROMISED_TERM];
            push_u64(&mut record, *epoch);
            push_u64(&mut record, *term);
            emit_record(emit, &record)?;
        }
        for ((epoch, term, voter), candidate) in &self.leader_votes {
            let mut record = vec![RECORD_LEADER_VOTE];
            push_u64(&mut record, *epoch);
            push_u64(&mut record, *term);
            push_u64(&mut record, voter.raw());
            push_u64(&mut record, candidate.raw());
            emit_record(emit, &record)?;
        }
        for certificate in self.leader_certificates.values() {
            emit_blob_record(
                emit,
                RECORD_LEADER_CERTIFICATE,
                encode_leader_certificate(certificate)?,
            )?;
        }
        Ok(())
    }

    fn emit_decision_records(
        &self,
        emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        for vote in self.decision_votes.values() {
            emit_blob_record(emit, RECORD_DECISION_VOTE, encode_decision_vote(vote))?;
        }
        for lock in self.decision_locks.values() {
            emit_blob_record(emit, RECORD_DECISION_LOCK, encode_decision_lock(lock)?)?;
        }
        let mut highest = vec![RECORD_HIGHEST_DECISION_LOCK_TERM];
        push_u64(&mut highest, self.highest_decision_lock_term);
        emit_record(emit, &highest)?;
        for certificate in self.joint_membership_certificates.values() {
            emit_blob_record(
                emit,
                RECORD_JOINT_MEMBERSHIP_CERTIFICATE,
                encode_joint_membership_certificate(certificate)?,
            )?;
        }
        Ok(())
    }

    fn emit_security_records(
        &self,
        emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        let mut policy_record = vec![RECORD_PEER_AUTH_POLICY];
        match &self.peer_auth_policy {
            Some(policy) => {
                policy_record.push(1);
                policy_record.extend_from_slice(&encode_peer_auth_policy(policy)?);
            }
            None => policy_record.push(0),
        }
        emit_record(emit, &policy_record)?;
        for (digest, receipt) in &self.authenticated_evidence {
            let mut record = vec![RECORD_AUTHENTICATED_EVIDENCE];
            push_digest(&mut record, *digest);
            push_digest(&mut record, receipt.proof_digest);
            push_digest(&mut record, receipt.payload_digest);
            push_u64(&mut record, receipt.trust_epoch);
            push_u64(&mut record, receipt.voter.raw());
            push_key(&mut record, receipt.signer);
            emit_record(emit, &record)?;
        }
        for ((epoch, voter, digest), key) in &self.authenticated_evidence_index {
            let mut record = vec![RECORD_AUTHENTICATED_EVIDENCE_INDEX];
            push_u64(&mut record, *epoch);
            push_u64(&mut record, voter.raw());
            push_digest(&mut record, *digest);
            push_key(&mut record, *key);
            emit_record(emit, &record)?;
        }
        Ok(())
    }

    fn emit_recovery_records(
        &self,
        emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        for ((previous, next, voter), ack) in &self.joint_membership_acks {
            let mut record = vec![RECORD_JOINT_MEMBERSHIP_ACK];
            push_u64(&mut record, *previous);
            push_u64(&mut record, *next);
            push_u64(&mut record, voter.raw());
            push_u64(&mut record, ack.term);
            push_u64(&mut record, ack.leader.raw());
            push_digest(&mut record, ack.next_membership_digest);
            emit_record(emit, &record)?;
        }
        for ((epoch, term, voter), ack) in &self.recovery_acks {
            let mut record = vec![RECORD_RECOVERY_ACK];
            push_u64(&mut record, *epoch);
            push_u64(&mut record, *term);
            push_u64(&mut record, voter.raw());
            push_u64(&mut record, ack.leader.raw());
            push_u64(&mut record, ack.trust_epoch);
            push_digest(&mut record, ack.payload_digest);
            push_digest(&mut record, ack.lock_frontier_digest);
            emit_record(emit, &record)?;
        }
        for (digest, locks) in &self.recovery_lock_frontiers {
            emit_blob_record(
                emit,
                RECORD_RECOVERY_LOCK_FRONTIER,
                encode_recovery_lock_frontier_owner(locks, *digest)?,
            )?;
        }
        let mut availability = vec![RECORD_QUORUM_AVAILABILITY];
        match self.quorum_availability {
            None => availability.push(0),
            Some(ReplicationQuorumAvailability::Available {
                membership_epoch,
                term,
            }) => {
                availability.push(1);
                push_u64(&mut availability, membership_epoch);
                push_u64(&mut availability, term);
            }
            Some(ReplicationQuorumAvailability::Lost(ReplicationQuorumLoss {
                membership_epoch,
                observed_term,
            })) => {
                availability.push(2);
                push_u64(&mut availability, membership_epoch);
                push_u64(&mut availability, observed_term);
            }
        }
        emit_record(emit, &availability)
    }

    fn emit_publication_records(
        &self,
        emit: &mut impl FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        for effect in &self.published_effects {
            let mut record = vec![RECORD_PUBLISHED_EFFECT];
            push_u128(&mut record, effect.0);
            emit_record(emit, &record)?;
        }
        for (id, head) in &self.published_branches {
            if head.branch != *id {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "published replication branch key disagrees with head",
                });
            }
            let mut record = vec![RECORD_PUBLISHED_BRANCH];
            push_u128(&mut record, id.raw());
            push_u64(&mut record, head.head_revision.raw());
            push_u128(&mut record, head.head_effect.0);
            record.push(u8::from(head.retired));
            emit_record(emit, &record)?;
        }
        Ok(())
    }

    fn empty() -> Self {
        Self {
            effects: BTreeMap::new(),
            branches: BTreeMap::new(),
            revision_frontiers: BTreeMap::new(),
            ordered_slots: BTreeMap::new(),
            sequencer_epochs: BTreeMap::new(),
            memberships: BTreeMap::new(),
            current_membership_epoch: None,
            quorum_certificates: BTreeMap::new(),
            effect_votes: BTreeMap::new(),
            membership_votes: BTreeMap::new(),
            membership_vote_successors: BTreeMap::new(),
            promised_terms: BTreeMap::new(),
            highest_promised_terms: BTreeMap::new(),
            leader_votes: BTreeMap::new(),
            leader_certificates: BTreeMap::new(),
            decision_votes: BTreeMap::new(),
            decision_locks: BTreeMap::new(),
            highest_decision_lock_term: 0,
            joint_membership_certificates: BTreeMap::new(),
            peer_auth_policy: None,
            authenticated_evidence: BTreeMap::new(),
            authenticated_evidence_index: BTreeMap::new(),
            joint_membership_acks: BTreeMap::new(),
            recovery_acks: BTreeMap::new(),
            recovery_lock_frontiers: BTreeMap::new(),
            quorum_availability: None,
            published_effects: BTreeSet::new(),
            published_branches: BTreeMap::new(),
        }
    }

    pub(crate) fn for_each_semantic_base_frame(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        let expected_records = self.record_count()?;
        let begin = encode_begin(expected_records)?;
        let begin_frame = super::super::journal_io::encode_replication_frame(
            KIND_SEMANTIC_AUTHORITY_BASE_BEGIN,
            &begin,
        )?;
        emit(&begin_frame)?;

        let mut hasher = Sha256::new();
        let mut record_count = 0_usize;
        let mut canonical_len = 0_u64;
        self.for_each_record(|record| {
            let len = u64::try_from(record.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
            hasher.update(len.to_le_bytes());
            hasher.update(record);
            canonical_len = canonical_len
                .checked_add(8)
                .and_then(|value| value.checked_add(len))
                .ok_or(DurabilityError::PayloadTooLarge)?;
            record_count = record_count
                .checked_add(1)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            let frame = super::super::journal_io::encode_replication_frame(
                KIND_SEMANTIC_AUTHORITY_BASE_RECORD,
                record,
            )?;
            emit(&frame)
        })?;
        if record_count != expected_records {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication semantic carrier record count changed during encoding",
            });
        }
        let digest: [u8; 32] = hasher.finalize().into();
        let end = encode_end(canonical_len, digest);
        let end_frame = super::super::journal_io::encode_replication_frame(
            KIND_SEMANTIC_AUTHORITY_BASE_END,
            &end,
        )?;
        emit(&end_frame)
    }

    pub(crate) fn install_without_physical_carrier(
        &self,
        journal: &mut super::ReplicationAuthorityJournal,
    ) -> Result<(), DurabilityError> {
        if !journal.is_empty_authority() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication semantic carrier target is not empty",
            });
        }
        self.clone().restore(journal);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn install_as_semantic_base(
        &self,
        journal: &mut super::ReplicationAuthorityJournal,
    ) -> Result<(), DurabilityError> {
        if !journal.is_empty_authority() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication semantic carrier target is not empty",
            });
        }
        let expected_records = self.record_count()?;
        let begin = encode_begin(expected_records)?;
        journal.append_frame(KIND_SEMANTIC_AUTHORITY_BASE_BEGIN, &begin)?;

        let mut hasher = Sha256::new();
        let mut record_count = 0_usize;
        let mut canonical_len = 0_u64;
        self.for_each_record(|record| {
            let len = u64::try_from(record.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
            hasher.update(len.to_le_bytes());
            hasher.update(record);
            canonical_len = canonical_len
                .checked_add(8)
                .and_then(|value| value.checked_add(len))
                .ok_or(DurabilityError::PayloadTooLarge)?;
            record_count = record_count
                .checked_add(1)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            journal.append_frame(KIND_SEMANTIC_AUTHORITY_BASE_RECORD, record)
        })?;
        if record_count != expected_records {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication semantic carrier record count changed during encoding",
            });
        }
        let digest: [u8; 32] = hasher.finalize().into();
        let end = encode_end(canonical_len, digest);
        journal.append_frame(KIND_SEMANTIC_AUTHORITY_BASE_END, &end)?;

        self.clone().restore(journal);
        let frames = journal.take_pending_single_file_frames();
        journal.commit_single_file_frames(frames);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn record_stats(&self) -> Result<(usize, u64, usize), DurabilityError> {
        let mut count = 0_usize;
        let mut canonical_len = 0_u64;
        let mut max_record = 0_usize;
        self.for_each_record(|record| {
            count = count
                .checked_add(1)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            let len = u64::try_from(record.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
            canonical_len = canonical_len
                .checked_add(8)
                .and_then(|value| value.checked_add(len))
                .ok_or(DurabilityError::PayloadTooLarge)?;
            max_record = max_record.max(record.len());
            Ok(())
        })?;
        Ok((count, canonical_len, max_record))
    }
}

#[derive(Debug)]
pub(in crate::replication::authority) struct SemanticAuthorityRecordDecoder {
    snapshot: ReplicationAuthoritySemanticSnapshot,
    seen_singletons: u8,
}

impl SemanticAuthorityRecordDecoder {
    pub(in crate::replication::authority) fn new() -> Self {
        Self {
            snapshot: ReplicationAuthoritySemanticSnapshot::empty(),
            seen_singletons: 0,
        }
    }

    pub(in crate::replication::authority) fn apply_record(
        &mut self,
        record: &[u8],
    ) -> Result<(), &'static str> {
        let (&tag, payload) = record
            .split_first()
            .ok_or("empty replication semantic authority record")?;
        match tag {
            RECORD_EFFECT..=RECORD_REVISION_FRONTIER => self.apply_causal_record(tag, payload),
            RECORD_ORDERED_SLOT..=RECORD_CURRENT_MEMBERSHIP => {
                self.apply_membership_state_record(tag, payload)
            }
            RECORD_QUORUM_CERTIFICATE..=RECORD_MEMBERSHIP_SUCCESSOR => {
                self.apply_membership_vote_record(tag, payload)
            }
            RECORD_PROMISED_TERM..=RECORD_LEADER_CERTIFICATE => {
                self.apply_election_record(tag, payload)
            }
            RECORD_DECISION_VOTE..=RECORD_JOINT_MEMBERSHIP_CERTIFICATE => {
                self.apply_decision_record(tag, payload)
            }
            RECORD_PEER_AUTH_POLICY..=RECORD_AUTHENTICATED_EVIDENCE_INDEX => {
                self.apply_security_record(tag, payload)
            }
            RECORD_JOINT_MEMBERSHIP_ACK..=RECORD_QUORUM_AVAILABILITY => {
                self.apply_recovery_record(tag, payload)
            }
            RECORD_PUBLISHED_EFFECT..=RECORD_PUBLISHED_BRANCH => {
                self.apply_publication_record(tag, payload)
            }
            _ => Err("unknown replication semantic authority record tag"),
        }
    }

    fn apply_causal_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), &'static str> {
        match tag {
            RECORD_EFFECT => {
                let envelope = decode_ingest(payload)?;
                let id = envelope.effect.id;
                insert_unique(
                    &mut self.snapshot.effects,
                    id,
                    envelope,
                    "duplicate replication effect",
                )
            }
            RECORD_BRANCH => {
                let mut c = Cursor::new(payload);
                let id = ReplicationBranchId::new(c.u128()?);
                let head = ReplicationBranchHead {
                    branch: id,
                    head_revision: RevisionId::new(c.u64()?),
                    head_effect: RevisionEffectId(c.u128()?),
                    retired: decode_bool(&mut c, "invalid branch retired flag")?,
                };
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.branches,
                    id,
                    head,
                    "duplicate replication branch",
                )
            }
            RECORD_REVISION_FRONTIER => {
                let mut c = Cursor::new(payload);
                let revision = RevisionId::new(c.u64()?);
                let mut frontier = BTreeSet::new();
                for _ in 0..c.len()? {
                    if !frontier.insert(RevisionEffectId(c.u128()?)) {
                        return Err("duplicate revision frontier effect");
                    }
                }
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.revision_frontiers,
                    revision,
                    frontier,
                    "duplicate revision frontier",
                )
            }
            _ => Err("invalid causal semantic authority record tag"),
        }
    }

    fn apply_membership_state_record(
        &mut self,
        tag: u8,
        payload: &[u8],
    ) -> Result<(), &'static str> {
        match tag {
            RECORD_ORDERED_SLOT => {
                let mut c = Cursor::new(payload);
                let order = DurableSequencerOrder {
                    sequencer: ReplicaId::new(c.u64()?),
                    epoch: c.u64()?,
                    position: c.u64()?,
                };
                let effect = RevisionEffectId(c.u128()?);
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.ordered_slots,
                    order,
                    effect,
                    "duplicate ordered slot",
                )
            }
            RECORD_SEQUENCER_EPOCH => {
                let mut c = Cursor::new(payload);
                let replica = ReplicaId::new(c.u64()?);
                let epoch = c.u64()?;
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.sequencer_epochs,
                    replica,
                    epoch,
                    "duplicate sequencer epoch",
                )
            }
            RECORD_MEMBERSHIP => {
                let (digest, membership) = decode_membership_successor_owner(payload)?;
                if replication_membership_digest(&membership)
                    .map_err(|_| "membership digest encode")?
                    != digest
                {
                    return Err("membership digest mismatch");
                }
                let epoch = membership.epoch;
                insert_unique(
                    &mut self.snapshot.memberships,
                    epoch,
                    membership,
                    "duplicate membership",
                )
            }
            RECORD_CURRENT_MEMBERSHIP => {
                singleton_once(
                    &mut self.seen_singletons,
                    SINGLETON_CURRENT_MEMBERSHIP,
                    "duplicate current membership record",
                )?;
                let mut c = Cursor::new(payload);
                self.snapshot.current_membership_epoch = match c.u8()? {
                    0 => None,
                    1 => Some(c.u64()?),
                    _ => return Err("invalid current membership tag"),
                };
                c.finish()
            }
            _ => Err("invalid membership-state semantic authority record tag"),
        }
    }

    fn apply_membership_vote_record(
        &mut self,
        tag: u8,
        payload: &[u8],
    ) -> Result<(), &'static str> {
        match tag {
            RECORD_QUORUM_CERTIFICATE => {
                let certificate = decode_quorum_certificate(payload)?;
                insert_unique(
                    &mut self.snapshot.quorum_certificates,
                    certificate.effect,
                    certificate,
                    "duplicate quorum certificate",
                )
            }
            RECORD_EFFECT_VOTE => {
                let mut c = Cursor::new(payload);
                let key = (c.u64()?, c.u64()?, ReplicaId::new(c.u64()?));
                let effect = RevisionEffectId(c.u128()?);
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.effect_votes,
                    key,
                    effect,
                    "duplicate effect vote",
                )
            }
            RECORD_MEMBERSHIP_VOTE => {
                let mut c = Cursor::new(payload);
                let key = (c.u64()?, ReplicaId::new(c.u64()?));
                let index = super::super::MembershipVoteIndex {
                    term: c.u64()?,
                    successor_digest: take_digest(&mut c)?,
                    payload_digest: take_digest(&mut c)?,
                };
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.membership_votes,
                    key,
                    index,
                    "duplicate membership vote",
                )
            }
            RECORD_MEMBERSHIP_SUCCESSOR => {
                let (digest, membership) = decode_membership_successor_owner(payload)?;
                insert_unique(
                    &mut self.snapshot.membership_vote_successors,
                    digest,
                    membership,
                    "duplicate membership successor",
                )
            }
            _ => Err("invalid membership-vote semantic authority record tag"),
        }
    }

    fn apply_election_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), &'static str> {
        match tag {
            RECORD_PROMISED_TERM => {
                let mut c = Cursor::new(payload);
                let key = (c.u64()?, ReplicaId::new(c.u64()?));
                let term = c.u64()?;
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.promised_terms,
                    key,
                    term,
                    "duplicate promised term",
                )
            }
            RECORD_HIGHEST_PROMISED_TERM => {
                let mut c = Cursor::new(payload);
                let epoch = c.u64()?;
                let term = c.u64()?;
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.highest_promised_terms,
                    epoch,
                    term,
                    "duplicate highest promised term",
                )
            }
            RECORD_LEADER_VOTE => {
                let mut c = Cursor::new(payload);
                let key = (c.u64()?, c.u64()?, ReplicaId::new(c.u64()?));
                let candidate = ReplicaId::new(c.u64()?);
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.leader_votes,
                    key,
                    candidate,
                    "duplicate leader vote",
                )
            }
            RECORD_LEADER_CERTIFICATE => {
                let certificate = decode_leader_certificate(payload)?;
                insert_unique(
                    &mut self.snapshot.leader_certificates,
                    (certificate.membership_epoch, certificate.term),
                    certificate,
                    "duplicate leader certificate",
                )
            }
            _ => Err("invalid election semantic authority record tag"),
        }
    }

    fn apply_decision_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), &'static str> {
        match tag {
            RECORD_DECISION_VOTE => {
                let vote = decode_decision_vote(payload)?;
                insert_unique(
                    &mut self.snapshot.decision_votes,
                    (vote.membership_epoch, vote.term, vote.position, vote.voter),
                    vote,
                    "duplicate decision vote",
                )
            }
            RECORD_DECISION_LOCK => {
                let lock = decode_decision_lock(payload)?;
                insert_unique(
                    &mut self.snapshot.decision_locks,
                    lock.position,
                    lock,
                    "duplicate decision lock",
                )
            }
            RECORD_HIGHEST_DECISION_LOCK_TERM => {
                singleton_once(
                    &mut self.seen_singletons,
                    SINGLETON_HIGHEST_DECISION_LOCK_TERM,
                    "duplicate highest decision lock term record",
                )?;
                let mut c = Cursor::new(payload);
                self.snapshot.highest_decision_lock_term = c.u64()?;
                c.finish()
            }
            RECORD_JOINT_MEMBERSHIP_CERTIFICATE => {
                let certificate = decode_joint_membership_certificate(payload)?;
                insert_unique(
                    &mut self.snapshot.joint_membership_certificates,
                    certificate.next.epoch,
                    certificate,
                    "duplicate joint membership certificate",
                )
            }
            _ => Err("invalid decision semantic authority record tag"),
        }
    }

    fn apply_security_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), &'static str> {
        match tag {
            RECORD_PEER_AUTH_POLICY => {
                singleton_once(
                    &mut self.seen_singletons,
                    SINGLETON_PEER_AUTH_POLICY,
                    "duplicate peer auth policy record",
                )?;
                let (&kind, encoded) = payload
                    .split_first()
                    .ok_or("truncated peer auth policy record")?;
                self.snapshot.peer_auth_policy = match kind {
                    0 if encoded.is_empty() => None,
                    1 => Some(decode_peer_auth_policy(encoded)?),
                    _ => return Err("invalid peer auth policy tag"),
                };
                Ok(())
            }
            RECORD_AUTHENTICATED_EVIDENCE => {
                let mut c = Cursor::new(payload);
                let digest = take_digest(&mut c)?;
                let receipt = ReplicationAuthenticationReceipt {
                    proof_digest: take_digest(&mut c)?,
                    payload_digest: take_digest(&mut c)?,
                    trust_epoch: c.u64()?,
                    voter: ReplicaId::new(c.u64()?),
                    signer: take_key(&mut c)?,
                };
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.authenticated_evidence,
                    digest,
                    receipt,
                    "duplicate authenticated evidence",
                )
            }
            RECORD_AUTHENTICATED_EVIDENCE_INDEX => {
                let mut c = Cursor::new(payload);
                let key = (c.u64()?, ReplicaId::new(c.u64()?), take_digest(&mut c)?);
                let signer = take_key(&mut c)?;
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.authenticated_evidence_index,
                    key,
                    signer,
                    "duplicate authenticated evidence index",
                )
            }
            _ => Err("invalid security semantic authority record tag"),
        }
    }

    fn apply_recovery_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), &'static str> {
        match tag {
            RECORD_JOINT_MEMBERSHIP_ACK => {
                let mut c = Cursor::new(payload);
                let previous = c.u64()?;
                let next = c.u64()?;
                let voter = ReplicaId::new(c.u64()?);
                let ack = ReplicationJointMembershipAck {
                    voter,
                    previous_membership_epoch: previous,
                    next_membership_epoch: next,
                    term: c.u64()?,
                    leader: ReplicaId::new(c.u64()?),
                    next_membership_digest: take_digest(&mut c)?,
                };
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.joint_membership_acks,
                    (previous, next, voter),
                    ack,
                    "duplicate joint membership ack",
                )
            }
            RECORD_RECOVERY_ACK => {
                let mut c = Cursor::new(payload);
                let key = (c.u64()?, c.u64()?, ReplicaId::new(c.u64()?));
                let ack = super::super::AuthenticatedRecoveryAckIndex {
                    leader: ReplicaId::new(c.u64()?),
                    trust_epoch: c.u64()?,
                    payload_digest: take_digest(&mut c)?,
                    lock_frontier_digest: take_digest(&mut c)?,
                };
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.recovery_acks,
                    key,
                    ack,
                    "duplicate recovery ack",
                )
            }
            RECORD_RECOVERY_LOCK_FRONTIER => {
                let (digest, locks) = decode_recovery_lock_frontier_owner(payload)?;
                insert_unique(
                    &mut self.snapshot.recovery_lock_frontiers,
                    digest,
                    locks,
                    "duplicate recovery lock frontier",
                )
            }
            RECORD_QUORUM_AVAILABILITY => {
                singleton_once(
                    &mut self.seen_singletons,
                    SINGLETON_QUORUM_AVAILABILITY,
                    "duplicate quorum availability record",
                )?;
                let mut c = Cursor::new(payload);
                self.snapshot.quorum_availability = match c.u8()? {
                    0 => None,
                    1 => Some(ReplicationQuorumAvailability::Available {
                        membership_epoch: c.u64()?,
                        term: c.u64()?,
                    }),
                    2 => Some(ReplicationQuorumAvailability::Lost(ReplicationQuorumLoss {
                        membership_epoch: c.u64()?,
                        observed_term: c.u64()?,
                    })),
                    _ => return Err("invalid quorum availability tag"),
                };
                c.finish()
            }
            _ => Err("invalid recovery semantic authority record tag"),
        }
    }

    fn apply_publication_record(&mut self, tag: u8, payload: &[u8]) -> Result<(), &'static str> {
        match tag {
            RECORD_PUBLISHED_EFFECT => {
                let mut c = Cursor::new(payload);
                let effect = RevisionEffectId(c.u128()?);
                c.finish()?;
                if self.snapshot.published_effects.insert(effect) {
                    Ok(())
                } else {
                    Err("duplicate published effect")
                }
            }
            RECORD_PUBLISHED_BRANCH => {
                let mut c = Cursor::new(payload);
                let id = ReplicationBranchId::new(c.u128()?);
                let head = ReplicationBranchHead {
                    branch: id,
                    head_revision: RevisionId::new(c.u64()?),
                    head_effect: RevisionEffectId(c.u128()?),
                    retired: decode_bool(&mut c, "invalid published branch retired flag")?,
                };
                c.finish()?;
                insert_unique(
                    &mut self.snapshot.published_branches,
                    id,
                    head,
                    "duplicate published branch",
                )
            }
            _ => Err("invalid publication semantic authority record tag"),
        }
    }

    pub(in crate::replication::authority) fn finish(
        self,
    ) -> Result<ReplicationAuthoritySemanticSnapshot, &'static str> {
        if self.seen_singletons != REQUIRED_SINGLETONS {
            return Err("replication semantic authority carrier is missing singleton state");
        }
        Ok(self.snapshot)
    }
}

#[cfg(test)]
pub(super) fn roundtrip_records(
    snapshot: &ReplicationAuthoritySemanticSnapshot,
) -> Result<ReplicationAuthoritySemanticSnapshot, DurabilityError> {
    let mut decoder = SemanticAuthorityRecordDecoder::new();
    snapshot.for_each_record(|record| {
        decoder
            .apply_record(record)
            .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })
    })?;
    decoder
        .finish()
        .map_err(|reason| DurabilityError::Protocol { offset: 0, reason })
}

pub(super) fn encode_begin(record_count: usize) -> Result<Vec<u8>, CodecError> {
    let mut begin = Vec::with_capacity(9);
    begin.push(CARRIER_VERSION);
    begin.extend_from_slice(
        &u64::try_from(record_count)
            .map_err(|_| CodecError::LengthOverflow)?
            .to_le_bytes(),
    );
    Ok(begin)
}

pub(in crate::replication::authority) fn decode_begin(
    payload: &[u8],
) -> Result<usize, &'static str> {
    if payload.len() != 9 || payload[0] != CARRIER_VERSION {
        return Err("unsupported replication semantic carrier header");
    }
    usize::try_from(u64::from_le_bytes(
        payload[1..9]
            .try_into()
            .map_err(|_| "semantic carrier record count decode")?,
    ))
    .map_err(|_| "semantic carrier record count overflow")
}

pub(super) fn encode_end(canonical_len: u64, digest: [u8; 32]) -> Vec<u8> {
    let mut end = Vec::with_capacity(40);
    end.extend_from_slice(&canonical_len.to_le_bytes());
    end.extend_from_slice(&digest);
    end
}

pub(in crate::replication::authority) fn decode_end(
    payload: &[u8],
) -> Result<(u64, [u8; 32]), &'static str> {
    if payload.len() != 40 {
        return Err("replication semantic carrier end has invalid length");
    }
    let len = u64::from_le_bytes(
        payload[..8]
            .try_into()
            .map_err(|_| "semantic carrier length decode")?,
    );
    let digest = payload[8..40]
        .try_into()
        .map_err(|_| "semantic carrier digest decode")?;
    Ok((len, digest))
}
