use kernel_auth::Sha256Digest;

use super::{MembershipVoteIndex, ReplicationAuthorityJournal, membership_vote_index};
use crate::replication::codec::{
    KIND_JOINT_MEMBERSHIP_CERTIFICATE, KIND_MEMBERSHIP, KIND_MEMBERSHIP_SUCCESSOR_OWNER,
    KIND_MEMBERSHIP_VOTE_REF, encode_joint_membership_certificate, encode_membership_change,
    encode_membership_successor_owner, encode_membership_vote_ref, validate_acknowledgements,
};
use crate::replication::{
    ReplicaId, ReplicationJointMembershipCertificate, ReplicationMembership,
    ReplicationMembershipChange, ReplicationMembershipVote, ReplicationPeerEvidence,
    ReplicationQuorumAvailability, protocol, replication_membership_digest,
};
use crate::runtime::DurabilityError;

impl ReplicationAuthorityJournal {
    fn validate_membership_change(
        &self,
        change: &ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        change.next.validate()?;
        match self.current_membership() {
            None => {
                if !change.acknowledged_by_previous.is_empty() {
                    return Err(protocol(
                        "replication membership bootstrap cannot claim prior acknowledgements",
                    ));
                }
            }
            Some(current) => {
                self.require_quorum_available()?;
                if change.next.epoch <= current.epoch {
                    return Err(protocol("replication membership epoch did not advance"));
                }
                validate_acknowledgements(
                    current,
                    &change.acknowledged_by_previous,
                    "replication membership change lacks previous-epoch quorum",
                )?;
                let mut decision_term = None;
                for voter in &change.acknowledged_by_previous {
                    let vote = self
                        .membership_votes
                        .get(&(current.epoch, *voter))
                        .ok_or_else(|| {
                            protocol("replication membership change lacks durable vote evidence")
                        })?;
                    let successor = self
                        .membership_vote_successors
                        .get(&vote.successor_digest)
                        .ok_or_else(|| {
                            protocol("replication membership vote successor owner is missing")
                        })?;
                    if successor != &change.next {
                        return Err(protocol(
                            "replication membership vote targets another successor",
                        ));
                    }
                    match decision_term {
                        None => decision_term = Some(vote.term),
                        Some(term) if term == vote.term => {}
                        Some(_) => {
                            return Err(protocol(
                                "replication membership quorum mixes election terms",
                            ));
                        }
                    }
                }
                if self.has_leader_certificate_for_epoch(current.epoch) {
                    let term = decision_term.expect("non-empty old quorum has a decision term");
                    if !self
                        .leader_certificates
                        .contains_key(&(current.epoch, term))
                    {
                        return Err(protocol(
                            "replication membership change is not bound to a certified leader term",
                        ));
                    }
                    if term < self.highest_promised_term(current.epoch) {
                        return Err(protocol(
                            "replication membership change uses a stale consensus term",
                        ));
                    }
                    let joint = self
                        .joint_membership_certificates
                        .get(&change.next.epoch)
                        .ok_or_else(|| {
                            protocol("replication membership change lacks joint quorum certificate")
                        })?;
                    if joint.previous_membership_epoch != current.epoch
                        || joint.term != term
                        || joint.next != change.next
                        || joint.acknowledged_by_previous != change.acknowledged_by_previous
                    {
                        return Err(protocol(
                            "replication membership change does not match joint quorum certificate",
                        ));
                    }
                }
            }
        }
        if self.memberships.contains_key(&change.next.epoch) {
            return Err(protocol(
                "replication membership epoch is already bound to another configuration",
            ));
        }
        Ok(())
    }

    pub(super) fn apply_membership_change(
        &mut self,
        change: ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        self.validate_membership_change(&change)?;
        let epoch = change.next.epoch;
        self.memberships.insert(epoch, change.next);
        self.current_membership_epoch = Some(epoch);
        self.quorum_availability = Some(ReplicationQuorumAvailability::Available {
            membership_epoch: epoch,
            term: 0,
        });
        Ok(())
    }

    pub(super) fn validate_joint_membership_certificate(
        &self,
        certificate: &ReplicationJointMembershipCertificate,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        certificate.next.validate()?;
        if self
            .joint_membership_certificates
            .get(&certificate.next.epoch)
            .is_some_and(|existing| existing != certificate)
        {
            return Err(protocol(
                "replication successor epoch already has a different joint certificate",
            ));
        }
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("joint membership certificate has no previous membership"))?;
        if current.epoch != certificate.previous_membership_epoch {
            return Err(protocol(
                "joint membership certificate uses a stale previous epoch",
            ));
        }
        if certificate.next.epoch <= current.epoch {
            return Err(protocol(
                "joint membership certificate successor epoch did not advance",
            ));
        }
        let leader = self
            .leader_certificates
            .get(&(current.epoch, certificate.term))
            .ok_or_else(|| {
                protocol("joint membership certificate has no durable leader certificate")
            })?;
        if leader.leader != certificate.leader {
            return Err(protocol(
                "joint membership certificate names a different leader",
            ));
        }
        if certificate.term < self.highest_promised_term(current.epoch) {
            return Err(protocol(
                "joint membership certificate is fenced by a higher promised term",
            ));
        }
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by_previous,
            "joint membership certificate lacks previous-membership quorum",
        )?;
        validate_acknowledgements(
            &certificate.next,
            &certificate.acknowledged_by_next,
            "joint membership certificate lacks successor-membership quorum",
        )?;
        if let Some(policy) = &self.peer_auth_policy
            && certificate
                .next
                .members
                .iter()
                .any(|member| !policy.peer_keys.contains_key(member))
        {
            return Err(protocol(
                "joint membership successor is not covered by peer-auth policy",
            ));
        }
        self.validate_joint_membership_previous_vote_evidence(current, certificate)?;
        if self.peer_auth_policy.is_some() {
            let next_digest = replication_membership_digest(&certificate.next)?;
            for voter in &certificate.acknowledged_by_next {
                let ack = self
                    .joint_membership_acks
                    .get(&(current.epoch, certificate.next.epoch, *voter))
                    .ok_or_else(|| {
                        protocol(
                            "joint membership certificate lacks authenticated successor evidence",
                        )
                    })?;
                if ack.term != certificate.term
                    || ack.leader != certificate.leader
                    || ack.next_membership_digest != next_digest
                {
                    return Err(protocol(
                        "joint membership successor evidence does not match certificate",
                    ));
                }
                self.require_authenticated_peer_evidence(
                    &ReplicationPeerEvidence::JointMembershipAck(*ack),
                )?;
            }
        }
        Ok(())
    }

    fn validate_joint_membership_previous_vote_evidence(
        &self,
        current: &ReplicationMembership,
        certificate: &ReplicationJointMembershipCertificate,
    ) -> Result<(), DurabilityError> {
        for voter in &certificate.acknowledged_by_previous {
            let vote = self
                .membership_votes
                .get(&(current.epoch, *voter))
                .ok_or_else(|| {
                    protocol(
                        "joint membership certificate lacks durable old-membership vote evidence",
                    )
                })?;
            let successor = self
                .membership_vote_successors
                .get(&vote.successor_digest)
                .ok_or_else(|| protocol("joint membership vote successor owner is missing"))?;
            if vote.term != certificate.term || successor != &certificate.next {
                return Err(protocol(
                    "joint membership certificate vote evidence does not match",
                ));
            }
            self.require_authenticated_payload(
                *voter,
                self.peer_auth_policy
                    .as_ref()
                    .map_or(0, |policy| policy.trust_epoch),
                vote.payload_digest,
            )?;
        }
        Ok(())
    }

    pub(super) fn validate_membership_vote(
        &self,
        vote: &ReplicationMembershipVote,
    ) -> Result<(), DurabilityError> {
        vote.next.validate()?;
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("membership vote has no previous durable membership"))?;
        if current.epoch != vote.previous_membership_epoch {
            return Err(protocol("membership vote uses a stale previous epoch"));
        }
        if vote.term == 0 || vote.next.epoch <= current.epoch {
            return Err(protocol(
                "membership vote has invalid term or successor epoch",
            ));
        }
        if !current.members.contains(&vote.voter) {
            return Err(protocol("membership vote references a non-member replica"));
        }
        let index = membership_vote_index(vote)?;
        if self
            .membership_votes
            .get(&(current.epoch, vote.voter))
            .is_some_and(|existing| *existing != index)
        {
            return Err(protocol(
                "replication voter already voted for another successor membership",
            ));
        }
        self.validate_membership_vote_successor_owner(&vote.next, index.successor_digest)?;
        Ok(())
    }

    pub(super) fn validate_membership_vote_successor_owner(
        &self,
        successor: &ReplicationMembership,
        digest: Sha256Digest,
    ) -> Result<(), DurabilityError> {
        if self
            .membership_vote_successors
            .get(&digest)
            .is_some_and(|existing| existing != successor)
        {
            return Err(protocol(
                "replication membership successor digest collision",
            ));
        }
        Ok(())
    }

    pub(super) fn ensure_membership_vote_successor_durable(
        &mut self,
        successor: &ReplicationMembership,
    ) -> Result<Sha256Digest, DurabilityError> {
        let digest = replication_membership_digest(successor)?;
        self.validate_membership_vote_successor_owner(successor, digest)?;
        if !self.membership_vote_successors.contains_key(&digest) {
            let payload = encode_membership_successor_owner(successor, digest)?;
            self.append_frame(KIND_MEMBERSHIP_SUCCESSOR_OWNER, &payload)?;
            self.membership_vote_successors
                .insert(digest, successor.clone());
        }
        Ok(digest)
    }

    pub(super) fn membership_vote_from_ref(
        &self,
        voter: ReplicaId,
        previous_membership_epoch: u64,
        term: u64,
        successor_digest: Sha256Digest,
    ) -> Result<ReplicationMembershipVote, DurabilityError> {
        let next = self
            .membership_vote_successors
            .get(&successor_digest)
            .ok_or_else(|| protocol("replication membership vote successor owner is missing"))?
            .clone();
        Ok(ReplicationMembershipVote {
            voter,
            previous_membership_epoch,
            term,
            next,
        })
    }

    pub(super) fn apply_membership_vote_index(
        &mut self,
        key: (u64, ReplicaId),
        index: MembershipVoteIndex,
        successor: ReplicationMembership,
    ) -> Result<(), DurabilityError> {
        self.validate_membership_vote_successor_owner(&successor, index.successor_digest)?;
        self.membership_vote_successors
            .entry(index.successor_digest)
            .or_insert(successor);
        self.membership_votes.insert(key, index);
        Ok(())
    }

    pub(crate) fn install_membership(
        &mut self,
        change: ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.memberships.get(&change.next.epoch) == Some(&change.next) {
            return Ok(());
        }
        if self.current_membership().is_some() {
            self.require_quorum_available()?;
        }
        self.validate_membership_change(&change)?;
        let payload = encode_membership_change(&change)?;
        self.append_frame(KIND_MEMBERSHIP, &payload)?;
        self.apply_membership_change(change)
    }

    pub(crate) fn record_membership_vote(
        &mut self,
        vote: ReplicationMembershipVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::MembershipVote(
            vote.clone(),
        ))?;
        self.validate_membership_vote(&vote)?;
        let key = (vote.previous_membership_epoch, vote.voter);
        let index = membership_vote_index(&vote)?;
        if let Some(existing) = self.membership_votes.get(&key) {
            return if *existing == index {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already voted for another successor membership",
                ))
            };
        }
        let successor_digest = self.ensure_membership_vote_successor_durable(&vote.next)?;
        debug_assert_eq!(successor_digest, index.successor_digest);
        let payload = encode_membership_vote_ref(&vote, successor_digest);
        self.append_frame(KIND_MEMBERSHIP_VOTE_REF, &payload)?;
        self.apply_membership_vote_index(key, index, vote.next)?;
        Ok(())
    }

    pub(crate) fn certify_joint_membership(
        &mut self,
        certificate: ReplicationJointMembershipCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self
            .joint_membership_certificates
            .get(&certificate.next.epoch)
        {
            return if existing == &certificate {
                Ok(())
            } else {
                Err(protocol(
                    "replication successor epoch already has a different joint certificate",
                ))
            };
        }
        self.validate_joint_membership_certificate(&certificate)?;
        let payload = encode_joint_membership_certificate(&certificate)?;
        self.append_frame(KIND_JOINT_MEMBERSHIP_CERTIFICATE, &payload)?;
        self.joint_membership_certificates
            .insert(certificate.next.epoch, certificate);
        Ok(())
    }
}
