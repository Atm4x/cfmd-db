impl PhysicalStore {
    pub fn install_observable_atom_state(
        &mut self,
        binding: SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let mut candidate = self.clone();
        let state = candidate.build_catalog_free_observable_atom_state(
            binding.clone(),
            context,
            registry,
        )?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        candidate
            .advisor_managed_artifacts_mut()
            .remove(&UnifiedArtifactId::semantic_observable(binding.clone()));
        candidate
            .observable_atom_states_mut_internal()
            .insert(binding, Arc::new(state));
        candidate.transition_epoch = next_epoch;
        candidate.state_identity = Arc::new(());
        *self = candidate;
        Ok(())
    }

    /// Publishes one advisor-selected SAMF observable-atom capability and retires only
    /// advisor-owned alternate duplicates for the same semantic binding. Manual pins are
    /// preserved. Replacement happens only after the SAMF candidate has been built and
    /// validated against the pinned semantic context.
    pub fn converge_observable_atom_candidate(
        &mut self,
        binding: SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<ObservableAtomConvergenceReport, PhysicalExecutionError> {
        let existing_manual = self.observable_atom_states.contains_key(&binding)
            && !self
                .advisor_managed_artifacts
                .contains(&UnifiedArtifactId::semantic_observable(binding.clone()));
        let compatible_existing = match self.observable_atom_states.get(&binding) {
            Some(state) => state.compatible_with(context, registry)?,
            None => false,
        };
        let retire_alternate_statistics = self
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::semantic_cardinality(binding.clone()));
        let retire_alternate_quotient = self
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::semantic_quotient(binding.clone()));
        let changed = !compatible_existing || retire_alternate_statistics || retire_alternate_quotient;
        let mut report = ObservableAtomConvergenceReport {
            retained_manual_observable: existing_manual && compatible_existing,
            capabilities: UnifiedArtifactId::semantic_observable(binding.clone()).capabilities(),
            ..ObservableAtomConvergenceReport::default()
        };
        if !changed {
            if !existing_manual {
                self.advisor_managed_artifacts_mut()
                    .insert(UnifiedArtifactId::semantic_observable(binding));
            }
            return Ok(report);
        }

        let mut candidate = self.clone();
        if !compatible_existing {
            let state = candidate.build_catalog_free_observable_atom_state(
                binding.clone(),
                context,
                registry,
            )?;
            let existed = candidate.observable_atom_states.contains_key(&binding);
            candidate
                .observable_atom_states_mut_internal()
                .insert(binding.clone(), Arc::new(state));
            if existing_manual {
                candidate.advisor_managed_artifacts_mut()
                    .remove(&UnifiedArtifactId::semantic_observable(binding.clone()));
            } else {
                candidate.advisor_managed_artifacts_mut()
                    .insert(UnifiedArtifactId::semantic_observable(binding.clone()));
            }
            if existed { report.rebuilt = true; } else { report.created = true; }
        } else if !existing_manual {
            candidate.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::semantic_observable(binding.clone()));
        }
        if retire_alternate_statistics {
            candidate.semantic_statistics_mut_internal().remove(&binding);
            candidate.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::semantic_cardinality(binding.clone()));
            report.retired_cardinality_profiles.push(binding.clone());
        }
        if retire_alternate_quotient {
            candidate.semantic_quotient_factors_mut().remove(&binding);
            candidate.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::semantic_quotient(binding.clone()));
            report.retired_quotient_profiles.push(binding);
        }
        candidate.transition_epoch = self.transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        candidate.state_identity = Arc::new(());
        *self = candidate;
        Ok(report)
    }

    /// Runs the current unified automatic physical-maintenance surface over SAMF
    /// observable atoms. Workload telemetry, decay, pressure and hysteresis are
    /// non-authoritative inputs; every selected state remains reconstructible from
    /// the pinned revision.
    pub(crate) fn advise_unified_observable_atoms(
        &mut self,
        workload: &[SemanticIndexWorkloadSample],
        inputs: UnifiedObservableAdvisorInputs<'_>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<UnifiedObservableAdvisorReport, PhysicalExecutionError> {
        let selected =
            unified_observable_advisor_selection(self, workload, inputs, context, registry)?;
        apply_unified_observable_selection(self, selected, context, registry)
    }

    pub fn observable_atom_probe_values(
        &self,
        binding: &SemanticIndexBinding,
        values: &[&Value],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Vec<PhysicalRowId>>, PhysicalExecutionError> {
        let state = self
            .observable_atom_states
            .get(binding)
            .ok_or(PhysicalExecutionError::MissingPhysicalIndex)?;
        state.probe_values(values, context, registry)
    }

    pub fn observable_atom_probe_value(
        &self,
        binding: &SemanticIndexBinding,
        slot: usize,
        value: &Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Vec<PhysicalRowId>, PhysicalExecutionError> {
        let state = self
            .observable_atom_states
            .get(binding)
            .ok_or(PhysicalExecutionError::MissingPhysicalIndex)?;
        state.probe_slot_value(slot, value, context, registry)
    }

    pub fn observable_atom_count_value(
        &self,
        binding: &SemanticIndexBinding,
        slot: usize,
        value: &Value,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<usize, PhysicalExecutionError> {
        let state = self
            .observable_atom_states
            .get(binding)
            .ok_or(PhysicalExecutionError::MissingPhysicalIndex)?;
        state.count_slot_value(slot, value, context, registry)
    }

    #[must_use]
    pub fn observable_atom_state(
        &self,
        binding: &SemanticIndexBinding,
    ) -> Option<&MaterializedObservableAtomState> {
        self.observable_atom_states.get(binding).map(Arc::as_ref)
    }

    /// Advises reconstructible Γ-QCN endpoint factors for prepared plans whose current
    /// physical cost model already selects the quotient execution path. This intentionally
    /// does not speculate that building a factor will itself make a currently rejected QCN
    /// path preferable; counterfactual path-shaping belongs to the broader physical advisor.
    /// `expected_executions` is a read-amortization signal only: write-rate / delta-maintenance
    /// cost is not yet part of this policy and remains an input for a future workload model.
    pub fn advise_semantic_quotient_factors(
        &mut self,
        workload: &[SemanticQuotientFactorWorkloadSample<'_>],
        policy: PhysicalArtifactAdvisorPolicy,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<SemanticQuotientFactorAdvisorReport, PhysicalExecutionError> {
        let SemanticQuotientFactorAdvisorSelection {
            selected,
            prepared_states,
            managed_estimated_bytes,
            mut report,
        } = semantic_quotient_factor_advisor_selection(self, workload, policy, context, registry)?;
        let prepared_bindings = prepared_states
            .iter()
            .map(|(binding, _)| binding.clone())
            .collect::<BTreeSet<_>>();
        let evicted = self
            .advisor_managed_artifacts
            .iter()
            .filter_map(|artifact| match artifact {
                UnifiedArtifactId::SemanticFiber { binding, profile: SemanticFiberProfile::Quotient }
                    if !selected.contains(binding) =>
                {
                    Some(binding.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let changed = !evicted.is_empty() || !prepared_states.is_empty();
        let next_epoch = if changed {
            Some(
                self.transition_epoch
                    .checked_add(1)
                    .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?,
            )
        } else {
            None
        };

        for binding in &evicted {
            self.semantic_quotient_factors_mut().remove(binding);
            self.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::semantic_quotient(binding.clone()));
            report.evicted.push(binding.clone());
        }
        for (binding, state) in prepared_states {
            let existed = self.semantic_quotient_factors.contains_key(&binding);
            self.semantic_quotient_factors_mut()
                .insert(binding.clone(), Arc::new(state));
            self.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::semantic_quotient(binding.clone()));
            if existed {
                report.rebuilt.push(binding);
            } else {
                report.created.push(binding);
            }
        }
        for binding in selected {
            if prepared_bindings.contains(&binding) {
                continue;
            }
            if self
                .advisor_managed_artifacts
                .contains(&UnifiedArtifactId::semantic_quotient(binding.clone()))
            {
                report.retained.push(binding);
            } else {
                report.reused_existing.push(binding);
            }
        }
        if let Some(next_epoch) = next_epoch {
            self.transition_epoch = next_epoch;
            self.state_identity = Arc::new(());
        }
        report.managed_estimated_bytes = managed_estimated_bytes;
        Ok(report)
    }

    pub(crate) fn ensure_relation_write_occurrence_atom(
        &mut self,
        relation: SemanticId,
        layout: LayoutBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let binding = full_row_occurrence_binding(relation, layout, context)?;
        self.ensure_full_row_occurrence_atom(&binding, context, registry)
    }

    fn ensure_full_row_occurrence_atom(
        &mut self,
        binding: &SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let key = (binding.relation, binding.layout.id);
        let expected_rows =
            native_row_count(&self.installed(binding.relation, binding.layout)?.data);
        let compatible = match self.row_occurrence_atoms.get(&key) {
            Some(state) => {
                state.binding == *binding
                    && state.compatible_with(context, registry)?
                    && state.row_count() == expected_rows
            }
            None => false,
        };
        if compatible {
            return Ok(());
        }
        let mut candidate = self.clone();
        let state = candidate.build_catalog_free_observable_atom_state(
            binding.clone(),
            context,
            registry,
        )?;
        candidate.row_occurrence_atoms_mut().insert(key, Arc::new(state));
        *self = candidate;
        Ok(())
    }

}
