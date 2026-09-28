use super::ReplicationAuthorityJournal;
use crate::replication::codec::{
    KIND_LEADER_CERTIFICATE, KIND_LEADER_VOTE, KIND_TERM_PROMISE, encode_leader_certificate,
    encode_leader_vote, encode_term_promise, validate_acknowledgements,
};
use crate::replication::{
    ReplicaId, ReplicationLeaderCertificate, ReplicationLeaderVote, ReplicationPeerEvidence,
    ReplicationTermPromise, protocol,
};
use crate::runtime::DurabilityError;

impl ReplicationAuthorityJournal {
    pub(super) fn validate_term_member(
        &self,
        membership_epoch: u64,
        voter: ReplicaId,
        term: u64,
    ) -> Result<(), DurabilityError> {
        if term == 0 {
            return Err(protocol("replication consensus term is zero"));
        }
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication consensus has no durable membership"))?;
        if current.epoch != membership_epoch {
            return Err(protocol(
                "replication consensus uses a stale membership epoch",
            ));
        }
        if !current.members.contains(&voter) {
            return Err(protocol(
                "replication consensus references a non-member replica",
            ));
        }
        Ok(())
    }

    pub(super) fn highest_promised_term(&self, membership_epoch: u64) -> u64 {
        self.highest_promised_terms
            .get(&membership_epoch)
            .copied()
            .unwrap_or(0)
    }

    pub(super) fn note_promised_term(
        &mut self,
        membership_epoch: u64,
        voter: ReplicaId,
        term: u64,
    ) {
        self.promised_terms
            .entry((membership_epoch, voter))
            .and_modify(|promised| *promised = (*promised).max(term))
            .or_insert(term);
        self.highest_promised_terms
            .entry(membership_epoch)
            .and_modify(|highest| *highest = (*highest).max(term))
            .or_insert(term);
    }

    pub(super) fn has_leader_certificate_for_epoch(&self, membership_epoch: u64) -> bool {
        self.leader_certificates
            .range((membership_epoch, 0)..=(membership_epoch, u64::MAX))
            .next()
            .is_some()
    }

    pub(super) fn validate_leader_vote(
        &self,
        vote: &ReplicationLeaderVote,
    ) -> Result<(), DurabilityError> {
        self.validate_term_member(vote.membership_epoch, vote.voter, vote.term)?;
        let current = self
            .current_membership()
            .expect("validated membership exists");
        if !current.members.contains(&vote.candidate) {
            return Err(protocol(
                "replication leader candidate is not a membership voter",
            ));
        }
        if self
            .promised_terms
            .get(&(vote.membership_epoch, vote.voter))
            .is_some_and(|promised| *promised > vote.term)
        {
            return Err(protocol("replication leader vote uses a stale term"));
        }
        if self
            .leader_votes
            .get(&(vote.membership_epoch, vote.term, vote.voter))
            .is_some_and(|candidate| *candidate != vote.candidate)
        {
            return Err(protocol(
                "replication voter already voted for another leader in this term",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_leader_certificate(
        &self,
        certificate: &ReplicationLeaderCertificate,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        self.validate_term_member(
            certificate.membership_epoch,
            certificate.leader,
            certificate.term,
        )?;
        let current = self
            .current_membership()
            .expect("validated membership exists");
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by,
            "replication leader certificate lacks configured quorum",
        )?;
        if certificate.term < self.highest_promised_term(certificate.membership_epoch) {
            return Err(protocol(
                "replication leader certificate is fenced by a higher promised term",
            ));
        }
        for voter in &certificate.acknowledged_by {
            if self
                .leader_votes
                .get(&(certificate.membership_epoch, certificate.term, *voter))
                != Some(&certificate.leader)
            {
                return Err(protocol(
                    "replication leader certificate lacks durable vote evidence",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::LeaderVote(
                ReplicationLeaderVote {
                    voter: *voter,
                    membership_epoch: certificate.membership_epoch,
                    term: certificate.term,
                    candidate: certificate.leader,
                },
            ))?;
        }
        Ok(())
    }

    pub(crate) fn record_term_promise(
        &mut self,
        promise: ReplicationTermPromise,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::TermPromise(promise))?;
        self.validate_term_member(promise.membership_epoch, promise.voter, promise.term)?;
        let key = (promise.membership_epoch, promise.voter);
        if let Some(existing) = self.promised_terms.get(&key) {
            if promise.term < *existing {
                return Err(protocol("replication term promise regressed"));
            }
            if promise.term == *existing {
                return Ok(());
            }
        }
        self.append_frame(KIND_TERM_PROMISE, &encode_term_promise(&promise))?;
        self.note_promised_term(promise.membership_epoch, promise.voter, promise.term);
        Ok(())
    }

    pub(crate) fn record_leader_vote(
        &mut self,
        vote: ReplicationLeaderVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::LeaderVote(vote))?;
        self.validate_leader_vote(&vote)?;
        let key = (vote.membership_epoch, vote.term, vote.voter);
        if let Some(existing) = self.leader_votes.get(&key) {
            return if *existing == vote.candidate {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already voted for another leader in this term",
                ))
            };
        }
        self.append_frame(KIND_LEADER_VOTE, &encode_leader_vote(&vote))?;
        self.note_promised_term(vote.membership_epoch, vote.voter, vote.term);
        self.leader_votes.insert(key, vote.candidate);
        Ok(())
    }

    pub(crate) fn certify_leader(
        &mut self,
        certificate: ReplicationLeaderCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self
            .leader_certificates
            .get(&(certificate.membership_epoch, certificate.term))
        {
            return if existing == &certificate {
                Ok(())
            } else {
                Err(protocol(
                    "replication term already has a different leader certificate",
                ))
            };
        }
        self.validate_leader_certificate(&certificate)?;
        let payload = encode_leader_certificate(&certificate)?;
        self.append_frame(KIND_LEADER_CERTIFICATE, &payload)?;
        let membership_epoch = certificate.membership_epoch;
        let term = certificate.term;
        self.leader_certificates
            .insert((membership_epoch, term), certificate);
        self.note_available_term(membership_epoch, term);
        Ok(())
    }
}
