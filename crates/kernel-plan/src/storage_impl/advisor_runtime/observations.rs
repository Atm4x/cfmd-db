fn semantic_index_state_for_advice<'a>(
    store: &'a PhysicalStore,
    binding: &SemanticIndexBinding,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<std::borrow::Cow<'a, MaterializedSemanticIndexState>, PhysicalExecutionError> {
    if let Some(existing) = store.semantic_indexes.get(binding)
        && existing.compatible_with(context, registry)?
    {
        return Ok(std::borrow::Cow::Borrowed(existing));
    }
    let relation = store.installed(binding.relation, binding.layout)?;
    Ok(std::borrow::Cow::Owned(
        MaterializedSemanticIndexState::build(binding.clone(), relation, context, registry)?,
    ))
}

fn primitive_binding_supported(
    binding: &SemanticIndexBinding,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, PhysicalExecutionError> {
    for part in &binding.key_parts {
        if registry
            .resolve_primitive_equivalence(context, part.equivalence)?
            .is_none()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn semantic_index_filter_advice_observation(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<SemanticIndexAdviceObservation>, PhysicalExecutionError> {
    let Some((relation, layout, predicates)) = collect_direct_filter_chain(plan) else {
        return Ok(None);
    };
    if predicates.is_empty() {
        return Ok(None);
    }
    let mut key_parts = predicates
        .iter()
        .map(|(column, equivalence, _)| SemanticIndexKeyPart {
            column: *column,
            equivalence: *equivalence,
        })
        .collect::<Vec<_>>();
    key_parts.sort_unstable();
    key_parts.dedup();
    let binding = SemanticIndexBinding {
        relation,
        layout,
        key_parts,
    };
    if !primitive_binding_supported(&binding, context, registry)? {
        return Ok(None);
    }
    let installed = store.installed(relation, layout)?;
    let row_count = native_row_count(&installed.data);
    let key_cells = row_count.saturating_mul(binding.key_parts.len());
    let state = semantic_index_state_for_advice(store, &binding, context, registry)?;
    let values = binding
        .key_parts
        .iter()
        .map(|part| {
            predicates
                .iter()
                .find(|(column, equivalence, _)| {
                    *column == part.column && *equivalence == part.equivalence
                })
                .map(|(_, _, value)| *value)
                .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let matching_rows = state
        .probe_values(&values, context, registry)?
        .map_or(0, kernel_semantic_index::SemanticBucket::len);
    let estimated_bytes = semantic_index_estimated_retained_bytes(&state);
    let scan_work = row_count as u128;
    let index_work = 1_u128.saturating_add(matching_rows as u128);
    Ok(Some(SemanticIndexAdviceObservation {
        binding,
        savings_per_execution: scan_work.saturating_sub(index_work),
        key_cells,
        estimated_bytes,
    }))
}

fn semantic_index_join_advice_observation(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<SemanticIndexAdviceObservation>, PhysicalExecutionError> {
    let Some((left, right, keys)) = direct_join_advice_summary(plan, store)? else {
        return Ok(None);
    };
    let mut key_parts = keys
        .into_iter()
        .map(|(column, equivalence)| SemanticIndexKeyPart {
            column,
            equivalence,
        })
        .collect::<Vec<_>>();
    key_parts.sort_unstable();
    let binding = SemanticIndexBinding {
        relation: right.0,
        layout: right.1,
        key_parts,
    };
    if !primitive_binding_supported(&binding, context, registry)? {
        return Ok(None);
    }
    let left_installed = store.installed(left.0, left.1)?;
    let right_installed = store.installed(right.0, right.1)?;
    let left_rows = native_row_count(&left_installed.data);
    let right_rows = native_row_count(&right_installed.data);
    let key_cells = right_rows.saturating_mul(binding.key_parts.len());
    let state = semantic_index_state_for_advice(store, &binding, context, registry)?;
    let distinct = state.distinct_key_count();
    let estimated_bytes = semantic_index_estimated_retained_bytes(&state);
    if left_rows == 0 || right_rows == 0 || distinct == 0 {
        return Ok(Some(SemanticIndexAdviceObservation {
            binding,
            savings_per_execution: 0,
            key_cells,
            estimated_bytes,
        }));
    }
    let fallback_work = SemanticAccessCostModel::canonical_bucket_join_access_work(
        left_rows,
        right_rows,
        binding.key_parts.len(),
    ) as u128;
    let indexed_work =
        SemanticAccessCostModel::persisted_join_access_work(left_rows, binding.key_parts.len())
            as u128;
    Ok(Some(SemanticIndexAdviceObservation {
        binding,
        savings_per_execution: fallback_work.saturating_sub(indexed_work),
        key_cells,
        estimated_bytes,
    }))
}

fn semantic_index_advice_observations(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<SemanticIndexAdviceObservation>, PhysicalExecutionError> {
    let mut observations = Vec::new();
    collect_semantic_index_advice_observations(plan, store, context, registry, &mut observations)?;
    Ok(observations)
}

fn collect_semantic_index_advice_observations(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    observations: &mut Vec<SemanticIndexAdviceObservation>,
) -> Result<(), PhysicalExecutionError> {
    if let Some(observation) =
        semantic_index_filter_advice_observation(plan, store, context, registry)?
    {
        observations.push(observation);
        return Ok(());
    }
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
        | Plan::PromoteToBag(input) => collect_semantic_index_advice_observations(
            input,
            store,
            context,
            registry,
            observations,
        ),
        Plan::JoinEq { left, right, .. }
        | Plan::Difference { left, right }
        | Plan::AntiJoin { left, right, .. } => {
            collect_semantic_index_advice_observations(
                left,
                store,
                context,
                registry,
                observations,
            )?;
            collect_semantic_index_advice_observations(
                right,
                store,
                context,
                registry,
                observations,
            )
        }
        Plan::Scan { .. } => Ok(()),
    }
}

