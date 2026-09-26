// HOSTILE[P184][ACTIVE][CLEAN]: generic row fallbacks are execution behavior; Plan only
// orchestrates them after typed/persisted fast paths decline.
pub(super) fn execute_distinct_plan(
    input: &Plan,
    column_equivalences: &[SemanticId],
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let rows = input.execute_native_rows(store, context, registry, stats)?;
    distinct_rows(rows, column_equivalences, context, registry)
}

// HOSTILE[P165][ACTIVE][FALLBACK][CLEAN-ASYMPTOTIC]: linear Γ-aware row filter; typed/native and persisted-index routes are attempted first.
pub(super) fn execute_filter_const(
    input: &Plan,
    column: usize,
    value: &Value,
    equivalence: SemanticId,
    store: &PhysicalStore,
    semantics: (
        &kernel_schema::SemanticContext,
        &kernel_semantics::SemanticRegistry,
    ),
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let (context, registry) = semantics;
    let input_rows = input.execute_native_rows(store, context, registry, stats)?;
    filter_rows(
        input_rows,
        column,
        value,
        equivalence,
        context,
        registry,
        stats,
    )
}

// HOSTILE[P164][ACTIVE][FALLBACK][CLEAN-ASYMPTOTIC]: row fallback is linear; typed storage
// uses canonical native-column comparison without reconstructing the input relation.
pub(super) fn execute_filter_columns(
    input: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    store: &PhysicalStore,
    semantics: (
        &kernel_schema::SemanticContext,
        &kernel_semantics::SemanticRegistry,
    ),
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let (context, registry) = semantics;
    let input_rows = input.execute_native_rows(store, context, registry, stats)?;
    filter_rows_columns(
        input_rows,
        left_column,
        right_column,
        equivalence,
        context,
        registry,
        stats,
    )
}

// HOSTILE[P164][ACTIVE][FALLBACK][CLEAN-ASYMPTOTIC]: Γ-canonical row fallback; typed storage uses the native monus producer.
pub(super) fn execute_difference_plan(
    left: &Plan,
    right: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let left_rows = left.execute_native_rows(store, context, registry, stats)?;
    let right_rows = right.execute_native_rows(store, context, registry, stats)?;
    let left_type = left.to_logical_expr().typecheck(context, registry)?;
    let right_type = right.to_logical_expr().typecheck(context, registry)?;
    if left_type != right_type {
        return Err(RelQueryError::TypeMismatch.into());
    }
    let column_equivalences = match &left_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    let left_value = relation_value_from_rows(left_rows, &left_type, context, registry)?;
    let right_value = relation_value_from_rows(right_rows, &right_type, context, registry)?;
    Ok(kernel_query::difference_relation_values(
        left_value,
        right_value,
        column_equivalences,
        context,
        registry,
    )?
    .into_rows())
}

