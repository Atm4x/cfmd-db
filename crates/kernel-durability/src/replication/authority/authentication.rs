use kernel_auth::{Sha256Digest, TrustRootSet, sha256};

use super::{AuthenticatedRecoveryAckIndex, ReplicationAuthorityJournal};
use crate::replication::codec::{
    KIND_AUTHENTICATED_MEMBERSHIP_VOTE_REF, KIND_AUTHENTICATED_PEER_EVIDENCE,
    KIND_AUTHENTICATED_RECOVERY_ACK_REF, KIND_PEER_AUTH_POLICY, authentication_receipt,
    encode_peer_auth_policy, encode_peer_evidence, encode_signed_membership_vote_ref,
    encode_signed_peer_evidence, encode_signed_recovery_ack_ref, evidence_voter,
    recovery_lock_frontier_digest,
};
use crate::replication::{
    ReplicaId, ReplicationAuthenticationReceipt, ReplicationJointMembershipAck,
    ReplicationPeerAuthPolicy, ReplicationPeerEvidence, SignedReplicationPeerEvidence, protocol,
    replication_peer_evidence_signing_message,
};
use crate::runtime::DurabilityError;
impl ReplicationAuthorityJournal {
    fn validate_peer_auth_policy(
        &self,
        policy: &ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
    ) -> Result<(), DurabilityError> {
        if policy.cluster.0 == [0; 32] {
            return Err(protocol("replication peer-auth cluster identity is zero"));
        }
        if policy.trust_epoch == 0 || trust.epoch() != policy.trust_epoch {
            return Err(protocol(
                "replication peer-auth trust epoch does not match trust roots",
            ));
        }
        if policy.peer_keys.is_empty() {
            return Err(protocol("replication peer-auth policy has no peer keys"));
        }
        if policy.peer_keys.keys().any(|replica| replica.raw() == 0) {
            return Err(protocol(
                "replication peer-auth policy contains replica zero",
            ));
        }
        if policy.peer_keys.values().any(|key| !trust.contains(*key)) {
            return Err(protocol(
                "replication peer-auth policy references an untrusted key",
            ));
        }
        if let Some(current) = self.current_membership()
            && current
                .members
                .iter()
                .any(|member| !policy.peer_keys.contains_key(member))
        {
            return Err(protocol(
                "replication peer-auth policy does not cover current membership",
            ));
        }
        if let Some(previous) = &self.peer_auth_policy {
            if previous.cluster != policy.cluster {
                return Err(protocol("replication peer-auth cluster identity changed"));
            }
            if policy.trust_epoch != previous.trust_epoch.saturating_add(1) {
                return Err(protocol(
                    "replication peer-auth trust epoch did not advance by one",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_replayed_peer_auth_policy(
        &self,
        policy: &ReplicationPeerAuthPolicy,
    ) -> Result<(), DurabilityError> {
        if policy.cluster.0 == [0; 32]
            || policy.trust_epoch == 0
            || policy.peer_keys.is_empty()
            || policy.peer_keys.keys().any(|replica| replica.raw() == 0)
        {
            return Err(protocol(
                "replication peer-auth policy is malformed during replay",
            ));
        }
        if let Some(current) = self.current_membership()
            && current
                .members
                .iter()
                .any(|member| !policy.peer_keys.contains_key(member))
        {
            return Err(protocol(
                "replication peer-auth policy does not cover current membership during replay",
            ));
        }
        if let Some(previous) = &self.peer_auth_policy
            && (previous.cluster != policy.cluster
                || policy.trust_epoch != previous.trust_epoch.saturating_add(1))
        {
            return Err(protocol(
                "replication peer-auth policy replay violates monotone trust epoch",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_replayed_peer_evidence(
        &self,
        signed: &SignedReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        let policy = self
            .peer_auth_policy
            .as_ref()
            .ok_or_else(|| protocol("replication peer evidence replay has no auth policy"))?;
        if signed.trust_epoch != policy.trust_epoch {
            return Err(protocol(
                "replication peer evidence replay uses stale trust epoch",
            ));
        }
        let voter = evidence_voter(&signed.evidence);
        if policy.peer_keys.get(&voter) != Some(&signed.signer) {
            return Err(protocol(
                "replication peer evidence replay signer does not match durable policy",
            ));
        }
        self.validate_peer_evidence_semantics(&signed.evidence)
    }

    fn verify_peer_evidence(
        &self,
        trust: &TrustRootSet,
        signed: &SignedReplicationPeerEvidence,
    ) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
        let policy = self
            .peer_auth_policy
            .as_ref()
            .ok_or_else(|| protocol("replication peer authentication is not activated"))?;
        if signed.trust_epoch != policy.trust_epoch || trust.epoch() != policy.trust_epoch {
            return Err(protocol(
                "replication peer evidence uses a stale trust epoch",
            ));
        }
        let voter = evidence_voter(&signed.evidence);
        let expected = policy
            .peer_keys
            .get(&voter)
            .ok_or_else(|| protocol("replication peer evidence voter has no authorized key"))?;
        if *expected != signed.signer || !trust.contains(signed.signer) {
            return Err(protocol(
                "replication peer evidence signer is not authorized for voter",
            ));
        }
        self.validate_peer_evidence_semantics(&signed.evidence)?;
        let message = replication_peer_evidence_signing_message(
            policy.cluster,
            signed.trust_epoch,
            signed.signer,
            &signed.evidence,
        )?;
        trust
            .verify_message(signed.signer, &message, &signed.signature)
            .map_err(|_| protocol("replication peer evidence signature verification failed"))?;
        authentication_receipt(signed)
    }

    fn validate_peer_evidence_semantics(
        &self,
        evidence: &ReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        match evidence {
            ReplicationPeerEvidence::TermPromise(value) => {
                self.validate_term_member(value.membership_epoch, value.voter, value.term)
            }
            ReplicationPeerEvidence::LeaderVote(value) => self.validate_leader_vote(value),
            ReplicationPeerEvidence::EffectVote(value) => self.validate_effect_vote(value),
            ReplicationPeerEvidence::MembershipVote(value) => self.validate_membership_vote(value),
            ReplicationPeerEvidence::DecisionVote(value) => self.validate_decision_vote(value),
            ReplicationPeerEvidence::JointMembershipAck(value) => {
                self.validate_joint_membership_ack(value)
            }
            ReplicationPeerEvidence::RecoveryAck(value) => self.validate_recovery_ack(value),
        }
    }

    pub(super) fn validate_authenticated_evidence_index(
        &self,
        signed: &SignedReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        match &signed.evidence {
            ReplicationPeerEvidence::JointMembershipAck(value) => {
                let key = (
                    value.previous_membership_epoch,
                    value.next_membership_epoch,
                    value.voter,
                );
                if self
                    .joint_membership_acks
                    .get(&key)
                    .is_some_and(|existing| existing != value)
                {
                    return Err(protocol(
                        "replication voter published conflicting joint-membership acknowledgements",
                    ));
                }
            }
            ReplicationPeerEvidence::RecoveryAck(value) => {
                let key = (value.membership_epoch, value.recovery_term, value.voter);
                let lock_frontier_digest = recovery_lock_frontier_digest(&value.locks)?;
                if let Some(existing) = self.recovery_acks.get(&key)
                    && (existing.leader != value.leader
                        || existing.lock_frontier_digest != lock_frontier_digest)
                {
                    return Err(protocol(
                        "replication voter published conflicting recovery acknowledgements",
                    ));
                }
                if let Some(existing) = self.recovery_lock_frontiers.get(&lock_frontier_digest)
                    && existing.as_slice() != value.locks.as_slice()
                {
                    return Err(protocol(
                        "replication recovery lock frontier digest collision",
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn apply_authenticated_peer_evidence(
        &mut self,
        signed: &SignedReplicationPeerEvidence,
        receipt: ReplicationAuthenticationReceipt,
    ) -> Result<(), DurabilityError> {
        self.authenticated_evidence
            .insert(receipt.proof_digest, receipt);
        self.authenticated_evidence_index.insert(
            (receipt.trust_epoch, receipt.voter, receipt.payload_digest),
            receipt.signer,
        );
        match &signed.evidence {
            ReplicationPeerEvidence::JointMembershipAck(value) => {
                self.joint_membership_acks.insert(
                    (
                        value.previous_membership_epoch,
                        value.next_membership_epoch,
                        value.voter,
                    ),
                    *value,
                );
            }
            ReplicationPeerEvidence::RecoveryAck(value) => {
                let lock_frontier_digest = recovery_lock_frontier_digest(&value.locks)?;
                self.recovery_lock_frontiers
                    .entry(lock_frontier_digest)
                    .or_insert_with(|| value.locks.clone());
                self.recovery_acks.insert(
                    (value.membership_epoch, value.recovery_term, value.voter),
                    AuthenticatedRecoveryAckIndex {
                        leader: value.leader,
                        trust_epoch: receipt.trust_epoch,
                        payload_digest: receipt.payload_digest,
                        lock_frontier_digest,
                    },
                );
            }
            _ => {}
        }
        Ok(())
    }

    fn commit_authenticated_peer_semantics(
        &mut self,
        evidence: ReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        match evidence {
            ReplicationPeerEvidence::TermPromise(value) => self.record_term_promise(value),
            ReplicationPeerEvidence::LeaderVote(value) => self.record_leader_vote(value),
            ReplicationPeerEvidence::EffectVote(value) => self.record_effect_vote(value),
            ReplicationPeerEvidence::MembershipVote(value) => self.record_membership_vote(value),
            ReplicationPeerEvidence::DecisionVote(value) => self.record_decision_vote(value),
            ReplicationPeerEvidence::JointMembershipAck(_)
            | ReplicationPeerEvidence::RecoveryAck(_) => Ok(()),
        }
    }

    pub(super) fn require_authenticated_payload(
        &self,
        voter: ReplicaId,
        trust_epoch: u64,
        payload_digest: Sha256Digest,
    ) -> Result<(), DurabilityError> {
        let Some(policy) = &self.peer_auth_policy else {
            return Ok(());
        };
        if trust_epoch == policy.trust_epoch
            && self
                .authenticated_evidence_index
                .get(&(trust_epoch, voter, payload_digest))
                .is_some_and(|signer| policy.peer_keys.get(&voter) == Some(signer))
        {
            Ok(())
        } else {
            Err(protocol(
                "replication peer evidence is not authenticated in current trust epoch",
            ))
        }
    }

    pub(super) fn require_authenticated_peer_evidence(
        &self,
        evidence: &ReplicationPeerEvidence,
    ) -> Result<(), DurabilityError> {
        let voter = evidence_voter(evidence);
        let (_, payload) = encode_peer_evidence(evidence)?;
        self.require_authenticated_payload(
            voter,
            self.peer_auth_policy
                .as_ref()
                .map_or(0, |policy| policy.trust_epoch),
            sha256(&payload),
        )
    }

    fn validate_joint_membership_ack(
        &self,
        ack: &ReplicationJointMembershipAck,
    ) -> Result<(), DurabilityError> {
        if ack.term == 0 || ack.next_membership_epoch <= ack.previous_membership_epoch {
            return Err(protocol(
                "joint membership acknowledgement has invalid epoch/term",
            ));
        }
        let current = self.current_membership().ok_or_else(|| {
            protocol("joint membership acknowledgement has no current membership")
        })?;
        if current.epoch != ack.previous_membership_epoch {
            return Err(protocol(
                "joint membership acknowledgement uses a stale previous epoch",
            ));
        }
        if !current.members.contains(&ack.leader) {
            return Err(protocol(
                "joint membership acknowledgement names a non-member leader",
            ));
        }
        Ok(())
    }
}

impl ReplicationAuthorityJournal {
    pub(crate) fn install_peer_auth_policy(
        &mut self,
        policy: ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.validate_peer_auth_policy(&policy, trust)?;
        if self.peer_auth_policy.as_ref() == Some(&policy) {
            return Ok(());
        }
        let payload = encode_peer_auth_policy(&policy)?;
        self.append_frame(KIND_PEER_AUTH_POLICY, &payload)?;
        self.peer_auth_policy = Some(policy);
        Ok(())
    }

    pub(crate) fn record_authenticated_peer_evidence(
        &mut self,
        trust: &TrustRootSet,
        signed: SignedReplicationPeerEvidence,
    ) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let receipt = self.verify_peer_evidence(trust, &signed)?;
        if let Some(existing) = self.authenticated_evidence.get(&receipt.proof_digest) {
            if *existing == receipt {
                self.commit_authenticated_peer_semantics(signed.evidence)?;
                return Ok(receipt);
            }
            return Err(protocol(
                "replication authentication proof digest collision",
            ));
        }
        self.validate_authenticated_evidence_index(&signed)?;
        match &signed.evidence {
            ReplicationPeerEvidence::MembershipVote(vote) => {
                let successor_digest = self.ensure_membership_vote_successor_durable(&vote.next)?;
                let payload = encode_signed_membership_vote_ref(&signed, successor_digest)?;
                self.append_frame(KIND_AUTHENTICATED_MEMBERSHIP_VOTE_REF, &payload)?;
            }
            ReplicationPeerEvidence::RecoveryAck(ack) => {
                let lock_frontier_digest =
                    self.ensure_recovery_lock_frontier_durable(&ack.locks)?;
                let payload = encode_signed_recovery_ack_ref(&signed, lock_frontier_digest)?;
                self.append_frame(KIND_AUTHENTICATED_RECOVERY_ACK_REF, &payload)?;
            }
            _ => {
                let payload = encode_signed_peer_evidence(&signed)?;
                self.append_frame(KIND_AUTHENTICATED_PEER_EVIDENCE, &payload)?;
            }
        }
        self.apply_authenticated_peer_evidence(&signed, receipt)?;
        self.commit_authenticated_peer_semantics(signed.evidence)?;
        Ok(receipt)
    }
}
