use std::path::Path;

use kernel_auth::{Sha256Digest, TrustRootSet};
use kernel_change::RevisionEffectId;

use super::DurableRevisionStore;
use super::causal_ledger::causal_prerequisites_for_replicated_effect;
use super::semantic_deployment::install_intent_semantic_modules;
use crate::domain::DurableRevisionEffectRecord;
use crate::replication::{
    ReplicaId, ReplicatedEffectEnvelope, ReplicationAuthenticationReceipt, ReplicationBranchHead,
    ReplicationBranchId, ReplicationDecisionLock, ReplicationDecisionVote, ReplicationEffectStage,
    ReplicationEffectVote, ReplicationIngestOutcome, ReplicationJointMembershipCertificate,
    ReplicationLeaderCertificate, ReplicationLeaderVote, ReplicationMembership,
    ReplicationMembershipChange, ReplicationMembershipVote, ReplicationPeerAuthPolicy,
    ReplicationQuorumAvailability, ReplicationQuorumCertificate, ReplicationQuorumLoss,
    ReplicationRecoveryCertificate, ReplicationTermPromise, SignedReplicationPeerEvidence,
};
use crate::replication_transport::{
    MAX_ANTI_ENTROPY_LOCKS, ReplicationAntiEntropyChunk, ReplicationAntiEntropyRequest,
    ReplicationAntiEntropySummary, ReplicationFailureDetector, ReplicationTransportIngress,
    ReplicationTransportPayload, SignedReplicationTransportFrame, replication_anti_entropy_summary,
    validate_replication_anti_entropy_chunk,
};
use crate::runtime::DurabilityError;

impl DurableRevisionStore {
    fn with_durable_replication_mutation<T>(
        &mut self,
        mutation: impl FnOnce(
            &mut crate::replication::authority::ReplicationAuthorityJournal,
        ) -> Result<T, DurabilityError>,
    ) -> Result<T, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let outcome = mutation(&mut self.replication);
        let frames = self.replication.take_pending_single_file_frames();
        if let Err(error) = self
            .backend
            .persist_replication_frames(&mut self.wal, &frames)
        {
            self.poisoned = true;
            return Err(error);
        }
        if !frames.is_empty() {
            self.replication.commit_single_file_frames(frames);
        }
        outcome
    }

    #[must_use]
    pub fn replication_branch_head(
        &self,
        branch: ReplicationBranchId,
    ) -> Option<ReplicationBranchHead> {
        self.replication.branch_head(branch)
    }

    #[must_use]
    pub fn replication_published_branch_head(
        &self,
        branch: ReplicationBranchId,
    ) -> Option<ReplicationBranchHead> {
        self.replication.published_branch_head(branch)
    }

    #[must_use]
    pub fn current_replication_membership(&self) -> Option<&ReplicationMembership> {
        self.replication.current_membership()
    }

    #[must_use]
    pub fn replication_effect_stage(
        &self,
        effect: RevisionEffectId,
    ) -> Option<ReplicationEffectStage> {
        self.replication.effect_stage(effect)
    }

    #[must_use]
    pub fn replicated_effect_record(
        &self,
        id: RevisionEffectId,
    ) -> Option<&DurableRevisionEffectRecord> {
        self.replication.effect(id)
    }

    #[must_use]
    pub fn replication_journal_path(&self) -> &Path {
        self.replication.path()
    }

    #[must_use]
    pub fn replication_peer_auth_policy(&self) -> Option<&ReplicationPeerAuthPolicy> {
        self.replication.peer_auth_policy()
    }

    #[must_use]
    pub const fn replication_quorum_availability(&self) -> Option<ReplicationQuorumAvailability> {
        self.replication.quorum_availability()
    }

    #[must_use]
    pub fn replication_authentication_receipt(
        &self,
        proof_digest: Sha256Digest,
    ) -> Option<ReplicationAuthenticationReceipt> {
        self.replication.authentication_receipt(proof_digest)
    }

    /// Compact non-authoritative summary used to decide whether peers must
    /// exchange decision-lock frontier chunks before quorum recovery.
    pub fn replication_anti_entropy_summary(
        &self,
    ) -> Result<Option<ReplicationAntiEntropySummary>, DurabilityError> {
        let Some(membership) = self.replication.current_membership() else {
            return Ok(None);
        };
        let locks = self.replication.decision_lock_summaries();
        replication_anti_entropy_summary(
            membership.epoch,
            self.replication.current_consensus_term(),
            &locks,
        )
        .map(Some)
    }

    /// Returns one bounded, ordered lock-frontier chunk. Chunks carry no
    /// authority by themselves; they only drive authenticated reconciliation.
    pub fn replication_anti_entropy_chunk(
        &self,
        request: ReplicationAntiEntropyRequest,
    ) -> Result<ReplicationAntiEntropyChunk, DurabilityError> {
        let current = self
            .replication
            .current_membership()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "replication anti-entropy request has no membership",
            })?;
        if request.membership_epoch != current.epoch {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication anti-entropy request uses stale membership",
            });
        }
        let requested = usize::try_from(request.max_locks).unwrap_or(usize::MAX);
        let limit = requested.min(MAX_ANTI_ENTROPY_LOCKS);
        if limit == 0 {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replication anti-entropy request has zero chunk bound",
            });
        }
        let (locks, complete) = self
            .replication
            .decision_lock_summary_chunk(request.from_position, limit);
        Ok(ReplicationAntiEntropyChunk {
            membership_epoch: current.epoch,
            locks,
            complete,
        })
    }

    /// Activates or rotates authenticated peer evidence for replication.
    /// `TrustRootSet` remains the cryptographic authority owned by `kernel-auth`;
    /// this journal durably binds replica identities to key identities and a
    /// monotone trust epoch.
    pub fn durably_install_replication_peer_auth_policy(
        &mut self,
        policy: ReplicationPeerAuthPolicy,
        trust: &TrustRootSet,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.install_peer_auth_policy(policy, trust)
        })
    }

    /// Verifies and durably records signed peer evidence. For ordinary vote /
    /// promise evidence this also appends the corresponding semantic journal
    /// frame, so callers cannot accidentally downgrade a verified message into
    /// an unauthenticated assertion.
    pub fn durably_record_authenticated_replication_peer_evidence(
        &mut self,
        trust: &TrustRootSet,
        signed: SignedReplicationPeerEvidence,
    ) -> Result<ReplicationAuthenticationReceipt, DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.record_authenticated_peer_evidence(trust, signed)
        })
    }

    /// Authenticates one semantic transport frame against the durable peer policy and
    /// routes authority-bearing peer evidence into the replication journal.
    /// Advisory heartbeat/anti-entropy payloads never create durable authority.
    pub fn durably_accept_replication_transport_frame(
        &mut self,
        ingress: &mut ReplicationTransportIngress,
        trust: &TrustRootSet,
        signed: SignedReplicationTransportFrame,
    ) -> Result<Option<ReplicationAuthenticationReceipt>, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let policy = self.replication_transport_policy()?;
        ingress.accept(&policy, trust, &signed)?;
        self.apply_accepted_replication_transport_frame(trust, signed)
    }

    /// Decodes the stateful physical wire lowering, authenticates the reconstructed
    /// semantic frame, and only then allows authority-bearing evidence to reach the
    /// durable replication journal.
    pub fn durably_accept_replication_transport_bytes(
        &mut self,
        ingress: &mut ReplicationTransportIngress,
        trust: &TrustRootSet,
        bytes: &[u8],
    ) -> Result<Option<ReplicationAuthenticationReceipt>, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        let policy = self.replication_transport_policy()?;
        let signed = ingress.accept_wire(&policy, trust, bytes)?;
        self.apply_accepted_replication_transport_frame(trust, signed)
    }

    fn replication_transport_policy(&self) -> Result<ReplicationPeerAuthPolicy, DurabilityError> {
        self.replication
            .peer_auth_policy()
            .cloned()
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "replication transport has no durable peer auth policy",
            })
    }

    fn apply_accepted_replication_transport_frame(
        &mut self,
        trust: &TrustRootSet,
        signed: SignedReplicationTransportFrame,
    ) -> Result<Option<ReplicationAuthenticationReceipt>, DurabilityError> {
        let current_epoch = {
            let current =
                self.replication
                    .current_membership()
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication transport has no durable membership",
                    })?;
            if !current.members.contains(&signed.frame.sender) {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replication transport sender is not in current membership",
                });
            }
            current.epoch
        };
        match signed.frame.payload {
            ReplicationTransportPayload::PeerEvidence(peer) => self
                .with_durable_replication_mutation(|replication| {
                    replication.record_authenticated_peer_evidence(trust, peer)
                })
                .map(Some),
            ReplicationTransportPayload::AntiEntropySummary(summary) => {
                if summary.membership_epoch != current_epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication anti-entropy summary uses stale membership",
                    });
                }
                Ok(None)
            }
            ReplicationTransportPayload::AntiEntropyRequest(request) => {
                if request.membership_epoch != current_epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication anti-entropy request uses stale membership",
                    });
                }
                Ok(None)
            }
            ReplicationTransportPayload::AntiEntropyChunk(chunk) => {
                if chunk.membership_epoch != current_epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication anti-entropy chunk uses stale membership",
                    });
                }
                validate_replication_anti_entropy_chunk(&chunk)?;
                Ok(None)
            }
            ReplicationTransportPayload::Heartbeat(heartbeat) => {
                if heartbeat.membership_epoch != current_epoch {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "replication heartbeat uses stale membership",
                    });
                }
                Ok(None)
            }
        }
    }

    /// Converts a local failure-detector observation into the existing durable
    /// quorum-loss safety fence. False suspicions may reduce liveness but cannot
    /// create authority; recovery still requires a separate authenticated quorum.
    pub fn durably_fence_replication_if_quorum_unreachable(
        &mut self,
        detector: &ReplicationFailureDetector,
        observed_term: u64,
    ) -> Result<bool, DurabilityError> {
        let membership =
            self.replication
                .current_membership()
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "replication failure detector has no durable membership",
                })?;
        let Some(loss) = detector.quorum_loss_observation(membership, observed_term) else {
            return Ok(false);
        };
        self.with_durable_replication_mutation(|replication| replication.mark_quorum_lost(loss))?;
        Ok(true)
    }

    /// Durably fences new consensus authority after local quorum-loss
    /// detection. Already-published state remains readable, while local effect
    /// durability may continue without becoming quorum/publication authority.
    pub fn durably_mark_replication_quorum_lost(
        &mut self,
        loss: ReplicationQuorumLoss,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| replication.mark_quorum_lost(loss))
    }

    /// Completes recovery only after an authenticated membership quorum reports
    /// a lock frontier reconciled with the local durable journal and the
    /// recovery term advances every known safety floor.
    pub fn durably_recover_replication_quorum(
        &mut self,
        certificate: &ReplicationRecoveryCertificate,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.recover_quorum(certificate)
        })
    }

    /// Durably ingests one already-ordered remote REIC effect without changing
    /// the single published revision head. Branch arrival is therefore never a
    /// second publication authority. The exact effect ontology is shared with
    /// local commits; only its lifecycle journal is separate from the linear
    /// transaction WAL.
    pub fn durably_ingest_replicated_effect(
        &mut self,
        envelope: ReplicatedEffectEnvelope,
    ) -> Result<ReplicationIngestOutcome, DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.revision_effects.contains_key(&envelope.effect.id) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect identity collides with local causal authority",
            });
        }
        if self
            .revision_effect_frontiers
            .contains_key(&envelope.effect.target_revision)
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated target revision already belongs to local causal authority",
            });
        }
        if !envelope.effect.intent.is_exact() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect does not carry exact executable intent",
            });
        }
        let expected = causal_prerequisites_for_replicated_effect(
            &self.revision_effect_frontiers,
            &self.replication,
            &envelope.effect,
        )?;
        if expected != envelope.effect.prerequisites {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "replicated effect prerequisites do not equal the authoritative causal cut",
            });
        }
        let mut provisional_registry = self.semantic_registry.clone();
        install_intent_semantic_modules(&mut provisional_registry, &envelope.effect.intent)?;
        let outcome =
            self.with_durable_replication_mutation(|replication| replication.ingest(envelope))?;
        self.semantic_registry = provisional_registry;
        Ok(outcome)
    }

    pub fn durably_retire_replication_branch(
        &mut self,
        branch: ReplicationBranchId,
        expected_head: RevisionEffectId,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.retire_branch(branch, expected_head)
        })
    }

    /// Durably installs a replication membership epoch. The first epoch is an
    /// explicit bootstrap; every later epoch must carry the previous epoch's
    /// configured quorum. Peer authentication is a caller/transport obligation,
    /// while this store enforces durable membership/threshold semantics.
    pub fn durably_install_replication_membership(
        &mut self,
        change: ReplicationMembershipChange,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| replication.install_membership(change))
    }

    /// Persists one already-authenticated peer vote for a replicated decision
    /// slot. The journal enforces vote-once for that membership epoch/position.
    pub fn durably_record_replicated_effect_vote(
        &mut self,
        vote: ReplicationEffectVote,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| replication.record_effect_vote(vote))
    }

    /// Persists one already-authenticated vote for the successor membership.
    /// One voter cannot durably support conflicting successors of one epoch.
    pub fn durably_record_replication_membership_vote(
        &mut self,
        vote: ReplicationMembershipVote,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.record_membership_vote(vote)
        })
    }

    #[must_use]
    pub fn replication_promised_term(
        &self,
        membership_epoch: u64,
        voter: ReplicaId,
    ) -> Option<u64> {
        self.replication.promised_term(membership_epoch, voter)
    }

    #[must_use]
    pub fn replication_leader_certificate(
        &self,
        membership_epoch: u64,
        term: u64,
    ) -> Option<&ReplicationLeaderCertificate> {
        self.replication.leader_certificate(membership_epoch, term)
    }

    #[must_use]
    pub fn replication_decision_lock(&self, position: u64) -> Option<&ReplicationDecisionLock> {
        self.replication.decision_lock(position)
    }

    pub fn durably_record_replication_term_promise(
        &mut self,
        promise: ReplicationTermPromise,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.record_term_promise(promise)
        })
    }

    pub fn durably_record_replication_leader_vote(
        &mut self,
        vote: ReplicationLeaderVote,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| replication.record_leader_vote(vote))
    }

    pub fn durably_certify_replication_leader(
        &mut self,
        certificate: ReplicationLeaderCertificate,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.certify_leader(certificate)
        })
    }

    pub fn durably_certify_replication_joint_membership(
        &mut self,
        certificate: ReplicationJointMembershipCertificate,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.certify_joint_membership(certificate)
        })
    }

    pub fn durably_record_replication_decision_vote(
        &mut self,
        vote: ReplicationDecisionVote,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| replication.record_decision_vote(vote))
    }

    pub fn durably_lock_replication_decision(
        &mut self,
        lock: ReplicationDecisionLock,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| replication.lock_decision(lock))
    }

    /// Advances one replicated effect from `LocalDurable` to `QuorumDurable`.
    pub fn durably_certify_replicated_effect_quorum(
        &mut self,
        certificate: ReplicationQuorumCertificate,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| {
            replication.certify_quorum(certificate)
        })
    }

    /// Marks one quorum-durable effect reader-publishable in branch order.
    /// This does not mutate the store's single linear `durable_head`.
    pub fn durably_publish_replicated_effect(
        &mut self,
        effect: RevisionEffectId,
    ) -> Result<(), DurabilityError> {
        self.with_durable_replication_mutation(|replication| replication.publish(effect))
    }
}
