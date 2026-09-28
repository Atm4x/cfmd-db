use std::collections::BTreeSet;

use kernel_change::RevisionEffectId;

use super::ReplicationAuthorityJournal;
use crate::binary_codec::push_u128;
use crate::replication::codec::{
    KIND_EFFECT_VOTE, KIND_INGEST, KIND_PUBLISH, KIND_QUORUM, KIND_RETIRE, encode_effect_vote,
    encode_ingest, encode_quorum_certificate, validate_acknowledgements,
};
use crate::replication::{
    ReplicatedEffectEnvelope, ReplicationBranchHead, ReplicationBranchId, ReplicationEffectVote,
    ReplicationIngestOutcome, ReplicationPeerEvidence, ReplicationQuorumCertificate, protocol,
    replicated_effect_id, replicated_origin,
};
use crate::runtime::DurabilityError;
impl ReplicationAuthorityJournal {
    fn validate_publish(&self, effect: RevisionEffectId) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        if !self.quorum_certificates.contains_key(&effect) {
            return Err(protocol(
                "replicated effect cannot publish before quorum durability",
            ));
        }
        let envelope = self
            .effects
            .get(&effect)
            .ok_or_else(|| protocol("published replicated effect is missing"))?;
        if envelope.effect.prerequisites.iter().any(|prerequisite| {
            replicated_origin(*prerequisite).is_some()
                && !self.quorum_certificates.contains_key(prerequisite)
        }) {
            return Err(protocol(
                "replicated effect cannot publish before replicated prerequisites are quorum durable",
            ));
        }
        if self
            .branches
            .get(&envelope.branch)
            .is_some_and(|branch| branch.retired)
        {
            return Err(protocol("retired replication branch cannot publish"));
        }
        if let Some(published) = self.published_branches.get(&envelope.branch) {
            if envelope.effect.source_revision != published.head_revision {
                return Err(protocol("replicated branch publication is not contiguous"));
            }
        } else if let Some(frontier) = self
            .revision_frontiers
            .get(&envelope.effect.source_revision)
            && frontier.iter().any(|predecessor| {
                self.effects
                    .get(predecessor)
                    .is_some_and(|candidate| candidate.branch == envelope.branch)
            })
        {
            return Err(protocol(
                "replicated branch publication skipped an earlier local-durable effect",
            ));
        }
        Ok(())
    }

    pub(super) fn apply_publish(
        &mut self,
        effect: RevisionEffectId,
    ) -> Result<(), DurabilityError> {
        self.validate_publish(effect)?;
        let envelope = self
            .effects
            .get(&effect)
            .expect("validated publication references durable effect");
        self.published_effects.insert(effect);
        self.published_branches.insert(
            envelope.branch,
            ReplicationBranchHead {
                branch: envelope.branch,
                head_revision: envelope.effect.target_revision,
                head_effect: effect,
                retired: false,
            },
        );
        Ok(())
    }

    fn validate_envelope(
        &self,
        envelope: &ReplicatedEffectEnvelope,
    ) -> Result<(), DurabilityError> {
        if envelope.origin.raw() == 0 || envelope.origin_sequence == 0 {
            return Err(protocol("replicated effect origin namespace is invalid"));
        }
        if envelope.branch.raw() == 0 {
            return Err(protocol("replication branch identity is zero"));
        }
        if envelope.ordered_by.sequencer.raw() == 0 || envelope.ordered_by.position == 0 {
            return Err(protocol(
                "replicated effect lacks a valid ordered admission witness",
            ));
        }
        if self
            .sequencer_epochs
            .get(&envelope.ordered_by.sequencer)
            .is_some_and(|current| envelope.ordered_by.epoch < *current)
        {
            return Err(protocol("replicated effect uses a stale sequencer epoch"));
        }
        let expected = replicated_effect_id(envelope.origin, envelope.origin_sequence);
        if envelope.effect.id != expected {
            return Err(protocol(
                "replicated effect id does not match origin sequence namespace",
            ));
        }
        envelope.effect.validate_identity().map_err(protocol)?;
        if let Some(existing) = self.ordered_slots.get(&envelope.ordered_by)
            && *existing != envelope.effect.id
        {
            return Err(protocol(
                "sequencer order slot is already bound to another effect",
            ));
        }
        if envelope
            .effect
            .prerequisites
            .iter()
            .any(|id| replicated_origin(*id).is_some() && !self.effects.contains_key(id))
        {
            return Err(protocol(
                "replicated effect is not down-closed over remote dependencies",
            ));
        }
        if let Some(head) = self.branches.get(&envelope.branch) {
            if head.retired {
                return Err(protocol("retired replication branch cannot be advanced"));
            }
            if envelope.effect.source_revision != head.head_revision {
                return Err(protocol(
                    "replicated effect does not advance the durable branch head",
                ));
            }
        }
        if self
            .revision_frontiers
            .contains_key(&envelope.effect.target_revision)
        {
            return Err(protocol(
                "replicated target revision already has a durable branch frontier",
            ));
        }
        Ok(())
    }

    pub(super) fn apply_ingest(
        &mut self,
        envelope: ReplicatedEffectEnvelope,
    ) -> Result<(), DurabilityError> {
        self.validate_envelope(&envelope)?;
        let id = envelope.effect.id;
        let target = envelope.effect.target_revision;
        self.ordered_slots.insert(envelope.ordered_by, id);
        self.sequencer_epochs
            .entry(envelope.ordered_by.sequencer)
            .and_modify(|epoch| *epoch = (*epoch).max(envelope.ordered_by.epoch))
            .or_insert(envelope.ordered_by.epoch);
        self.revision_frontiers.insert(target, BTreeSet::from([id]));
        self.branches.insert(
            envelope.branch,
            ReplicationBranchHead {
                branch: envelope.branch,
                head_revision: target,
                head_effect: id,
                retired: false,
            },
        );
        self.effects.insert(id, envelope);
        Ok(())
    }
}

impl ReplicationAuthorityJournal {
    pub(crate) fn ingest(
        &mut self,
        envelope: ReplicatedEffectEnvelope,
    ) -> Result<ReplicationIngestOutcome, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if let Some(existing) = self.effects.get(&envelope.effect.id) {
            return if existing == &envelope {
                Ok(ReplicationIngestOutcome::AlreadyPresent)
            } else {
                Err(protocol(
                    "replicated effect identity conflicts with durable branch journal",
                ))
            };
        }
        self.validate_envelope(&envelope)?;
        let payload = encode_ingest(&envelope)?;
        self.append_frame(KIND_INGEST, &payload)?;
        self.apply_ingest(envelope)?;
        Ok(ReplicationIngestOutcome::Inserted)
    }

    pub(crate) fn retire_branch(
        &mut self,
        branch: ReplicationBranchId,
        expected_head: RevisionEffectId,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let head = self
            .branches
            .get(&branch)
            .copied()
            .ok_or_else(|| protocol("replication branch does not exist"))?;
        if head.retired {
            return Err(protocol("replication branch is already retired"));
        }
        if head.head_effect != expected_head {
            return Err(protocol("replication branch retirement head changed"));
        }
        let mut payload = Vec::new();
        push_u128(&mut payload, branch.raw());
        push_u128(&mut payload, expected_head.0);
        self.append_frame(KIND_RETIRE, &payload)?;
        self.branches
            .get_mut(&branch)
            .expect("validated branch exists")
            .retired = true;
        if let Some(published) = self.published_branches.get_mut(&branch) {
            published.retired = true;
        }
        Ok(())
    }

    pub(crate) fn record_effect_vote(
        &mut self,
        vote: ReplicationEffectVote,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::EffectVote(vote))?;
        self.validate_effect_vote(&vote)?;
        let envelope = self
            .effects
            .get(&vote.effect)
            .expect("validated vote references local-durable effect");
        let key = (
            vote.membership_epoch,
            envelope.ordered_by.position,
            vote.voter,
        );
        if let Some(existing) = self.effect_votes.get(&key) {
            return if *existing == vote.effect {
                Ok(())
            } else {
                Err(protocol(
                    "replication voter already voted for another effect in this decision slot",
                ))
            };
        }
        let payload = encode_effect_vote(&vote);
        self.append_frame(KIND_EFFECT_VOTE, &payload)?;
        self.effect_votes.insert(key, vote.effect);
        Ok(())
    }

    pub(crate) fn certify_quorum(
        &mut self,
        certificate: ReplicationQuorumCertificate,
    ) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if let Some(existing) = self.quorum_certificates.get(&certificate.effect) {
            return if existing == &certificate {
                Ok(())
            } else {
                Err(protocol(
                    "replicated effect already has a different durable quorum certificate",
                ))
            };
        }
        self.validate_quorum_certificate(&certificate)?;
        let payload = encode_quorum_certificate(&certificate)?;
        self.append_frame(KIND_QUORUM, &payload)?;
        self.quorum_certificates
            .insert(certificate.effect, certificate);
        Ok(())
    }

    pub(crate) fn publish(&mut self, effect: RevisionEffectId) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        self.require_quorum_available()?;
        if self.published_effects.contains(&effect) {
            return Ok(());
        }
        self.validate_publish(effect)?;
        let mut payload = Vec::new();
        push_u128(&mut payload, effect.0);
        self.append_frame(KIND_PUBLISH, &payload)?;
        self.apply_publish(effect)
    }
}

impl ReplicationAuthorityJournal {
    pub(super) fn validate_quorum_certificate(
        &self,
        certificate: &ReplicationQuorumCertificate,
    ) -> Result<(), DurabilityError> {
        self.require_quorum_available()?;
        if !self.effects.contains_key(&certificate.effect) {
            return Err(protocol(
                "quorum certificate references a non-local-durable replicated effect",
            ));
        }
        let Some(current) = self.current_membership() else {
            return Err(protocol("replication quorum has no durable membership"));
        };
        if certificate.membership_epoch != current.epoch {
            return Err(protocol(
                "replication quorum certificate uses a stale membership epoch",
            ));
        }
        validate_acknowledgements(
            current,
            &certificate.acknowledged_by,
            "replication quorum certificate lacks configured quorum",
        )?;
        let position = self
            .effects
            .get(&certificate.effect)
            .expect("validated certificate references local-durable effect")
            .ordered_by
            .position;
        if self.has_leader_certificate_for_epoch(current.epoch) {
            let lock = self
                .decision_locks
                .get(&position)
                .ok_or_else(|| protocol("replication quorum lacks consensus decision lock"))?;
            if lock.effect != certificate.effect || lock.membership_epoch != current.epoch {
                return Err(protocol(
                    "replication quorum does not match consensus decision lock",
                ));
            }
        }
        let position = self
            .effects
            .get(&certificate.effect)
            .expect("validated certificate references local-durable effect")
            .ordered_by
            .position;
        for voter in &certificate.acknowledged_by {
            if self
                .effect_votes
                .get(&(certificate.membership_epoch, position, *voter))
                != Some(&certificate.effect)
            {
                return Err(protocol(
                    "replication quorum certificate lacks durable vote evidence",
                ));
            }
            self.require_authenticated_peer_evidence(&ReplicationPeerEvidence::EffectVote(
                ReplicationEffectVote {
                    voter: *voter,
                    effect: certificate.effect,
                    membership_epoch: certificate.membership_epoch,
                },
            ))?;
        }
        Ok(())
    }

    pub(super) fn validate_effect_vote(
        &self,
        vote: &ReplicationEffectVote,
    ) -> Result<(), DurabilityError> {
        let Some(current) = self.current_membership() else {
            return Err(protocol("replication vote has no durable membership"));
        };
        if vote.membership_epoch != current.epoch {
            return Err(protocol("replication vote uses a stale membership epoch"));
        }
        if !current.members.contains(&vote.voter) {
            return Err(protocol("replication vote references a non-member replica"));
        }
        let envelope = self
            .effects
            .get(&vote.effect)
            .ok_or_else(|| protocol("replication vote references a non-local-durable effect"))?;
        let key = (
            vote.membership_epoch,
            envelope.ordered_by.position,
            vote.voter,
        );
        if self
            .effect_votes
            .get(&key)
            .is_some_and(|existing| *existing != vote.effect)
        {
            return Err(protocol(
                "replication voter already voted for another effect in this decision slot",
            ));
        }
        Ok(())
    }
}
