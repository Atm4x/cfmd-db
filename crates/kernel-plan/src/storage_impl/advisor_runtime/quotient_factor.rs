fn semantic_quotient_factor_advice_scores(
    store: &PhysicalStore,
    workload: &[SemanticQuotientFactorWorkloadSample<'_>],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<BTreeMap<SemanticIndexBinding, (u128, usize)>, PhysicalExecutionError> {
    let mut scores = BTreeMap::<SemanticIndexBinding, (u128, usize)>::new();
    for sample in workload {
        if sample.expected_executions == 0 {
            continue;
        }
        if sample.plan.semantic_context() != context {
            return Err(RelQueryError::SemanticRevisionMismatch.into());
        }
        for binding in sample
            .plan
            .semantic_quotient_advisor_factor_bindings(store, registry)?
        {
            let relation = store.installed(binding.relation, binding.layout)?;
            let row_count = native_row_count(&relation.data);
            let entry = scores.entry(binding).or_insert((0, row_count));
            entry.0 = entry.0.saturating_add(
                (row_count as u128).saturating_mul(sample.expected_executions as u128),
            );
            entry.1 = entry.1.max(row_count);
        }
    }
    Ok(scores)
}

fn semantic_quotient_factor_advice_choices(
    store: &PhysicalStore,
    scores: BTreeMap<SemanticIndexBinding, (u128, usize)>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    report: &mut SemanticQuotientFactorAdvisorReport,
) -> Result<Vec<SemanticQuotientFactorAdviceScore>, PhysicalExecutionError> {
    let mut choices = Vec::new();
    for (binding, (gross_savings, row_count)) in scores {
        let existing = store.semantic_quotient_factors.get(&binding);
        let compatible_existing = match existing {
            Some(state) => state.compatible_with(context, registry)?,
            None => false,
        };
        let build_work = if compatible_existing {
            0
        } else {
            row_count as u128
        };
        if gross_savings <= build_work {
            report.rejected_unprofitable.push(binding);
            continue;
        }
        let (estimated_bytes, prepared_state) = if compatible_existing {
            (
                existing.map_or(0, |state| quotient_factor_estimated_retained_bytes(state)),
                None,
            )
        } else {
            let relation = store.installed(binding.relation, binding.layout)?;
            let candidate = MaterializedSemanticQuotientFactorState::build(
                binding.clone(),
                relation,
                context,
                registry,
            )?;
            (
                quotient_factor_estimated_retained_bytes(&candidate),
                Some(candidate),
            )
        };
        let advisor_managed = store
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::SemanticQuotientFactor(binding.clone()));
        let replaced_fixed_bytes = if !compatible_existing && !advisor_managed {
            existing.map_or(0, |state| quotient_factor_estimated_retained_bytes(state))
        } else {
            0
        };
        choices.push(SemanticQuotientFactorAdviceScore {
            binding,
            work: PhysicalWorkEstimate {
                read_work_saved: gross_savings,
                maintenance_work: 0,
                build_work,
            },
            row_count,
            estimated_bytes,
            compatible_existing,
            advisor_managed,
            prepared_state,
            replaced_fixed_bytes,
        });
    }
    Ok(choices)
}

const fn semantic_quotient_factor_managed_cost(
    choice: &SemanticQuotientFactorAdviceScore,
) -> usize {
    if choice.compatible_existing && !choice.advisor_managed {
        0
    } else {
        choice.estimated_bytes
    }
}

fn semantic_quotient_factor_advisor_selection(
    store: &PhysicalStore,
    workload: &[SemanticQuotientFactorWorkloadSample<'_>],
    policy: PhysicalArtifactAdvisorPolicy,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<SemanticQuotientFactorAdvisorSelection, PhysicalExecutionError> {
    let scores = semantic_quotient_factor_advice_scores(store, workload, context, registry)?;
    let mut report = SemanticQuotientFactorAdvisorReport::default();
    let mut choices =
        semantic_quotient_factor_advice_choices(store, scores, context, registry, &mut report)?;
    let current_memory = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    let replaceable_managed_bytes = saturating_usize_sum(
        store
            .semantic_quotient_factors
            .iter()
            .filter(|(binding, _)| {
                store.advisor_managed_artifacts.contains(
                    &UnifiedArtifactId::SemanticQuotientFactor((*binding).clone()),
                )
            })
            .map(|(_, state)| quotient_factor_estimated_retained_bytes(state)),
    );
    let fixed_estimated_bytes = current_memory.saturating_sub(replaceable_managed_bytes);
    let candidates = choices
        .iter()
        .map(|choice| advisor::AdmissionCandidate {
            key: choice.binding.clone(),
            capabilities: BTreeSet::from([PhysicalCapability::QuotientFiber]),
            work: choice.work,
            managed_units: 0,
            footprint: ResourceFootprint::from_atom(
                choice.binding.clone(),
                semantic_quotient_factor_managed_cost(choice),
            ),
            replaced_fixed_bytes: choice.replaced_fixed_bytes,
            existing_manual: choice.compatible_existing && !choice.advisor_managed,
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
            prepared_states.push((choice.binding.clone(), state));
        }
    }
    report.rejected_budget.extend(selection.rejected_budget);
    report
        .rejected_unprofitable
        .extend(selection.rejected_unprofitable);
    report.fixed_estimated_bytes = selection.fixed_estimated_bytes;
    report.total_estimated_bytes_after = selection.total_estimated_bytes_after;
    Ok(SemanticQuotientFactorAdvisorSelection {
        selected: selection.selected,
        prepared_states,
        managed_estimated_bytes: selection.managed_estimated_bytes,
        report,
    })
}

