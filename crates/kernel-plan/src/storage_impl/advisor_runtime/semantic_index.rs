fn semantic_index_advice_scores(
    store: &PhysicalStore,
    workload: &[SemanticIndexWorkloadSample],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<BTreeMap<SemanticIndexBinding, SemanticIndexAdviceScore>, PhysicalExecutionError> {
    let mut scores = BTreeMap::<SemanticIndexBinding, SemanticIndexAdviceScore>::new();
    for sample in workload {
        if sample.expected_executions == 0 {
            continue;
        }
        for observation in
            semantic_index_advice_observations(&sample.plan, store, context, registry)?
        {
            let entry = scores
                .entry(observation.binding.clone())
                .or_insert_with(|| SemanticIndexAdviceScore {
                    binding: observation.binding.clone(),
                    gross_savings: 0,
                    key_cells: observation.key_cells,
                    estimated_bytes: observation.estimated_bytes,
                });
            entry.gross_savings = entry.gross_savings.saturating_add(
                observation
                    .savings_per_execution
                    .saturating_mul(sample.expected_executions as u128),
            );
            entry.key_cells = entry.key_cells.max(observation.key_cells);
            entry.estimated_bytes = entry.estimated_bytes.max(observation.estimated_bytes);
        }
    }
    Ok(scores)
}

fn semantic_index_advice_choices(
    store: &PhysicalStore,
    scores: BTreeMap<SemanticIndexBinding, SemanticIndexAdviceScore>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    report: &mut SemanticIndexAdvisorReport,
) -> Result<Vec<SemanticIndexAdviceChoice>, PhysicalExecutionError> {
    let mut choices = Vec::new();
    for score in scores.into_values() {
        let existing = store.semantic_indexes.get(&score.binding);
        let compatible_existing = match existing {
            Some(state) => state.compatible_with(context, registry)?,
            None => false,
        };
        let build_work = if compatible_existing {
            0
        } else {
            score.key_cells as u128
        };
        if score.gross_savings <= build_work {
            report.rejected_unprofitable.push(score.binding);
            continue;
        }
        let advisor_managed = store
            .advisor_managed_artifacts
            .contains(&UnifiedArtifactId::SemanticIndex(score.binding.clone()));
        let replaced_fixed_bytes = if !compatible_existing && !advisor_managed {
            existing.map_or(0, |state| semantic_index_estimated_retained_bytes(state))
        } else {
            0
        };
        choices.push(SemanticIndexAdviceChoice {
            binding: score.binding,
            work: PhysicalWorkEstimate {
                read_work_saved: score.gross_savings,
                maintenance_work: 0,
                build_work,
            },
            key_cells: score.key_cells,
            estimated_bytes: score.estimated_bytes,
            compatible_existing,
            advisor_managed,
            replaced_fixed_bytes,
        });
    }
    Ok(choices)
}

const fn semantic_index_managed_cost(choice: &SemanticIndexAdviceChoice) -> usize {
    if choice.compatible_existing && !choice.advisor_managed {
        0
    } else {
        choice.key_cells
    }
}

const fn semantic_index_managed_estimated_bytes(choice: &SemanticIndexAdviceChoice) -> usize {
    if choice.compatible_existing && !choice.advisor_managed {
        0
    } else {
        choice.estimated_bytes
    }
}

fn semantic_index_advisor_selection(
    store: &PhysicalStore,
    workload: &[SemanticIndexWorkloadSample],
    policy: SemanticIndexAdvisorPolicy,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<
    (
        BTreeSet<SemanticIndexBinding>,
        usize,
        usize,
        SemanticIndexAdvisorReport,
    ),
    PhysicalExecutionError,
> {
    let scores = semantic_index_advice_scores(store, workload, context, registry)?;
    let mut report = SemanticIndexAdvisorReport::default();
    let choices = semantic_index_advice_choices(store, scores, context, registry, &mut report)?;

    let current_memory = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    let replaceable_managed_bytes = saturating_usize_sum(
        store
            .semantic_indexes
            .iter()
            .filter(|(binding, _)| {
                store
                    .advisor_managed_artifacts
                    .contains(&UnifiedArtifactId::SemanticIndex((*binding).clone()))
            })
            .map(|(_, state)| semantic_index_estimated_retained_bytes(state)),
    );
    let fixed_estimated_bytes = current_memory.saturating_sub(replaceable_managed_bytes);
    let candidates = choices
        .into_iter()
        .map(|choice| {
            let managed_units = semantic_index_managed_cost(&choice);
            let managed_bytes = semantic_index_managed_estimated_bytes(&choice);
            advisor::AdmissionCandidate {
                key: choice.binding.clone(),
                capabilities: BTreeSet::from([PhysicalCapability::PointLookup]),
                work: choice.work,
                managed_units,
                footprint: ResourceFootprint::from_atom(choice.binding, managed_bytes),
                replaced_fixed_bytes: choice.replaced_fixed_bytes,
                existing_manual: choice.compatible_existing && !choice.advisor_managed,
                existing_advisor_managed: choice.advisor_managed,
                tie_break_work: 0,
            }
        })
        .collect();
    let selection = advisor::select_candidates(
        candidates,
        fixed_estimated_bytes,
        UnifiedAdvisorPolicy {
            max_managed_units: policy.max_managed_key_cells,
            max_managed_estimated_bytes: policy.max_managed_estimated_bytes,
            max_total_estimated_bytes: policy.max_total_estimated_bytes,
            build_threshold: 0,
            retain_threshold: 0,
        },
    );
    report.rejected_budget.extend(selection.rejected_budget);
    report
        .rejected_unprofitable
        .extend(selection.rejected_unprofitable);
    report.fixed_estimated_bytes = selection.fixed_estimated_bytes;
    report.total_estimated_bytes_after = selection.total_estimated_bytes_after;
    Ok((
        selection.selected,
        selection.managed_units,
        selection.managed_estimated_bytes,
        report,
    ))
}

