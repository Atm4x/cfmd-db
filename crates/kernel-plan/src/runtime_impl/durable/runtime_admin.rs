impl DurableRuntime {
    pub fn transaction_outcome(
        &self,
        transaction_id: ClientTransactionId,
    ) -> Result<DurableTransactionOutcome, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.transaction_outcome(transaction_id))
    }

    pub fn transaction_outcome_at(
        &self,
        epoch: IdempotencyEpoch,
        transaction_id: ClientTransactionId,
    ) -> Result<DurableTransactionOutcome, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.transaction_outcome_at(epoch, transaction_id))
    }

    pub fn retry_horizon(&self) -> Result<(IdempotencyEpoch, IdempotencyEpoch), DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok((
            durability.current_idempotency_epoch(),
            durability.minimum_retry_epoch(),
        ))
    }

    pub fn advance_idempotency_epoch(&self, next: IdempotencyEpoch) -> Result<(), DurabilityError> {
        let mut durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.advance_idempotency_epoch(next)
    }

    /// Advances the durable retry watermark in memory and drops retry-ledger
    /// payloads below it. Call `checkpoint` to publish the watermark and GC
    /// boundary durably. Γ-REIC causal records remain self-contained.
    pub fn expire_retry_history_before(
        &self,
        minimum: IdempotencyEpoch,
    ) -> Result<usize, DurabilityError> {
        let mut durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.expire_retry_history_before(minimum)
    }

    pub fn revision_effect_ideal(
        &self,
        revision: RevisionId,
    ) -> Result<Option<kernel_change::RevisionEffectIdeal<DurableTransactionIntent>>, DurabilityError>
    {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        durability.revision_effect_ideal(revision)
    }

    pub fn causal_coverage_root(&self) -> Result<RevisionId, DurabilityError> {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.causal_coverage_root())
    }

    pub fn local_historical_complement_chain(
        &self,
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
    ) -> Result<kernel_durability::LocalHistoricalComplementChain, DurableRuntimeHistoricalError>
    {
        let durability = self
            .durability
            .lock()
            .map_err(|_| DurabilityError::Poisoned)?;
        Ok(durability.local_historical_complement_chain(source, target)?)
    }

    pub fn restore_historical_value(
        &self,
        source: kernel_types::SchemaRevisionId,
        target: kernel_types::SchemaRevisionId,
        target_value: &kernel_model::Value,
        registry: &kernel_durability::HistoricalLensRegistry,
    ) -> Result<kernel_model::Value, DurableRuntimeHistoricalError> {
        let chain = self.local_historical_complement_chain(source, target)?;
        Ok(chain.restore_value(target_value, registry)?)
    }

    pub fn checkpoint(&self) -> Result<DurableGenerationReceipt, DurableRuntimeCheckpointError> {
        let mut durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        self.cell.checkpoint_durable(&mut durability)
    }

    pub fn compact_obsolete_generations(&self) -> Result<(), DurableRuntimeCheckpointError> {
        let durability = self.durability.lock().map_err(|_| {
            let _ = self.cell.force_recovery_required();
            DurableRuntimeCheckpointError::DurabilityUncertain(DurabilityError::Poisoned)
        })?;
        durability
            .compact_obsolete_generations()
            .map_err(DurableRuntimeCheckpointError::DurabilityUncertain)
    }

    pub fn install_i64_index(
        &self,
        index: I64IndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        self.cell.install_i64_index(index, registry)
    }

    pub fn install_semantic_index(
        &self,
        index: SemanticIndexBinding,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        self.cell.install_semantic_index(index, registry)
    }

    pub fn install_observable_atom_state(
        &self,
        binding: SemanticIndexBinding,
    ) -> Result<(), PhysicalExecutionError> {
        self.cell
            .install_observable_atom_state(binding, &self.registry)
    }

    pub fn advise_semantic_indexes(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: SemanticIndexAdvisorPolicy,
    ) -> Result<SemanticIndexAdvisorReport, PhysicalExecutionError> {
        self.cell
            .advise_semantic_indexes(workload, policy, &self.registry)
    }

    pub fn advise_i64_indexes(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
    ) -> Result<I64IndexAdvisorReport, PhysicalExecutionError> {
        self.cell
            .advise_i64_indexes(workload, policy, &self.registry)
    }

    pub fn advise_semantic_statistics(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
    ) -> Result<SemanticStatisticsAdvisorReport, PhysicalExecutionError> {
        self.cell
            .advise_semantic_statistics(workload, policy, &self.registry)
    }

    pub(crate) fn advise_unified_observable_atoms(
        &self,
        workload: &[SemanticIndexWorkloadSample],
        telemetry: &UnifiedAdvisorTelemetry,
        policy: UnifiedAdvisorPolicy,
        pressure_policy: PhysicalPressurePolicy,
        pressure_sample: PhysicalPressureSample,
    ) -> Result<UnifiedObservableAdvisorReport, PhysicalExecutionError> {
        self.cell.advise_unified_observable_atoms(
            workload,
            telemetry,
            policy,
            pressure_policy,
            pressure_sample,
            &self.registry,
        )
    }
}
