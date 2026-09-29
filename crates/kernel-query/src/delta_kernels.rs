use super::{
    Change, MaterializedSetSupportState, OrderComparison, RelExpr, RelQueryError, RelType,
    RelationDelta, Row, Value, project_rows, query_types_compatible, relation_column_equivalence,
    relation_column_equivalences, unmatched_semantic_rows, validate_query_equivalence,
    value_shape_matches_type,
};

pub(super) fn rel_delta_scan(
    relation: kernel_types::SemanticId,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let prepared = RelExpr::Scan(relation).prepare(context, registry)?;
    let result_type = prepared.result_type().clone();
    let column_equivalences = relation_column_equivalences(&result_type);
    let old_rows = old
        .relations
        .get(&relation)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let next = match change {
        Change::NoChange => old,
        Change::Replace(next) => next,
        Change::Fine(fine) => fine.endpoint(),
    };
    let next_rows = next
        .relations
        .get(&relation)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            next_rows,
            old_rows,
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            old_rows,
            next_rows,
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

pub(super) fn rel_delta_filter(
    input_delta: RelationDelta,
    column: usize,
    value: &Value,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let column_type = input_delta
        .result_type
        .columns
        .get(column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    validate_query_equivalence(equivalence, column_type, context, registry)?;
    let input_equivalence = relation_column_equivalence(&input_delta.result_type, column)?;
    if !registry.equivalence_refines(context, input_equivalence, equivalence)? {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }
    if !value_shape_matches_type(value, column_type) {
        return Err(RelQueryError::TypeMismatch);
    }

    let filter_rows = |rows: Vec<Row>| -> Result<Vec<Row>, RelQueryError> {
        rows.into_iter()
            .filter_map(|row| {
                let Some(candidate) = row.get(column) else {
                    return Some(Err(RelQueryError::ColumnOutOfBounds));
                };
                match registry.equivalent(context, equivalence, candidate, value) {
                    Ok(true) => Some(Ok(row)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error.into())),
                }
            })
            .collect()
    };

    Ok(RelationDelta {
        inserted: filter_rows(input_delta.inserted)?,
        removed: filter_rows(input_delta.removed)?,
        result_type: input_delta.result_type,
    })
}

pub(super) fn rel_delta_filter_order_const(
    input_delta: RelationDelta,
    column: usize,
    value: &Value,
    ordering: kernel_types::SemanticId,
    comparison: OrderComparison,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let column_type = input_delta
        .result_type
        .columns
        .get(column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let ordering_domain = registry.ordering_domain(context, ordering)?;
    let expected = kernel_semantics::domain_for_type(column_type)
        .map(kernel_semantics::OrderingDomain::from)
        .ok_or(RelQueryError::TypeMismatch)?;
    if ordering_domain != expected {
        return Err(RelQueryError::TypeMismatch);
    }
    let equivalence = relation_column_equivalence(&input_delta.result_type, column)?;
    if !registry.ordering_congruent_with_equivalence(context, ordering, equivalence)? {
        return Err(RelQueryError::OrderingNotCongruentWithEquality);
    }
    if !value_shape_matches_type(value, column_type) {
        return Err(RelQueryError::TypeMismatch);
    }

    let filter_rows = |rows: Vec<Row>| -> Result<Vec<Row>, RelQueryError> {
        rows.into_iter()
            .filter_map(|row| {
                let Some(candidate) = row.get(column) else {
                    return Some(Err(RelQueryError::ColumnOutOfBounds));
                };
                match registry.compare(context, ordering, candidate, value) {
                    Ok(order) => {
                        let passes = match comparison {
                            OrderComparison::Less => order.is_lt(),
                            OrderComparison::LessOrEqual => order.is_le(),
                            OrderComparison::Greater => order.is_gt(),
                            OrderComparison::GreaterOrEqual => order.is_ge(),
                        };
                        passes.then_some(Ok(row))
                    }
                    Err(error) => Some(Err(error.into())),
                }
            })
            .collect()
    };

    Ok(RelationDelta {
        inserted: filter_rows(input_delta.inserted)?,
        removed: filter_rows(input_delta.removed)?,
        result_type: input_delta.result_type,
    })
}

pub(super) fn rel_delta_filter_columns(
    input_delta: RelationDelta,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let left_type = input_delta
        .result_type
        .columns
        .get(left_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let right_type = input_delta
        .result_type
        .columns
        .get(right_column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    if !query_types_compatible(left_type, right_type, &context.schema) {
        return Err(RelQueryError::TypeMismatch);
    }
    validate_query_equivalence(equivalence, left_type, context, registry)?;
    validate_query_equivalence(equivalence, right_type, context, registry)?;
    let left_input_equivalence =
        relation_column_equivalence(&input_delta.result_type, left_column)?;
    let right_input_equivalence =
        relation_column_equivalence(&input_delta.result_type, right_column)?;
    if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
        || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
    {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }

    let filter_rows = |rows: Vec<Row>| -> Result<Vec<Row>, RelQueryError> {
        rows.into_iter()
            .filter_map(|row| {
                let Some(left) = row.get(left_column) else {
                    return Some(Err(RelQueryError::ColumnOutOfBounds));
                };
                let Some(right) = row.get(right_column) else {
                    return Some(Err(RelQueryError::ColumnOutOfBounds));
                };
                match registry.equivalent(context, equivalence, left, right) {
                    Ok(true) => Some(Ok(row)),
                    Ok(false) => None,
                    Err(error) => Some(Err(error.into())),
                }
            })
            .collect()
    };

    Ok(RelationDelta {
        inserted: filter_rows(input_delta.inserted)?,
        removed: filter_rows(input_delta.removed)?,
        result_type: input_delta.result_type,
    })
}

pub(super) fn rel_delta_project_bag(
    input_delta: RelationDelta,
    columns: &[usize],
    query: &RelExpr,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let result_type = query.typecheck(context, registry)?;
    let inserted = project_rows(input_delta.inserted, columns)?;
    let removed = project_rows(input_delta.removed, columns)?;
    let column_equivalences = relation_column_equivalences(&result_type);
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            &inserted,
            &removed,
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            &removed,
            &inserted,
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

pub(super) fn rel_delta_project_set(
    input: &RelExpr,
    input_delta: RelationDelta,
    columns: &[usize],
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let result_type = query.typecheck(context, registry)?;
    let old_input = input.evaluate(old, context, registry)?.into_rows();
    let old_rows = project_rows(old_input, columns)?;
    let inserted = project_rows(input_delta.inserted, columns)?;
    let removed = project_rows(input_delta.removed, columns)?;
    set_output_delta_from_supports(&old_rows, inserted, removed, result_type, context, registry)
}

pub(super) fn rel_delta_distinct(
    input: &RelExpr,
    input_delta: RelationDelta,
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let result_type = query.typecheck(context, registry)?;
    let old_rows = input.evaluate(old, context, registry)?.into_rows();
    set_output_delta_from_supports(
        &old_rows,
        input_delta.inserted,
        input_delta.removed,
        result_type,
        context,
        registry,
    )
}

fn set_output_delta_from_supports(
    old_rows: &[Row],
    inserted_rows: Vec<Row>,
    removed_rows: Vec<Row>,
    result_type: RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let mut state = MaterializedSetSupportState::build(old_rows, result_type, context, registry)?;
    state.apply_rows_delta(inserted_rows, removed_rows, context, registry)
}
