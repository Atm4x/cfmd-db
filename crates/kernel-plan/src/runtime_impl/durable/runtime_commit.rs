impl DurableRuntime {
    pub(crate) fn check_mixed_client_retry(
        &self,
        transaction_id: ClientTransactionId,
        client_semantic_revision: kernel_types::SemanticRevision,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_model_delta: &DurableModelDelta,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        requested_target: RevisionId,
    ) -> Result<Option<RevisionId>, DurableRuntimeCommitError> {
        let durable_client_mutations = Self::canonical_durable_relation_mutations(client_mutations)?;
        let durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        let Some(committed_intent) = durability.transaction_intent(transaction_id) else {
            return Ok(None);
        };
        if committed_intent.matches_mixed_client_intent(
            client_semantic_revision,
            &durable_client_mutations,
            client_model_delta,
            client_guard_digest,
        ) {
            return Ok(Some(committed_intent.target_revision()));
        }
        Err(DurableRuntimeCommitError::TransactionIdConflict {
            transaction_id,
            committed_target: committed_intent.target_revision(),
            requested_target,
        })
    }

    /// Test-only adapter for the retired arbitrary-target relation commit API.
    ///
    /// Production durability accepts relation-data intent only through
    /// `DerivedRelationTransitionRequest`, where the runtime derives the target
    /// from the authoritative source plus exact deltas. Keeping this adapter
    /// under `cfg(test)` lets old hostile regressions exercise the same law
    /// without retaining a production compatibility path or full-target WAL
    /// witness.
    #[cfg(test)]
    pub(crate) fn commit_revision(
        &self,
        transaction_id: ClientTransactionId,
        request: &RevisionTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let snapshot = self.snapshot()?;
        let source_revision = snapshot.revision_id();
        drop(snapshot);
        self.commit_derived_relation_data(
            transaction_id,
            &DerivedRelationTransitionRequest {
                source_revision,
                target_revision: request.target_revision.id(),
                mutations: request.mutations,
            },
        )
    }

    /// Atomically commits a validated mixed logical revision while applying
    /// only its declared relation deltas to the physical store. Lifecycle,
    /// carrier and field state come from the exact target Revision. Durable
    /// identity retains the full target bytes until a compact non-relation WAL
    /// delta format is introduced, so crash/retry semantics remain exact.
    pub fn commit_mixed_revision(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_mixed_revision_guarded(transaction_id, request, None)
    }

    pub fn commit_mixed_revision_guarded(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_mixed_revision_guarded_with_dependencies(
            transaction_id,
            request,
            client_guard_digest,
            None,
        )
    }

    pub fn commit_mixed_revision_guarded_with_dependencies(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_mixed_revision_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            request,
            client_guard_digest,
            guard_observation,
            &[],
        )
    }

    pub fn commit_mixed_revision_guarded_with_dependencies_and_relational_observations(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let durable_mutations = Self::canonical_durable_relation_mutations(request.mutations)?;
        let requested_intent = DurableTransactionIntent::mixed_revision(
            request.source_revision,
            request.target_revision,
            request.target_revision.semantic_revision(),
            durable_mutations,
            request.model_delta.clone(),
            request.model_complement.clone(),
            &self.registry,
        )
        .map_err(DurabilityError::Encode)
        .map_err(DurableRuntimeCommitError::PrepareDurability)?
        .with_client_guard_digest(client_guard_digest)
        .with_causal_observations(
            guard_observation
                .into_iter()
                .flat_map(|observation| observation.coordinates().iter().map(|coordinate| {
                    durable_causal_observation(
                        coordinate,
                        observation.exact_value(coordinate),
                        observation.preservation_rule(coordinate),
                    )
                })),
        )
        .with_causal_observation_groups(
            guard_observation
                .into_iter()
                .flat_map(|observation| observation.joint_groups().iter().filter_map(durable_causal_observation_group)),
        )
        .with_relational_causal_observations(
            relational_observations
                .iter()
                .map(durable_relational_causal_observation),
        );
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            if committed_intent.same_client_intent(&requested_intent) {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: committed_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: requested_intent.target_revision(),
            });
        }
        let authorized_request = MixedRevisionTransitionRequest {
            source_revision: request.source_revision,
            target_revision: request.target_revision,
            mutations: request.mutations,
            model_delta: request.model_delta,
            model_complement: request.model_complement,
            registry: &self.registry,
        };
        let snapshot = self.cell.snapshot()?;
        if let Some(guard_observation) = guard_observation {
            match snapshot
                .certify_read_dependencies_stable(guard_observation)
                .map_err(|error| match error {
                    RuntimeHistoricalSnapshotError::Runtime(error) => DurableRuntimeCommitError::Runtime(error),
                    RuntimeHistoricalSnapshotError::Durability(error) => DurableRuntimeCommitError::PrepareDurability(error),
                    _ => DurableRuntimeCommitError::Runtime(PhysicalExecutionError::InvalidRevisionTransition),
                })?
            {
                RuntimeTransitionRebaseOutcome::Certified(_) => {}
                RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                    return Err(DurableRuntimeCommitError::GuardDependencyConflict(conflict));
                }
            }
        }
        let prepared = snapshot.prepare_mixed_revision(&authorized_request)?.with_causal_observations(guard_observation);
        drop(snapshot);
        self.cell
            .commit_prepared_durable_guarded(
                transaction_id,
                prepared,
                client_guard_digest,
                &self.registry,
                &mut *durability,
            )
            .map(|receipt| {
                let relations = request
                    .mutations
                    .iter()
                    .map(|mutation| mutation.relation)
                    .collect::<Vec<_>>();
                self.signal_relation_publication(&relations);
                DurableRuntimeCommitOutcome::Committed(receipt)
            })
    }

    /// Publishes a certified residual mixed-data effect while retaining the
    /// original client relation/model effect as durable retry identity.
    pub fn commit_mixed_revision_residual(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_model_delta: &DurableModelDelta,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_mixed_revision_residual_guarded(
            transaction_id,
            request,
            client_mutations,
            client_model_delta,
            None,
        )
    }

    pub fn commit_mixed_revision_residual_guarded(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_model_delta: &DurableModelDelta,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_mixed_revision_residual_guarded_with_dependencies(
            transaction_id,
            request,
            client_mutations,
            client_model_delta,
            client_guard_digest,
            None,
        )
    }

    /// Security-critical residual publication with passive dependency proof
    /// bound to the same immutable source root as transition preparation.
    /// Retry identity remains the original client effect + guard digest; the
    /// dependency list is proof material, not a second idempotency identity.
    pub fn commit_mixed_revision_residual_guarded_with_dependencies(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_model_delta: &DurableModelDelta,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_mixed_revision_residual_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            request,
            client_mutations,
            client_model_delta,
            client_guard_digest,
            guard_observation,
            &[],
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    pub fn commit_mixed_revision_residual_guarded_with_dependencies_and_relational_observations(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_model_delta: &DurableModelDelta,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_mixed_revision_residual_guarded_with_client_semantics_and_dependencies(
            transaction_id,
            request,
            client_mutations,
            client_model_delta,
            request.target_revision.semantic_revision(),
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    pub(crate) fn commit_mixed_revision_residual_guarded_with_client_semantics_and_dependencies(
        &self,
        transaction_id: ClientTransactionId,
        request: &MixedRevisionTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_model_delta: &DurableModelDelta,
        client_semantic_revision: kernel_types::SemanticRevision,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let durable_client_mutations = Self::canonical_durable_relation_mutations(client_mutations)?;
        let durable_causal_observations = guard_observation
            .map(|observation| {
                observation.coordinates().iter()
                    .map(|coordinate| durable_causal_observation(
                        coordinate,
                        observation.exact_value(coordinate),
                        observation.preservation_rule(coordinate),
                    ))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let requested_intent = DurableTransactionIntent::mixed_revision(
            request.source_revision,
            request.target_revision,
            client_semantic_revision,
            durable_client_mutations.clone(),
            client_model_delta.clone(),
            request.model_complement.clone(),
            &self.registry,
        )
        .map_err(DurabilityError::Encode)
        .map_err(DurableRuntimeCommitError::PrepareDurability)?
        .with_client_guard_digest(client_guard_digest)
        .with_causal_observations(durable_causal_observations)
        .with_causal_observation_groups(
            guard_observation
                .into_iter()
                .flat_map(|observation| observation.joint_groups().iter().filter_map(durable_causal_observation_group)),
        )
        .with_relational_causal_observations(
            relational_observations
                .iter()
                .map(durable_relational_causal_observation),
        );
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            if committed_intent.same_client_intent(&requested_intent) {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: committed_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: request.target_revision.id(),
            });
        }
        let authorized_request = MixedRevisionTransitionRequest {
            source_revision: request.source_revision,
            target_revision: request.target_revision,
            mutations: request.mutations,
            model_delta: request.model_delta,
            model_complement: request.model_complement,
            registry: &self.registry,
        };
        let snapshot = self.cell.snapshot()?;
        if let Some(guard_observation) = guard_observation {
            match snapshot
                .certify_read_dependencies_stable(guard_observation)
                .map_err(|error| match error {
                    RuntimeHistoricalSnapshotError::Runtime(error) => DurableRuntimeCommitError::Runtime(error),
                    RuntimeHistoricalSnapshotError::Durability(error) => DurableRuntimeCommitError::PrepareDurability(error),
                    _ => DurableRuntimeCommitError::Runtime(PhysicalExecutionError::InvalidRevisionTransition),
                })?
            {
                RuntimeTransitionRebaseOutcome::Certified(_) => {}
                RuntimeTransitionRebaseOutcome::Conflict(conflict) => {
                    return Err(DurableRuntimeCommitError::GuardDependencyConflict(conflict));
                }
            }
        }
        let prepared = snapshot
            .prepare_mixed_revision(&authorized_request)?
            .with_causal_observations(guard_observation);
        drop(snapshot);
        self.cell
            .commit_prepared_mixed_residual_durable(
                transaction_id,
                prepared,
                durable_client_mutations,
                client_model_delta.clone(),
                request.model_complement.clone(),
                client_semantic_revision,
                client_guard_digest,
                &self.registry,
                &mut *durability,
            )
            .map(|receipt| {
                let relations = request
                    .mutations
                    .iter()
                    .map(|mutation| mutation.relation)
                    .collect::<Vec<_>>();
                self.signal_relation_publication(&relations);
                DurableRuntimeCommitOutcome::Committed(receipt)
            })
    }

    /// Delta-authoritative compact durability path. The caller names source
    /// and target revisions and supplies only typed relation deltas; the target
    /// Revision content is constructed from the live authoritative source by
    /// this runtime and therefore cannot be independently substituted.
    pub fn commit_derived_relation_data(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_relation_data_guarded(transaction_id, request, None)
    }

    /// Checks durable retry authority for one exact relation-data client intent without
    /// publishing an unknown transaction. The caller supplies the canonical client mutation
    /// vocabulary directly, so an uncertain retry remains recognizable even after the current
    /// semantic schema has advanced beyond the request's formation revision.
    pub fn check_relation_data_retry(
        &self,
        transaction_id: ClientTransactionId,
        mut durable_mutations: Vec<DurableRelationMutation>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        requested_target: RevisionId,
    ) -> Result<Option<RevisionId>, DurableRuntimeCommitError> {
        durable_mutations.sort_by_key(|mutation| mutation.relation);
        if let Some(pair) = durable_mutations
            .windows(2)
            .find(|pair| pair[0].relation == pair[1].relation)
        {
            return Err(PhysicalExecutionError::DuplicateRelationMutation(pair[0].relation).into());
        }
        let durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        let Some(committed_intent) = durability.transaction_intent(transaction_id) else {
            return Ok(None);
        };
        let matches = matches!(
                &committed_intent.intent,
                DurableClientIntent::RelationData { relation_mutations, guard_digest, .. }
                    if relation_mutations == &durable_mutations && *guard_digest == client_guard_digest
            );
        if matches {
            return Ok(Some(committed_intent.target_revision()));
        }
        Err(DurableRuntimeCommitError::TransactionIdConflict {
            transaction_id,
            committed_target: committed_intent.target_revision(),
            requested_target,
        })
    }

    pub fn commit_derived_relation_data_guarded(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_relation_data_guarded_with_dependencies(
            transaction_id,
            request,
            client_guard_digest,
            None,
        )
    }

    pub fn commit_derived_relation_data_guarded_with_dependencies(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_relation_data_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            request,
            client_guard_digest,
            guard_observation,
            &[],
        )
    }

    pub fn commit_derived_relation_data_guarded_with_dependencies_and_relational_observations(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let durable_mutations = Self::canonical_durable_relation_mutations(request.mutations)?;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            let matches = matches!(
                &committed_intent.intent,
                DurableClientIntent::RelationData { relation_mutations, guard_digest, .. }
                    if relation_mutations == &durable_mutations && *guard_digest == client_guard_digest
            );
            if matches {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: committed_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: request.target_revision,
            });
        }

        let snapshot = self.cell.snapshot()?;
        if snapshot.revision().id() != request.source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition.into());
        }
        let target = self.derive_relation_target(
            snapshot.revision(),
            request.target_revision,
            request.mutations,
        )?;
        let prepared =
            snapshot
                .root()
                .prepare_revision_derived(&target, request.mutations, &self.registry)?
                .with_causal_observations(guard_observation)
                .with_relational_causal_observations(relational_observations.iter().cloned());
        drop(snapshot);
        self.cell
            .commit_prepared_durable_guarded(
                transaction_id,
                prepared,
                client_guard_digest,
                &self.registry,
                &mut *durability,
            )
            .map(|receipt| {
                let relations = request
                    .mutations
                    .iter()
                    .map(|mutation| mutation.relation)
                    .collect::<Vec<_>>();
                self.signal_relation_publication(&relations);
                DurableRuntimeCommitOutcome::Committed(receipt)
            })
    }

    /// Publishes a certified residual relation-data effect while retaining the
    /// original client mutation set as the durable retry identity.
    pub fn commit_derived_relation_data_residual(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_relation_data_residual_guarded(
            transaction_id,
            request,
            client_mutations,
            None,
        )
    }

    pub fn commit_derived_relation_data_residual_guarded(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let client_semantic_revision = self.snapshot()?.revision().semantic_revision();
        self.commit_derived_relation_data_residual_guarded_with_client_semantics_and_dependencies(
            transaction_id,
            request,
            client_mutations,
            client_semantic_revision,
            client_guard_digest,
            None,
            &[],
        )
    }

    pub fn commit_derived_relation_data_residual_guarded_with_dependencies(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_relation_data_residual_guarded_with_dependencies_and_relational_observations(
            transaction_id,
            request,
            client_mutations,
            client_guard_digest,
            guard_observation,
            &[],
        )
    }

    pub fn commit_derived_relation_data_residual_guarded_with_dependencies_and_relational_observations(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let client_semantic_revision = self.snapshot()?.revision().semantic_revision();
        self.commit_derived_relation_data_residual_guarded_with_client_semantics_and_dependencies(
            transaction_id,
            request,
            client_mutations,
            client_semantic_revision,
            client_guard_digest,
            guard_observation,
            relational_observations,
        )
    }

    pub(crate) fn commit_derived_relation_data_residual_guarded_with_client_semantics(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_semantic_revision: kernel_types::SemanticRevision,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_relation_data_residual_guarded_with_client_semantics_and_dependencies(
            transaction_id,
            request,
            client_mutations,
            client_semantic_revision,
            client_guard_digest,
            None,
            &[],
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    pub(crate) fn commit_derived_relation_data_residual_guarded_with_client_semantics_and_dependencies(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationTransitionRequest<'_>,
        client_mutations: &[RevisionRelationMutation<'_>],
        client_semantic_revision: kernel_types::SemanticRevision,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        guard_observation: Option<&RuntimeGuardObservationFootprint>,
        relational_observations: &[RuntimeRelationalCausalObservation],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let durable_client_mutations = Self::canonical_durable_relation_mutations(client_mutations)?;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            let matches = matches!(
                &committed_intent.intent,
                DurableClientIntent::RelationData { semantic_revision, relation_mutations, guard_digest, .. }
                    if *semantic_revision == client_semantic_revision
                        && relation_mutations == &durable_client_mutations
                        && *guard_digest == client_guard_digest
            );
            if matches {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: committed_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: request.target_revision,
            });
        }

        let snapshot = self.cell.snapshot()?;
        if snapshot.revision().id() != request.source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition.into());
        }
        let target = self.derive_relation_target(
            snapshot.revision(),
            request.target_revision,
            request.mutations,
        )?;
        let prepared = snapshot
            .root()
            .prepare_revision_derived(&target, request.mutations, &self.registry)?
            .with_causal_observations(guard_observation)
            .with_relational_causal_observations(relational_observations.iter().cloned());
        drop(snapshot);
        self.cell
            .commit_prepared_relation_residual_durable(
                transaction_id,
                prepared,
                durable_client_mutations,
                client_semantic_revision,
                client_guard_digest,
                &self.registry,
                &mut *durability,
            )
            .map(|receipt| {
                let relations = request
                    .mutations
                    .iter()
                    .map(|mutation| mutation.relation)
                    .collect::<Vec<_>>();
                self.signal_relation_publication(&relations);
                DurableRuntimeCommitOutcome::Committed(receipt)
            })
    }

    /// Delta-authoritative durable Rewrite path. Exact transaction identity
    /// includes RewriteSpec/law-set IDs in addition to source/target/delta, so
    /// endpoint-equivalent intents are not collapsed across restart/retry.
    pub fn commit_derived_relation_rewrites<I>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let (durable_mutations, durable_rewrite_intents) =
            Self::canonical_durable_relation_rewrites(request.rewrites)?;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            let matches = matches!(
                &committed_intent.intent,
                DurableClientIntent::RelationRewrite { relation_mutations, rewrite_intents, .. }
                    if relation_mutations == &durable_mutations
                        && rewrite_intents == &durable_rewrite_intents
            );
            if matches {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: committed_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: request.target_revision,
            });
        }

        let snapshot = self.cell.snapshot()?;
        if snapshot.revision().id() != request.source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition.into());
        }
        let mutations = request
            .rewrites
            .iter()
            .map(|rewrite| RevisionRelationMutation {
                relation: rewrite.relation,
                delta: rewrite.rewrite.delta(),
            object_field_writes: &[],
            authorization: kernel_durability::DurableRelationAuthorization::default(),
            })
            .collect::<Vec<_>>();
        let target =
            self.derive_relation_target(snapshot.revision(), request.target_revision, &mutations)?;
        drop(snapshot);
        let prepared =
            self.cell
                .prepare_rewrites_derived(&target, request.rewrites, &self.registry)?;
        self.cell
            .commit_prepared_durable(transaction_id, prepared, &self.registry, &mut *durability)
            .map(|receipt| {
                let relations = request
                    .rewrites
                    .iter()
                    .map(|rewrite| rewrite.relation)
                    .collect::<Vec<_>>();
                self.signal_relation_publication(&relations);
                DurableRuntimeCommitOutcome::Committed(receipt)
            })
    }

    /// Durable single-relation resolution publication. The target is still
    /// derived from the authoritative source + `RelationDelta`, but publication
    /// is additionally gated by a complete residual cube and exact candidate
    /// Γ-VMF closure before WAL PREPARE is emitted.
    pub fn commit_derived_coherent_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        cube: RewriteResidualCubeCertificate<RelationValue, I>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        if request.rewrites.len() != 1 {
            return Err(PhysicalExecutionError::ResolutionRequiresSingleRelationRewrite.into());
        }
        let relation = request.rewrites[0].relation;
        let (durable_mutations, durable_rewrite_intents) =
            Self::canonical_durable_relation_rewrites(request.rewrites)?;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            let matches = matches!(
                &committed_intent.intent,
                DurableClientIntent::RelationRewrite { relation_mutations, rewrite_intents, .. }
                    if relation_mutations == &durable_mutations
                        && rewrite_intents == &durable_rewrite_intents
            );
            if matches {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: committed_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: request.target_revision,
            });
        }

        let snapshot = self.cell.snapshot()?;
        if snapshot.revision().id() != request.source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition.into());
        }
        let mutations = request
            .rewrites
            .iter()
            .map(|rewrite| RevisionRelationMutation {
                relation: rewrite.relation,
                delta: rewrite.rewrite.delta(),
            object_field_writes: &[],
            authorization: kernel_durability::DurableRelationAuthorization::default(),
            })
            .collect::<Vec<_>>();
        let target =
            self.derive_relation_target(snapshot.revision(), request.target_revision, &mutations)?;
        drop(snapshot);
        let prepared =
            self.cell
                .prepare_rewrites_derived(&target, request.rewrites, &self.registry)?;
        let coherent = prepared.bind_coherent_resolution(relation, cube, &self.registry)?;
        self.cell
            .commit_coherent_resolution_durable(
                transaction_id,
                coherent,
                &self.registry,
                &mut *durability,
            )
            .map(|receipt| {
                self.signal_relation_publication(&[relation]);
                DurableRuntimeCommitOutcome::Committed(receipt)
            })
    }

    /// Durable coherent resolution whose causal authority explicitly joins
    /// two or more already-covered revision frontiers. Parent revisions are
    /// part of exact transaction identity; the durability owner derives raw
    /// effect prerequisites from those frontiers at PREPARE/recovery time.
    pub fn commit_derived_multi_parent_coherent_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        cube: RewriteResidualCubeCertificate<RelationValue, I>,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let coherence_endpoint = cube.common_endpoint().clone();
        drop(cube);
        self.commit_derived_multi_parent_resolution_endpoint(
            transaction_id,
            request,
            &coherence_endpoint,
            causal_parents,
        )
    }

    /// Durable multi-parent resolution gated by an arbitrary-depth registered
    /// residual-chain certificate. The proof object remains in kernel-change;
    /// this runtime independently rechecks that its certified endpoint is the
    /// concrete prepared candidate and that candidate Γ-VMF closure is zero.
    pub fn commit_derived_multi_parent_residual_chain_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        chain: &RevisionEffectResidualChainCertificate<RelationValue, I>,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_multi_parent_resolution_endpoint(
            transaction_id,
            request,
            chain.common_endpoint(),
            causal_parents,
        )
    }

    /// Durable multi-parent resolution gated by a certified non-singleton
    /// causal frontier cube. Keeping the frontier wrapper intact prevents the
    /// caller from discarding the causal-layer membership proof and passing a
    /// detached raw cube as if it certified an arbitrary effect set.
    pub fn commit_derived_multi_parent_frontier_cube_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        frontier: &RevisionEffectResidualCubeLayerCertificate<RelationValue, I>,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_multi_parent_resolution_endpoint(
            transaction_id,
            request,
            frontier.common_endpoint(),
            causal_parents,
        )
    }

    /// Durable multi-parent resolution gated by a certified 2x2 concurrent
    /// causal frontier. Each branch pair has already been normalized through
    /// both concurrent orders to one exact composite Rewrite family before the
    /// cross-branch residual certificate is accepted.
    pub fn commit_derived_multi_parent_frontier_square_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        frontier: &RevisionEffectResidualSquareLayerCertificate<RelationValue, I>,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_multi_parent_resolution_endpoint(
            transaction_id,
            request,
            frontier.common_endpoint(),
            causal_parents,
        )
    }

    /// Durable multi-parent resolution gated by a single causal frontier whose
    /// branch antichains have each been normalized order-independently to one
    /// exact Rewrite family. The normalization certificate supports branch
    /// widths 1..=3 and retains the concurrent-order proof in kernel-change;
    /// this boundary consumes only its certified endpoint and independently
    /// rechecks the concrete candidate plus Γ-VMF closure before PREPARE.
    pub fn commit_derived_multi_parent_normalized_frontier_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        frontier: &RevisionEffectResidualNormalizedLayerCertificate<RelationValue, I>,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_multi_parent_resolution_endpoint(
            transaction_id,
            request,
            frontier.common_endpoint(),
            causal_parents,
        )
    }

    /// Durable multi-parent resolution gated by a causal chain whose first
    /// layer is an order-independently normalized 2x2 frontier and whose
    /// remaining layers are exact singleton residual-chain steps.
    pub fn commit_derived_multi_parent_square_chain_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        chain: &RevisionEffectResidualSquareChainCertificate<RelationValue, I>,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_multi_parent_resolution_endpoint(
            transaction_id,
            request,
            chain.common_endpoint(),
            causal_parents,
        )
    }

    /// Durable multi-parent resolution gated by a mixed causal chain whose
    /// layers may be exact singleton pairs or order-independently normalized
    /// 2x2 concurrent squares. The complete proof remains owned by
    /// kernel-change; this boundary consumes only its certified final endpoint
    /// and rechecks the concrete candidate plus Γ-VMF closure before PREPARE.
    pub fn commit_derived_multi_parent_mixed_chain_resolution<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        chain: &RevisionEffectResidualMixedChainCertificate<RelationValue, I>,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        self.commit_derived_multi_parent_resolution_endpoint(
            transaction_id,
            request,
            chain.common_endpoint(),
            causal_parents,
        )
    }

    fn commit_derived_multi_parent_resolution_endpoint<I: Clone + PartialEq + Eq>(
        &self,
        transaction_id: ClientTransactionId,
        request: &DerivedRelationRewriteTransitionRequest<'_, I>,
        coherence_endpoint: &RelationValue,
        causal_parents: &[RevisionId],
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        if request.rewrites.len() != 1 {
            return Err(PhysicalExecutionError::ResolutionRequiresSingleRelationRewrite.into());
        }
        let mut causal_parents = causal_parents.to_vec();
        causal_parents.sort();
        if causal_parents.len() < 2
            || causal_parents.windows(2).any(|pair| pair[0] == pair[1])
            || causal_parents
                .binary_search(&request.source_revision)
                .is_err()
        {
            return Err(DurableRuntimeCommitError::PrepareDurability(
                DurabilityError::Encode(kernel_durability::CodecError::CollectionTooLarge),
            ));
        }
        let relation = request.rewrites[0].relation;
        let (durable_mutations, durable_rewrite_intents) =
            Self::canonical_durable_relation_rewrites(request.rewrites)?;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            let matches = matches!(
                &committed_intent.intent,
                DurableClientIntent::RelationResolution {
                    relation_mutations,
                    rewrite_intents,
                    causal_parents: committed_parents,
                    ..
                } if relation_mutations == &durable_mutations
                    && rewrite_intents == &durable_rewrite_intents
                    && committed_parents == &causal_parents
            );
            if matches {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: request.target_revision,
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: request.target_revision,
            });
        }

        let snapshot = self.cell.snapshot()?;
        if snapshot.revision().id() != request.source_revision {
            return Err(PhysicalExecutionError::InvalidRevisionTransition.into());
        }
        let mutations = request
            .rewrites
            .iter()
            .map(|rewrite| RevisionRelationMutation {
                relation: rewrite.relation,
                delta: rewrite.rewrite.delta(),
            object_field_writes: &[],
            authorization: kernel_durability::DurableRelationAuthorization::default(),
            })
            .collect::<Vec<_>>();
        let target =
            self.derive_relation_target(snapshot.revision(), request.target_revision, &mutations)?;
        drop(snapshot);
        let prepared =
            self.cell
                .prepare_rewrites_derived(&target, request.rewrites, &self.registry)?;
        prepared.require_coherent_resolution_endpoint(relation, coherence_endpoint)?;
        self.cell
            .commit_multi_parent_resolution_prepared_durable(
                transaction_id,
                prepared,
                causal_parents,
                &self.registry,
                &mut *durability,
            )
            .map(|receipt| {
                self.signal_relation_publication(&[relation]);
                DurableRuntimeCommitOutcome::Committed(receipt)
            })
    }

}
