use super::*;
use crate::join_access::test_support::{JoinAccessObservation, observe_join_access};

pub(crate) fn execute_order_preserving_quotient_join_for_test(
    plan: &Plan,
    prepared_program: Option<&PreparedSemanticQuotientProgram>,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    let mut leaves = Vec::new();
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_tree(plan, store, &mut leaves, &mut predicates)? else {
        return Ok(None);
    };
    let search_order = prepared_program
        .and_then(|program| program.hypergraph_order.clone())
        .unwrap_or_else(|| (0..leaves.len()).collect());
    execute_order_preserving_quotient_join(
        QuotientJoinExecutionRequest {
            leaves: &leaves,
            predicates: &predicates,
            search_order: &search_order,
            prepared_program,
            store,
            context,
            registry,
        },
        stats,
    )
}

pub(crate) fn estimated_quotient_build_work_for_test(
    plan: &Plan,
    prepared_program: Option<&PreparedSemanticQuotientProgram>,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<u128, PhysicalExecutionError> {
    let mut leaves = Vec::new();
    let mut predicates = Vec::new();
    let Some(_) = flatten_multiway_join_tree(plan, store, &mut leaves, &mut predicates)? else {
        return Ok(0);
    };
    estimated_quotient_build_work(
        store,
        &leaves,
        &predicates,
        prepared_program,
        context,
        registry,
    )
}

pub(crate) fn optimize_contiguous_multiway_join_for_test(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<Plan>, PhysicalExecutionError> {
    optimize_contiguous_multiway_join(plan, store, context, registry)
}

pub(crate) fn execute_multiway_join_candidate_for_test(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    execute_multiway_join_candidate(plan, store, context, registry, stats)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn multiway_right_access_estimate_for_test(
    store: &PhysicalStore,
    relation: SemanticId,
    layout: LayoutBinding,
    right_rows: usize,
    right_column: usize,
    equivalence: SemanticId,
    left_rows: usize,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<JoinAccessObservation, PhysicalExecutionError> {
    let leaf = MultiwayJoinLeaf {
        plan: Plan::Scan { relation, layout },
        relation,
        layout,
        width: right_column.saturating_add(1),
        rows: right_rows,
    };
    multiway_right_access_estimate(
        store,
        &leaf,
        MultiwayJoinPredicate {
            left: MultiwayJoinColumnRef { leaf: 0, column: 0 },
            right: MultiwayJoinColumnRef {
                leaf: 1,
                column: right_column,
            },
            equivalence,
        },
        left_rows,
        context,
        registry,
    )
    .map(observe_join_access)
}
