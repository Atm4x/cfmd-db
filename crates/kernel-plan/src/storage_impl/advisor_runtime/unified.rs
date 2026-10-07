#[derive(Clone, Copy)]
pub(super) struct UnifiedObservableAdvisorInputs<'a> {
    pub(super) telemetry: &'a UnifiedAdvisorTelemetry,
    pub(super) policy: UnifiedAdvisorPolicy,
    pub(super) pressure_policy: PhysicalPressurePolicy,
    pub(super) pressure_sample: PhysicalPressureSample,
}

struct UnifiedObservableAdvisorSelection {
    selection: advisor::AdmissionSelection<UnifiedArtifactId>,
    prepared: BTreeSet<SemanticIndexBinding>,
}

fn unified_observable_advisor_selection(
    store: &PhysicalStore,
    workload: &[SemanticIndexWorkloadSample],
    inputs: UnifiedObservableAdvisorInputs<'_>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<UnifiedObservableAdvisorSelection, PhysicalExecutionError> {
    let scores = observable_atom_advice_scores(store, workload, context, registry)?;
    let mut demanded = scores.keys().cloned().collect::<BTreeSet<_>>();
    demanded.extend(
        store
            .advisor_managed_artifacts
            .iter()
            .filter_map(|artifact| match artifact {
                UnifiedArtifactId::SemanticFiber { binding, profile: SemanticFiberProfile::Observable } => Some(binding.clone()),
                _ => None,
            }),
    );

    let current_memory = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    let replaceable_observable_bytes = saturating_usize_sum(
        store
            .observable_atom_states
            .iter()
            .filter(|(binding, _)| {
                store
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::semantic_observable((*binding).clone()))
            })
            .map(|(_, state)| observable_atom_estimated_retained_bytes(state)),
    );
    let fixed_estimated_bytes = current_memory.saturating_sub(replaceable_observable_bytes);
    let mut prepared = BTreeSet::<SemanticIndexBinding>::new();
    let mut candidates = Vec::new();

    for binding in demanded {
        let id = UnifiedArtifactId::semantic_observable(binding.clone());
        let advisor_managed = store.advisor_managed_artifacts.contains(&id);
        let existing = store.observable_atom_states.get(&binding);
        if existing.is_some() && !advisor_managed {
            continue;
        }
        let compatible_existing = match existing {
            Some(state) => state.compatible_with(context, registry)?,
            None => false,
        };
        let advice = scores.get(&binding);
        let baseline_read = advice.map_or(0, |entry| entry.gross_savings);
        let baseline_build = if compatible_existing {
            0
        } else {
            advice.map_or(0, |entry| entry.key_cells as u128)
        };
        let estimated_bytes = if compatible_existing {
            existing.map_or(0, |state| observable_atom_estimated_retained_bytes(state))
        } else {
            let mut candidate_store = store.clone();
            let state = candidate_store.build_catalog_free_observable_atom_state(
                binding.clone(),
                context,
                registry,
            )?;
            let bytes = observable_atom_estimated_retained_bytes(&state);
            prepared.insert(binding.clone());
            bytes
        };
        let work = inputs.telemetry.get(&id).apply_to(
            PhysicalWorkEstimate {
                read_work_saved: baseline_read,
                maintenance_work: 0,
                build_work: baseline_build,
            },
            compatible_existing,
        );
        candidates.push(advisor::AdmissionCandidate {
            key: id.clone(),
            capabilities: id.capabilities(),
            work,
            managed_units: 1,
            footprint: ResourceFootprint::from_atom(id, estimated_bytes),
            replaced_fixed_bytes: 0,
            existing_manual: false,
            existing_advisor_managed: advisor_managed,
            tie_break_work: advice.map_or(0, |entry| entry.key_cells),
        });
    }

    let selection = advisor::select_candidates_with_pressure(
        candidates,
        fixed_estimated_bytes,
        inputs.policy,
        inputs.pressure_policy,
        inputs.pressure_sample,
    );
    Ok(UnifiedObservableAdvisorSelection {
        selection,
        prepared,
    })
}

fn observable_binding_ids(ids: Vec<UnifiedArtifactId>) -> Vec<SemanticIndexBinding> {
    ids.into_iter()
        .filter_map(|id| match id {
            UnifiedArtifactId::SemanticFiber { binding, profile: SemanticFiberProfile::Observable } => Some(binding),
            _ => None,
        })
        .collect()
}

fn retire_advisor_legacy_for_observable(
    store: &mut PhysicalStore,
    binding: &SemanticIndexBinding,
    report: &mut UnifiedObservableAdvisorReport,
) {
    let statistics = UnifiedArtifactId::semantic_cardinality(binding.clone());
    if store.advisor_managed_artifacts.contains(&statistics) {
        store.semantic_statistics_mut_internal().remove(binding);
        store.advisor_managed_artifacts_mut().remove(&statistics);
        report.retired_cardinality_profiles.push(binding.clone());
    }
    let quotient = UnifiedArtifactId::semantic_quotient(binding.clone());
    if store.advisor_managed_artifacts.contains(&quotient) {
        store.semantic_quotient_factors_mut().remove(binding);
        store.advisor_managed_artifacts_mut().remove(&quotient);
        report.retired_quotient_profiles.push(binding.clone());
    }
}

fn apply_unified_observable_selection(
    store: &mut PhysicalStore,
    mut selected: UnifiedObservableAdvisorSelection,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<UnifiedObservableAdvisorReport, PhysicalExecutionError> {
    let selected_bindings = selected
        .selection
        .selected
        .iter()
        .filter_map(|id| match id {
            UnifiedArtifactId::SemanticFiber { binding, profile: SemanticFiberProfile::Observable } => Some(binding.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let evicted = store
        .advisor_managed_artifacts
        .iter()
        .filter_map(|artifact| match artifact {
            UnifiedArtifactId::SemanticFiber { binding, profile: SemanticFiberProfile::Observable } if !selected_bindings.contains(binding) => {
                Some(binding.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let selected_rebuild = selected_bindings
        .iter()
        .any(|binding| selected.prepared.contains(binding));
    let retires_legacy = selected_bindings.iter().any(|binding| {
        store
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::semantic_cardinality(binding.clone()))
            || store
                .advisor_managed_artifacts
                .contains(&UnifiedArtifactId::semantic_quotient(binding.clone()))
    });
    let changed = !evicted.is_empty() || selected_rebuild || retires_legacy;
    let next_epoch = if changed {
        Some(
            store
                .transition_epoch
                .checked_add(1)
                .ok_or(PhysicalExecutionError::TransitionEpochExhausted)?,
        )
    } else {
        None
    };
    let mut report = UnifiedObservableAdvisorReport {
        rejected_unprofitable: observable_binding_ids(selected.selection.rejected_unprofitable),
        rejected_budget: observable_binding_ids(selected.selection.rejected_budget),
        rejected_pressure: observable_binding_ids(selected.selection.rejected_pressure),
        rejected_resource_conflict: observable_binding_ids(
            selected.selection.rejected_resource_conflict,
        ),
        managed_estimated_bytes: selected.selection.managed_estimated_bytes,
        fixed_estimated_bytes: selected.selection.fixed_estimated_bytes,
        total_estimated_bytes_after: selected.selection.total_estimated_bytes_after,
        ..UnifiedObservableAdvisorReport::default()
    };

    for binding in evicted {
        store.observable_atom_states_mut_internal().remove(&binding);
        store
            .advisor_managed_artifacts_mut()
            .remove(&UnifiedArtifactId::semantic_observable(binding.clone()));
        report.evicted.push(binding);
    }
    for binding in selected_bindings {
        if selected.prepared.remove(&binding) {
            let existed = store.observable_atom_states.contains_key(&binding);
            let state = store.build_catalog_free_observable_atom_state(
                binding.clone(),
                context,
                registry,
            )?;
            store
                .observable_atom_states_mut_internal()
                .insert(binding.clone(), Arc::new(state));
            store
                .advisor_managed_artifacts_mut()
                .insert(UnifiedArtifactId::semantic_observable(binding.clone()));
            if existed {
                report.rebuilt.push(binding.clone());
            } else {
                report.created.push(binding.clone());
            }
        } else {
            report.retained.push(binding.clone());
        }
        retire_advisor_legacy_for_observable(store, &binding, &mut report);
    }
    if let Some(next_epoch) = next_epoch {
        store.transition_epoch = next_epoch;
        store.state_identity = Arc::new(());
    }
    report.total_estimated_bytes_after = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    Ok(report)
}

