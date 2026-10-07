impl RuntimeRevisionCell {
    /// Commits one runtime revision through the durable logical WAL boundary.
    ///
    /// The exact ordering is:
    ///
    /// 1. build the detached runtime candidate;
    /// 2. durably append PREPARE;
    /// 3. acquire the exclusive freshness seal;
    /// 4. durably append COMMIT;
    /// 5. publish the already-sealed runtime root infallibly.
    ///
    /// A stale transition discovered at step 3 leaves only an uncommitted
    /// durable PREPARE, which recovery ignores. If the COMMIT durability call
    /// returns an error, the runtime root is not published and the caller must
    /// treat the durable outcome as uncertain until WAL recovery.
    #[cfg(test)]
    pub(crate) fn commit_revision_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        request: &RevisionTransitionRequest<'_>,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let prepared = self.prepare_revision(request)?;
        self.commit_prepared_durable(transaction_id, prepared, request.registry, durability)
    }

    pub(crate) fn commit_full_revision_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        request: &FullRevisionTransitionRequest<'_>,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let prepared = self.prepare_full_revision(request)?;
        self.commit_prepared_durable(transaction_id, prepared, request.registry, durability)
    }

    pub(crate) fn commit_schema_migration_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        request: &FullRevisionTransitionRequest<'_>,
        migration_program: &kernel_transport::SchemaMigrationProgram,
        migration_complement: &DurableMigrationComplement,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let prepared = self.prepare_full_revision(request)?;
        let descriptor = DurableRevisionDescriptor::schema_migration_program(
            transaction_id,
            prepared.descriptor().source_revision(),
            prepared.descriptor().target().id(),
            migration_program.clone(),
            migration_complement.clone(),
            request.registry,
        )
        .map_err(DurabilityError::Encode)
        .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let durable_prepare = durability
            .durably_prepare(&descriptor)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let prepared = prepared.bind_committed_schema_migration_history_effect(
            durable_prepare.revision_effect_id().0,
            migration_program,
        );
        let sealed = prepared.seal(self)?;
        let durable = match durability.durably_commit(durable_prepare) {
            Ok(durable) => durable,
            Err(error) => {
                sealed.require_recovery();
                return Err(DurableRuntimeCommitError::CommitDurabilityUncertain(error));
            }
        };
        let _ = sealed.publish();
        Ok(DurableRuntimeCommitReceipt {
            durable,
            publication: RuntimePublicationEffect::Rebuilt,
        })
    }

    pub(crate) fn commit_revision_and_materializations_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        request: &RevisionAndMaterializationsTransitionRequest<'_>,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let prepared = self.prepare_revision_and_materializations(request)?;
        self.commit_prepared_durable(transaction_id, prepared, request.registry, durability)
    }

    fn commit_prepared_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        prepared: PreparedRuntimeRevisionTransition,
        registry: &kernel_semantics::SemanticRegistry,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        self.commit_prepared_durable_guarded(transaction_id, prepared, None, registry, durability)
    }

    fn commit_prepared_durable_guarded<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        prepared: PreparedRuntimeRevisionTransition,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        registry: &kernel_semantics::SemanticRegistry,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let mut durable_descriptor = prepared
            .descriptor()
            .durable_descriptor(transaction_id, registry)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        durable_descriptor.intent = durable_descriptor
            .intent
            .with_client_guard_digest(client_guard_digest)
            .with_causal_observations(prepared.descriptor().causal_observations().iter().map(
                |coordinate| {
                    durable_causal_observation(
                        coordinate,
                        prepared
                            .descriptor()
                            .causal_observation_exact_value(coordinate),
                        prepared
                            .descriptor()
                            .causal_observation_preservation_rule(coordinate),
                    )
                },
            ))
            .with_causal_observation_groups(
                prepared
                    .descriptor()
                    .causal_observation_groups()
                    .iter()
                    .filter_map(durable_causal_observation_group),
            )
            .with_relational_causal_observations(
                prepared
                    .descriptor()
                    .relational_causal_observations()
                    .iter()
                    .map(durable_relational_causal_observation),
            );
        let durable_prepare = durability
            .durably_prepare(&durable_descriptor)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let prepared = prepared.bind_committed_history_effect(
            durable_prepare.revision_effect_id().0,
            registry,
        )?;
        let sealed = prepared.seal(self)?;
        let durable = match durability.durably_commit(durable_prepare) {
            Ok(durable) => durable,
            Err(error) => {
                sealed.require_recovery();
                return Err(DurableRuntimeCommitError::CommitDurabilityUncertain(error));
            }
        };
        let rebuilt = matches!(
            sealed.descriptor().change(),
            RevisionCommitChange::FullRevision
                | RevisionCommitChange::FullRevisionAndMaterializations { .. }
        );
        let output_deltas = sealed.publish();
        Ok(DurableRuntimeCommitReceipt {
            durable,
            publication: if rebuilt {
                RuntimePublicationEffect::Rebuilt
            } else {
                RuntimePublicationEffect::Incremental(output_deltas)
            },
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    fn commit_prepared_relation_residual_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        prepared: PreparedRuntimeRevisionTransition,
        client_mutations: Vec<DurableRelationMutation>,
        client_semantic_revision: kernel_types::SemanticRevision,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        registry: &kernel_semantics::SemanticRegistry,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let mut durable_descriptor = prepared
            .descriptor()
            .durable_descriptor(transaction_id, registry)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let realized_mutations = match &durable_descriptor.change {
            DurableRevisionChange::RelationData {
                relation_mutations, ..
            } => relation_mutations.clone(),
            _ => {
                return Err(DurableRuntimeCommitError::PrepareDurability(
                    DurabilityError::Protocol {
                        offset: 0,
                        reason: "relation residual publication requires relation-data change",
                    },
                ));
            }
        };
        durable_descriptor.intent = DurableTransactionIntent::relation_data_residual(
            durable_descriptor.source_revision,
            prepared.descriptor().target(),
            client_semantic_revision,
            client_mutations,
            realized_mutations,
            registry,
        )
        .map_err(DurabilityError::Encode)
        .map_err(DurableRuntimeCommitError::PrepareDurability)?
        .with_client_guard_digest(client_guard_digest)
        .with_causal_observations(prepared.descriptor().causal_observations().iter().map(
            |coordinate| {
                durable_causal_observation(
                    coordinate,
                    prepared
                        .descriptor()
                        .causal_observation_exact_value(coordinate),
                    prepared
                        .descriptor()
                        .causal_observation_preservation_rule(coordinate),
                )
            },
        ))
        .with_causal_observation_groups(
            prepared
                .descriptor()
                .causal_observation_groups()
                .iter()
                .filter_map(durable_causal_observation_group),
        )
        .with_relational_causal_observations(
            prepared
                .descriptor()
                .relational_causal_observations()
                .iter()
                .map(durable_relational_causal_observation),
        );
        let durable_prepare = durability
            .durably_prepare(&durable_descriptor)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let prepared = prepared.bind_committed_history_effect(
            durable_prepare.revision_effect_id().0,
            registry,
        )?;
        let sealed = prepared.seal(self)?;
        let durable = match durability.durably_commit(durable_prepare) {
            Ok(durable) => durable,
            Err(error) => {
                sealed.require_recovery();
                return Err(DurableRuntimeCommitError::CommitDurabilityUncertain(error));
            }
        };
        let output_deltas = sealed.publish();
        Ok(DurableRuntimeCommitReceipt {
            durable,
            publication: RuntimePublicationEffect::Incremental(output_deltas),
        })
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    pub(crate) fn commit_prepared_mixed_residual_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        prepared: PreparedRuntimeRevisionTransition,
        client_relation_mutations: Vec<DurableRelationMutation>,
        client_model_delta: DurableModelDelta,
        realized_model_complement: DurableModelDelta,
        client_semantic_revision: kernel_types::SemanticRevision,
        client_guard_digest: Option<kernel_durability::ClientIntentGuardDigest>,
        registry: &kernel_semantics::SemanticRegistry,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let mut durable_descriptor = prepared
            .descriptor()
            .durable_descriptor(transaction_id, registry)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let (_realized_semantic_revision, realized_relation_mutations, realized_model_delta) =
            match &durable_descriptor.change {
                DurableRevisionChange::MixedRevision {
                    semantic_revision,
                    relation_mutations,
                    model_delta,
                    ..
                } => (
                    *semantic_revision,
                    relation_mutations.clone(),
                    model_delta.clone(),
                ),
                _ => {
                    return Err(DurableRuntimeCommitError::PrepareDurability(
                        DurabilityError::Protocol {
                            offset: 0,
                            reason: "mixed residual publication requires mixed revision change",
                        },
                    ));
                }
            };
        durable_descriptor.intent = DurableTransactionIntent::mixed_revision_residual(
            durable_descriptor.source_revision,
            prepared.descriptor().target(),
            client_semantic_revision,
            client_relation_mutations,
            client_model_delta,
            realized_relation_mutations,
            realized_model_delta,
            realized_model_complement,
            registry,
        )
        .map_err(DurabilityError::Encode)
        .map_err(DurableRuntimeCommitError::PrepareDurability)?
        .with_client_guard_digest(client_guard_digest)
        .with_causal_observations(prepared.descriptor().causal_observations().iter().map(
            |coordinate| {
                durable_causal_observation(
                    coordinate,
                    prepared
                        .descriptor()
                        .causal_observation_exact_value(coordinate),
                    prepared
                        .descriptor()
                        .causal_observation_preservation_rule(coordinate),
                )
            },
        ))
        .with_causal_observation_groups(
            prepared
                .descriptor()
                .causal_observation_groups()
                .iter()
                .filter_map(durable_causal_observation_group),
        )
        .with_relational_causal_observations(
            prepared
                .descriptor()
                .relational_causal_observations()
                .iter()
                .map(durable_relational_causal_observation),
        );
        let durable_prepare = durability
            .durably_prepare(&durable_descriptor)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let prepared = prepared.bind_committed_history_effect(
            durable_prepare.revision_effect_id().0,
            registry,
        )?;
        let sealed = prepared.seal(self)?;
        let durable = match durability.durably_commit(durable_prepare) {
            Ok(durable) => durable,
            Err(error) => {
                sealed.require_recovery();
                return Err(DurableRuntimeCommitError::CommitDurabilityUncertain(error));
            }
        };
        let output_deltas = sealed.publish();
        Ok(DurableRuntimeCommitReceipt {
            durable,
            publication: RuntimePublicationEffect::Incremental(output_deltas),
        })
    }

    fn commit_coherent_resolution_durable<D: RevisionDurability, I>(
        &self,
        transaction_id: ClientTransactionId,
        resolution: PreparedCoherentResolutionTransition<I>,
        registry: &kernel_semantics::SemanticRegistry,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        self.commit_prepared_durable(transaction_id, resolution.prepared, registry, durability)
    }

    fn commit_multi_parent_resolution_prepared_durable<D: RevisionDurability>(
        &self,
        transaction_id: ClientTransactionId,
        prepared: PreparedRuntimeRevisionTransition,
        causal_parents: Vec<RevisionId>,
        registry: &kernel_semantics::SemanticRegistry,
        durability: &mut D,
    ) -> Result<DurableRuntimeCommitReceipt, DurableRuntimeCommitError> {
        let durable_descriptor = prepared
            .descriptor()
            .durable_resolution_descriptor(transaction_id, causal_parents, registry)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let durable_prepare = durability
            .durably_prepare(&durable_descriptor)
            .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let prepared = prepared.bind_committed_history_effect(
            durable_prepare.revision_effect_id().0,
            registry,
        )?;
        let sealed = prepared.seal(self)?;
        let durable = match durability.durably_commit(durable_prepare) {
            Ok(durable) => durable,
            Err(error) => {
                sealed.require_recovery();
                return Err(DurableRuntimeCommitError::CommitDurabilityUncertain(error));
            }
        };
        let output_deltas = sealed.publish();
        Ok(DurableRuntimeCommitReceipt {
            durable,
            publication: RuntimePublicationEffect::Incremental(output_deltas),
        })
    }

    fn checkpoint_durable(
        &self,
        durability: &mut DurableRevisionStore,
    ) -> Result<DurableGenerationReceipt, DurableRuntimeCheckpointError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        let RuntimeRevisionCellState::Serving(live) = &*state else {
            return Err(PhysicalExecutionError::RuntimeRecoveryRequired.into());
        };
        if durability.durable_head() != live.revision.id() {
            return Err(PhysicalExecutionError::DurableHeadMismatch.into());
        }
        let materialization_specs = live.durable_materialization_specs();
        let physical_artifact_specs = live.durable_physical_artifact_specs();
        let artifact_cores = live.durable_artifact_cores()?;
        match durability.rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
            live.revision(),
            &materialization_specs,
            &physical_artifact_specs,
            &artifact_cores,
        ) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                if durability.requires_recovery() {
                    *state = RuntimeRevisionCellState::RecoveryRequired;
                    Err(DurableRuntimeCheckpointError::DurabilityUncertain(error))
                } else {
                    Err(DurableRuntimeCheckpointError::Durability(error))
                }
            }
        }
    }

    fn force_recovery_required(&self) -> Result<(), PhysicalExecutionError> {
        let mut state = self
            .root
            .write()
            .map_err(|_| PhysicalExecutionError::RuntimePublicationPoisoned)?;
        *state = RuntimeRevisionCellState::RecoveryRequired;
        Ok(())
    }
}
