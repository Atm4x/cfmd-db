impl PhysicalStore {
    pub fn install_semantic_index(
        &mut self,
        index: SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let relation = self.installed(index.relation, index.layout)?;
        let state =
            MaterializedSemanticIndexState::build(index.clone(), relation, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        self.advisor_managed_artifacts_mut()
            .remove(&UnifiedArtifactId::SemanticIndex(index.clone()));
        self.semantic_indexes_mut_internal().insert(index, Arc::new(state));
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        Ok(())
    }

    pub fn install_observable_atom_state(
        &mut self,
        binding: SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let relation = self.installed(binding.relation, binding.layout)?;
        let state =
            MaterializedObservableAtomState::build(binding.clone(), relation, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        self.advisor_managed_artifacts_mut()
            .remove(&UnifiedArtifactId::ObservableAtom(binding.clone()));
        self.observable_atom_states_mut_internal()
            .insert(binding, Arc::new(state));
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        Ok(())
    }

    /// Publishes one advisor-selected SAMF observable-atom capability and retires only
    /// advisor-owned legacy duplicates for the same semantic binding. Manual pins are
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
                .contains(&UnifiedArtifactId::ObservableAtom(binding.clone()));
        let compatible_existing = match self.observable_atom_states.get(&binding) {
            Some(state) => state.compatible_with(context, registry)?,
            None => false,
        };
        let prepared = if compatible_existing {
            None
        } else {
            let relation = self.installed(binding.relation, binding.layout)?;
            Some(MaterializedObservableAtomState::build(
                binding.clone(),
                relation,
                context,
                registry,
            )?)
        };

        let retire_legacy_index = self
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::SemanticIndex(binding.clone()));
        let retire_legacy_statistics = self
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::SemanticStatistics(binding.clone()));
        let retire_legacy_quotient = self
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone()));
        let changed = prepared.is_some()
            || retire_legacy_index
            || retire_legacy_statistics
            || retire_legacy_quotient;
        let next_epoch = if changed {
            Some(
                self.transition_epoch
                    .checked_add(1)
                    .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?,
            )
        } else {
            None
        };

        let mut report = ObservableAtomConvergenceReport {
            retained_manual_observable: existing_manual && compatible_existing,
            capabilities: UnifiedArtifactId::ObservableAtom(binding.clone()).capabilities(),
            ..ObservableAtomConvergenceReport::default()
        };
        if let Some(state) = prepared {
            let existed = self.observable_atom_states.contains_key(&binding);
            self.observable_atom_states_mut_internal()
                .insert(binding.clone(), Arc::new(state));
            if existing_manual {
                self.advisor_managed_artifacts_mut()
                    .remove(&UnifiedArtifactId::ObservableAtom(binding.clone()));
            } else {
                self.advisor_managed_artifacts_mut()
                    .insert(UnifiedArtifactId::ObservableAtom(binding.clone()));
            }
            if existed {
                report.rebuilt = true;
            } else {
                report.created = true;
            }
        } else if !existing_manual {
            self.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::ObservableAtom(binding.clone()));
        }

        if retire_legacy_index {
            self.semantic_indexes_mut_internal().remove(&binding);
            self.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::SemanticIndex(binding.clone()));
            report.retired_legacy_indexes.push(binding.clone());
        }
        if retire_legacy_statistics {
            self.semantic_statistics_mut_internal().remove(&binding);
            self.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::SemanticStatistics(binding.clone()));
            report.retired_legacy_statistics.push(binding.clone());
        }
        if retire_legacy_quotient {
            self.semantic_quotient_factors_mut().remove(&binding);
            self.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone()));
            report.retired_legacy_quotient_factors.push(binding);
        }

        if let Some(next_epoch) = next_epoch {
            self.transition_epoch = next_epoch;
            self.state_identity = Arc::new(());
        }
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
        apply_unified_observable_selection(self, selected)
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

    pub fn advise_semantic_indexes(
        &mut self,
        workload: &[SemanticIndexWorkloadSample],
        policy: SemanticIndexAdvisorPolicy,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<SemanticIndexAdvisorReport, PhysicalExecutionError> {
        let (selected, managed_key_cells, managed_estimated_bytes, mut report) =
            semantic_index_advisor_selection(self, workload, policy, context, registry)?;
        let prepared_states =
            self.prepare_advised_semantic_indexes(&selected, context, registry)?;
        let prepared_bindings = prepared_states
            .iter()
            .map(|(binding, _)| binding.clone())
            .collect::<BTreeSet<_>>();
        let evicted = self
            .advisor_managed_artifacts
            .iter()
            .filter_map(|artifact| match artifact {
                UnifiedArtifactId::SemanticIndex(binding) if !selected.contains(binding) => {
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
            self.semantic_indexes_mut_internal().remove(binding);
            self.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::SemanticIndex(binding.clone()));
            report.evicted.push(binding.clone());
        }
        for (binding, state) in prepared_states {
            let existed = self.semantic_indexes.contains_key(&binding);
            self.semantic_indexes_mut_internal()
                .insert(binding.clone(), Arc::new(state));
            self.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::SemanticIndex(binding.clone()));
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
                .contains(&UnifiedArtifactId::SemanticIndex(binding.clone()))
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
        report.managed_key_cells = managed_key_cells;
        report.managed_estimated_bytes = managed_estimated_bytes;
        Ok(report)
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
                UnifiedArtifactId::SemanticQuotientFactor(binding)
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
                .remove(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone()));
            report.evicted.push(binding.clone());
        }
        for (binding, state) in prepared_states {
            let existed = self.semantic_quotient_factors.contains_key(&binding);
            self.semantic_quotient_factors_mut()
                .insert(binding.clone(), Arc::new(state));
            self.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::SemanticQuotientFactor(binding.clone()));
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
                .contains(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone()))
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

    fn prepare_advised_semantic_indexes(
        &self,
        selected: &BTreeSet<SemanticIndexBinding>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Vec<(SemanticIndexBinding, MaterializedSemanticIndexState)>, PhysicalExecutionError>
    {
        let mut prepared = Vec::new();
        for binding in selected {
            let compatible = match self.semantic_indexes.get(binding) {
                Some(state) => state.compatible_with(context, registry)?,
                None => false,
            };
            if compatible {
                continue;
            }
            let relation = self.installed(binding.relation, binding.layout)?;
            prepared.push((
                binding.clone(),
                MaterializedSemanticIndexState::build(
                    binding.clone(),
                    relation,
                    context,
                    registry,
                )?,
            ));
        }
        Ok(prepared)
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
        let relation = self.installed(binding.relation, binding.layout)?;
        let state =
            MaterializedObservableAtomState::build(binding.clone(), relation, context, registry)?;
        self.row_occurrence_atoms_mut().insert(key, Arc::new(state));
        Ok(())
    }

}
