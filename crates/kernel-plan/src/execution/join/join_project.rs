#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn try_execute_indexed_i64_join_project(
    left: &Plan,
    right: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    projection: &[usize],
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
    let Some(resolved) = registry.resolve_primitive_equivalence(context, equivalence)? else {
        return Ok(None);
    };
    if !matches!(
        resolved.bind_right(&Value::I64(0)),
        Ok(kernel_semantics::BoundPrimitivePredicate::I64(_))
    ) {
        return Ok(None);
    }

    let left_installed = store.installed(left_relation, left_layout)?;
    let right_installed = store.installed(right_relation, right_layout)?;
    let left_data = &left_installed.data;
    let right_data = &right_installed.data;
    validate_indexable_i64_relation(context, left_relation, left_data)?;
    validate_indexable_i64_relation(context, right_relation, right_data)?;
    let Some(left_columns) = native_all_i64_columns(left_data) else {
        return Ok(None);
    };
    let Some(right_columns) = native_all_i64_columns(right_data) else {
        return Ok(None);
    };
    let Some(left_keys) = left_columns.get(left_column).copied() else {
        return Ok(None);
    };
    let Some(right_keys) = right_columns.get(right_column).copied() else {
        return Ok(None);
    };
    let joined_width = left_columns.len().saturating_add(right_columns.len());
    if projection.iter().any(|column| *column >= joined_width) {
        return Err(RelQueryError::ColumnOutOfBounds.into());
    }

    let mut out = Vec::new();
    let mut push_projected = |left_index: usize, right_index: usize| {
        let mut row = Vec::with_capacity(projection.len());
        for &column in projection {
            let value = if column < left_columns.len() {
                left_columns[column].value(left_index)
            } else {
                right_columns[column - left_columns.len()].value(right_index)
            };
            row.push(Value::I64(value));
        }
        out.push(row);
    };

    if let Some(index) = store.i64_index_capability(I64IndexBinding {
        relation: right_relation,
        layout: right_layout,
        key_column: right_column,
        equivalence,
    }) {
        for left_index in left_installed.scan_positions() {
            if let Some(right_rows) = index.probe(left_keys.value(left_index)) {
                for right_row_id in right_rows {
                    let right_index = right_installed
                        .position(right_row_id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                    push_projected(left_index, right_index);
                }
            }
        }
        stats.scanned_rows = stats.scanned_rows.saturating_add(left_keys.len());
        stats.values_read = stats.values_read.saturating_add(
            left_keys
                .len()
                .saturating_add(out.len().saturating_mul(projection.len())),
        );
        stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
        stats.fused_join_project_hits = stats.fused_join_project_hits.saturating_add(1);
        return Ok(Some(out));
    }

    let mut right_index: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for row_index in right_installed.scan_positions() {
        right_index
            .entry(right_keys.value(row_index))
            .or_default()
            .push(row_index);
    }
    for left_index in left_installed.scan_positions() {
        if let Some(right_rows) = right_index.get(&left_keys.value(left_index)) {
            for &right_index in right_rows {
                push_projected(left_index, right_index);
            }
        }
    }
    stats.scanned_rows = stats
        .scanned_rows
        .saturating_add(left_keys.len().saturating_add(right_keys.len()));
    stats.values_read = stats.values_read.saturating_add(
        left_keys
            .len()
            .saturating_add(right_keys.len())
            .saturating_add(out.len().saturating_mul(projection.len())),
    );
    stats.ephemeral_index_builds = stats.ephemeral_index_builds.saturating_add(1);
    stats.fused_join_project_hits = stats.fused_join_project_hits.saturating_add(1);
    Ok(Some(out))
}

fn join_rows(
    left_rows: &[kernel_query::Row],
    right_rows: &[kernel_query::Row],
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let mut right_buckets =
        BTreeMap::<kernel_semantics::CanonicalEqKey, Vec<&kernel_query::Row>>::new();
    for right in right_rows {
        let right_key = right
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let canonical = registry.canonical_equivalence_key(context, equivalence, right_key)?;
        right_buckets.entry(canonical).or_default().push(right);
    }

    let mut out = Vec::new();
    for left in left_rows {
        let left_key = left
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let canonical = registry.canonical_equivalence_key(context, equivalence, left_key)?;
        let Some(matches) = right_buckets.get(&canonical) else {
            continue;
        };
        for right in matches {
            let mut row = Vec::with_capacity(left.len() + right.len());
            row.extend(left.iter().cloned());
            row.extend(right.iter().cloned());
            out.push(row);
        }
    }
    Ok(out)
}

