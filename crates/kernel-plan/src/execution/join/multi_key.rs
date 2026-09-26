fn binding_join_key_order(
    binding: &SemanticIndexBinding,
    keys: &[JoinKeySpec],
) -> Option<Vec<usize>> {
    if binding.key_parts.len() != keys.len() {
        return None;
    }
    let mut used = vec![false; keys.len()];
    let mut order = Vec::with_capacity(keys.len());
    for part in &binding.key_parts {
        let (index, _) = keys.iter().enumerate().find(|(index, key)| {
            !used[*index] && key.right_column == part.column && key.equivalence == part.equivalence
        })?;
        used[index] = true;
        order.push(index);
    }
    Some(order)
}

fn persisted_multi_key_semantic_candidate<'a>(
    store: &'a PhysicalStore,
    right_relation: SemanticId,
    right_layout: LayoutBinding,
    keys: &[JoinKeySpec],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<(SemanticFiberCapability<'a>, Vec<usize>)>, PhysicalExecutionError> {
    for binding in store.semantic_fiber_bindings_for(right_relation, right_layout) {
        let Some(order) = binding_join_key_order(&binding, keys) else {
            continue;
        };
        if let Some(capability) = store.semantic_fiber_capability(&binding, context, registry)? {
            return Ok(Some((capability, order)));
        }
    }
    Ok(None)
}

fn try_execute_persisted_semantic_multi_key_join_plan(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let Some(shape) = direct_join_shape(plan, store)? else {
        return Ok(None);
    };
    if shape.keys.len() < 2 {
        return Ok(None);
    }
    let (left_relation, left_layout) = shape.left_scan;
    let (right_relation, right_layout) = shape.right_scan;
    let left_installed = store.installed(left_relation, left_layout)?;
    let right_installed = store.installed(right_relation, right_layout)?;
    let keys = shape.keys;

    let Some((index, key_order)) = persisted_multi_key_semantic_candidate(
        store,
        right_relation,
        right_layout,
        &keys,
        context,
        registry,
    )?
    else {
        return Ok(None);
    };

    let mut out = Vec::new();
    let mut probe_scratch = SemanticFiberProbeScratch::default();
    for left_position in left_installed.scan_positions() {
        let left_row = materialize_native_row(&left_installed.data, left_position)?;
        stats.scanned_rows = stats.scanned_rows.saturating_add(1);
        stats.values_read = stats.values_read.saturating_add(key_order.len());
        if let Some(right_ids) = index.probe_row_columns_with_scratch(
            &left_row,
            key_order.iter().map(|&key_index| keys[key_index].left_column),
            context,
            registry,
            &mut probe_scratch,
        )? {
            for right_id in right_ids {
                let right_position = right_installed
                    .position(right_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let right_row = materialize_native_row(&right_installed.data, right_position)?;
                for key in &keys {
                    let left_value = left_row
                        .get(key.left_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?;
                    let right_value = right_row
                        .get(key.right_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?;
                    stats.values_read = stats.values_read.saturating_add(2);
                    if !registry.equivalent(context, key.equivalence, left_value, right_value)? {
                        return Err(RelQueryError::InconsistentIncrementalDelta.into());
                    }
                }
                let mut row = left_row.clone();
                row.extend(right_row);
                stats.values_read = stats.values_read.saturating_add(row.len());
                out.push(row);
            }
        }
    }
    stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
    Ok(Some(out))
}

