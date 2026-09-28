use std::collections::BTreeSet;

use kernel_auth::Sha256Digest;

use super::ReplicationAuthorityJournal;
use crate::replication::codec::{
    KIND_QUORUM_LOSS, KIND_QUORUM_RECOVERY, KIND_RECOVERY_LOCK_FRONTIER_OWNER,
    SignedRecoveryAckRef, encode_quorum_loss, encode_recovery_certificate,
    encode_recovery_lock_frontier_owner, recovery_lock_frontier_digest, validate_acknowledgements,
    validate_lock_summaries,
};
use crate::replication::{
    ReplicationLeaderCertificate, ReplicationLockSummary, ReplicationPeerEvidence,
    ReplicationQuorumAvailability, ReplicationQuorumLoss, ReplicationRecoveryAck,
    ReplicationRecoveryCertificate, SignedReplicationPeerEvidence, protocol,
};
use crate::runtime::DurabilityError;

impl ReplicationAuthorityJournal {
    pub(super) fn validate_recovery_ack(
        &self,
        ack: &ReplicationRecoveryAck,
    ) -> Result<(), DurabilityError> {
        validate_lock_summaries(&ack.locks)?;
        let ReplicationQuorumAvailability::Lost(loss) =
            self.quorum_availability.ok_or_else(|| {
                protocol("replication recovery acknowledgement has no quorum-loss fence")
            })?
        else {
            return Err(protocol(
                "replication recovery acknowledgement requires quorum loss",
            ));
        };
        if ack.membership_epoch != loss.membership_epoch {
            return Err(protocol(
                "replication recovery acknowledgement uses a stale membership",
            ));
        }
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication recovery acknowledgement has no membership"))?;
        if !current.members.contains(&ack.voter) || !current.members.contains(&ack.leader) {
            return Err(protocol(
                "replication recovery acknowledgement references a non-member",
            ));
        }
        let floor = loss
            .observed_term
            .max(self.highest_promised_term(loss.membership_epoch))
            .max(self.highest_decision_lock_term);
        if ack.recovery_term <= floor {
            return Err(protocol(
                "replication recovery term does not advance the durable safety floor",
            ));
        }
        Ok(())
    }

    pub(super) fn validate_quorum_loss(
        &self,
        loss: ReplicationQuorumLoss,
    ) -> Result<(), DurabilityError> {
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication quorum loss has no durable membership"))?;
        if loss.membership_epoch != current.epoch {
            return Err(protocol(
                "replication quorum loss uses a stale membership epoch",
            ));
        }
        if loss.observed_term < self.highest_promised_term(current.epoch) {
            return Err(protocol(
                "replication quorum loss observed term is below durable promises",
            ));
        }
        if let Some(ReplicationQuorumAvailability::Lost(existing)) = self.quorum_availability
            && loss.observed_term < existing.observed_term
        {
            return Err(protocol("replication quorum loss observed term regressed"));
        }
        Ok(())
    }

    pub(super) fn validate_recovery_certificate(
        &self,
        certificate: &ReplicationRecoveryCertificate,
    ) -> Result<(), DurabilityError> {
        validate_lock_summaries(&certificate.reconciled_locks)?;
        let ReplicationQuorumAvailability::Lost(loss) = self
            .quorum_availability
            .ok_or_else(|| protocol("replication recovery has no durable quorum-loss fence"))?
        else {
            return Err(protocol("replication recovery requires quorum loss"));
        };
        if certificate.membership_epoch != loss.membership_epoch {
            return Err(protocol(
                "replication recovery uses a stale membership epoch",
            ));
        }
        let current = self
            .current_membership()
            .ok_or_else(|| protocol("replication recovery has no durable membership"))?;
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by,
            "replication recovery lacks configured quorum",
        )?;
        let safety_floor = loss
            .observed_term
            .max(self.highest_promised_term(loss.membership_epoch))
            .max(self.highest_decision_lock_term);
        if certificate.recovery_term <= safety_floor {
            return Err(protocol(
                "replication recovery term does not advance the durable safety floor",
            ));
        }
        if !current.members.contains(&certificate.leader) {
            return Err(protocol(
                "replication recovery leader is not a membership voter",
            ));
        }
        if certificate.reconciled_locks.len() != self.decision_locks.len()
            || certificate
                .reconciled_locks
                .iter()
                .zip(self.decision_locks.values())
                .any(|(summary, lock)| {
                    summary.position != lock.position
                        || summary.term != lock.term
                        || summary.effect != lock.effect
                })
        {
            return Err(protocol(
                "replication recovery did not reconcile the local lock frontier",
            ));
        }
        let mut lock_frontiers = BTreeSet::new();
        for voter in &certificate.acknowledged_by {
            let ack = self
                .recovery_acks
                .get(&(
                    certificate.membership_epoch,
                    certificate.recovery_term,
                    *voter,
                ))
                .ok_or_else(|| {
                    protocol("replication recovery lacks authenticated peer evidence")
                })?;
            if ack.leader != certificate.leader {
                return Err(protocol(
                    "replication recovery acknowledgements disagree on leader",
                ));
            }
            self.require_authenticated_payload(*voter, ack.trust_epoch, ack.payload_digest)?;
            lock_frontiers.insert(ack.lock_frontier_digest);
        }
        for lock_frontier_digest in lock_frontiers {
            let remote_locks = self
                .recovery_lock_frontiers
                .get(&lock_frontier_digest)
                .ok_or_else(|| protocol("replication recovery lock frontier owner is missing"))?;
            for remote in remote_locks {
                let Some(local) = self.decision_locks.get(&remote.position) else {
                    return Err(protocol(
                        "replication recovery requires anti-entropy for missing lock",
                    ));
                };
                if local.effect != remote.effect || local.term < remote.term {
                    return Err(protocol(
                        "replication recovery lock frontier conflicts with peer evidence",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn apply_recovery_certificate(
        &mut self,
        certificate: &ReplicationRecoveryCertificate,
    ) {
        self.leader_certificates.insert(
            (certificate.membership_epoch, certificate.recovery_term),
            ReplicationLeaderCertificate {
                membership_epoch: certificate.membership_epoch,
                term: certificate.recovery_term,
                leader: certificate.leader,
                acknowledged_by: certificate.acknowledged_by.clone(),
            },
        );
        for voter in &certificate.acknowledged_by {
            self.note_promised_term(
                certificate.membership_epoch,
                *voter,
                certificate.recovery_term,
            );
        }
        self.quorum_availability = Some(ReplicationQuorumAvailability::Available {
            membership_epoch: certificate.membership_epoch,
            term: certificate.recovery_term,
        });
    }

    pub(super) fn require_quorum_available(&self) -> Result<(), DurabilityError> {
        if matches!(
            self.quorum_availability,
            Some(ReplicationQuorumAvailability::Lost(_))
        ) {
            Err(protocol(
                "replication consensus authority is fenced by quorum loss",
            ))
        } else {
            Ok(())
        }
    }

    pub(super) fn note_available_term(&mut self, membership_epoch: u64, term: u64) {
        if let Some(ReplicationQuorumAvailability::Available {
            membership_epoch: current_epoch,
            term: current_term,
        }) = &mut self.quorum_availability
            && *current_epoch == membership_epoch
        {
            *current_term = (*current_term).max(term);
        }
    }

    pub(crate) fn mark_quorum_lost(
        &mut self,
        loss: ReplicationQuorumLoss,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.validate_quorum_loss(loss)?;
        if self.quorum_availability == Some(ReplicationQuorumAvailability::Lost(loss)) {
            return Ok(());
        }
        self.append_frame(KIND_QUORUM_LOSS, &encode_quorum_loss(loss))?;
        self.quorum_availability = Some(ReplicationQuorumAvailability::Lost(loss));
        Ok(())
    }

    pub(crate) fn recover_quorum(
        &mut self,
        certificate: &ReplicationRecoveryCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.validate_recovery_certificate(certificate)?;
        let payload = encode_recovery_certificate(certificate)?;
        self.append_frame(KIND_QUORUM_RECOVERY, &payload)?;
        self.apply_recovery_certificate(certificate);
        Ok(())
    }
}

impl ReplicationAuthorityJournal {
    pub(super) fn ensure_recovery_lock_frontier_durable(
        &mut self,
        locks: &[ReplicationLockSummary],
    ) -> Result<Sha256Digest, DurabilityError> {
        validate_lock_summaries(locks)?;
        let digest = recovery_lock_frontier_digest(locks)?;
        if let Some(existing) = self.recovery_lock_frontiers.get(&digest) {
            if existing.as_slice() != locks {
                return Err(protocol(
                    "replication recovery lock frontier digest collision",
                ));
            }
            return Ok(digest);
        }
        let payload = encode_recovery_lock_frontier_owner(locks, digest)?;
        self.append_frame(KIND_RECOVERY_LOCK_FRONTIER_OWNER, &payload)?;
        self.recovery_lock_frontiers.insert(digest, locks.to_vec());
        Ok(digest)
    }

    pub(super) fn signed_recovery_ack_from_ref(
        &self,
        reference: SignedRecoveryAckRef,
    ) -> Result<SignedReplicationPeerEvidence, DurabilityError> {
        let locks = self
            .recovery_lock_frontiers
            .get(&reference.lock_frontier_digest)
            .ok_or_else(|| protocol("replication recovery lock frontier owner is missing"))?
            .clone();
        Ok(SignedReplicationPeerEvidence {
            trust_epoch: reference.trust_epoch,
            signer: reference.signer,
            evidence: ReplicationPeerEvidence::RecoveryAck(ReplicationRecoveryAck {
                voter: reference.voter,
                membership_epoch: reference.membership_epoch,
                recovery_term: reference.recovery_term,
                leader: reference.leader,
                locks,
            }),
            signature: reference.signature,
        })
    }
}
