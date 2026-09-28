use super::{
    Change, RelDifferentialProgram, RelExpr, RelQueryError, RelationDelta,
    relation_column_equivalences, rows_as_multisets_equivalent, unmatched_semantic_rows,
};

pub fn relation_deltas_semantically_equivalent(
    left: &RelationDelta,
    right: &RelationDelta,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    if left.result_type != right.result_type {
        return Ok(false);
    }
    let column_equivalences = relation_column_equivalences(&left.result_type);
    Ok(rows_as_multisets_equivalent(
        &left.inserted,
        &right.inserted,
        column_equivalences,
        context,
        registry,
    )? && rows_as_multisets_equivalent(
        &left.removed,
        &right.removed,
        column_equivalences,
        context,
        registry,
    )?)
}

pub fn rel_delta_by_recompute(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let prepared = query.prepare(context, registry)?;
    let old_output = prepared.evaluate(old, context, registry)?;
    let next = change.apply(old);
    let new_output = prepared.evaluate(&next, context, registry)?;
    let result_type = prepared.result_type().clone();
    let column_equivalences = match &result_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            new_output.rows(),
            old_output.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            old_output.rows(),
            new_output.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

pub fn rel_delta_optimized(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Option<RelationDelta>, RelQueryError> {
    let program = RelDifferentialProgram::compile(query, context, registry)?;
    program.apply(old, change, context, registry).map(Some)
}
