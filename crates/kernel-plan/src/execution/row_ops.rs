// HOSTILE[P180][ACTIVE][CLEAN]: scan/filter/project/distinct row operations are separated from
// JoinEq physical dispatch without introducing a new representation boundary.
pub(super) fn execute_project_plan(
    input: &Plan,
    columns: &[usize],
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    if let Some(rows) =
        try_execute_persisted_semantic_multi_key_join_plan(input, store, context, registry, stats)?
    {
        return project_rows(rows, columns, stats);
    }
    if let Plan::JoinEq {
        left,
        right,
        left_column,
        right_column,
        equivalence,
    } = input
    {
        let key = JoinKeySpec {
            left_column: *left_column,
            right_column: *right_column,
            equivalence: *equivalence,
        };
        if let Some(decision) =
            direct_join_access_decision(left, right, key, store, context, registry)?
        {
            match decision.family {
                JoinAccessFamily::PersistedSemantic => {
                    if let Some(rows) = try_execute_persisted_semantic_join(
                        left,
                        right,
                        *left_column,
                        *right_column,
                        *equivalence,
                        store,
                        context,
                        registry,
                        stats,
                    )? {
                        return project_rows(rows, columns, stats);
                    }
                }
                JoinAccessFamily::PersistedI64 | JoinAccessFamily::EphemeralI64 => {
                    if let Some(rows) = try_execute_indexed_i64_join_project(
                        left,
                        right,
                        *left_column,
                        *right_column,
                        *equivalence,
                        columns,
                        store,
                        context,
                        registry,
                        stats,
                    )? {
                        return Ok(rows);
                    }
                }
                JoinAccessFamily::FullScan => {}
            }
        }
    }
    if let Some(rows) =
        try_execute_persisted_semantic_filter_plan(input, store, context, registry, stats)?
    {
        return project_rows(rows, columns, stats);
    }
    if let Plan::FilterEqConst {
        input: filter_input,
        column,
        value,
        equivalence,
    } = input
        && let Plan::Scan { relation, layout } = filter_input.as_ref()
    {
        return execute_fused_filter_project_scan(
            store,
            *relation,
            *layout,
            *column,
            value,
            *equivalence,
            columns,
            context,
            registry,
            stats,
        );
    }
    let input_rows = input.execute_native_rows(store, context, registry, stats)?;
    project_rows(input_rows, columns, stats)
}

pub(super) fn scan_rows(
    store: &PhysicalStore,
    relation: SemanticId,
    layout: LayoutBinding,
    context: &kernel_schema::SemanticContext,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let installed = store.installed(relation, layout)?;
    if !installed.scan_order_is_physical {
        let row_count = installed.scan_positions().count();
        stats.scanned_rows = stats.scanned_rows.saturating_add(row_count);
        stats.values_read = stats
            .values_read
            .saturating_add(native_column_count(&installed.data).saturating_mul(row_count));
        let mut rows = Vec::with_capacity(row_count);
        for position in installed.scan_positions() {
            rows.push(materialize_native_row(&installed.data, position)?);
        }
        return Ok(rows);
    }
    match &installed.data {
        NativeRelation::RowStore(rows) => {
            stats.scanned_rows = stats.scanned_rows.saturating_add(rows.len());
            stats.values_read = stats
                .values_read
                .saturating_add(rows.iter().map(Vec::len).sum::<usize>());
            Ok(rows.iter().cloned().collect())
        }
        NativeRelation::Columnar { columns, row_count } => {
            stats.scanned_rows = stats.scanned_rows.saturating_add(*row_count);
            stats.values_read = stats
                .values_read
                .saturating_add(columns.len().saturating_mul(*row_count));
            let mut rows = Vec::with_capacity(*row_count);
            for row_index in 0..*row_count {
                rows.push(
                    columns
                        .iter()
                        .map(|column| column[row_index].clone())
                        .collect(),
                );
            }
            Ok(rows)
        }
        NativeRelation::I64Columnar { columns, row_count } => {
            validate_i64_columnar_schema(context, relation, columns.len())?;
            stats.scanned_rows = stats.scanned_rows.saturating_add(*row_count);
            stats.values_read = stats
                .values_read
                .saturating_add(columns.len().saturating_mul(*row_count));
            let mut rows = Vec::with_capacity(*row_count);
            for row_index in 0..*row_count {
                rows.push(
                    columns
                        .iter()
                        .map(|column| Value::I64(column[row_index]))
                        .collect(),
                );
            }
            Ok(rows)
        }
        NativeRelation::TypedColumnar { columns, row_count } => {
            validate_typed_columnar_schema(context, relation, columns)?;
            stats.scanned_rows = stats.scanned_rows.saturating_add(*row_count);
            stats.values_read = stats
                .values_read
                .saturating_add(columns.len().saturating_mul(*row_count));
            let mut rows = Vec::with_capacity(*row_count);
            for row_index in 0..*row_count {
                rows.push(
                    columns
                        .iter()
                        .map(|column| column.value_at(row_index))
                        .collect(),
                );
            }
            Ok(rows)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_fused_filter_project_scan(
    store: &PhysicalStore,
    relation: SemanticId,
    layout: LayoutBinding,
    predicate_column: usize,
    predicate_value: &Value,
    equivalence: SemanticId,
    projection: &[usize],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let installed = store.installed(relation, layout)?;
    if !installed.scan_order_is_physical {
        let rows = scan_rows(store, relation, layout, context, stats)?;
        let filtered = filter_rows(
            rows,
            predicate_column,
            predicate_value,
            equivalence,
            context,
            registry,
            stats,
        )?;
        return project_rows(filtered, projection, stats);
    }
    let resolved = registry.resolve_primitive_equivalence(context, equivalence)?;
    let bound = resolved
        .as_ref()
        .map(|resolved| resolved.bind_right(predicate_value))
        .transpose()?;
    match &installed.data {
        NativeRelation::I64Columnar { columns, row_count }
            if matches!(
                bound,
                Some(kernel_semantics::BoundPrimitivePredicate::I64(_))
            ) =>
        {
            execute_i64_fused(
                columns,
                *row_count,
                relation,
                predicate_column,
                projection,
                bound.as_ref(),
                context,
                stats,
            )
        }
        NativeRelation::Columnar { columns, row_count } => execute_value_columnar_fused(
            columns,
            *row_count,
            predicate_column,
            predicate_value,
            equivalence,
            projection,
            bound.as_ref(),
            resolved.as_ref(),
            context,
            registry,
            stats,
        ),
        NativeRelation::I64Columnar { columns, row_count } => execute_i64_generic_fused(
            columns,
            *row_count,
            relation,
            predicate_column,
            predicate_value,
            equivalence,
            projection,
            bound.as_ref(),
            resolved.as_ref(),
            context,
            registry,
            stats,
        ),
        NativeRelation::TypedColumnar { columns, row_count } => execute_typed_columnar_fused(
            columns,
            *row_count,
            relation,
            predicate_column,
            predicate_value,
            equivalence,
            projection,
            bound.as_ref(),
            context,
            registry,
            stats,
        ),
        NativeRelation::RowStore(rows) => execute_row_store_fused(
            rows,
            predicate_column,
            predicate_value,
            equivalence,
            projection,
            bound.as_ref(),
            resolved.as_ref(),
            context,
            registry,
            stats,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_typed_columnar_fused(
    columns: &[NativeColumn],
    row_count: usize,
    relation: SemanticId,
    predicate_column: usize,
    predicate_value: &Value,
    equivalence: SemanticId,
    projection: &[usize],
    bound: Option<&kernel_semantics::BoundPrimitivePredicate>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    validate_typed_columnar_schema(context, relation, columns)?;
    let predicate = columns
        .get(predicate_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    validate_projection(projection, columns.len())?;
    stats.scanned_rows = stats.scanned_rows.saturating_add(row_count);
    let rows = if let Some(bound) = bound {
        execute_bound_typed_filter(columns, predicate, bound, projection, row_count)?
    } else if let NativeColumn::Algebraic(column) = predicate {
        let request = AlgebraicFilterRequest {
            predicate_value,
            equivalence,
            projection,
            context,
            registry,
        };
        execute_algebraic_equivalence_filter(columns, column, row_count, &request)?
    } else {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    };
    account_fused_reads(stats, row_count, rows.len(), projection.len());
    Ok(rows)
}

struct AlgebraicFilterRequest<'a> {
    predicate_value: &'a Value,
    equivalence: SemanticId,
    projection: &'a [usize],
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

fn execute_algebraic_equivalence_filter(
    columns: &[NativeColumn],
    predicate: &algebraic_native::AlgebraicNativeColumn,
    row_count: usize,
    request: &AlgebraicFilterRequest<'_>,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let compiled = request
        .registry
        .compile_equivalence(request.context, request.equivalence)?;
    let expected = compiled.canonical_key(request.predicate_value)?;
    let mut matching = Vec::new();
    for row_index in 0..row_count {
        if predicate.canonical_key_at_compiled(row_index, &compiled)? == expected {
            matching.push(row_index);
        }
    }
    let mut rows = Vec::with_capacity(matching.len());
    push_typed_matches(&mut rows, columns, request.projection, matching);
    Ok(rows)
}

fn execute_bound_typed_filter(
    columns: &[NativeColumn],
    predicate: &NativeColumn,
    bound: &kernel_semantics::BoundPrimitivePredicate,
    projection: &[usize],
    row_count: usize,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let mut rows = Vec::new();
    match (predicate, bound) {
        (NativeColumn::Unit(_), kernel_semantics::BoundPrimitivePredicate::Unit) => {
            push_typed_matches(&mut rows, columns, projection, 0..row_count);
        }
        (NativeColumn::Bool(values), kernel_semantics::BoundPrimitivePredicate::Bool(expected)) => {
            push_typed_matches(
                &mut rows,
                columns,
                projection,
                matching_indices(values, |candidate| candidate == expected),
            );
        }
        (NativeColumn::I64(values), kernel_semantics::BoundPrimitivePredicate::I64(expected)) => {
            return Ok(execute_bound_i64_filter(
                columns, values, *expected, projection,
            ));
        }
        (
            NativeColumn::F64Bits(values),
            kernel_semantics::BoundPrimitivePredicate::F64Bits(expected),
        ) => push_typed_matches(
            &mut rows,
            columns,
            projection,
            matching_indices(values, |candidate| candidate == expected),
        ),
        (
            NativeColumn::Text(values),
            kernel_semantics::BoundPrimitivePredicate::TextExact(expected),
        ) => push_typed_matches(
            &mut rows,
            columns,
            projection,
            matching_indices(values, |candidate| candidate == expected),
        ),
        (
            NativeColumn::Text(values),
            kernel_semantics::BoundPrimitivePredicate::TextAsciiCaseInsensitive(expected),
        ) => push_typed_matches(
            &mut rows,
            columns,
            projection,
            matching_indices(values, |candidate| candidate.eq_ignore_ascii_case(expected)),
        ),
        (
            NativeColumn::LiveEntityIds {
                entity_type,
                values,
            },
            kernel_semantics::BoundPrimitivePredicate::LiveEntityId {
                entity_type: expected_type,
                id: expected,
            },
        ) if entity_type == expected_type => push_typed_matches(
            &mut rows,
            columns,
            projection,
            matching_indices(values, |candidate| candidate == expected),
        ),
        (
            NativeColumn::DenseLiveEntityIds {
                entity_type,
                ids,
                values,
            },
            kernel_semantics::BoundPrimitivePredicate::LiveEntityId {
                entity_type: expected_type,
                id: expected,
            },
        ) if entity_type == expected_type => {
            if let Some(expected) = ids.local(*expected) {
                push_typed_matches(
                    &mut rows,
                    columns,
                    projection,
                    matching_indices(values, |candidate| *candidate == expected),
                );
            }
        }
        (
            NativeColumn::HistoricalEntityIds {
                entity_type,
                values,
            },
            kernel_semantics::BoundPrimitivePredicate::HistoricalEntityId {
                entity_type: expected_type,
                id: expected,
            },
        ) if entity_type == expected_type => push_typed_matches(
            &mut rows,
            columns,
            projection,
            matching_indices(values, |candidate| candidate == expected),
        ),
        _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
    }
    Ok(rows)
}

fn matching_indices<'a, T: 'a>(
    values: &'a [T],
    predicate: impl Fn(&T) -> bool + 'a,
) -> impl Iterator<Item = usize> + 'a {
    values
        .iter()
        .enumerate()
        .filter_map(move |(index, value)| predicate(value).then_some(index))
}

fn execute_bound_i64_filter(
    columns: &[NativeColumn],
    predicate: &[i64],
    expected: i64,
    projection: &[usize],
) -> Vec<kernel_query::Row> {
    let mut rows = Vec::new();
    if let [projected_column] = projection
        && let NativeColumn::I64(projected) = &columns[*projected_column]
    {
        for (row_index, candidate) in predicate.iter().enumerate() {
            if *candidate == expected {
                rows.push(vec![Value::I64(projected[row_index])]);
            }
        }
    } else {
        push_typed_matches(
            &mut rows,
            columns,
            projection,
            matching_indices(predicate, |candidate| *candidate == expected),
        );
    }
    rows
}

fn push_typed_matches(
    rows: &mut Vec<kernel_query::Row>,
    columns: &[NativeColumn],
    projection: &[usize],
    indices: impl IntoIterator<Item = usize>,
) {
    for row_index in indices {
        push_typed_projection(rows, columns, projection, row_index);
    }
}

fn push_typed_projection(
    rows: &mut Vec<kernel_query::Row>,
    columns: &[NativeColumn],
    projection: &[usize],
    row_index: usize,
) {
    rows.push(
        projection
            .iter()
            .map(|column| columns[*column].value_at(row_index))
            .collect(),
    );
}

#[allow(clippy::too_many_arguments)]
fn execute_i64_fused(
    columns: &[PersistentPhysicalVec<i64>],
    row_count: usize,
    relation: SemanticId,
    predicate_column: usize,
    projection: &[usize],
    bound: Option<&kernel_semantics::BoundPrimitivePredicate>,
    context: &kernel_schema::SemanticContext,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    validate_i64_columnar_schema(context, relation, columns.len())?;
    let Some(kernel_semantics::BoundPrimitivePredicate::I64(expected)) = bound else {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    };
    let predicate_values = columns
        .get(predicate_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?
        .as_slice();
    validate_projection(projection, columns.len())?;
    stats.scanned_rows = stats.scanned_rows.saturating_add(row_count);
    let mut rows = Vec::new();
    if let [projected_column] = projection {
        let projected_values = columns[*projected_column].as_slice();
        for row_index in 0..row_count {
            if predicate_values[row_index] == *expected {
                rows.push(vec![Value::I64(projected_values[row_index])]);
            }
        }
    } else {
        for row_index in 0..row_count {
            if predicate_values[row_index] == *expected {
                rows.push(
                    projection
                        .iter()
                        .map(|column| Value::I64(columns[*column][row_index]))
                        .collect(),
                );
            }
        }
    }
    account_fused_reads(stats, row_count, rows.len(), projection.len());
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn execute_value_columnar_fused(
    columns: &[PersistentPhysicalVec<Value>],
    row_count: usize,
    predicate_column: usize,
    predicate_value: &Value,
    equivalence: SemanticId,
    projection: &[usize],
    bound: Option<&kernel_semantics::BoundPrimitivePredicate>,
    resolved: Option<&kernel_semantics::ResolvedPrimitiveEquivalence>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let predicate_values = columns
        .get(predicate_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?
        .as_slice();
    validate_projection(projection, columns.len())?;
    stats.scanned_rows = stats.scanned_rows.saturating_add(row_count);
    let mut rows = Vec::new();
    for (row_index, candidate) in predicate_values.iter().take(row_count).enumerate() {
        if matches_bound_or_resolved(
            bound,
            resolved,
            registry,
            context,
            equivalence,
            candidate,
            predicate_value,
        )? {
            rows.push(
                projection
                    .iter()
                    .map(|column| columns[*column].as_slice()[row_index].clone())
                    .collect(),
            );
        }
    }
    account_fused_reads(stats, row_count, rows.len(), projection.len());
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn execute_i64_generic_fused(
    columns: &[PersistentPhysicalVec<i64>],
    row_count: usize,
    relation: SemanticId,
    predicate_column: usize,
    predicate_value: &Value,
    equivalence: SemanticId,
    projection: &[usize],
    bound: Option<&kernel_semantics::BoundPrimitivePredicate>,
    resolved: Option<&kernel_semantics::ResolvedPrimitiveEquivalence>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    validate_i64_columnar_schema(context, relation, columns.len())?;
    let predicate_values = columns
        .get(predicate_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?
        .as_slice();
    validate_projection(projection, columns.len())?;
    stats.scanned_rows = stats.scanned_rows.saturating_add(row_count);
    let mut rows = Vec::new();
    for (row_index, value) in predicate_values.iter().take(row_count).enumerate() {
        let candidate = Value::I64(*value);
        if matches_bound_or_resolved(
            bound,
            resolved,
            registry,
            context,
            equivalence,
            &candidate,
            predicate_value,
        )? {
            rows.push(
                projection
                    .iter()
                    .map(|column| Value::I64(columns[*column].as_slice()[row_index]))
                    .collect(),
            );
        }
    }
    account_fused_reads(stats, row_count, rows.len(), projection.len());
    Ok(rows)
}

#[allow(clippy::too_many_arguments)]
fn execute_row_store_fused(
    rows: &PersistentPhysicalVec<kernel_query::Row>,
    predicate_column: usize,
    predicate_value: &Value,
    equivalence: SemanticId,
    projection: &[usize],
    bound: Option<&kernel_semantics::BoundPrimitivePredicate>,
    resolved: Option<&kernel_semantics::ResolvedPrimitiveEquivalence>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    stats.scanned_rows = stats.scanned_rows.saturating_add(rows.len());
    let mut out = Vec::new();
    for source in rows {
        let candidate = source
            .get(predicate_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if matches_bound_or_resolved(
            bound,
            resolved,
            registry,
            context,
            equivalence,
            candidate,
            predicate_value,
        )? {
            out.push(
                projection
                    .iter()
                    .map(|column| {
                        source
                            .get(*column)
                            .cloned()
                            .ok_or(RelQueryError::ColumnOutOfBounds)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
    }
    account_fused_reads(stats, rows.len(), out.len(), projection.len());
    Ok(out)
}

fn validate_projection(
    projection: &[usize],
    column_count: usize,
) -> Result<(), PhysicalExecutionError> {
    if projection.iter().any(|column| *column >= column_count) {
        return Err(RelQueryError::ColumnOutOfBounds.into());
    }
    Ok(())
}

fn account_fused_reads(
    stats: &mut ExecutionStats,
    row_count: usize,
    output_rows: usize,
    projection_width: usize,
) {
    stats.values_read = stats
        .values_read
        .saturating_add(row_count.saturating_add(output_rows.saturating_mul(projection_width)));
}

fn matches_bound_or_resolved(
    bound: Option<&kernel_semantics::BoundPrimitivePredicate>,
    resolved: Option<&kernel_semantics::ResolvedPrimitiveEquivalence>,
    registry: &kernel_semantics::SemanticRegistry,
    context: &kernel_schema::SemanticContext,
    equivalence: SemanticId,
    left: &Value,
    right: &Value,
) -> Result<bool, PhysicalExecutionError> {
    if let Some(bound) = bound {
        return Ok(bound.matches(left));
    }
    match resolved {
        Some(resolved) => Ok(resolved.equivalent(left, right)?),
        None => Ok(registry.equivalent(context, equivalence, left, right)?),
    }
}

 fn filter_rows(
    rows: Vec<kernel_query::Row>,
    column: usize,
    value: &Value,
    equivalence: SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let resolved = registry.resolve_primitive_equivalence(context, equivalence)?;
    let bound = resolved
        .as_ref()
        .map(|resolved| resolved.bind_right(value))
        .transpose()?;
    let mut out = Vec::new();
    for row in rows {
        let candidate = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
        stats.values_read = stats.values_read.saturating_add(1);
        if matches_bound_or_resolved(
            bound.as_ref(),
            resolved.as_ref(),
            registry,
            context,
            equivalence,
            candidate,
            value,
        )? {
            out.push(row);
        }
    }
    Ok(out)
}

pub(super) fn filter_rows_columns(
    rows: Vec<kernel_query::Row>,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let resolved = registry.resolve_primitive_equivalence(context, equivalence)?;
    let mut out = Vec::new();
    for row in rows {
        let left = row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let right = row
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        stats.values_read = stats.values_read.saturating_add(2);
        let matched = if let Some(resolved) = &resolved {
            resolved.equivalent(left, right)?
        } else {
            registry.equivalent(context, equivalence, left, right)?
        };
        if matched {
            out.push(row);
        }
    }
    Ok(out)
}

// HOSTILE[P165][ACTIVE][FALLBACK][CLEAN-ASYMPTOTIC]: linear projection fallback; typed producer chains preserve columns/positions when available.
fn project_rows(
    rows: Vec<kernel_query::Row>,
    columns: &[usize],
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut projected = Vec::with_capacity(columns.len());
        for &column in columns {
            projected.push(
                row.get(column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?
                    .clone(),
            );
            stats.values_read = stats.values_read.saturating_add(1);
        }
        out.push(projected);
    }
    Ok(out)
}

// HOSTILE[P161][ACTIVE][GENERIC][CLEAN-ASYMPTOTIC]: canonical set, no quadratic equality replay.
 fn distinct_rows(
    rows: Vec<kernel_query::Row>,
    equivalences: &[SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let mut seen = BTreeSet::<Vec<kernel_semantics::CanonicalEqKey>>::new();
    let mut unique = Vec::with_capacity(rows.len());
    for row in rows {
        let key = canonical_semantic_row_key(&row, equivalences, context, registry)?;
        if seen.insert(key) {
            unique.push(row);
        }
    }
    Ok(unique)
}

// HOSTILE[P161][ACTIVE][GENERIC][CLEAN-ASYMPTOTIC]: Γ-canonical bucket fallback, not nested loop.
