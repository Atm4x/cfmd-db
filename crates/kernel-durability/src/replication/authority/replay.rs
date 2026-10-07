use sha2::Digest;

use super::{
    ReplicationAuthorityJournal, SemanticAuthorityBaseReplay, corruption, membership_vote_index,
};
use crate::replication::codec::{
    KIND_AUTHENTICATED_MEMBERSHIP_VOTE_REF, KIND_AUTHENTICATED_PEER_EVIDENCE,
    KIND_AUTHENTICATED_RECOVERY_ACK_REF, KIND_DECISION_LOCK, KIND_DECISION_VOTE, KIND_EFFECT_VOTE,
    KIND_INGEST, KIND_JOINT_MEMBERSHIP_CERTIFICATE, KIND_LEADER_CERTIFICATE, KIND_LEADER_VOTE,
    KIND_MEMBERSHIP, KIND_MEMBERSHIP_SUCCESSOR_OWNER, KIND_MEMBERSHIP_VOTE,
    KIND_MEMBERSHIP_VOTE_REF, KIND_PEER_AUTH_POLICY, KIND_PUBLISH, KIND_QUORUM, KIND_QUORUM_LOSS,
    KIND_QUORUM_RECOVERY, KIND_RECOVERY_LOCK_FRONTIER_OWNER, KIND_RETIRE,
    KIND_SEMANTIC_AUTHORITY_BASE_BEGIN, KIND_SEMANTIC_AUTHORITY_BASE_END,
    KIND_SEMANTIC_AUTHORITY_BASE_RECORD, KIND_TERM_PROMISE, SignedMembershipVoteRef,
    authentication_receipt, decode_decision_lock, decode_decision_vote, decode_effect_vote,
    decode_ingest, decode_joint_membership_certificate, decode_leader_certificate,
    decode_leader_vote, decode_membership_change, decode_membership_successor_owner,
    decode_membership_vote, decode_membership_vote_ref, decode_peer_auth_policy, decode_publish,
    decode_quorum_certificate, decode_quorum_loss, decode_recovery_certificate,
    decode_recovery_lock_frontier_owner, decode_retire, decode_signed_membership_vote_ref,
    decode_signed_peer_evidence, decode_signed_recovery_ack_ref, decode_term_promise,
    recovery_lock_frontier_digest, validate_lock_summaries,
};
use crate::replication::{
    ReplicationPeerEvidence, ReplicationQuorumAvailability, SignedReplicationPeerEvidence,
    replication_membership_digest,
};
use crate::runtime::DurabilityError;
impl ReplicationAuthorityJournal {
    fn replay_authenticated_peer_evidence(
        &mut self,
        signed: &SignedReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        self.validate_replayed_peer_evidence(signed)?;
        let receipt = authentication_receipt(signed)?;
        self.validate_authenticated_evidence_index(signed)?;
        self.apply_authenticated_peer_evidence(signed, receipt)
    }

    fn signed_membership_vote_from_ref(
        &self,
        reference: SignedMembershipVoteRef,
    ) -> Result<SignedReplicationPeerEvidence, DurabilityError> {
        let vote = self.membership_vote_from_ref(
            reference.voter,
            reference.previous_membership_epoch,
            reference.term,
            reference.successor_digest,
        )?;
        Ok(SignedReplicationPeerEvidence {
            trust_epoch: reference.trust_epoch,
            signer: reference.signer,
            evidence: ReplicationPeerEvidence::MembershipVote(vote),
            signature: reference.signature,
        })
    }

    fn replay_authenticated_frame(
        &mut self,
        kind: u8,
        payload: &[u8],
        offset: usize,
    ) -> Result<bool, DurabilityError> {
        let signed = match kind {
            KIND_PEER_AUTH_POLICY => {
                let policy = decode_peer_auth_policy(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_replayed_peer_auth_policy(&policy)?;
                self.peer_auth_policy = Some(policy);
                return Ok(true);
            }
            KIND_AUTHENTICATED_PEER_EVIDENCE => decode_signed_peer_evidence(payload)
                .map_err(|reason| DurabilityError::Corruption { offset, reason })?,
            KIND_AUTHENTICATED_MEMBERSHIP_VOTE_REF => {
                let reference = decode_signed_membership_vote_ref(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.signed_membership_vote_from_ref(reference)?
            }
            KIND_AUTHENTICATED_RECOVERY_ACK_REF => {
                let reference = decode_signed_recovery_ack_ref(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.signed_recovery_ack_from_ref(reference)?
            }
            _ => return Ok(false),
        };
        self.replay_authenticated_peer_evidence(&signed)?;
        Ok(true)
    }

    fn replay_consensus_frame(
        &mut self,
        kind: u8,
        payload: &[u8],
        offset: usize,
    ) -> Result<bool, DurabilityError> {
        if self.replay_authenticated_frame(kind, payload, offset)? {
            return Ok(true);
        }
        match kind {
            KIND_TERM_PROMISE => {
                let promise = decode_term_promise(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::TermPromise(
                    promise,
                ))?;
                self.validate_term_member(promise.membership_epoch, promise.voter, promise.term)?;
                let key = (promise.membership_epoch, promise.voter);
                if self
                    .promised_terms
                    .get(&key)
                    .is_some_and(|term| *term > promise.term)
                {
                    return Err(corruption(
                        offset,
                        "replication term promise regressed during replay",
                    ));
                }
                self.note_promised_term(promise.membership_epoch, promise.voter, promise.term);
            }
            KIND_LEADER_VOTE => {
                let vote = decode_leader_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::LeaderVote(
                    vote,
                ))?;
                self.validate_leader_vote(&vote)?;
                self.note_promised_term(vote.membership_epoch, vote.voter, vote.term);
                self.leader_votes.insert(
                    (vote.membership_epoch, vote.term, vote.voter),
                    vote.candidate,
                );
            }
            KIND_LEADER_CERTIFICATE => {
                let certificate = decode_leader_certificate(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_leader_certificate(&certificate)?;
                let membership_epoch = certificate.membership_epoch;
                let term = certificate.term;
                self.leader_certificates
                    .insert((membership_epoch, term), certificate);
                self.note_available_term(membership_epoch, term);
            }
            KIND_DECISION_VOTE => {
                let vote = decode_decision_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::DecisionVote(
                    vote,
                ))?;
                self.validate_decision_vote(&vote)?;
                self.decision_votes.insert(
                    (vote.membership_epoch, vote.term, vote.position, vote.voter),
                    vote,
                );
            }
            KIND_DECISION_LOCK => {
                let lock = decode_decision_lock(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_decision_lock(&lock)?;
                self.highest_decision_lock_term = self.highest_decision_lock_term.max(lock.term);
                self.decision_locks.insert(lock.position, lock);
            }
            KIND_JOINT_MEMBERSHIP_CERTIFICATE => {
                let certificate = decode_joint_membership_certificate(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_joint_membership_certificate(&certificate)?;
                self.joint_membership_certificates
                    .insert(certificate.next.epoch, certificate);
            }
            KIND_QUORUM_LOSS => {
                let loss = decode_quorum_loss(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_quorum_loss(loss)?;
                self.quorum_availability = Some(ReplicationQuorumAvailability::Lost(loss));
            }
            KIND_QUORUM_RECOVERY => {
                let certificate = decode_recovery_certificate(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_recovery_certificate(&certificate)?;
                self.apply_recovery_certificate(&certificate);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn replay_vote_frame(
        &mut self,
        kind: u8,
        payload: &[u8],
        offset: usize,
    ) -> Result<bool, DurabilityError> {
        match kind {
            KIND_EFFECT_VOTE => {
                let vote = decode_effect_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::EffectVote(
                    vote,
                ))?;
                self.validate_effect_vote(&vote)?;
                let envelope = self
                    .effects
                    .get(&vote.effect)
                    .expect("validated replay vote references effect");
                self.effect_votes.insert(
                    (
                        vote.membership_epoch,
                        envelope.ordered_by.position,
                        vote.voter,
                    ),
                    vote.effect,
                );
            }
            KIND_MEMBERSHIP_VOTE => {
                let vote = decode_membership_vote(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.require_authenticated_peer_evidence(
                    &ReplicationPeerEvidence::MembershipVote(vote.clone()),
                )?;
                self.validate_membership_vote(&vote)?;
                let key = (vote.previous_membership_epoch, vote.voter);
                let index = membership_vote_index(&vote)?;
                self.validate_membership_vote_successor_owner(&vote.next, index.successor_digest)?;
                self.apply_membership_vote_index(key, index, vote.next)?;
            }
            KIND_MEMBERSHIP_VOTE_REF => {
                let reference = decode_membership_vote_ref(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                let vote = self.membership_vote_from_ref(
                    reference.voter,
                    reference.previous_membership_epoch,
                    reference.term,
                    reference.successor_digest,
                )?;
                self.require_authenticated_peer_evidence(
                    &ReplicationPeerEvidence::MembershipVote(vote.clone()),
                )?;
                self.validate_membership_vote(&vote)?;
                let key = (vote.previous_membership_epoch, vote.voter);
                let index = membership_vote_index(&vote)?;
                if index.successor_digest != reference.successor_digest {
                    return Err(corruption(
                        offset,
                        "replication membership vote reference digest mismatch",
                    ));
                }
                self.apply_membership_vote_index(key, index, vote.next)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn replay_semantic_base_frame(
        &mut self,
        kind: u8,
        payload: &[u8],
        offset: usize,
    ) -> Result<bool, DurabilityError> {
        if kind == KIND_SEMANTIC_AUTHORITY_BASE_BEGIN {
            if !self.is_empty_authority() || self.semantic_base_replay.is_some() {
                return Err(corruption(
                    offset,
                    "replication semantic authority base begin is not the first authority frame",
                ));
            }
            let expected_records = super::semantic_snapshot::decode_semantic_base_begin(payload)
                .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
            self.semantic_base_replay = Some(SemanticAuthorityBaseReplay {
                expected_records,
                record_count: 0,
                canonical_len: 0,
                hasher: sha2::Sha256::new(),
                decoder: super::semantic_snapshot::SemanticAuthorityRecordDecoder::new(),
            });
            return Ok(true);
        }
        if kind == KIND_SEMANTIC_AUTHORITY_BASE_RECORD {
            let replay = self.semantic_base_replay.as_mut().ok_or_else(|| {
                corruption(
                    offset,
                    "replication semantic authority record has no begin frame",
                )
            })?;
            if replay.record_count >= replay.expected_records {
                return Err(corruption(
                    offset,
                    "replication semantic authority base exceeds declared record count",
                ));
            }
            let record_len =
                u64::try_from(payload.len()).map_err(|_| DurabilityError::PayloadTooLarge)?;
            replay.hasher.update(record_len.to_le_bytes());
            replay.hasher.update(payload);
            replay.canonical_len = replay
                .canonical_len
                .checked_add(8)
                .and_then(|value| value.checked_add(record_len))
                .ok_or(DurabilityError::PayloadTooLarge)?;
            replay
                .decoder
                .apply_record(payload)
                .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
            replay.record_count = replay
                .record_count
                .checked_add(1)
                .ok_or(DurabilityError::PayloadTooLarge)?;
            return Ok(true);
        }
        if kind == KIND_SEMANTIC_AUTHORITY_BASE_END {
            let replay = self.semantic_base_replay.take().ok_or_else(|| {
                corruption(
                    offset,
                    "replication semantic authority base end has no begin frame",
                )
            })?;
            let (expected_len, expected_digest) =
                super::semantic_snapshot::decode_semantic_base_end(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
            if replay.record_count != replay.expected_records
                || replay.canonical_len != expected_len
            {
                return Err(corruption(
                    offset,
                    "replication semantic authority base record count or length mismatch",
                ));
            }
            let actual_digest: [u8; 32] = replay.hasher.finalize().into();
            if actual_digest != expected_digest {
                return Err(corruption(
                    offset,
                    "replication semantic authority base digest mismatch",
                ));
            }
            let snapshot = replay
                .decoder
                .finish()
                .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
            snapshot.restore(self);
            return Ok(true);
        }
        if self.semantic_base_replay.is_some() {
            return Err(corruption(
                offset,
                "replication authority delta interrupted semantic authority base",
            ));
        }
        Ok(false)
    }

    pub(super) fn apply_replay_frame(
        &mut self,
        kind: u8,
        payload: &[u8],
        offset: usize,
    ) -> Result<(), DurabilityError> {
        if self.replay_semantic_base_frame(kind, payload, offset)? {
            return Ok(());
        }
        match kind {
            KIND_INGEST => self.apply_ingest(
                decode_ingest(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?,
            )?,
            KIND_RETIRE => {
                let (branch, expected_head) = decode_retire(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                let head = self
                    .branches
                    .get_mut(&branch)
                    .ok_or_else(|| corruption(offset, "retirement references unknown branch"))?;
                if head.retired || head.head_effect != expected_head {
                    return Err(corruption(
                        offset,
                        "replication branch retirement conflicts with journal state",
                    ));
                }
                head.retired = true;
                if let Some(published) = self.published_branches.get_mut(&branch) {
                    published.retired = true;
                }
            }
            KIND_MEMBERSHIP => self.apply_membership_change(
                decode_membership_change(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?,
            )?,
            KIND_MEMBERSHIP_SUCCESSOR_OWNER => {
                let (digest, successor) = decode_membership_successor_owner(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                successor.validate()?;
                let actual = replication_membership_digest(&successor)?;
                if actual != digest {
                    return Err(corruption(
                        offset,
                        "replication membership successor owner digest mismatch",
                    ));
                }
                self.validate_membership_vote_successor_owner(&successor, digest)?;
                self.membership_vote_successors.insert(digest, successor);
            }
            KIND_RECOVERY_LOCK_FRONTIER_OWNER => {
                let (digest, locks) = decode_recovery_lock_frontier_owner(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                validate_lock_summaries(&locks)?;
                let actual = recovery_lock_frontier_digest(&locks)?;
                if actual != digest {
                    return Err(corruption(
                        offset,
                        "replication recovery lock frontier owner digest mismatch",
                    ));
                }
                if self
                    .recovery_lock_frontiers
                    .get(&digest)
                    .is_some_and(|existing| existing.as_slice() != locks.as_slice())
                {
                    return Err(corruption(
                        offset,
                        "replication recovery lock frontier owner digest collision",
                    ));
                }
                self.recovery_lock_frontiers.entry(digest).or_insert(locks);
            }
            KIND_QUORUM => {
                let certificate = decode_quorum_certificate(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.validate_quorum_certificate(&certificate)?;
                self.quorum_certificates
                    .insert(certificate.effect, certificate);
            }
            KIND_PUBLISH => {
                let effect = decode_publish(payload)
                    .map_err(|reason| DurabilityError::Corruption { offset, reason })?;
                self.apply_publish(effect)?;
            }
            _ if self.replay_vote_frame(kind, payload, offset)? => {}
            _ if self.replay_consensus_frame(kind, payload, offset)? => {}
            _ => return Err(corruption(offset, "unknown replication journal frame kind")),
        }
        Ok(())
    }
}
