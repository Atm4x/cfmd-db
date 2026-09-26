// HOSTILE[P173][TEST-ONLY][CLEAN]: execution unit tests use wrappers; production helpers stay private.
// HOSTILE[P177][TEST-ONLY][CLEAN]: batch-index and TopK representation types are hidden behind
// behavior-oriented test probes instead of leaking into the root test module.
use super::{
    ExecutionStats, JoinBatchIndex, NativeColumn, StatefulBatchEnv, TopKBatchSpec,
    build_indexed_join_batch_program, execute_bound_typed_filter,
    execute_fused_filter_project_scan, merge_execution_stats, select_join_batch_index,
    select_top_k_i64_positions, select_top_k_semantic_positions, typed_column_matches_bound,
};
use crate::{LayoutBinding, PhysicalExecutionError, PhysicalStore, Plan};
use kernel_model::Value;
use kernel_query::{OrderDirection, Row};
use kernel_types::SemanticId;

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_fused_filter_project_scan_for_test(
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
) -> Result<Vec<Row>, PhysicalExecutionError> {
    execute_fused_filter_project_scan(
        store,
        relation,
        layout,
        predicate_column,
        predicate_value,
        equivalence,
        projection,
        context,
        registry,
        stats,
    )
}

pub(crate) fn execute_bound_typed_filter_for_test(
    columns: &[NativeColumn],
    predicate: &NativeColumn,
    bound: &kernel_semantics::BoundPrimitivePredicate,
    projection: &[usize],
    row_count: usize,
) -> Result<Vec<Row>, PhysicalExecutionError> {
    execute_bound_typed_filter(columns, predicate, bound, projection, row_count)
}

pub(crate) fn merge_execution_stats_for_test(target: &mut ExecutionStats, source: ExecutionStats) {
    merge_execution_stats(target, source);
}

pub(crate) fn select_top_k_i64_positions_for_test(
    positions: &mut Vec<usize>,
    values: &[i64],
    direction: OrderDirection,
    k: usize,
    stats: &mut ExecutionStats,
) {
    select_top_k_i64_positions(positions, values, direction, k, stats);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn select_top_k_semantic_positions_for_test(
    positions: &mut Vec<usize>,
    order_column: &NativeColumn,
    column: usize,
    ordering: SemanticId,
    direction: OrderDirection,
    k: usize,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<(), PhysicalExecutionError> {
    select_top_k_semantic_positions(
        positions,
        order_column,
        TopKBatchSpec {
            column,
            ordering,
            direction,
            k,
        },
        &StatefulBatchEnv { context, registry },
        stats,
    )
}

pub(crate) fn join_batch_selection_is_ephemeral_for_test(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<bool, PhysicalExecutionError> {
    let Some(program) = build_indexed_join_batch_program(plan, store, context, registry)? else {
        return Ok(false);
    };
    Ok(matches!(
        select_join_batch_index(&program, store, context, registry, stats)?,
        Some(JoinBatchIndex::Ephemeral(_))
    ))
}

pub(crate) fn typed_column_matches_bound_for_test(
    column: &NativeColumn,
    index: usize,
    bound: &kernel_semantics::BoundPrimitivePredicate,
) -> Result<bool, PhysicalExecutionError> {
    typed_column_matches_bound(column, index, bound)
}
