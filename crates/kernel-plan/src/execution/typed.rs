// HOSTILE[P164][ACTIVE][FALLBACK][CLEAN-ASYMPTOTIC]: Γ-canonical row fallback; typed storage uses the native blocker producer.
pub(super) fn execute_anti_join_plan(
    left: &Plan,
    right: &Plan,
    columns: (usize, usize),
    equivalence: SemanticId,
    store: &PhysicalStore,
    semantics: (
        &kernel_schema::SemanticContext,
        &kernel_semantics::SemanticRegistry,
    ),
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let (left_column, right_column) = columns;
    let (context, registry) = semantics;
    let left_rows = left.execute_native_rows(store, context, registry, stats)?;
    let right_rows = right.execute_native_rows(store, context, registry, stats)?;
    let left_type = left
        .to_logical_expr()
        .typecheck(context, registry)?;
    let right_type = right
        .to_logical_expr()
        .typecheck(context, registry)?;
    let left_value = relation_value_from_rows(left_rows, &left_type, context, registry)?;
    let right_value = relation_value_from_rows(right_rows, &right_type, context, registry)?;
    Ok(kernel_query::anti_join_relation_values(
        left_value,
        &right_value,
        left_column,
        right_column,
        equivalence,
        context,
        registry,
    )?
    .into_rows())
}

pub(super) fn try_execute_native_fast_path(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    if let Some(rows) =
        try_execute_persisted_semantic_multi_key_join_plan(plan, store, context, registry, stats)?
    {
        return Ok(Some(rows));
    }
    if let Some(rows) =
        try_execute_persisted_semantic_filter_plan(plan, store, context, registry, stats)?
    {
        return Ok(Some(rows));
    }
    if let Some((rows, batch_stats)) =
        try_execute_typed_stateful_producer_chain(plan, store, context, registry)?
    {
        merge_execution_stats(stats, batch_stats);
        return Ok(Some(rows));
    }
    if let Some((rows, batch_stats)) =
        try_execute_typed_stateful_batch(plan, store, context, registry)?
    {
        merge_execution_stats(stats, batch_stats);
        return Ok(Some(rows));
    }
    if let Some((rows, batch_stats)) =
        try_execute_indexed_join_batch_chain(plan, store, context, registry)?
    {
        merge_execution_stats(stats, batch_stats);
        return Ok(Some(rows));
    }
    if let Some((rows, batch_stats)) =
        try_execute_typed_batch_chain(plan, store, context, registry)?
    {
        merge_execution_stats(stats, batch_stats);
        return Ok(Some(rows));
    }
    Ok(None)
}

// HOSTILE[P181][ACTIVE][CLEAN]: indexed Join-batch implementation is physically isolated while remaining in the execution module scope.
include!("typed/join_batch.rs");

// HOSTILE[P181][ACTIVE][CLEAN]: ordinary typed-batch selection/materialization stays one private execution owner.
include!("typed/batch_core.rs");

// HOSTILE[P181][ACTIVE][CLEAN]: stateful typed producer pipeline is physically isolated without widening its Rust visibility.
include!("typed/stateful_producer.rs");

// HOSTILE[P181][ACTIVE][CLEAN]: stateful Distinct/Group/TopK batch execution is isolated from ordinary typed batch evaluation.
include!("typed/stateful_batch.rs");

