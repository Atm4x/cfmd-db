#[allow(clippy::too_many_arguments)]
pub(super) fn execute_group_plan(
    input: &Plan,
    group_columns: &[usize],
    group_equivalences: &[SemanticId],
    aggregate: &AggregateSpec,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let rows = input.execute_native_rows(store, context, registry, stats)?;
    group_rows(
        rows,
        group_columns,
        group_equivalences,
        aggregate,
        context,
        registry,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_top_k_plan(
    input: &Plan,
    column: usize,
    ordering: SemanticId,
    direction: OrderDirection,
    k: usize,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let rows = input.execute_native_rows(store, context, registry, stats)?;
    top_k_rows(rows, column, ordering, direction, k, context, registry)
}

// HOSTILE[P161][ACTIVE][GENERIC:P160.N]: canonical grouping fallback; typed maintained paths exist.
fn group_rows(
    rows: Vec<kernel_query::Row>,
    group_columns: &[usize],
    group_equivalences: &[SemanticId],
    aggregate: &AggregateSpec,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    enum State {
        Count(kernel_aggregate::ExactCount),
        ExactF64Sum(kernel_aggregate::ExactF64Sum),
    }

    let mut groups: Vec<(Vec<Value>, State)> = Vec::new();
    let mut group_lookup = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, usize>::new();
    if rows.is_empty() && group_columns.is_empty() {
        let state = match aggregate {
            AggregateSpec::Count { .. } => State::Count(kernel_aggregate::ExactCount::default()),
            AggregateSpec::ExactF64Sum { .. } => {
                State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
            }
        };
        groups.push((Vec::new(), state));
    }
    for row in rows {
        let key = group_columns
            .iter()
            .map(|column| {
                row.get(*column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let canonical = canonical_semantic_row_key(&key, group_equivalences, context, registry)?;
        let index = if let Some(index) = group_lookup.get(&canonical).copied() {
            index
        } else {
            let state = match aggregate {
                AggregateSpec::Count { .. } => {
                    State::Count(kernel_aggregate::ExactCount::default())
                }
                AggregateSpec::ExactF64Sum { .. } => {
                    State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
                }
            };
            let index = groups.len();
            groups.push((key, state));
            group_lookup.insert(canonical, index);
            index
        };
        match (&mut groups[index].1, aggregate) {
            (State::Count(count), AggregateSpec::Count { .. }) => count.add_one(),
            (State::ExactF64Sum(sum), AggregateSpec::ExactF64Sum { value_column, .. }) => {
                let value = row
                    .get(*value_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                let Value::F64Bits(bits) = value else {
                    return Err(RelQueryError::TypeMismatch.into());
                };
                sum.add(f64::from_bits(*bits))
                    .map_err(RelQueryError::from)?;
            }
            _ => return Err(RelQueryError::TypeMismatch.into()),
        }
    }
    let mut output = Vec::with_capacity(groups.len());
    for (mut key, state) in groups {
        let value = match state {
            State::Count(count) => Value::I64(count.finish_i64().map_err(RelQueryError::from)?),
            State::ExactF64Sum(sum) => Value::F64Bits(sum.finish().to_bits()),
        };
        key.push(value);
        output.push(key);
    }
    Ok(output)
}

// HOSTILE[P163][ACTIVE][GENERIC][CLEAN:P160.N]: bounded Γ-order frontier; no full-result sort.
fn top_k_rows(
    rows: Vec<kernel_query::Row>,
    column: usize,
    ordering: SemanticId,
    direction: OrderDirection,
    k: usize,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    if k == 0 || rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut frontier =
        BTreeMap::<kernel_semantics::CanonicalOrderClassKey, Vec<kernel_query::Row>>::new();
    let mut retained = 0_usize;
    for row in rows {
        let value = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
        let key = registry.canonical_order_key(context, ordering, value)?;
        frontier.entry(key).or_default().push(row);
        retained = retained.saturating_add(1);

        while retained > k {
            let worst_count = match direction {
                OrderDirection::Ascending => frontier
                    .last_key_value()
                    .map_or(0, |(_, bucket)| bucket.len()),
                OrderDirection::Descending => frontier
                    .first_key_value()
                    .map_or(0, |(_, bucket)| bucket.len()),
            };
            if worst_count == 0 || retained.saturating_sub(worst_count) < k {
                break;
            }
            let removed = match direction {
                OrderDirection::Ascending => frontier.pop_last(),
                OrderDirection::Descending => frontier.pop_first(),
            }
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            retained = retained.saturating_sub(removed.1.len());
        }
    }

    let mut output = Vec::with_capacity(retained);
    match direction {
        OrderDirection::Ascending => {
            for (_, mut bucket) in frontier {
                output.append(&mut bucket);
            }
        }
        OrderDirection::Descending => {
            while let Some((_, mut bucket)) = frontier.pop_last() {
                output.append(&mut bucket);
            }
        }
    }
    Ok(output)
}

