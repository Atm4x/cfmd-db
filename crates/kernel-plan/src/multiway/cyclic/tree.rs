pub(super) fn try_execute_multiway_join(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Option<Vec<kernel_query::Row>>, PhysicalExecutionError> {
    if !matches!(plan, Plan::JoinEq { .. } | Plan::FilterEqColumns { .. }) {
        return Ok(None);
    }
    if let Some(rows) =
        try_execute_nway_order_preserving_join(plan, None, store, context, registry, stats)?
    {
        stats.multiway_join_reorders = stats.multiway_join_reorders.saturating_add(1);
        stats.multiway_join_order_preserving_enumerations = stats
            .multiway_join_order_preserving_enumerations
            .saturating_add(1);
        return Ok(Some(rows));
    }
    let Some(optimized) = optimize_contiguous_multiway_join(plan, store, context, registry)? else {
        return Ok(None);
    };
    stats.multiway_join_reorders = stats.multiway_join_reorders.saturating_add(1);
    execute_multiway_join_candidate(&optimized, store, context, registry, stats).map(Some)
}

pub(super) fn flatten_multiway_join_tree(
    plan: &Plan,
    store: &PhysicalStore,
    leaves: &mut Vec<MultiwayJoinLeaf>,
    predicates: &mut Vec<MultiwayJoinPredicate>,
) -> Result<Option<Vec<MultiwayJoinColumnRef>>, PhysicalExecutionError> {
    match plan {
        Plan::Scan { relation, layout } => {
            let installed = store.installed(*relation, *layout)?;
            let width = native_column_count(&installed.data);
            let leaf = leaves.len();
            leaves.push(MultiwayJoinLeaf {
                plan: plan.clone(),
                relation: *relation,
                layout: *layout,
                width,
                rows: native_row_count(&installed.data),
            });
            Ok(Some(
                (0..width)
                    .map(|column| MultiwayJoinColumnRef { leaf, column })
                    .collect(),
            ))
        }
        Plan::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
            ..
        } => {
            let Some(mut left_columns) =
                flatten_multiway_join_tree(left, store, leaves, predicates)?
            else {
                return Ok(None);
            };
            let Some(right_columns) = flatten_multiway_join_tree(right, store, leaves, predicates)?
            else {
                return Ok(None);
            };
            let Some(left_ref) = left_columns.get(*left_column).copied() else {
                return Err(RelQueryError::ColumnOutOfBounds.into());
            };
            let Some(right_ref) = right_columns.get(*right_column).copied() else {
                return Err(RelQueryError::ColumnOutOfBounds.into());
            };
            predicates.push(MultiwayJoinPredicate {
                left: left_ref,
                right: right_ref,
                equivalence: *equivalence,
            });
            left_columns.extend(right_columns);
            Ok(Some(left_columns))
        }
        Plan::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => {
            let Some(columns) = flatten_multiway_join_tree(input, store, leaves, predicates)?
            else {
                return Ok(None);
            };
            let Some(left_ref) = columns.get(*left_column).copied() else {
                return Err(RelQueryError::ColumnOutOfBounds.into());
            };
            let Some(right_ref) = columns.get(*right_column).copied() else {
                return Err(RelQueryError::ColumnOutOfBounds.into());
            };
            if left_ref.leaf == right_ref.leaf {
                return Ok(None);
            }
            predicates.push(MultiwayJoinPredicate {
                left: left_ref,
                right: right_ref,
                equivalence: *equivalence,
            });
            Ok(Some(columns))
        }
        Plan::Project { input, columns } => {
            let Some(input_columns) = flatten_multiway_join_tree(input, store, leaves, predicates)?
            else {
                return Ok(None);
            };
            if columns.len() != input_columns.len()
                || columns.iter().copied().ne(0..input_columns.len())
            {
                return Ok(None);
            }
            Ok(Some(input_columns))
        }
        Plan::PromoteToBag(input) => flatten_multiway_join_tree(input, store, leaves, predicates),
        _ => Ok(None),
    }
}

fn execute_multiway_join_candidate(
    plan: &Plan,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    match plan {
        Plan::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => {
            let input_rows =
                execute_multiway_join_candidate(input, store, context, registry, stats)?;
            filter_rows_columns(
                input_rows,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
                stats,
            )
        }
        Plan::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => execute_join_plan(
            left,
            right,
            *left_column,
            *right_column,
            *equivalence,
            store,
            context,
            registry,
            stats,
        ),
        _ => plan.execute_native_rows(store, context, registry, stats),
    }
}

