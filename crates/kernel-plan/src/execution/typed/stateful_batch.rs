fn try_execute_typed_stateful_batch(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<(Vec<kernel_query::Row>, ExecutionStats)>, PhysicalExecutionError> {
    let env = StatefulBatchEnv { context, registry };
    match plan {
        Plan::Distinct {
            input,
            column_equivalences,
        } => try_execute_typed_distinct(input, column_equivalences, store, &env),
        Plan::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } => try_execute_typed_group(
            input,
            GroupBatchSpec {
                group_columns,
                group_equivalences,
                aggregate,
            },
            store,
            &env,
        ),
        Plan::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } => try_execute_typed_top_k(
            input,
            TopKBatchSpec {
                column: *column,
                ordering: *ordering,
                direction: *direction,
                k: *k,
            },
            store,
            &env,
        ),
        Plan::Scan { .. }
        | Plan::FilterEqConst { .. }
        | Plan::FilterEqColumns { .. }
        | Plan::Project { .. }
        | Plan::JoinEq { .. }
        | Plan::Difference { .. }
        | Plan::AntiJoin { .. }
        | Plan::PromoteToBag(_) => Ok(None),
    }
}

struct StatefulBatchEnv<'a> {
    pub(super) context: &'a kernel_schema::SemanticContext,
    pub(super) registry: &'a kernel_semantics::SemanticRegistry,
}

#[derive(Clone, Copy)]
struct GroupBatchSpec<'a> {
    group_columns: &'a [usize],
    group_equivalences: &'a [SemanticId],
    aggregate: &'a AggregateSpec,
}

#[derive(Clone, Copy)]
struct TopKBatchSpec {
    column: usize,
    ordering: SemanticId,
    direction: OrderDirection,
    k: usize,
}

fn try_execute_typed_distinct(
    input: &Plan,
    column_equivalences: &[SemanticId],
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<(Vec<kernel_query::Row>, ExecutionStats)>, PhysicalExecutionError> {
    let Some(mut selection) =
        execute_typed_batch_selection(input, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let NativeRelation::TypedColumnar { columns, .. } = &selection.program.installed.data else {
        return Ok(None);
    };
    let mut unique: Vec<kernel_query::Row> = Vec::new();
    if selection.program.columns.len() == 1
        && column_equivalences.len() == 1
        && equivalence_is_i64_exact(column_equivalences[0], env.context, env.registry)?
    {
        let physical_column = selection.program.columns[0];
        if let Some(NativeColumn::I64(values)) = columns.get(physical_column) {
            let mut seen = BTreeSet::new();
            for position in selection.positions {
                selection.stats.values_read = selection.stats.values_read.saturating_add(1);
                let value = values[position];
                if seen.insert(value) {
                    unique.push(vec![Value::I64(value)]);
                }
            }
            selection.stats.typed_stateful_batch_hits =
                selection.stats.typed_stateful_batch_hits.saturating_add(1);
            return Ok(Some((unique, selection.stats)));
        }
    }
    let mut seen = BTreeSet::<Vec<kernel_semantics::CanonicalEqKey>>::new();
    for position in selection.positions {
        let row = materialize_typed_batch_row(
            &selection.program,
            columns,
            position,
            &mut selection.stats,
        )?;
        let class =
            canonical_semantic_row_key(&row, column_equivalences, env.context, env.registry)?;
        if seen.insert(class) {
            unique.push(row);
        }
    }
    selection.stats.typed_stateful_batch_hits =
        selection.stats.typed_stateful_batch_hits.saturating_add(1);
    Ok(Some((unique, selection.stats)))
}

fn try_execute_typed_group(
    input: &Plan,
    spec: GroupBatchSpec<'_>,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<(Vec<kernel_query::Row>, ExecutionStats)>, PhysicalExecutionError> {
    let Some(mut selection) =
        execute_typed_batch_selection(input, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let NativeRelation::TypedColumnar { columns, .. } = &selection.program.installed.data else {
        return Ok(None);
    };
    let rows = group_typed_batch_selection(
        &selection.program,
        columns,
        &selection.positions,
        spec,
        env,
        &mut selection.stats,
    )?;
    selection.stats.typed_stateful_batch_hits =
        selection.stats.typed_stateful_batch_hits.saturating_add(1);
    Ok(Some((rows, selection.stats)))
}

fn try_execute_typed_top_k(
    input: &Plan,
    spec: TopKBatchSpec,
    store: &PhysicalStore,
    env: &StatefulBatchEnv<'_>,
) -> Result<Option<(Vec<kernel_query::Row>, ExecutionStats)>, PhysicalExecutionError> {
    let Some(mut selection) =
        execute_typed_batch_selection(input, store, env.context, env.registry)?
    else {
        return Ok(None);
    };
    let NativeRelation::TypedColumnar { columns, .. } = &selection.program.installed.data else {
        return Ok(None);
    };
    let rows = top_k_typed_batch_selection(
        &selection.program,
        columns,
        selection.positions,
        spec,
        env,
        &mut selection.stats,
    )?;
    selection.stats.typed_stateful_batch_hits =
        selection.stats.typed_stateful_batch_hits.saturating_add(1);
    Ok(Some((rows, selection.stats)))
}

fn try_group_i64_exact_batch(
    program: &TypedBatchProgram<'_>,
    columns: &[NativeColumn],
    positions: &[usize],
    spec: GroupBatchSpec<'_>,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    if spec.group_columns.len() != 1
        || spec.group_equivalences.len() != 1
        || !equivalence_is_i64_exact(spec.group_equivalences[0], env.context, env.registry)?
    {
        return Ok(None);
    }
    let physical_group_column = *program
        .columns
        .get(spec.group_columns[0])
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let Some(NativeColumn::I64(keys)) = columns.get(physical_group_column) else {
        return Ok(None);
    };
    let mut lookup = BTreeMap::<i64, usize>::new();
    match spec.aggregate {
        AggregateSpec::Count { .. } => {
            stats.values_read = stats.values_read.saturating_add(positions.len());
            let mut groups: Vec<(i64, kernel_aggregate::ExactCount)> = Vec::new();
            for &position in positions {
                let key = keys[position];
                let index = if let Some(index) = lookup.get(&key).copied() {
                    index
                } else {
                    let index = groups.len();
                    groups.push((key, kernel_aggregate::ExactCount::default()));
                    lookup.insert(key, index);
                    index
                };
                groups[index].1.add_one();
            }
            let rows = groups
                .into_iter()
                .map(|(key, count)| {
                    Ok(vec![
                        Value::I64(key),
                        Value::I64(count.finish_i64().map_err(RelQueryError::from)?),
                    ])
                })
                .collect::<Result<Vec<_>, PhysicalExecutionError>>()?;
            Ok(Some(rows))
        }
        AggregateSpec::ExactF64Sum { value_column, .. } => {
            stats.values_read = stats
                .values_read
                .saturating_add(positions.len().saturating_mul(2));
            let physical_value_column = *program
                .columns
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let Some(NativeColumn::F64Bits(values)) = columns.get(physical_value_column) else {
                return Ok(None);
            };
            let mut groups: Vec<(i64, kernel_aggregate::ExactF64Sum)> = Vec::new();
            for &position in positions {
                let key = keys[position];
                let index = if let Some(index) = lookup.get(&key).copied() {
                    index
                } else {
                    let index = groups.len();
                    groups.push((key, kernel_aggregate::ExactF64Sum::default()));
                    lookup.insert(key, index);
                    index
                };
                groups[index]
                    .1
                    .add(f64::from_bits(values[position]))
                    .map_err(RelQueryError::from)?;
            }
            Ok(Some(
                groups
                    .into_iter()
                    .map(|(key, sum)| vec![Value::I64(key), Value::F64Bits(sum.finish().to_bits())])
                    .collect(),
            ))
        }
    }
}

enum TypedGroupState {
    Count(kernel_aggregate::ExactCount),
    ExactF64Sum(kernel_aggregate::ExactF64Sum),
}

fn new_typed_group_state(aggregate: &AggregateSpec) -> TypedGroupState {
    match aggregate {
        AggregateSpec::Count { .. } => {
            TypedGroupState::Count(kernel_aggregate::ExactCount::default())
        }
        AggregateSpec::ExactF64Sum { .. } => {
            TypedGroupState::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
        }
    }
}

fn finish_typed_groups(
    groups: Vec<(Vec<Value>, TypedGroupState)>,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    groups
        .into_iter()
        .map(|(mut key, state)| {
            key.push(match state {
                TypedGroupState::Count(count) => {
                    Value::I64(count.finish_i64().map_err(RelQueryError::from)?)
                }
                TypedGroupState::ExactF64Sum(sum) => Value::F64Bits(sum.finish().to_bits()),
            });
            Ok(key)
        })
        .collect()
}

fn group_typed_batch_selection(
    program: &TypedBatchProgram<'_>,
    columns: &[NativeColumn],
    positions: &[usize],
    spec: GroupBatchSpec<'_>,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    if spec.group_columns.len() != spec.group_equivalences.len() {
        return Err(RelQueryError::TypeMismatch.into());
    }
    if let Some(rows) = try_group_i64_exact_batch(program, columns, positions, spec, env, stats)? {
        return Ok(rows);
    }
    let physical_group_columns = spec
        .group_columns
        .iter()
        .map(|column| {
            program
                .columns
                .get(*column)
                .copied()
                .ok_or(RelQueryError::ColumnOutOfBounds)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let physical_sum_column = match spec.aggregate {
        AggregateSpec::ExactF64Sum { value_column, .. } => Some(
            *program
                .columns
                .get(*value_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?,
        ),
        AggregateSpec::Count { .. } => None,
    };

    let mut groups: Vec<(Vec<Value>, TypedGroupState)> = Vec::new();
    let mut group_by_class = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, usize>::new();
    if positions.is_empty() && spec.group_columns.is_empty() {
        groups.push((Vec::new(), new_typed_group_state(spec.aggregate)));
        group_by_class.insert(Vec::new(), 0);
    }
    for &position in positions {
        let mut key = Vec::with_capacity(physical_group_columns.len());
        for &physical_column in &physical_group_columns {
            key.push(
                columns
                    .get(physical_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?
                    .value_at(position),
            );
        }
        stats.values_read = stats
            .values_read
            .saturating_add(physical_group_columns.len());
        let class =
            canonical_semantic_row_key(&key, spec.group_equivalences, env.context, env.registry)?;
        let index = if let Some(&index) = group_by_class.get(&class) {
            index
        } else {
            let index = groups.len();
            groups.push((key, new_typed_group_state(spec.aggregate)));
            group_by_class.insert(class, index);
            index
        };
        match (&mut groups[index].1, spec.aggregate) {
            (TypedGroupState::Count(count), AggregateSpec::Count { .. }) => count.add_one(),
            (TypedGroupState::ExactF64Sum(sum), AggregateSpec::ExactF64Sum { .. }) => {
                let physical_column =
                    physical_sum_column.ok_or(RelQueryError::ColumnOutOfBounds)?;
                let column = columns
                    .get(physical_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                let NativeColumn::F64Bits(values) = column else {
                    return Err(PhysicalExecutionError::PhysicalTypeMismatch);
                };
                let bits = *values
                    .get(position)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                stats.values_read = stats.values_read.saturating_add(1);
                sum.add(f64::from_bits(bits)).map_err(RelQueryError::from)?;
            }
            _ => return Err(RelQueryError::TypeMismatch.into()),
        }
    }
    finish_typed_groups(groups)
}

fn top_k_typed_batch_selection(
    program: &TypedBatchProgram<'_>,
    columns: &[NativeColumn],
    mut positions: Vec<usize>,
    spec: TopKBatchSpec,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    if spec.k == 0 || positions.is_empty() {
        return Ok(Vec::new());
    }
    let physical_column = *program
        .columns
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
        select_top_k_i64_positions(&mut positions, values, spec.direction, spec.k, stats);
        return positions
            .into_iter()
            .map(|position| materialize_typed_batch_row(program, columns, position, stats))
            .collect();
    }
    select_top_k_semantic_positions(&mut positions, order_column, spec, env, stats)?;
    positions
        .into_iter()
        .map(|position| materialize_typed_batch_row(program, columns, position, stats))
        .collect()
}

fn select_top_k_i64_positions(
    positions: &mut Vec<usize>,
    values: &[i64],
    direction: OrderDirection,
    k: usize,
    stats: &mut ExecutionStats,
) {
    stats.values_read = stats.values_read.saturating_add(positions.len());
    if k < positions.len() {
        let mut keys = positions
            .iter()
            .map(|&position| values[position])
            .collect::<Vec<_>>();
        let threshold_index = match direction {
            OrderDirection::Ascending => k - 1,
            OrderDirection::Descending => keys.len() - k,
        };
        let (_, threshold, _) = keys.select_nth_unstable(threshold_index);
        let threshold = *threshold;
        match direction {
            OrderDirection::Ascending => {
                positions.retain(|&position| values[position] <= threshold);
                positions.sort_by_key(|&position| values[position]);
            }
            OrderDirection::Descending => {
                positions.retain(|&position| values[position] >= threshold);
                positions.sort_by_key(|&position| std::cmp::Reverse(values[position]));
            }
        }
    } else {
        match direction {
            OrderDirection::Ascending => positions.sort_by_key(|&position| values[position]),
            OrderDirection::Descending => {
                positions.sort_by_key(|&position| std::cmp::Reverse(values[position]));
            }
        }
    }
}

// HOSTILE[P165][ACTIVE][CLEAN:P165.T]: non-I64 typed TopK keeps only the semantic
// order-class frontier needed for K-with-ties. It never full-sorts the native input positions
// when k is smaller than the input; rows remain unmaterialized until the surviving frontier.
fn select_top_k_semantic_positions(
    positions: &mut Vec<usize>,
    order_column: &NativeColumn,
    spec: TopKBatchSpec,
    env: &StatefulBatchEnv<'_>,
    stats: &mut ExecutionStats,
) -> Result<(), PhysicalExecutionError> {
    let mut frontier = BTreeMap::<kernel_semantics::CanonicalOrderClassKey, Vec<usize>>::new();
    let mut retained = 0_usize;
    for position in positions.drain(..) {
        let value = order_column.value_at(position);
        stats.values_read = stats.values_read.saturating_add(1);
        let key = env
            .registry
            .canonical_order_key(env.context, spec.ordering, &value)?;
        frontier.entry(key).or_default().push(position);
        retained = retained.saturating_add(1);

        while retained > spec.k {
            let worst_count = match spec.direction {
                OrderDirection::Ascending => frontier
                    .last_key_value()
                    .map_or(0, |(_, bucket)| bucket.len()),
                OrderDirection::Descending => frontier
                    .first_key_value()
                    .map_or(0, |(_, bucket)| bucket.len()),
            };
            if worst_count == 0 || retained.saturating_sub(worst_count) < spec.k {
                break;
            }
            let removed = match spec.direction {
                OrderDirection::Ascending => frontier.pop_last(),
                OrderDirection::Descending => frontier.pop_first(),
            }
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            retained = retained.saturating_sub(removed.1.len());
        }
    }

    positions.reserve(retained);
    match spec.direction {
        OrderDirection::Ascending => {
            for (_, mut bucket) in frontier {
                positions.append(&mut bucket);
            }
        }
        OrderDirection::Descending => {
            while let Some((_, mut bucket)) = frontier.pop_last() {
                positions.append(&mut bucket);
            }
        }
    }
    Ok(())
}

