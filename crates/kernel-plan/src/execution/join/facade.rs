// HOSTILE[P180][ACTIVE][CLEAN]: physical access-family dispatch belongs to execution.
// Multiway planning consumes access-cost facts but does not select executor implementations.
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_join_plan(
    left: &Plan,
    right: &Plan,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    store: &PhysicalStore,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    stats: &mut ExecutionStats,
) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
    let key = JoinKeySpec {
        left_column,
        right_column,
        equivalence,
    };
    let transparent_left = transparent_direct_scan_relation(left, store);
    let transparent_right = transparent_direct_scan_relation(right, store);
    if transparent_left.is_none() && transparent_right.is_some() {
        let left_rows = left.execute_native_rows(store, context, registry, stats)?;
        if let Some(rows) = try_execute_indexed_right_scan_join_from_rows(
            &left_rows, right, key, store, context, registry, stats,
        )? {
            return Ok(rows);
        }
        let right_rows = right.execute_native_rows(store, context, registry, stats)?;
        return join_rows(
            &left_rows,
            &right_rows,
            left_column,
            right_column,
            equivalence,
            context,
            registry,
        );
    }
    if let Some(decision) = direct_join_access_decision(left, right, key, store, context, registry)?
    {
        let rows = match decision.family {
            JoinAccessFamily::FullScan => None,
            JoinAccessFamily::PersistedI64 => try_execute_indexed_i64_join(
                left,
                right,
                left_column,
                right_column,
                equivalence,
                store,
                context,
                registry,
                stats,
                false,
            )?,
            JoinAccessFamily::PersistedSemantic => try_execute_persisted_semantic_join(
                left,
                right,
                left_column,
                right_column,
                equivalence,
                store,
                context,
                registry,
                stats,
            )?,
            JoinAccessFamily::EphemeralI64 => try_execute_indexed_i64_join(
                left,
                right,
                left_column,
                right_column,
                equivalence,
                store,
                context,
                registry,
                stats,
                true,
            )?,
        };
        if let Some(rows) = rows {
            return Ok(rows);
        }
    }
    let left_rows = left.execute_native_rows(store, context, registry, stats)?;
    let right_rows = right.execute_native_rows(store, context, registry, stats)?;
    join_rows(
        &left_rows,
        &right_rows,
        left_column,
        right_column,
        equivalence,
        context,
        registry,
    )
}

