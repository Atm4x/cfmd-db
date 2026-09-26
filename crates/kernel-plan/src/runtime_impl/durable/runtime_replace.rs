impl DurableRuntime {
    pub fn replace_revision(
        &self,
        transaction_id: ClientTransactionId,
        request: &FullRevisionTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let requested_intent =
            DurableTransactionIntent::revision(request.target_revision, &self.registry)
                .map_err(DurabilityError::Encode)
                .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            if committed_intent == &requested_intent {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: requested_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: requested_intent.target_revision(),
            });
        }
        let authorized_request = FullRevisionTransitionRequest {
            target_revision: request.target_revision,
            registry: &self.registry,
        };
        self.cell
            .commit_full_revision_durable(transaction_id, &authorized_request, &mut *durability)
            .map(DurableRuntimeCommitOutcome::Committed)
    }

    /// Atomically publishes one validated full revision and the complement
    /// authority required to invert its schema migration.  The complement is
    /// carried by the same durable PREPARE/COMMIT identity as the target.
    pub fn migrate_schema(
        &self,
        transaction_id: ClientTransactionId,
        request: &FullRevisionTransitionRequest<'_>,
        migration_complement: &DurableMigrationComplement,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        let source_revision = self.cell.snapshot()?.revision().id();
        let requested_intent = DurableTransactionIntent::schema_migration(
            source_revision,
            request.target_revision,
            migration_complement.clone(),
            &self.registry,
        )
        .map_err(DurabilityError::Encode)
        .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            if committed_intent == &requested_intent {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: requested_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: requested_intent.target_revision(),
            });
        }
        let authorized_request = FullRevisionTransitionRequest {
            target_revision: request.target_revision,
            registry: &self.registry,
        };
        self.cell
            .commit_schema_migration_durable(
                transaction_id,
                &authorized_request,
                migration_complement,
                &mut *durability,
            )
            .map(DurableRuntimeCommitOutcome::Committed)
    }

    pub fn replace_revision_and_materializations(
        &self,
        transaction_id: ClientTransactionId,
        request: &RevisionAndMaterializationsTransitionRequest<'_>,
    ) -> Result<DurableRuntimeCommitOutcome, DurableRuntimeCommitError> {
        let mut desired = request
            .materializations
            .iter()
            .map(|spec| DurableMaterializationSpec {
                id: spec.id,
                query: spec.query.clone(),
            })
            .collect::<Vec<_>>();
        desired.sort_by_key(|spec| spec.id);
        if let Some(duplicate) = desired.windows(2).find(|pair| pair[0].id == pair[1].id) {
            return Err(PhysicalExecutionError::DuplicateMaterialization(duplicate[0].id).into());
        }
        let requested_intent = DurableTransactionIntent::revision_and_materializations(
            request.target_revision,
            &desired,
            &self.registry,
        )
        .map_err(DurabilityError::Encode)
        .map_err(DurableRuntimeCommitError::PrepareDurability)?;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCommitError::PrepareDurability(DurabilityError::Poisoned)
        })?;
        if let Some(committed_intent) = durability.transaction_intent(transaction_id) {
            if committed_intent == &requested_intent {
                return Ok(DurableRuntimeCommitOutcome::AlreadyCommitted {
                    target_revision: requested_intent.target_revision(),
                });
            }
            return Err(DurableRuntimeCommitError::TransactionIdConflict {
                transaction_id,
                committed_target: committed_intent.target_revision(),
                requested_target: requested_intent.target_revision(),
            });
        }
        let authorized_request = RevisionAndMaterializationsTransitionRequest {
            target_revision: request.target_revision,
            materializations: request.materializations,
            registry: &self.registry,
        };
        self.cell
            .commit_revision_and_materializations_durable(
                transaction_id,
                &authorized_request,
                &mut *durability,
            )
            .map(DurableRuntimeCommitOutcome::Committed)
    }

    pub fn reconfigure_materializations(
        &self,
        specs: &[RuntimeMaterializationSpec],
    ) -> Result<DurableMaterializationConfigOutcome, DurableRuntimeCheckpointError> {
        let registry = &self.registry;
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        let live = self.cell.snapshot()?;
        let mut desired = specs
            .iter()
            .map(|spec| DurableMaterializationSpec {
                id: spec.id,
                query: spec.query.clone(),
            })
            .collect::<Vec<_>>();
        desired.sort_by_key(|spec| spec.id);
        if let Some(duplicate) = desired.windows(2).find(|pair| pair[0].id == pair[1].id) {
            return Err(PhysicalExecutionError::DuplicateMaterialization(duplicate[0].id).into());
        }
        if live.durable_materialization_specs() == desired {
            return Ok(DurableMaterializationConfigOutcome::AlreadyApplied);
        }
        drop(live);

        let prepared = self
            .cell
            .prepare_materialization_configuration(specs, registry)?;
        let sealed = prepared.seal(&self.cell)?;
        let durable_specs = sealed.durable_materialization_specs();
        let physical_artifact_specs = sealed.candidate.durable_physical_artifact_specs();
        let artifact_cores = sealed.candidate.durable_artifact_cores()?;
        let receipt = match durability
            .rotate_checkpoint_with_materializations_physical_artifacts_and_cores(
                sealed.revision(),
                &durable_specs,
                &physical_artifact_specs,
                &artifact_cores,
            ) {
            Ok(receipt) => receipt,
            Err(error) => {
                if durability.requires_recovery() {
                    sealed.require_recovery();
                    return Err(DurableRuntimeCheckpointError::DurabilityUncertain(error));
                }
                return Err(DurableRuntimeCheckpointError::Durability(error));
            }
        };
        sealed.publish();
        Ok(DurableMaterializationConfigOutcome::Applied(receipt))
    }

}
