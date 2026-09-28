use super::ReplicationAuthorityJournal;
use crate::replication::codec::{
    KIND_DECISION_LOCK, KIND_DECISION_VOTE, encode_decision_lock, encode_decision_vote,
    validate_acknowledgements,
};
use crate::replication::{
    ReplicationDecisionLock, ReplicationDecisionVote, ReplicationPeerEvidence, protocol,
};
use crate::runtime::DurabilityError;

impl ReplicationAuthorityJournal {
    pub(super) fn validate_decision_vote(
        &self,
        vote: &ReplicationDecisionVote,
    ) -> Result<(), DurabilityError> {
        self.validate_term_member(vote.membership_epoch, vote.voter, vote.term)?;
        let certificate = self
            .leader_certificates
            .get(&(vote.membership_epoch, vote.term))
            .ok_or_else(|| {
                protocol("replication decision vote has no durable leader certificate")
            })?;
        if certificate.leader != vote.leader {
            return Err(protocol(
                "replication decision vote names a different leader",
            ));
        }
        if vote.term < self.highest_promised_term(vote.membership_epoch) {
            return Err(protocol("replication decision vote uses a stale term"));
        }
        let envelope = self.effects.get(&vote.effect).ok_or_else(|| {
            protocol("replication decision vote references a non-local-durable effect")
        })?;
        if envelope.ordered_by.position != vote.position {
            return Err(protocol(
                "replication decision vote position does not match effect ordering",
            ));
        }
        if let Some(lock) = self.decision_locks.get(&vote.position) {
            if lock.effect != vote.effect {
                return Err(protocol(
                    "replication decision conflicts with a durable locked value",
                ));
            }
            if vote.term > lock.term && vote.carried_from_term != Some(lock.term) {
                return Err(protocol(
                    "replication later-term decision did not carry forward the durable lock",
                ));
            }
        } else if vote.carried_from_term.is_some() {
            return Err(protocol(
                "replication decision claims a carry-forward without a prior lock",
            ));
        }
        if self
            .decision_votes
            .get(&(vote.membership_epoch, vote.term, vote.position, vote.voter))
            .is_some_and(|existing| existing != vote)
        {
            return Err(protocol(
                "replication voter already accepted another value in this term/position",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_decision_lock(
        &self,
        lock: &ReplicationDecisionLock,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        self.validate_term_member(lock.membership_epoch, lock.leader, lock.term)?;
        let current = self
            .current_membership()
            .expect("validated membership exists");
        validate_acknowledgements(
            current,
            &lock.acknowledged_by,
            "replication decision lock lacks configured quorum",
        )?;
        let certificate = self
            .leader_certificates
            .get(&(lock.membership_epoch, lock.term))
            .ok_or_else(|| {
                protocol("replication decision lock has no durable leader certificate")
            })?;
        if certificate.leader != lock.leader {
            return Err(protocol(
                "replication decision lock names a different leader",
            ));
        }
        if lock.term < self.highest_promised_term(lock.membership_epoch) {
            return Err(protocol("replication decision lock uses a stale term"));
        }
        for voter in &lock.acknowledged_by {
            let vote = self
                .decision_votes
                .get(&(lock.membership_epoch, lock.term, lock.position, *voter))
                .ok_or_else(|| protocol("replication decision lock lacks durable vote evidence"))?;
            if vote.effect != lock.effect
                || vote.leader != lock.leader
                || vote.carried_from_term != lock.carried_from_term
            {
                return Err(protocol(
                    "replication decision lock vote evidence does not match",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::DecisionVote(
                *vote,
            ))?;
        }
        if let Some(existing) = self.decision_locks.get(&lock.position) {
            if existing.effect != lock.effect {
                return Err(protocol(
                    "replication decision lock conflicts with prior locked value",
                ));
            }
            if lock.term <= existing.term {
                return Err(protocol("replication decision lock did not advance term"));
            }
            if lock.carried_from_term != Some(existing.term) {
                return Err(protocol(
                    "replication later-term lock did not carry forward prior lock term",
                ));
            }
        } else if lock.carried_from_term.is_some() {
            return Err(protocol(
                "replication decision lock claims carry-forward without prior lock",
            ));
        }
        Ok(())
    }

    pub(crate) fn record_decision_vote(
        &mut self,
        vote: ReplicationDecisionVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::DecisionVote(vote))?;
        self.validate_decision_vote(&vote)?;
        let key = (vote.membership_epoch, vote.term, vote.position, vote.voter);
        if let Some(existing) = self.decision_votes.get(&key) {
            return if existing == &vote {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already accepted another value in this term/position",
                ))
            };
        }
        self.append_frame(KIND_DECISION_VOTE, &encode_decision_vote(&vote))?;
        self.decision_votes.insert(key, vote);
        Ok(())
    }

    pub(crate) fn lock_decision(
        &mut self,
        lock: ReplicationDecisionLock,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self.decision_locks.get(&lock.position) {
            if existing == &lock {
                return Ok(());
            }
            if lock.term <= existing.term {
                return Err(protocol("replication decision lock did not advance term"));
            }
        }
        self.validate_decision_lock(&lock)?;
        let payload = encode_decision_lock(&lock)?;
        self.append_frame(KIND_DECISION_LOCK, &payload)?;
        self.highest_decision_lock_term = self.highest_decision_lock_term.max(lock.term);
        self.decision_locks.insert(lock.position, lock);
        Ok(())
    }
}
