#[allow(clippy::too_many_arguments)]
fn try_execute_persisted_semantic_join(
    left: &Plan,
    right: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let Some((left_relation, left_layout)) = transparent_direct_scan_relation(left, store) else {
        return Ok(None);
    };
    let Some((right_relation, right_layout)) = transparent_direct_scan_relation(right, store)
    else {
        return Ok(None);
    };
    registry.equivalence_domain(context, equivalence)?;
    let binding =
        SemanticIndexBinding::single(right_relation, right_layout, right_column, equivalence);
    let Some(index) = store.semantic_fiber_capability(&binding, context, registry)? else {
        return Ok(None);
    };
    let left_installed = store.installed(left_relation, left_layout)?;
    let right_installed = store.installed(right_relation, right_layout)?;
    let mut out = Vec::new();
    let mut probe_scratch = SemanticFiberProbeScratch::default();
    for left_position in left_installed.scan_positions() {
        let left_row = materialize_native_row(&left_installed.data, left_position)?;
        let left_key = left_row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        stats.values_read = stats.values_read.saturating_add(1);
        if let Some(right_ids) = index.probe_row_columns_with_scratch(
            &left_row,
            std::iter::once(left_column),
            context,
            registry,
            &mut probe_scratch,
        )? {
            for right_id in right_ids {
                let right_position = right_installed
                    .position(right_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let right_row = materialize_native_row(&right_installed.data, right_position)?;
                let right_key = right_row
                    .get(right_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                if !registry.equivalent(context, equivalence, left_key, right_key)? {
                    return Err(RelQueryError::InconsistentIncrementalDelta.into());
                }
                let mut row = left_row.clone();
                row.extend(right_row);
                stats.values_read = stats
                    .values_read
                    .saturating_add(row.len().saturating_add(1));
                out.push(row);
            }
        }
    }
    stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
    Ok(Some(out))
}

