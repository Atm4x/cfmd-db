#[derive(Debug)]
struct TypedBatchProgram<'a> {
    installed: &'a InstalledRelation,
    columns: Vec<usize>,
    predicates: Vec<TypedBatchPredicate>,
}

#[derive(Debug)]
struct TypedBatchPredicate {
    physical_column: usize,
    kind: TypedBatchPredicateKind,
}

#[derive(Debug)]
enum TypedBatchPredicateKind {
    Primitive(kernel_semantics::BoundPrimitivePredicate),
    Algebraic {
        compiled: kernel_semantics::CompiledEquivalence,
        expected: kernel_semantics::CanonicalEqKey,
    },
}

fn merge_execution_stats(target: &mut ExecutionStats, source: ExecutionStats) {
    target.scanned_rows = target.scanned_rows.saturating_add(source.scanned_rows);
    target.values_read = target.values_read.saturating_add(source.values_read);
    target.persisted_index_hits = target
        .persisted_index_hits
        .saturating_add(source.persisted_index_hits);
    target.persisted_index_cost_rejections = target
        .persisted_index_cost_rejections
        .saturating_add(source.persisted_index_cost_rejections);
    target.ephemeral_index_builds = target
        .ephemeral_index_builds
        .saturating_add(source.ephemeral_index_builds);
    target.fused_join_project_hits = target
        .fused_join_project_hits
        .saturating_add(source.fused_join_project_hits);
    target.typed_batch_chain_hits = target
        .typed_batch_chain_hits
        .saturating_add(source.typed_batch_chain_hits);
    target.typed_stateful_batch_hits = target
        .typed_stateful_batch_hits
        .saturating_add(source.typed_stateful_batch_hits);
    target.typed_stateful_producer_hits = target
        .typed_stateful_producer_hits
        .saturating_add(source.typed_stateful_producer_hits);
    target.multiway_join_reorders = target
        .multiway_join_reorders
        .saturating_add(source.multiway_join_reorders);
    target.multiway_join_order_preserving_enumerations = target
        .multiway_join_order_preserving_enumerations
        .saturating_add(source.multiway_join_order_preserving_enumerations);
    target.multiway_join_semantic_quotient_constraints = target
        .multiway_join_semantic_quotient_constraints
        .saturating_add(source.multiway_join_semantic_quotient_constraints);
    target.multiway_join_semantic_quotient_pruned_rows = target
        .multiway_join_semantic_quotient_pruned_rows
        .saturating_add(source.multiway_join_semantic_quotient_pruned_rows);
    target.multiway_join_prepared_quotient_hits = target
        .multiway_join_prepared_quotient_hits
        .saturating_add(source.multiway_join_prepared_quotient_hits);
    target.multiway_join_maintained_quotient_key_hits = target
        .multiway_join_maintained_quotient_key_hits
        .saturating_add(source.multiway_join_maintained_quotient_key_hits);
    target.multiway_join_maintained_quotient_support_hits = target
        .multiway_join_maintained_quotient_support_hits
        .saturating_add(source.multiway_join_maintained_quotient_support_hits);
    target.multiway_join_cyclic_budget_rejections = target
        .multiway_join_cyclic_budget_rejections
        .saturating_add(source.multiway_join_cyclic_budget_rejections);
    target.multiway_join_cyclic_prefix_index_lookups = target
        .multiway_join_cyclic_prefix_index_lookups
        .saturating_add(source.multiway_join_cyclic_prefix_index_lookups);
    target.multiway_join_semantic_quotient_candidate_visits = target
        .multiway_join_semantic_quotient_candidate_visits
        .saturating_add(source.multiway_join_semantic_quotient_candidate_visits);
}

struct TypedBatchSelection<'a> {
    program: TypedBatchProgram<'a>,
    positions: Vec<usize>,
    stats: ExecutionStats,
}

fn execute_typed_batch_selection<'a>(
    plan: &Plan,
    store: &'a PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<TypedBatchSelection<'a>>, PhysicalExecutionError> {
    let Some(program) = build_typed_batch_program(plan, store, context, registry)? else {
        return Ok(None);
    };
    let NativeRelation::TypedColumnar { columns, .. } = &program.installed.data else {
        return Ok(None);
    };
    let mut stats = ExecutionStats::default();
    let positions = if program.predicates.is_empty() {
        if program.installed.scan_order_is_physical {
            (0..native_row_count(&program.installed.data)).collect::<Vec<_>>()
        } else {
            program.installed.scan_positions().collect::<Vec<_>>()
        }
    } else {
        let source: Box<dyn Iterator<Item = usize> + '_> =
            if program.installed.scan_order_is_physical {
                Box::new(0..native_row_count(&program.installed.data))
            } else {
                program.installed.scan_positions()
            };
        let mut selected = Vec::new();
        for position in source {
            let mut matched = true;
            for predicate in &program.predicates {
                stats.values_read = stats.values_read.saturating_add(1);
                if !typed_batch_predicate_matches(
                    columns
                        .get(predicate.physical_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?,
                    position,
                    predicate,
                )? {
                    matched = false;
                    break;
                }
            }
            if matched {
                selected.push(position);
            }
        }
        selected
    };
    stats.scanned_rows = native_row_count(&program.installed.data);
    stats.typed_batch_chain_hits = stats.typed_batch_chain_hits.saturating_add(1);
    Ok(Some(TypedBatchSelection {
        program,
        positions,
        stats,
    }))
}

fn materialize_typed_batch_row(
    program: &TypedBatchProgram<'_>,
    columns: &[NativeColumn],
    position: usize,
    stats: &mut ExecutionStats,
) -> Result<kernel_query::Row, PhysicalExecutionError> {
    let mut row = Vec::with_capacity(program.columns.len());
    for &physical_column in &program.columns {
        row.push(
            columns
                .get(physical_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
                .value_at(position),
        );
    }
    stats.values_read = stats.values_read.saturating_add(program.columns.len());
    Ok(row)
}

fn equivalence_is_i64_exact(
    equivalence: SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, PhysicalExecutionError> {
    let Some(resolved) = registry.resolve_primitive_equivalence(context, equivalence)? else {
        return Ok(false);
    };
    Ok(matches!(
        resolved.bind_right(&Value::I64(0)),
        Ok(kernel_semantics::BoundPrimitivePredicate::I64(0))
    ))
}

fn try_execute_typed_batch_chain(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<(Vec<kernel_query::Row>, ExecutionStats)>, PhysicalExecutionError> {
    let Some(program) = build_typed_batch_program(plan, store, context, registry)? else {
        return Ok(None);
    };
    let NativeRelation::TypedColumnar { columns, .. } = &program.installed.data else {
        return Ok(None);
    };
    if let Some(result) = try_execute_i64_batch_program(&program, columns) {
        return Ok(Some(result));
    }
    let mut stats = ExecutionStats::default();
    let mut rows = Vec::new();
    if program.installed.scan_order_is_physical {
        for position in 0..native_row_count(&program.installed.data) {
            execute_typed_batch_position(position, &program, columns, &mut rows, &mut stats)?;
        }
    } else {
        for position in program.installed.scan_positions() {
            execute_typed_batch_position(position, &program, columns, &mut rows, &mut stats)?;
        }
    }
    stats.typed_batch_chain_hits = stats.typed_batch_chain_hits.saturating_add(1);
    Ok(Some((rows, stats)))
}

#[inline]
fn execute_typed_batch_position(
    position: usize,
    program: &TypedBatchProgram<'_>,
    columns: &[NativeColumn],
    rows: &mut Vec<kernel_query::Row>,
    stats: &mut ExecutionStats,
) -> Result<(), PhysicalExecutionError> {
    stats.scanned_rows = stats.scanned_rows.saturating_add(1);
    for predicate in &program.predicates {
        stats.values_read = stats.values_read.saturating_add(1);
        if !typed_batch_predicate_matches(
            columns
                .get(predicate.physical_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?,
            position,
            predicate,
        )? {
            return Ok(());
        }
    }
    let mut row = Vec::with_capacity(program.columns.len());
    for &column in &program.columns {
        row.push(
            columns
                .get(column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?
                .value_at(position),
        );
    }
    stats.values_read = stats.values_read.saturating_add(program.columns.len());
    rows.push(row);
    Ok(())
}

fn try_execute_i64_batch_program(
    program: &TypedBatchProgram<'_>,
    columns: &[NativeColumn],
) -> Option<(Vec<kernel_query::Row>, ExecutionStats)> {
    let mut projected = Vec::with_capacity(program.columns.len());
    for &column in &program.columns {
        let Some(NativeColumn::I64(values)) = columns.get(column) else {
            return None;
        };
        projected.push(NativeI64ColumnView::Persistent(values));
    }
    let mut predicates = Vec::with_capacity(program.predicates.len());
    for predicate in &program.predicates {
        let Some(NativeColumn::I64(values)) = columns.get(predicate.physical_column) else {
            return None;
        };
        let TypedBatchPredicateKind::Primitive(kernel_semantics::BoundPrimitivePredicate::I64(
            expected,
        )) = &predicate.kind
        else {
            return None;
        };
        predicates.push((NativeI64ColumnView::Persistent(values), *expected));
    }

    let mut stats = ExecutionStats::default();
    let mut rows = Vec::new();
    if program.installed.scan_order_is_physical {
        execute_i64_batch_positions(
            0..native_row_count(&program.installed.data),
            &predicates,
            &projected,
            &mut rows,
            &mut stats,
        );
    } else {
        execute_i64_batch_positions(
            program.installed.scan_positions(),
            &predicates,
            &projected,
            &mut rows,
            &mut stats,
        );
    }
    stats.typed_batch_chain_hits = stats.typed_batch_chain_hits.saturating_add(1);
    Some((rows, stats))
}

fn execute_i64_batch_positions(
    positions: impl IntoIterator<Item = usize>,
    predicates: &[(NativeI64ColumnView<'_>, i64)],
    projected: &[NativeI64ColumnView<'_>],
    rows: &mut Vec<kernel_query::Row>,
    stats: &mut ExecutionStats,
) {
    match predicates {
        [] => {
            for position in positions {
                stats.scanned_rows = stats.scanned_rows.saturating_add(1);
                push_i64_batch_projection(projected, position, rows, stats);
            }
        }
        [(first, expected)] => {
            for position in positions {
                stats.scanned_rows = stats.scanned_rows.saturating_add(1);
                stats.values_read = stats.values_read.saturating_add(1);
                if first.value(position) == *expected {
                    push_i64_batch_projection(projected, position, rows, stats);
                }
            }
        }
        [(first, first_expected), (second, second_expected)] => {
            for position in positions {
                stats.scanned_rows = stats.scanned_rows.saturating_add(1);
                stats.values_read = stats.values_read.saturating_add(1);
                if first.value(position) != *first_expected {
                    continue;
                }
                stats.values_read = stats.values_read.saturating_add(1);
                if second.value(position) == *second_expected {
                    push_i64_batch_projection(projected, position, rows, stats);
                }
            }
        }
        _ => {
            for position in positions {
                stats.scanned_rows = stats.scanned_rows.saturating_add(1);
                let mut matched = true;
                for (values, expected) in predicates {
                    stats.values_read = stats.values_read.saturating_add(1);
                    if values.value(position) != *expected {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    push_i64_batch_projection(projected, position, rows, stats);
                }
            }
        }
    }
}

fn push_i64_batch_projection(
    projected: &[NativeI64ColumnView<'_>],
    position: usize,
    rows: &mut Vec<kernel_query::Row>,
    stats: &mut ExecutionStats,
) {
    if let [values] = projected {
        rows.push(vec![Value::I64(values.value(position))]);
    } else {
        rows.push(
            projected
                .iter()
                .map(|values| Value::I64(values.value(position)))
                .collect(),
        );
    }
    stats.values_read = stats.values_read.saturating_add(projected.len());
}

fn build_typed_batch_program<'a>(
    plan: &Plan,
    store: &'a PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<TypedBatchProgram<'a>>, PhysicalExecutionError> {
    match plan {
        Plan::Scan { relation, layout } => {
            let installed = store.installed(*relation, *layout)?;
            let NativeRelation::TypedColumnar { columns, .. } = &installed.data else {
                return Ok(None);
            };
            validate_typed_columnar_schema(context, *relation, columns)?;
            Ok(Some(TypedBatchProgram {
                installed,
                columns: (0..columns.len()).collect(),
                predicates: Vec::new(),
            }))
        }
        Plan::Project { input, columns } => {
            let Some(mut program) = build_typed_batch_program(input, store, context, registry)?
            else {
                return Ok(None);
            };
            let mut projected = Vec::with_capacity(columns.len());
            for &column in columns {
                projected.push(
                    *program
                        .columns
                        .get(column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?,
                );
            }
            program.columns = projected;
            Ok(Some(program))
        }
        Plan::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => {
            let Some(mut program) = build_typed_batch_program(input, store, context, registry)?
            else {
                return Ok(None);
            };
            let Some(&physical_column) = program.columns.get(*column) else {
                return Err(RelQueryError::ColumnOutOfBounds.into());
            };
            let NativeRelation::TypedColumnar { columns, .. } = &program.installed.data else {
                return Ok(None);
            };
            let predicate = columns
                .get(physical_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let kind = if let Some(resolved) =
                registry.resolve_primitive_equivalence(context, *equivalence)?
            {
                let bound = resolved.bind_right(value)?;
                // Reject a physical/schema mismatch before entering the scan loop.
                if let Some(position) = program.installed.scan_positions().next() {
                    let _ = typed_column_matches_bound(predicate, position, &bound)?;
                } else if !typed_column_bound_compatible(predicate, &bound) {
                    return Err(PhysicalExecutionError::PhysicalTypeMismatch);
                }
                TypedBatchPredicateKind::Primitive(bound)
            } else if let NativeColumn::Algebraic(column) = predicate
                && context
                    .schema
                    .structural_equivalence(*equivalence)
                    .is_some()
            {
                let compiled = registry.compile_equivalence(context, *equivalence)?;
                let expected = compiled.canonical_key(value)?;
                if let Some(position) = program.installed.scan_positions().next() {
                    let _ = column.canonical_key_at_compiled(position, &compiled)?;
                }
                TypedBatchPredicateKind::Algebraic { compiled, expected }
            } else {
                return Ok(None);
            };
            program.predicates.push(TypedBatchPredicate {
                physical_column,
                kind,
            });
            Ok(Some(program))
        }
        Plan::PromoteToBag(input) => build_typed_batch_program(input, store, context, registry),
        Plan::FilterEqColumns { .. }
        | Plan::JoinEq { .. }
        | Plan::Difference { .. }
        | Plan::AntiJoin { .. }
        | Plan::Distinct { .. }
        | Plan::Group { .. }
        | Plan::TopKWithTies { .. } => Ok(None),
    }
}

fn typed_column_bound_compatible(
    column: &NativeColumn,
    bound: &kernel_semantics::BoundPrimitivePredicate,
) -> bool {
    matches!(
        (column, bound),
        (
            NativeColumn::Unit(_),
            kernel_semantics::BoundPrimitivePredicate::Unit
        ) | (
            NativeColumn::Bool(_),
            kernel_semantics::BoundPrimitivePredicate::Bool(_)
        ) | (
            NativeColumn::I64(_),
            kernel_semantics::BoundPrimitivePredicate::I64(_)
        ) | (
            NativeColumn::F64Bits(_),
            kernel_semantics::BoundPrimitivePredicate::F64Bits(_)
        ) | (
            NativeColumn::Text(_),
            kernel_semantics::BoundPrimitivePredicate::TextExact(_)
                | kernel_semantics::BoundPrimitivePredicate::TextAsciiCaseInsensitive(_)
        )
    ) || matches!(
        (column, bound),
        (
            NativeColumn::LiveEntityIds { entity_type, .. },
            kernel_semantics::BoundPrimitivePredicate::LiveEntityId {
                entity_type: expected_type,
                ..
            }
        ) if entity_type == expected_type
    ) || matches!(
        (column, bound),
        (
            NativeColumn::DenseLiveEntityIds { entity_type, .. },
            kernel_semantics::BoundPrimitivePredicate::LiveEntityId {
                entity_type: expected_type,
                ..
            }
        ) if entity_type == expected_type
    ) || matches!(
        (column, bound),
        (
            NativeColumn::HistoricalEntityIds { entity_type, .. },
            kernel_semantics::BoundPrimitivePredicate::HistoricalEntityId {
                entity_type: expected_type,
                ..
            }
        ) if entity_type == expected_type
    )
}

fn typed_column_matches_bound(
    column: &NativeColumn,
    index: usize,
    bound: &kernel_semantics::BoundPrimitivePredicate,
) -> Result<bool, PhysicalExecutionError> {
    if !typed_column_bound_compatible(column, bound) {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    let matched = match (column, bound) {
        (NativeColumn::Unit(_), kernel_semantics::BoundPrimitivePredicate::Unit) => true,
        (NativeColumn::Bool(values), kernel_semantics::BoundPrimitivePredicate::Bool(expected)) => {
            values.get(index).is_some_and(|value| value == expected)
        }
        (NativeColumn::I64(values), kernel_semantics::BoundPrimitivePredicate::I64(expected)) => {
            values.get(index).is_some_and(|value| value == expected)
        }
        (
            NativeColumn::F64Bits(values),
            kernel_semantics::BoundPrimitivePredicate::F64Bits(expected),
        ) => values.get(index).is_some_and(|value| value == expected),
        (
            NativeColumn::Text(values),
            kernel_semantics::BoundPrimitivePredicate::TextExact(expected),
        ) => values.get(index).is_some_and(|value| value == expected),
        (
            NativeColumn::Text(values),
            kernel_semantics::BoundPrimitivePredicate::TextAsciiCaseInsensitive(expected),
        ) => values
            .get(index)
            .is_some_and(|value| value.eq_ignore_ascii_case(expected)),
        (
            NativeColumn::LiveEntityIds { values, .. },
            kernel_semantics::BoundPrimitivePredicate::LiveEntityId { id: expected, .. },
        ) => values.get(index).is_some_and(|value| value == expected),
        (
            NativeColumn::DenseLiveEntityIds { ids, values, .. },
            kernel_semantics::BoundPrimitivePredicate::LiveEntityId { id: expected, .. },
        ) => ids
            .local(*expected)
            .is_some_and(|expected| values.get(index).is_some_and(|value| *value == expected)),
        (
            NativeColumn::HistoricalEntityIds { values, .. },
            kernel_semantics::BoundPrimitivePredicate::HistoricalEntityId { id: expected, .. },
        ) => values.get(index).is_some_and(|value| value == expected),
        _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
    };
    Ok(matched)
}

fn typed_batch_predicate_matches(
    column: &NativeColumn,
    index: usize,
    predicate: &TypedBatchPredicate,
) -> Result<bool, PhysicalExecutionError> {
    match &predicate.kind {
        TypedBatchPredicateKind::Primitive(bound) => {
            typed_column_matches_bound(column, index, bound)
        }
        TypedBatchPredicateKind::Algebraic { compiled, expected } => {
            let NativeColumn::Algebraic(column) = column else {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            };
            Ok(column.canonical_key_at_compiled(index, compiled)? == *expected)
        }
    }
}

