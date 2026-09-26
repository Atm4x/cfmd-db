fn semantic_binding_as_i64(
    binding: &SemanticIndexBinding,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<I64IndexBinding>, PhysicalExecutionError> {
    let [part] = binding.key_parts.as_slice() else {
        return Ok(None);
    };
    let Some(resolved) = registry.resolve_primitive_equivalence(context, part.equivalence)? else {
        return Ok(None);
    };
    if !matches!(
        resolved.bind_right(&Value::I64(0)),
        Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
    ) {
        return Ok(None);
    }
    Ok(Some(I64IndexBinding {
        relation: binding.relation,
        layout: binding.layout,
        key_column: part.column,
        equivalence: part.equivalence,
    }))
}

fn i64_index_advice_scores(
    store: &PhysicalStore,
    workload: &[SemanticIndexWorkloadSample],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<BTreeMap<I64IndexBinding, u128>, PhysicalExecutionError> {
    let mut gross_savings = BTreeMap::<I64IndexBinding, u128>::new();
    for sample in workload {
        if sample.expected_executions == 0 {
            continue;
        }
        let mut observations = Vec::new();
        collect_i64_join_advice_observations(
            &sample.plan,
            store,
            context,
            registry,
            &mut observations,
        )?;
        for observation in observations {
            let Some(binding) = semantic_binding_as_i64(&observation.binding, context, registry)?
            else {
                continue;
            };
            let contribution = observation
                .savings_per_execution
                .saturating_mul(sample.expected_executions as u128);
            let accumulated = gross_savings.entry(binding).or_default();
            *accumulated = accumulated.saturating_add(contribution);
        }
    }
    Ok(gross_savings)
}

fn i64_index_advice_choices(
    store: &PhysicalStore,
    scores: BTreeMap<I64IndexBinding, u128>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    report: &mut I64IndexAdvisorReport,
) -> Result<Vec<I64IndexAdviceChoice>, PhysicalExecutionError> {
    let mut choices = Vec::new();
    for (binding, gross_savings) in scores {
        let existing = store.i64_indexes.get(&binding);
        let advisor_managed = store
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::I64Index(binding));
        let relation = store.installed(binding.relation, binding.layout)?;
        let row_count = native_row_count(&relation.data);
        let build_work = if existing.is_some() {
            0
        } else {
            row_count as u128
        };
        if gross_savings <= build_work {
            report.rejected_unprofitable.push(binding);
            continue;
        }
        let (estimated_bytes, prepared_state) = if let Some(existing) = existing {
            (i64_index_estimated_retained_bytes(existing), None)
        } else {
            let candidate = MaterializedI64IndexState::build(binding, relation, context, registry)?;
            (
                i64_index_estimated_retained_bytes(&candidate),
                Some(candidate),
            )
        };
        choices.push(I64IndexAdviceChoice {
            binding,
            work: PhysicalWorkEstimate {
                read_work_saved: gross_savings,
                maintenance_work: 0,
                build_work,
            },
            row_count,
            estimated_bytes,
            existing_manual: existing.is_some() && !advisor_managed,
            advisor_managed,
            prepared_state,
        });
    }
    Ok(choices)
}

fn i64_index_advisor_selection(
    store: &PhysicalStore,
    workload: &[SemanticIndexWorkloadSample],
    policy: PhysicalArtifactAdvisorPolicy,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<I64IndexAdvisorSelection, PhysicalExecutionError> {
    let scores = i64_index_advice_scores(store, workload, context, registry)?;
    let mut report = I64IndexAdvisorReport::default();
    let mut choices = i64_index_advice_choices(store, scores, context, registry, &mut report)?;
    let current_memory = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    let replaceable_managed_bytes = saturating_usize_sum(
        store
            .i64_indexes
            .iter()
            .filter(|(binding, _)| {
                store
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::I64Index(**binding))
            })
            .map(|(_, state)| i64_index_estimated_retained_bytes(state)),
    );
    let fixed_estimated_bytes = current_memory.saturating_sub(replaceable_managed_bytes);
    let candidates = choices
        .iter()
        .map(|choice| advisor::AdmissionCandidate {
            key: choice.binding,
            capabilities: BTreeSet::from([PhysicalCapability::PointLookup]),
            work: choice.work,
            managed_units: 0,
            footprint: ResourceFootprint::from_atom(choice.binding, choice.estimated_bytes),
            replaced_fixed_bytes: 0,
            existing_manual: choice.existing_manual,
            existing_advisor_managed: choice.advisor_managed,
            tie_break_work: choice.row_count,
        })
        .collect();
    let selection = advisor::select_candidates(
        candidates,
        fixed_estimated_bytes,
        UnifiedAdvisorPolicy {
            max_managed_units: usize::MAX,
            max_managed_estimated_bytes: policy.max_managed_estimated_bytes,
            max_total_estimated_bytes: policy.max_total_estimated_bytes,
            build_threshold: 0,
            retain_threshold: 0,
        },
    );
    let mut prepared_states = Vec::new();
    for choice in &mut choices {
        if selection.selected.contains(&choice.binding)
            && let Some(state) = choice.prepared_state.take()
        {
            prepared_states.push((choice.binding, state));
        }
    }
    report.rejected_budget.extend(selection.rejected_budget);
    report
        .rejected_unprofitable
        .extend(selection.rejected_unprofitable);
    report.fixed_estimated_bytes = selection.fixed_estimated_bytes;
    report.total_estimated_bytes_after = selection.total_estimated_bytes_after;
    Ok(I64IndexAdvisorSelection {
        selected: selection.selected,
        prepared_states,
        managed_estimated_bytes: selection.managed_estimated_bytes,
        report,
    })
}

fn collect_i64_join_advice_observations(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    observations: &mut Vec<SemanticIndexAdviceObservation>,
) -> Result<(), PhysicalExecutionError> {
    if let Some(observation) =
        semantic_index_join_advice_observation(plan, store, context, registry)?
    {
        observations.push(observation);
        return Ok(());
    }
    match plan {
        Plan::FilterEqConst { input, .. }
        | Plan::FilterEqColumns { input, .. }
        | Plan::Project { input, .. }
        | Plan::Distinct { input, .. }
        | Plan::Group { input, .. }
        | Plan::TopKWithTies { input, .. }
        | Plan::PromoteToBag(input) => {
            collect_i64_join_advice_observations(input, store, context, registry, observations)
        }
        Plan::JoinEq { left, right, .. }
        | Plan::Difference { left, right }
        | Plan::AntiJoin { left, right, .. } => {
            collect_i64_join_advice_observations(left, store, context, registry, observations)?;
            collect_i64_join_advice_observations(right, store, context, registry, observations)
        }
        Plan::Scan { .. } => Ok(()),
    }
}

