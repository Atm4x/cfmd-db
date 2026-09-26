struct DirectI64JoinInputs<'a> {
    left: &'a InstalledRelation,
    right: &'a InstalledRelation,
    left_keys: NativeI64ColumnView<'a>,
    right_column: usize,
}

fn execute_persisted_i64_join(
    inputs: &DirectI64JoinInputs<'_>,
    index: I64IndexCapability<'_>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let left_data = &inputs.left.data;
    let right_data = &inputs.right.data;
    let left_i64_columns = native_all_i64_columns(left_data);
    let right_i64_columns = native_all_i64_columns(right_data);
    let mut out = Vec::with_capacity(inputs.left_keys.len());
    for left_index in inputs.left.scan_positions() {
        let key = inputs.left_keys.value(left_index);
        if let Some(right_rows) = index.probe(key) {
            for right_row_id in right_rows {
                let right_index = inputs
                    .right
                    .position(right_row_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let mut row = Vec::with_capacity(
                    native_column_count(left_data).saturating_add(native_column_count(right_data)),
                );
                if let (Some(left_columns), Some(right_columns)) =
                    (&left_i64_columns, &right_i64_columns)
                {
                    append_i64_row_values(&mut row, left_columns, left_index);
                    append_i64_row_values(&mut row, right_columns, right_index);
                } else {
                    append_native_row_values(&mut row, left_data, left_index)?;
                    append_native_row_values(&mut row, right_data, right_index)?;
                }
                out.push(row);
            }
        }
    }
    stats.scanned_rows = stats.scanned_rows.saturating_add(inputs.left_keys.len());
    account_join_reads(
        stats,
        inputs.left_keys.len(),
        0,
        out.len(),
        left_data,
        right_data,
    );
    stats.persisted_index_hits = stats.persisted_index_hits.saturating_add(1);
    Ok(Some(out))
}

fn execute_ephemeral_i64_join(
    inputs: &DirectI64JoinInputs<'_>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let left_data = &inputs.left.data;
    let right_data = &inputs.right.data;
    let Some(right_keys) = native_i64_column(right_data, inputs.right_column) else {
        return Ok(None);
    };
    stats.ephemeral_index_builds = stats.ephemeral_index_builds.saturating_add(1);
    let mut right_index: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    for row_index in inputs.right.scan_positions() {
        let key = right_keys.value(row_index);
        right_index.entry(key).or_default().push(row_index);
    }
    let mut out = Vec::with_capacity(inputs.left_keys.len());
    for left_index in inputs.left.scan_positions() {
        let key = inputs.left_keys.value(left_index);
        if let Some(right_rows) = right_index.get(&key) {
            for right_index in right_rows {
                let mut row = Vec::with_capacity(
                    native_column_count(left_data).saturating_add(native_column_count(right_data)),
                );
                append_native_row_values(&mut row, left_data, left_index)?;
                append_native_row_values(&mut row, right_data, *right_index)?;
                out.push(row);
            }
        }
    }
    stats.scanned_rows = stats
        .scanned_rows
        .saturating_add(inputs.left_keys.len().saturating_add(right_keys.len()));
    account_join_reads(
        stats,
        inputs.left_keys.len(),
        right_keys.len(),
        out.len(),
        left_data,
        right_data,
    );
    Ok(Some(out))
}

#[allow(clippy::too_many_arguments)]
fn try_execute_indexed_i64_join(
    left: &Plan,
    right: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
    allow_ephemeral: bool,
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
    validate_indexable_i64_relation(context, left_relation, &left_installed.data)?;
    validate_indexable_i64_relation(context, right_relation, &right_installed.data)?;
    let Some(left_keys) = native_i64_column(&left_installed.data, left_column) else {
        return Ok(None);
    };
    let inputs = DirectI64JoinInputs {
        left: left_installed,
        right: right_installed,
        left_keys,
        right_column,
    };
    let binding = I64IndexBinding {
        relation: right_relation,
        layout: right_layout,
        key_column: right_column,
        equivalence,
    };
    if let Some(index) = store.i64_index_capability(binding) {
        if allow_ephemeral {
            return Ok(None);
        }
        return execute_persisted_i64_join(&inputs, index, stats);
    }
    if !allow_ephemeral {
        return Ok(None);
    }
    execute_ephemeral_i64_join(&inputs, stats)
}

fn account_join_reads(
    stats: &mut ExecutionStats,
    left_keys: usize,
    right_keys: usize,
    output_rows: usize,
    left: &NativeRelation,
    right: &NativeRelation,
) {
    let output_width = native_column_count(left).saturating_add(native_column_count(right));
    stats.values_read = stats.values_read.saturating_add(
        left_keys
            .saturating_add(right_keys)
            .saturating_add(output_rows.saturating_mul(output_width)),
    );
}

