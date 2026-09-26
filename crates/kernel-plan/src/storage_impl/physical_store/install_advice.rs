impl PhysicalStore {
    pub fn install(
        &mut self,
        relation: SemanticId,
        binding: LayoutBinding,
        data: NativeRelation,
    ) -> Result<(), PhysicalExecutionError> {
        if self.revision.is_some() {
            return Err(PhysicalExecutionError::RevisionBoundMutationRequiresPreparedTransition);
        }
        let family_matches = matches!(
            (binding.family, &data),
            (LayoutFamily::RowStore, NativeRelation::RowStore(_))
                | (
                    LayoutFamily::Columnar,
                    NativeRelation::Columnar { .. }
                        | NativeRelation::I64Columnar { .. }
                        | NativeRelation::TypedColumnar { .. },
                )
        );
        if !family_matches {
            return Err(PhysicalExecutionError::LayoutFamilyMismatch);
        }
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        self.relations_mut_internal().insert(
            (relation, binding.id),
            Arc::new(InstalledRelation::new(data)),
        );
        self.i64_indexes_mut_internal()
            .retain(|index, _| index.relation != relation || index.layout.id != binding.id);
        self.semantic_indexes_mut_internal()
            .retain(|index, _| index.relation != relation || index.layout.id != binding.id);
        self.semantic_quotient_factors_mut()
            .retain(|index, _| index.relation != relation || index.layout.id != binding.id);
        self.semantic_quotient_supports_mut()
            .retain(|_, state| !state.supports_relation(relation, binding));
        self.semantic_statistics_mut_internal()
            .retain(|index, _| index.relation != relation || index.layout.id != binding.id);
        self.observable_atom_states_mut_internal()
            .retain(|index, _| index.relation != relation || index.layout.id != binding.id);
        self.row_occurrence_atoms_mut()
            .remove(&(relation, binding.id));
        self.advisor_managed_artifacts_mut()
            .retain(|artifact| !artifact.touches_relation_layout(relation, binding));
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        Ok(())
    }

    pub fn install_i64_index(
        &mut self,
        index: I64IndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), PhysicalExecutionError> {
        let relation = self.installed(index.relation, index.layout)?;
        let state = MaterializedI64IndexState::build(index, relation, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        self.advisor_managed_artifacts_mut()
            .remove(&UnifiedArtifactId::I64Index(index));
        self.i64_indexes_mut_internal().insert(index, Arc::new(state));
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        Ok(())
    }

    pub fn advise_i64_indexes(
        &mut self,
        workload: &[SemanticIndexWorkloadSample],
        policy: PhysicalArtifactAdvisorPolicy,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<I64IndexAdvisorReport, PhysicalExecutionError> {
        let I64IndexAdvisorSelection {
            selected,
            prepared_states,
            managed_estimated_bytes,
            mut report,
        } = i64_index_advisor_selection(self, workload, policy, context, registry)?;
        let prepared_bindings = prepared_states
            .iter()
            .map(|(binding, _)| *binding)
            .collect::<BTreeSet<_>>();
        let evicted = self
            .advisor_managed_artifacts
            .iter()
            .filter_map(|artifact| match artifact {
                UnifiedArtifactId::I64Index(binding) if !selected.contains(binding) => {
                    Some(*binding)
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
        for binding in evicted {
            self.i64_indexes_mut_internal().remove(&binding);
            self.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::I64Index(binding));
            report.evicted.push(binding);
        }
        for (binding, state) in prepared_states {
            self.i64_indexes_mut_internal().insert(binding, Arc::new(state));
            self.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::I64Index(binding));
            report.created.push(binding);
        }
        for binding in selected {
            if prepared_bindings.contains(&binding) {
                continue;
            }
            if self
                .advisor_managed_artifacts
                .contains(&UnifiedArtifactId::I64Index(binding))
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

    pub fn install_semantic_statistics(
        &mut self,
        binding: SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<SemanticKeyStatistics, PhysicalExecutionError> {
        let relation = self.installed(binding.relation, binding.layout)?;
        let state = MaterializedSemanticStatisticsState::build(
            binding.clone(),
            relation,
            context,
            registry,
        )?;
        let snapshot = state.snapshot();
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?;
        self.advisor_managed_artifacts_mut()
            .remove(&UnifiedArtifactId::SemanticStatistics(binding.clone()));
        self.semantic_statistics_mut_internal()
            .insert(binding, Arc::new(state));
        self.transition_epoch = next_epoch;
        self.state_identity = Arc::new(());
        Ok(snapshot)
    }

    // HOSTILE[P162][COMPAT][RETIRING:P160.C]: the old direct-Join statistics advisor no longer
    // creates artifacts. Manual statistics remain valid for multiway cardinality; this surface
    // only retires previously advisor-owned statistics until a real counterfactual consumer exists.
    pub fn advise_semantic_statistics(
        &mut self,
        _workload: &[SemanticIndexWorkloadSample],
        _policy: PhysicalArtifactAdvisorPolicy,
        _context: &kernel_schema::SemanticContext,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<SemanticStatisticsAdvisorReport, PhysicalExecutionError> {
        let SemanticStatisticsAdvisorSelection {
            selected,
            prepared_states,
            managed_estimated_bytes,
            mut report,
        } = semantic_statistics_advisor_selection(self);
        let prepared_bindings = prepared_states
            .iter()
            .map(|(binding, _)| binding.clone())
            .collect::<BTreeSet<_>>();
        let evicted = self
            .advisor_managed_artifacts
            .iter()
            .filter_map(|artifact| match artifact {
                UnifiedArtifactId::SemanticStatistics(binding) if !selected.contains(binding) => {
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
            self.semantic_statistics_mut_internal().remove(binding);
            self.advisor_managed_artifacts_mut()
                .remove(&UnifiedArtifactId::SemanticStatistics(binding.clone()));
            report.evicted.push(binding.clone());
        }
        for (binding, state) in prepared_states {
            let existed = self.semantic_statistics.contains_key(&binding);
            self.semantic_statistics_mut_internal()
                .insert(binding.clone(), Arc::new(state));
            self.advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::SemanticStatistics(binding.clone()));
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
                .contains(&UnifiedArtifactId::SemanticStatistics(binding.clone()))
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

    pub fn semantic_statistics(
        &self,
        binding: &SemanticIndexBinding,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<SemanticKeyStatistics>, PhysicalExecutionError> {
        if let Some(capability) = self.semantic_fiber_capability(binding, context, registry)? {
            return Ok(Some(SemanticKeyStatistics {
                row_count: capability.row_count(),
                distinct_key_count: capability.distinct_key_count(),
            }));
        }
        let Some(state) = self.semantic_statistics.get(binding) else {
            return Ok(None);
        };
        if !state.compatible_with(context, registry)? {
            return Ok(None);
        }
        let relation = self.installed(binding.relation, binding.layout)?;
        if state.row_count != native_row_count(&relation.data) {
            return Ok(None);
        }
        Ok(Some(state.snapshot()))
    }

}
