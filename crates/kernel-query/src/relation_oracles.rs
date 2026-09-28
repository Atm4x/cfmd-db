use super::{
    BTreeMap, CanonicalRowKey, Change, ExactQuery, Impact, QueryResult, RelExpr, RelQueryError,
    RelQueryResult, RelType, RelationValue, Row, Value, canonical_row_key,
};
#[cfg(test)]
use crate::{
    Expr, QueryError, derivative_by_recompute, derivative_seq_splice, impact_by_recompute,
};
#[cfg(test)]
use kernel_change::SeqSplice;

#[must_use]
pub fn rel_derivative_by_recompute(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Change<RelQueryResult> {
    let next = change.apply(old);
    let new_output = query.evaluate(&next, context, registry);
    if rel_impact_by_recompute(query, old, change, context, registry) == Impact::Unaffected {
        Change::NoChange
    } else {
        Change::Replace(new_output)
    }
}

#[must_use]
pub fn rel_impact_by_recompute(
    query: &RelExpr,
    old: &kernel_model::FiniteModel,
    change: &Change<kernel_model::FiniteModel>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Impact {
    if matches!(change, Change::NoChange) {
        return Impact::Unaffected;
    }
    let Ok(prepared) = query.prepare(context, registry) else {
        return Impact::Unknown;
    };
    let old_output = prepared.evaluate(old, context, registry);
    let next = change.apply(old);
    let new_output = prepared.evaluate(&next, context, registry);
    match (&old_output, &new_output) {
        (Ok(left), Ok(right)) => match relation_values_semantically_equivalent(
            left,
            right,
            prepared.result_type(),
            context,
            registry,
        ) {
            Ok(true) => Impact::Unaffected,
            Ok(false) => Impact::Changed,
            Err(_) => Impact::Unknown,
        },
        (Err(left), Err(right)) if left == right => Impact::Unaffected,
        (Err(_) | Ok(_), Err(_)) | (Err(_), Ok(_)) => Impact::Changed,
    }
}

pub(super) fn relation_values_semantically_equivalent(
    left: &RelationValue,
    right: &RelationValue,
    relation_type: &RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    let expected_set = matches!(
        relation_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    if expected_set != matches!(left, RelationValue::Set { .. })
        || expected_set != matches!(right, RelationValue::Set { .. })
    {
        return Ok(false);
    }
    let column_equivalences = match &relation_type.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    };
    rows_as_multisets_equivalent(
        left.rows(),
        right.rows(),
        column_equivalences,
        context,
        registry,
    )
}

pub(super) fn rows_as_multisets_equivalent(
    left: &[Row],
    right: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    if left.len() != right.len() {
        return Ok(false);
    }
    Ok(
        canonical_row_multiset_counts(left, column_equivalences, context, registry)?
            == canonical_row_multiset_counts(right, column_equivalences, context, registry)?,
    )
}

#[cfg(test)]
pub(super) fn rows_as_multisets_equivalent_by_matching(
    left: &[Row],
    right: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    let mut matched = vec![false; right.len()];
    for left_row in left {
        if left_row.len() != column_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        let mut found = None;
        for (index, right_row) in right.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if rows_semantically_equal(left_row, right_row, column_equivalences, context, registry)?
            {
                found = Some(index);
                break;
            }
        }
        let Some(index) = found else {
            return Ok(false);
        };
        matched[index] = true;
    }
    Ok(true)
}

pub(super) fn canonical_row_multiset_counts(
    rows: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<BTreeMap<CanonicalRowKey, usize>, RelQueryError> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let key = canonical_row_key(row, column_equivalences, context, registry)?;
        *counts.entry(key).or_insert(0) += 1;
    }
    Ok(counts)
}

pub(super) fn rows_semantically_equal(
    left: &Row,
    right: &Row,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<bool, RelQueryError> {
    if left.len() != column_equivalences.len() || right.len() != column_equivalences.len() {
        return Err(RelQueryError::EquivalenceArityMismatch);
    }
    for ((left_value, right_value), equivalence) in left.iter().zip(right).zip(column_equivalences)
    {
        if !registry.equivalent(context, *equivalence, left_value, right_value)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn unmatched_semantic_rows(
    source: &[Row],
    target: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<Row>, RelQueryError> {
    let mut target_counts =
        canonical_row_multiset_counts(target, column_equivalences, context, registry)?;
    let mut unmatched = Vec::new();
    for source_row in source {
        let key = canonical_row_key(source_row, column_equivalences, context, registry)?;
        let supported = target_counts.get_mut(&key).is_some_and(|count| {
            if *count == 0 {
                false
            } else {
                *count -= 1;
                true
            }
        });
        if !supported {
            unmatched.push(source_row.clone());
        }
    }
    Ok(unmatched)
}

#[cfg(test)]
pub(super) fn unmatched_semantic_rows_by_matching(
    source: &[Row],
    target: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<Row>, RelQueryError> {
    let mut matched = vec![false; target.len()];
    let mut unmatched = Vec::new();
    for source_row in source {
        if source_row.len() != column_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        let mut found = None;
        for (index, target_row) in target.iter().enumerate() {
            if matched[index] {
                continue;
            }
            if rows_semantically_equal(
                source_row,
                target_row,
                column_equivalences,
                context,
                registry,
            )? {
                found = Some(index);
                break;
            }
        }
        if let Some(index) = found {
            matched[index] = true;
        } else {
            unmatched.push(source_row.clone());
        }
    }
    Ok(unmatched)
}

#[must_use]
pub fn check_derivative_law(
    query: &ExactQuery,
    old: &Value,
    input_change: &Change<Value>,
    output_change: &Change<QueryResult>,
) -> bool {
    let old_output = query.evaluate(old);
    let via_delta = output_change.apply(&old_output);
    let next_input = input_change.apply(old);
    let from_scratch = query.evaluate(&next_input);
    via_delta == from_scratch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(values: &[i64]) -> Value {
        Value::Seq(values.iter().copied().map(Value::I64).collect())
    }

    #[test]
    fn universal_derivative_satisfies_from_scratch_law() {
        let query = ExactQuery::new(Expr::SeqSumI64(Box::new(Expr::Input)));
        let old = seq(&[1, 2, 3]);
        let change = Change::Replace(seq(&[1, 2, 3, 4]));
        let output_change = derivative_by_recompute(&query, &old, &change);
        assert!(check_derivative_law(&query, &old, &change, &output_change));
    }

    #[test]
    fn impact_can_certify_unchanged_result_without_host_callback() {
        let query = ExactQuery::new(Expr::SeqLength(Box::new(Expr::Input)));
        let old = seq(&[1, 2]);
        let change = Change::Replace(seq(&[7, 8]));
        assert_eq!(
            impact_by_recompute(&query, &old, &change),
            Impact::Unaffected
        );
    }

    #[test]
    fn logical_errors_are_deterministic_values_of_query_semantics() {
        let query = ExactQuery::new(Expr::SeqSumI64(Box::new(Expr::Input)));
        let bad = Value::Seq(vec![Value::Text("not an integer".into())]);
        assert_eq!(query.evaluate(&bad), Err(QueryError::TypeMismatch));
    }

    #[test]
    fn optimized_sequence_length_delta_matches_from_scratch() {
        let query = ExactQuery::new(Expr::SeqLength(Box::new(Expr::Input)));
        let old = seq(&[1, 2, 3, 4]);
        let splice = SeqSplice {
            start: 1,
            delete_count: 1,
            insert: vec![Value::I64(8), Value::I64(9), Value::I64(10)],
        };
        let fine = derivative_seq_splice(&query, &old, &splice)
            .unwrap()
            .expect("supported fine rule");
        let Value::Seq(old_values) = &old else {
            unreachable!();
        };
        let next = Value::Seq(splice.apply(old_values).unwrap());
        let universal = derivative_by_recompute(&query, &old, &Change::Replace(next.clone()));
        assert_eq!(fine, universal);
        assert!(check_derivative_law(
            &query,
            &old,
            &Change::Replace(next),
            &fine
        ));
    }

    #[test]
    fn integer_overflow_is_not_plan_dependent_wraparound() {
        let query = ExactQuery::new(Expr::AddI64(
            Box::new(Expr::Const(Value::I64(i64::MAX))),
            Box::new(Expr::Const(Value::I64(1))),
        ));
        assert_eq!(
            query.evaluate(&Value::Unit),
            Err(QueryError::ArithmeticOverflow)
        );
    }
}
