fn produce_typed_top_k(
    input: &Plan,
    spec: TopKBatchSpec,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<OwnedTypedBatch>, PhysicalExecutionError> {
    if let Some(selection) = execute_typed_batch_selection(input, store, env.context, env.registry)?
    {
        return produce_top_k_from_raw_selection(selection, spec, env).map(Some);
    }
    let Some(batch) = try_produce_typed_stateful_batch(input, store, env)? else {
        return Ok(None);
    };
    produce_top_k_from_owned_batch(batch, spec, env).map(Some)
}

fn produce_top_k_from_raw_selection(
    selection: TypedBatchSelection<'_>,
    spec: TopKBatchSpec,
    env: &StatefulBatchEnv<'_>,
) -> Result<OwnedTypedBatch, PhysicalExecutionError> {
    let TypedBatchSelection {
        program,
        mut positions,
        mut stats,
    } = selection;
    let NativeRelation::TypedColumnar { columns, .. } = &program.installed.data else {
        return Err(PhysicalExecutionError::UnsupportedPhysicalPlan);
    };
    retain_top_k_positions(
        columns,
        &program.columns,
        &mut positions,
        spec,
        env,
        &mut stats,
    )?;
    let compact = select_logical_columns(columns, &program.columns, &positions)?;
    OwnedTypedBatch::dense(compact, stats)
}

fn produce_top_k_from_owned_batch(
    batch: OwnedTypedBatch,
    spec: TopKBatchSpec,
    env: &StatefulBatchEnv<'_>,
) -> Result<OwnedTypedBatch, PhysicalExecutionError> {
    let OwnedTypedBatch {
        columns,
        mut positions,
        mut stats,
    } = batch;
    let logical_columns = (0..columns.len()).collect::<Vec<_>>();
    retain_top_k_positions(
        &columns,
        &logical_columns,
        &mut positions,
        spec,
        env,
        &mut stats,
    )?;
    let compact = select_logical_columns(&columns, &logical_columns, &positions)?;
    OwnedTypedBatch::dense(compact, stats)
}

fn retain_top_k_positions(
    columns: &[NativeColumn],
    logical_columns: &[usize],
    positions: &mut Vec<usize>,
    spec: TopKBatchSpec,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<(), PhysicalExecutionError> {
    if spec.k == 0 || positions.is_empty() {
        positions.clear();
        return Ok(());
    }
    let physical_column = *logical_columns
        .get(spec.column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let order_column = columns
        .get(physical_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    if matches!(
        env.registry.ordering_domain(env.context, spec.ordering)?,
        kernel_semantics::OrderingDomain::I64
    ) && let NativeColumn::I64(values) = order_column
    {
        select_top_k_i64_positions(positions, values, spec.direction, spec.k, stats);
        return Ok(());
    }
    select_top_k_semantic_positions(positions, order_column, spec, env, stats)
}

fn select_logical_columns(
    columns: &[NativeColumn],
    logical_columns: &[usize],
    positions: &[usize],
) -> Result<Vec<NativeColumn>, PhysicalExecutionError> {
    logical_columns
        .iter()
        .map(|&column| {
            columns
                .get(column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
                .select_positions(positions)
        })
        .collect()
}

