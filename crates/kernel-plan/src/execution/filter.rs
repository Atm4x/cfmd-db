// HOSTILE[P184][ACTIVE][CLEAN]: persisted semantic-filter probing/revalidation belongs to
// execution; direct plan-shape recognition is supplied by the neutral filter_shape capability.
fn try_execute_persisted_semantic_filter_plan(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let Some((relation, layout, predicates)) = collect_direct_filter_chain(plan) else {
        return Ok(None);
    };
    if predicates.is_empty() {
        return Ok(None);
    }
    let installed = store.installed(relation, layout)?;
    if let Some(rows) = try_execute_revision_semantic_filter(
        store,
        installed,
        relation,
        layout,
        &predicates,
        context,
        registry,
        stats,
    )? {
        return Ok(Some(rows));
    }
    let row_count = native_row_count(&installed.data);
    let bindings = store.semantic_fiber_bindings_for(relation, layout);
    let mut rejected_by_cost = false;
    let mut best: Option<(SemanticIndexBinding, Vec<&Value>, usize, usize)> = None;
    let mut probe_scratch = SemanticFiberProbeScratch::default();

    for binding in bindings {
        let Some(capability) = store.semantic_fiber_capability(&binding, context, registry)? else {
            continue;
        };
        let mut values = Vec::with_capacity(binding.key_parts.len());
        let mut complete = true;
        for part in &binding.key_parts {
            let Some((_, _, value)) = predicates.iter().find(|(column, equivalence, _)| {
                *column == part.column && *equivalence == part.equivalence
            }) else {
                complete = false;
                break;
            };
            values.push(*value);
        }
        if !complete {
            continue;
        }
        let matching_rows = capability
            .probe_values_with_scratch(&values, context, registry, &mut probe_scratch)?
            .map_or(0, |rows| rows.len());
        if SemanticAccessCostModel::choose_filter(row_count, matching_rows)
            == SemanticAccessPath::FullScan
        {
            rejected_by_cost = true;
            continue;
        }
        let replace = best.as_ref().is_none_or(|(_, _, best_rows, best_parts)| {
            matching_rows < *best_rows
                || (matching_rows == *best_rows && binding.key_parts.len() > *best_parts)
        });
        if replace {
            let parts = binding.key_parts.len();
            best = Some((binding, values, matching_rows, parts));
        }
    }

    let Some((binding, values, _, _)) = best else {
        if rejected_by_cost {
            stats.persisted_index_cost_rejections =
                stats.persisted_index_cost_rejections.saturating_add(1);
        }
        return Ok(None);
    };
    let capability = store
        .semantic_fiber_capability(&binding, context, registry)?
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
    let Some(row_ids) = capability.probe_values_with_scratch(
        &values,
        context,
        registry,
        &mut probe_scratch,
    )? else {
        stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
        return Ok(Some(Vec::new()));
    };
    let rows = materialize_semantic_filter_rows(
        installed,
        row_ids,
        &predicates,
        context,
        registry,
        stats,
    )?;
    stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
    Ok(Some(rows))
}

fn materialize_semantic_filter_rows<I>(
    installed: &InstalledRelation,
    row_ids: I,
    predicates: &[(usize, SemanticId, &Value)],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError>
where
    I: ExactSizeIterator<Item = crate::PhysicalRowId>,
{
    let mut rows = Vec::with_capacity(row_ids.len());
    for row_id in row_ids {
        let position = installed
            .position(row_id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let row = materialize_native_row(&installed.data, position)?;
        let mut matched = true;
        for (column, equivalence, value) in predicates {
            let candidate = row
                .get(*column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            stats.values_read = stats.values_read.saturating_add(1);
            if !registry.equivalent(context, *equivalence, candidate, value)? {
                matched = false;
                break;
            }
        }
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        if matched {
            stats.values_read = stats.values_read.saturating_add(row.len());
            rows.push(row);
        }
    }
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn try_execute_revision_semantic_filter(
    store: &PhysicalStore,
    installed: &InstalledRelation,
    relation: SemanticId,
    layout: LayoutBinding,
    predicates: &[(usize, SemanticId, &Value)],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let mut candidate_rows: Option<kernel_persistent::PersistentOrdSet<crate::PhysicalRowId>> = None;
    for (column, equivalence, value) in predicates {
        let Some(rows) = store.revision_semantic_probe_value(
            (relation, layout, *column),
            *equivalence,
            value,
            context,
            registry,
        )? else {
            return Ok(None);
        };
        candidate_rows = Some(match candidate_rows {
            None => rows,
            Some(existing) => existing
                .iter()
                .filter(|row| rows.contains(*row))
                .copied()
                .collect(),
        });
    }
    let row_ids = candidate_rows.unwrap_or_default();
    let mut rows = Vec::with_capacity(row_ids.len());
    for row_id in &row_ids {
        let position = installed
            .position(*row_id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let row = materialize_native_row(&installed.data, position)?;
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        stats.values_read = stats.values_read.saturating_add(row.len());
        rows.push(row);
    }
    stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
    Ok(Some(rows))
}

