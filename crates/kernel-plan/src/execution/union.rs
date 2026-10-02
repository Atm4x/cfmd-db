pub(super) fn execute_union_plan(
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
    Ok(kernel_query::union_relation_values(
        left_value,
        right_value,
        column_equivalences,
        context,
        registry,
    )?
    .into_rows())
}
