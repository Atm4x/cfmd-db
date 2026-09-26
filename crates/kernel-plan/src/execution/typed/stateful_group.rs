fn produce_typed_group(
    input: &Plan,
    spec: GroupBatchSpec<'_>,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<OwnedTypedBatch>, PhysicalExecutionError> {
    if let Some(selection) = execute_typed_batch_selection(input, store, env.context, env.registry)?
    {
        return produce_group_from_raw_selection(selection, spec, env).map(Some);
    }
    let Some(batch) = try_produce_typed_stateful_batch(input, store, env)? else {
        return Ok(None);
    };
    produce_group_from_owned_batch(batch, spec, env).map(Some)
}

fn produce_group_from_raw_selection(
    selection: TypedBatchSelection<'_>,
    spec: GroupBatchSpec<'_>,
    env: &StatefulBatchEnv<'_>,
) -> Result<OwnedTypedBatch, PhysicalExecutionError> {
    validate_group_batch_spec(spec)?;
    let TypedBatchSelection {
        program,
        positions,
        mut stats,
    } = selection;
    let NativeRelation::TypedColumnar { columns, .. } = &program.installed.data else {
        return Err(PhysicalExecutionError::UnsupportedPhysicalPlan);
    };
    let physical_group_columns = resolve_group_columns(&program.columns, spec.group_columns)?;
    let output_columns = produce_group_columns(
        columns,
        &program.columns,
        &positions,
        &physical_group_columns,
        spec,
        env,
        &mut stats,
    )?
    .ok_or(PhysicalExecutionError::UnsupportedPhysicalPlan)?;
    OwnedTypedBatch::dense(output_columns, stats)
}

fn produce_group_from_owned_batch(
    batch: OwnedTypedBatch,
    spec: GroupBatchSpec<'_>,
    env: &StatefulBatchEnv<'_>,
) -> Result<OwnedTypedBatch, PhysicalExecutionError> {
    validate_group_batch_spec(spec)?;
    let OwnedTypedBatch {
        columns,
        positions,
        mut stats,
    } = batch;
    let logical_columns = (0..columns.len()).collect::<Vec<_>>();
    let group_columns = resolve_group_columns(&logical_columns, spec.group_columns)?;
    let output_columns = produce_group_columns(
        &columns,
        &logical_columns,
        &positions,
        &group_columns,
        spec,
        env,
        &mut stats,
    )?
    .ok_or(PhysicalExecutionError::UnsupportedPhysicalPlan)?;
    OwnedTypedBatch::dense(output_columns, stats)
}

fn validate_group_batch_spec(spec: GroupBatchSpec<'_>) -> Result<(), PhysicalExecutionError> {
    if spec.group_columns.len() != spec.group_equivalences.len() {
        return Err(RelQueryError::TypeMismatch.into());
    }
    Ok(())
}

fn resolve_group_columns(
    logical_columns: &[usize],
    group_columns: &[usize],
) -> Result<Vec<usize>, PhysicalExecutionError> {
    group_columns
        .iter()
        .map(|column| {
            logical_columns
                .get(*column)
                .copied()
                .ok_or_else(|| RelQueryError::ColumnOutOfBounds.into())
        })
        .collect()
}

fn produce_group_columns(
    columns: &[NativeColumn],
    logical_columns: &[usize],
    positions: &[usize],
    physical_group_columns: &[usize],
    spec: GroupBatchSpec<'_>,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<NativeColumn>>, PhysicalExecutionError> {
    if let Some(output_columns) = try_produce_exact_i64_group_columns(
        logical_columns,
        columns,
        positions,
        physical_group_columns,
        spec,
        env,
        stats,
    )? {
        return Ok(Some(output_columns));
    }
    produce_primitive_group_columns(
        logical_columns,
        columns,
        positions,
        physical_group_columns,
        spec,
        env,
        stats,
    )
}

fn try_produce_exact_i64_group_columns(
    logical_columns: &[usize],
    columns: &[NativeColumn],
    positions: &[usize],
    physical_group_columns: &[usize],
    spec: GroupBatchSpec<'_>,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<NativeColumn>>, PhysicalExecutionError> {
    if spec.group_columns.len() != 1
        || !equivalence_is_i64_exact(spec.group_equivalences[0], env.context, env.registry)?
    {
        return Ok(None);
    }
    let Some(NativeColumn::I64(keys)) = columns.get(physical_group_columns[0]) else {
        return Ok(None);
    };
    let mut lookup = BTreeMap::<i64, usize>::new();
    let mut output_keys = Vec::<i64>::new();
    match spec.aggregate {
        AggregateSpec::Count { .. } => {
            let mut counts = Vec::<kernel_aggregate::ExactCount>::new();
            for &position in positions {
                let key = keys[position];
                let index = *lookup.entry(key).or_insert_with(|| {
                    let index = output_keys.len();
                    output_keys.push(key);
                    counts.push(kernel_aggregate::ExactCount::default());
                    index
                });
                counts[index].add_one();
            }
            stats.values_read = stats.values_read.saturating_add(positions.len());
            let output_counts = counts
                .into_iter()
                .map(|count| count.finish_i64().map_err(RelQueryError::from))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(vec![
                NativeColumn::I64(output_keys.into()),
                NativeColumn::I64(output_counts.into()),
            ]))
        }
        AggregateSpec::ExactF64Sum { value_column, .. } => {
            let physical_value_column = *logical_columns
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let Some(NativeColumn::F64Bits(values)) = columns.get(physical_value_column) else {
                return Ok(None);
            };
            let mut sums = Vec::<kernel_aggregate::ExactF64Sum>::new();
            for &position in positions {
                let key = keys[position];
                let index = *lookup.entry(key).or_insert_with(|| {
                    let index = output_keys.len();
                    output_keys.push(key);
                    sums.push(kernel_aggregate::ExactF64Sum::default());
                    index
                });
                sums[index]
                    .add(f64::from_bits(values[position]))
                    .map_err(RelQueryError::from)?;
            }
            stats.values_read = stats
                .values_read
                .saturating_add(positions.len().saturating_mul(2));
            Ok(Some(vec![
                NativeColumn::I64(output_keys.into()),
                NativeColumn::F64Bits(sums.into_iter().map(|sum| sum.finish().to_bits()).collect()),
            ]))
        }
    }
}

fn produce_primitive_group_columns(
    logical_columns: &[usize],
    columns: &[NativeColumn],
    positions: &[usize],
    physical_group_columns: &[usize],
    spec: GroupBatchSpec<'_>,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<NativeColumn>>, PhysicalExecutionError> {
    let mut encoders = Vec::with_capacity(spec.group_equivalences.len());
    for &equivalence in spec.group_equivalences {
        let Some(encoder) = env
            .registry
            .resolve_primitive_equivalence(env.context, equivalence)?
        else {
            return Ok(None);
        };
        encoders.push(encoder);
    }
    let Some(accumulation) = accumulate_primitive_groups(
        logical_columns,
        columns,
        positions,
        physical_group_columns,
        spec,
        &encoders,
        stats,
    )?
    else {
        return Ok(None);
    };
    finish_primitive_group_columns(
        columns,
        physical_group_columns,
        spec.aggregate,
        &accumulation.representatives,
        accumulation.aggregates,
    )
    .map(Some)
}

struct PrimitiveGroupAccumulation {
    representatives: Vec<Vec<Value>>,
    aggregates: Vec<TypedGroupState>,
}

fn accumulate_primitive_groups(
    logical_columns: &[usize],
    columns: &[NativeColumn],
    positions: &[usize],
    physical_group_columns: &[usize],
    spec: GroupBatchSpec<'_>,
    encoders: &[kernel_semantics::ResolvedPrimitiveEquivalence],
    stats: &mut ExecutionStats,
) -> Result<Option<PrimitiveGroupAccumulation>, PhysicalExecutionError> {
    let mut lookup = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, usize>::new();
    let mut representatives = Vec::<Vec<Value>>::new();
    let mut aggregates = Vec::<TypedGroupState>::new();
    if positions.is_empty() && spec.group_columns.is_empty() {
        representatives.push(Vec::new());
        aggregates.push(new_typed_group_state(spec.aggregate));
    }
    for &position in positions {
        let mut canonical = Vec::with_capacity(physical_group_columns.len());
        let mut representative = Vec::with_capacity(physical_group_columns.len());
        for (&physical_column, encoder) in physical_group_columns.iter().zip(encoders) {
            let value = columns
                .get(physical_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
                .value_at(position);
            canonical.push(encoder.canonical_key(&value)?);
            representative.push(value);
        }
        stats.values_read = stats
            .values_read
            .saturating_add(physical_group_columns.len());
        let index = if let Some(index) = lookup.get(&canonical).copied() {
            index
        } else {
            let index = representatives.len();
            lookup.insert(canonical, index);
            representatives.push(representative);
            aggregates.push(new_typed_group_state(spec.aggregate));
            index
        };
        if !update_typed_group_state(
            &mut aggregates[index],
            spec.aggregate,
            logical_columns,
            columns,
            position,
            stats,
        )? {
            return Ok(None);
        }
    }
    Ok(Some(PrimitiveGroupAccumulation {
        representatives,
        aggregates,
    }))
}

fn update_typed_group_state(
    aggregate_state: &mut TypedGroupState,
    aggregate: &AggregateSpec,
    logical_columns: &[usize],
    columns: &[NativeColumn],
    position: usize,
    execution_stats: &mut ExecutionStats,
) -> Result<bool, PhysicalExecutionError> {
    match (aggregate_state, aggregate) {
        (TypedGroupState::Count(count), AggregateSpec::Count { .. }) => count.add_one(),
        (TypedGroupState::ExactF64Sum(sum), AggregateSpec::ExactF64Sum { value_column, .. }) => {
            let physical_value_column = *logical_columns
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let Some(NativeColumn::F64Bits(values)) = columns.get(physical_value_column) else {
                return Ok(false);
            };
            sum.add(f64::from_bits(values[position]))
                .map_err(RelQueryError::from)?;
            execution_stats.values_read = execution_stats.values_read.saturating_add(1);
        }
        _ => return Err(RelQueryError::TypeMismatch.into()),
    }
    Ok(true)
}

fn finish_primitive_group_columns(
    columns: &[NativeColumn],
    physical_group_columns: &[usize],
    aggregate: &AggregateSpec,
    representatives: &[Vec<Value>],
    aggregates: Vec<TypedGroupState>,
) -> Result<Vec<NativeColumn>, PhysicalExecutionError> {
    let mut output_columns = Vec::with_capacity(physical_group_columns.len() + 1);
    for (key_index, &physical_column) in physical_group_columns.iter().enumerate() {
        let values = representatives
            .iter()
            .map(|key| key[key_index].clone())
            .collect::<Vec<_>>();
        output_columns.push(NativeColumn::from_values_like(
            columns
                .get(physical_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?,
            values,
        )?);
    }
    match aggregate {
        AggregateSpec::Count { .. } => output_columns.push(NativeColumn::I64(
            aggregates
                .into_iter()
                .map(|state| match state {
                    TypedGroupState::Count(count) => {
                        count.finish_i64().map_err(RelQueryError::from)
                    }
                    TypedGroupState::ExactF64Sum(_) => Err(RelQueryError::TypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        )),
        AggregateSpec::ExactF64Sum { .. } => output_columns.push(NativeColumn::F64Bits(
            aggregates
                .into_iter()
                .map(|state| match state {
                    TypedGroupState::ExactF64Sum(sum) => Ok(sum.finish().to_bits()),
                    TypedGroupState::Count(_) => Err(RelQueryError::TypeMismatch),
                })
                .collect::<Result<Vec<_>, RelQueryError>>()?
                .into(),
        )),
    }
    Ok(output_columns)
}

