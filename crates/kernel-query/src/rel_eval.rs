use super::{
    AggregateSpec, BTreeMap, BTreeSet, CanonicalRowKey, OrderComparison, OrderDirection,
    PreparedRelExpr, RelExpr, RelQueryError, RelType, RelationOccurrenceCertificate,
    RelationScanOccurrenceSeed, RelationValue, Row, Value, anti_join_relation_values,
    canonical_row_key, difference_relation_values, distinct_rows,
    distinct_rows_with_canonical_keys, group_relation_value, project_rows, query_types_compatible,
    relation_column_equivalence, relation_column_equivalences, union_relation_values,
    validate_query_equivalence, value_shape_matches_type,
};
use crate::relation_state::CanonicalRowEvidence;

pub(super) fn collect_rel_source_relations(
    query: &RelExpr,
    out: &mut BTreeSet<kernel_types::SemanticId>,
) {
    match query {
        RelExpr::Scan(relation) => {
            out.insert(*relation);
        }
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterOrderConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::Project { input, .. }
        | RelExpr::Distinct { input, .. }
        | RelExpr::Group { input, .. }
        | RelExpr::TopKWithTies { input, .. }
        | RelExpr::PromoteToBag(input) => collect_rel_source_relations(input, out),
        RelExpr::JoinEq { left, right, .. }
        | RelExpr::Difference { left, right }
        | RelExpr::Union { left, right }
        | RelExpr::AntiJoin { left, right, .. } => {
            collect_rel_source_relations(left, out);
            collect_rel_source_relations(right, out);
        }
    }
}

fn collect_rel_orderings(query: &RelExpr, out: &mut BTreeSet<kernel_types::SemanticId>) {
    match query {
        RelExpr::FilterOrderConst {
            input, ordering, ..
        }
        | RelExpr::TopKWithTies {
            input, ordering, ..
        } => {
            out.insert(*ordering);
            collect_rel_orderings(input, out);
        }
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::Project { input, .. }
        | RelExpr::Distinct { input, .. }
        | RelExpr::Group { input, .. }
        | RelExpr::PromoteToBag(input) => collect_rel_orderings(input, out),
        RelExpr::JoinEq { left, right, .. }
        | RelExpr::Difference { left, right }
        | RelExpr::Union { left, right }
        | RelExpr::AntiJoin { left, right, .. } => {
            collect_rel_orderings(left, out);
            collect_rel_orderings(right, out);
        }
        RelExpr::Scan(_) => {}
    }
}

struct RelEvalContext<'a> {
    model: &'a kernel_model::FiniteModel,
    semantic: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
    compiled_orderings: &'a BTreeMap<kernel_types::SemanticId, kernel_semantics::CompiledOrdering>,
    scan_seeds: Option<&'a BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>>,
}

pub(super) fn evaluate_prepared_expr(
    expr: &RelExpr,
    model: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    compiled_orderings: &BTreeMap<kernel_types::SemanticId, kernel_semantics::CompiledOrdering>,
) -> Result<RelationValue, RelQueryError> {
    let eval = RelEvalContext {
        model,
        semantic,
        registry,
        compiled_orderings,
        scan_seeds: None,
    };
    expr.evaluate_unchecked(&eval)
}

pub(super) fn evaluate_prepared_expr_with_occurrence_certificate(
    expr: &RelExpr,
    model: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    compiled_orderings: &BTreeMap<kernel_types::SemanticId, kernel_semantics::CompiledOrdering>,
) -> Result<(RelationValue, RelationOccurrenceCertificate), RelQueryError> {
    let eval = RelEvalContext {
        model,
        semantic,
        registry,
        compiled_orderings,
        scan_seeds: None,
    };
    let (value, result_type, canonical_keys_by_row) =
        evaluate_expr_with_canonical_keys(expr, &eval)?;
    let certificate = RelationOccurrenceCertificate::from_dense_set_keys(
        result_type,
        semantic,
        canonical_keys_by_row,
    )?;
    Ok((value, certificate))
}

pub(super) fn evaluate_prepared_expr_with_occurrence_certificate_seeded(
    expr: &RelExpr,
    model: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    compiled_orderings: &BTreeMap<kernel_types::SemanticId, kernel_semantics::CompiledOrdering>,
    scan_seeds: &BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>,
) -> Result<(RelationValue, RelationOccurrenceCertificate), RelQueryError> {
    let eval = RelEvalContext {
        model,
        semantic,
        registry,
        compiled_orderings,
        scan_seeds: Some(scan_seeds),
    };
    let (value, result_type, canonical_keys_by_row) =
        evaluate_expr_with_canonical_keys(expr, &eval)?;
    let certificate = RelationOccurrenceCertificate::from_dense_set_keys(
        result_type,
        semantic,
        canonical_keys_by_row,
    )?;
    Ok((value, certificate))
}

pub(super) fn evaluate_prepared_expr_seeded(
    expr: &RelExpr,
    model: &kernel_model::FiniteModel,
    semantic: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
    compiled_orderings: &BTreeMap<kernel_types::SemanticId, kernel_semantics::CompiledOrdering>,
    scan_seeds: &BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>,
) -> Result<RelationValue, RelQueryError> {
    let eval = RelEvalContext {
        model,
        semantic,
        registry,
        compiled_orderings,
        scan_seeds: Some(scan_seeds),
    };
    evaluate_expr_with_canonical_keys(expr, &eval).map(|(value, _, _)| value)
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn evaluate_expr_with_canonical_keys(
    expr: &RelExpr,
    eval: &RelEvalContext<'_>,
) -> Result<(RelationValue, RelType, CanonicalRowEvidence), RelQueryError> {
    let result_type = expr.typecheck(eval.semantic, eval.registry)?;
    if !matches!(
        result_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    ) {
        return Err(RelQueryError::CanonicalObservationUnavailable);
    }

    let (value, canonical_keys_by_row) = match expr {
        RelExpr::Scan(relation) => {
            let seed = eval
                .scan_seeds
                .and_then(|seeds| seeds.get(relation))
                .ok_or(RelQueryError::CanonicalObservationUnavailable)?;
            if seed.relation() != *relation
                || seed.result_type() != &result_type
                || seed.semantic_context() != eval.semantic
            {
                return Err(RelQueryError::StructuralRewriteBaseMismatch);
            }
            let value = RelExpr::eval_scan(*relation, eval)?;
            if value.rows().len() != seed.row_count() {
                return Err(RelQueryError::StructuralRewriteBaseMismatch);
            }
            (value, seed.canonical_keys_by_row())
        }
        RelExpr::Union { left, right } => {
            let equivalences = relation_column_equivalences(&result_type).to_vec();
            match (
                evaluate_expr_with_canonical_keys(left, eval),
                evaluate_expr_with_canonical_keys(right, eval),
            ) {
                (Ok((left_value, _, left_keys)), Ok((right_value, _, right_keys))) => {
                    union_relation_values_from_canonical_keys(
                        left_value,
                        left_keys,
                        right_value,
                        right_keys,
                        &equivalences,
                    )?
                }
                (Err(RelQueryError::CanonicalObservationUnavailable), _)
                | (_, Err(RelQueryError::CanonicalObservationUnavailable)) => {
                    let left_value = left.evaluate_unchecked(eval)?;
                    let right_value = right.evaluate_unchecked(eval)?;
                    union_relation_values_with_canonical_keys(
                        left_value,
                        right_value,
                        &equivalences,
                        eval.semantic,
                        eval.registry,
                    )?
                }
                (Err(error), _) | (_, Err(error)) => return Err(error),
            }
        }
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            let (left_value, _left_type, left_keys) =
                evaluate_expr_with_canonical_keys(left, eval)?;
            let (right_value, _right_type, right_keys) =
                evaluate_expr_with_canonical_keys(right, eval)?;
            join_relation_values_with_canonical_keys(
                left_value,
                left_keys,
                right_value,
                right_keys,
                *left_column,
                *right_column,
                *equivalence,
                eval.semantic,
                eval.registry,
            )?
        }
        RelExpr::Difference { left, right } => {
            let equivalences = relation_column_equivalences(&result_type).to_vec();
            match (
                evaluate_expr_with_canonical_keys(left, eval),
                evaluate_expr_with_canonical_keys(right, eval),
            ) {
                (Ok((left_value, _, left_keys)), Ok((right_value, _, right_keys))) => {
                    difference_relation_values_from_canonical_keys(
                        left_value,
                        left_keys,
                        right_value,
                        right_keys,
                        &equivalences,
                    )?
                }
                (Err(RelQueryError::CanonicalObservationUnavailable), _)
                | (_, Err(RelQueryError::CanonicalObservationUnavailable)) => {
                    let left_value = left.evaluate_unchecked(eval)?;
                    let right_value = right.evaluate_unchecked(eval)?;
                    difference_relation_values_with_canonical_keys(
                        left_value,
                        right_value,
                        &equivalences,
                        eval.semantic,
                        eval.registry,
                    )?
                }
                (Err(error), _) | (_, Err(error)) => return Err(error),
            }
        }
        RelExpr::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            let (left_value, left_type, left_keys) = evaluate_expr_with_canonical_keys(left, eval)?;
            if left_type != result_type {
                return Err(RelQueryError::TypeMismatch);
            }
            let right_value = right.evaluate_unchecked(eval)?;
            anti_join_relation_values_with_canonical_keys(
                left_value,
                left_keys,
                &right_value,
                *left_column,
                *right_column,
                *equivalence,
                eval.semantic,
                eval.registry,
            )?
        }
        RelExpr::Distinct {
            input,
            column_equivalences,
        } => match evaluate_expr_with_canonical_keys(input, eval) {
            Ok((
                RelationValue::Set {
                    rows,
                    column_equivalences: input_equivalences,
                },
                _,
                keys,
            )) if input_equivalences == *column_equivalences => (
                RelationValue::Set {
                    rows,
                    column_equivalences: input_equivalences,
                },
                keys,
            ),
            Ok(_) | Err(RelQueryError::CanonicalObservationUnavailable) => {
                let rows = input.evaluate_unchecked(eval)?.into_rows();
                let (rows, canonical_keys) = distinct_rows_with_canonical_keys(
                    rows,
                    column_equivalences,
                    eval.semantic,
                    eval.registry,
                )?;
                (
                    RelationValue::Set {
                        rows,
                        column_equivalences: column_equivalences.clone(),
                    },
                    CanonicalRowEvidence::from_dense(canonical_keys),
                )
            }
            Err(error) => return Err(error),
        },
        RelExpr::Project { input, columns } => {
            let equivalences = relation_column_equivalences(&result_type).to_vec();
            match evaluate_expr_with_canonical_keys(input, eval) {
                Ok((RelationValue::Set { rows, .. }, _, input_keys)) => {
                    project_relation_value_from_canonical_keys(
                        rows,
                        input_keys,
                        columns,
                        equivalences,
                    )?
                }
                Ok(_) | Err(RelQueryError::CanonicalObservationUnavailable) => {
                    let input_value = input.evaluate_unchecked(eval)?;
                    let RelationValue::Set { rows, .. } = input_value else {
                        return Err(RelQueryError::CanonicalObservationUnavailable);
                    };
                    let rows = project_rows(rows, columns)?;
                    let (rows, canonical_keys) = distinct_rows_with_canonical_keys(
                        rows,
                        &equivalences,
                        eval.semantic,
                        eval.registry,
                    )?;
                    (
                        RelationValue::Set {
                            rows,
                            column_equivalences: equivalences,
                        },
                        CanonicalRowEvidence::from_dense(canonical_keys),
                    )
                }
                Err(error) => return Err(error),
            }
        }
        _ => return Err(RelQueryError::CanonicalObservationUnavailable),
    };

    if canonical_keys_by_row.len() != value.rows().len() {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }
    Ok((value, result_type, canonical_keys_by_row))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Preserve the existing value-taking boundary contract."
)]
fn union_relation_values_from_canonical_keys(
    left: RelationValue,
    left_keys_by_row: CanonicalRowEvidence,
    right: RelationValue,
    right_keys_by_row: CanonicalRowEvidence,
    column_equivalences: &[kernel_types::SemanticId],
) -> Result<(RelationValue, CanonicalRowEvidence), RelQueryError> {
    let (
        RelationValue::Set {
            rows: left_rows,
            column_equivalences: left_equivalences,
        },
        RelationValue::Set {
            rows: right_rows,
            column_equivalences: right_equivalences,
        },
    ) = (left, right)
    else {
        return Err(RelQueryError::CanonicalObservationUnavailable);
    };
    if left_equivalences != column_equivalences
        || right_equivalences != column_equivalences
        || left_rows.len() != left_keys_by_row.len()
        || right_rows.len() != right_keys_by_row.len()
    {
        return Err(RelQueryError::TypeMismatch);
    }
    let mut keyed_rows = Vec::with_capacity(left_rows.len() + right_rows.len());
    keyed_rows.extend(
        left_rows
            .into_iter()
            .zip(left_keys_by_row.iter().cloned())
            .map(|(row, key)| (key, row)),
    );
    keyed_rows.extend(
        right_rows
            .into_iter()
            .zip(right_keys_by_row.iter().cloned())
            .map(|(row, key)| (key, row)),
    );
    keyed_rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    keyed_rows.dedup_by(|left, right| left.0 == right.0);
    let mut rows = Vec::with_capacity(keyed_rows.len());
    let mut keys = Vec::with_capacity(keyed_rows.len());
    for (key, row) in keyed_rows {
        keys.push(key);
        rows.push(row);
    }
    Ok((
        RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.to_vec(),
        },
        CanonicalRowEvidence::from_dense(keys),
    ))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Preserve the existing value-taking boundary contract."
)]
fn difference_relation_values_from_canonical_keys(
    left: RelationValue,
    left_keys_by_row: CanonicalRowEvidence,
    right: RelationValue,
    right_keys_by_row: CanonicalRowEvidence,
    column_equivalences: &[kernel_types::SemanticId],
) -> Result<(RelationValue, CanonicalRowEvidence), RelQueryError> {
    let (
        RelationValue::Set {
            rows: left_rows,
            column_equivalences: left_equivalences,
        },
        RelationValue::Set {
            rows: right_rows,
            column_equivalences: right_equivalences,
        },
    ) = (left, right)
    else {
        return Err(RelQueryError::CanonicalObservationUnavailable);
    };
    if left_equivalences != column_equivalences
        || right_equivalences != column_equivalences
        || left_rows.len() != left_keys_by_row.len()
        || right_rows.len() != right_keys_by_row.len()
    {
        return Err(RelQueryError::TypeMismatch);
    }
    let blocked = right_keys_by_row.iter().cloned().collect::<BTreeSet<_>>();
    let mut rows = Vec::with_capacity(left_rows.len());
    let mut keys = Vec::with_capacity(left_rows.len());
    for (row, key) in left_rows.into_iter().zip(left_keys_by_row.iter()) {
        if blocked.contains(key) {
            continue;
        }
        rows.push(row);
        keys.push(key.clone());
    }
    Ok((
        RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.to_vec(),
        },
        CanonicalRowEvidence::from_dense(keys),
    ))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Preserve the existing value-taking boundary contract."
)]
fn project_relation_value_from_canonical_keys(
    rows: Vec<Row>,
    input_keys_by_row: CanonicalRowEvidence,
    columns: &[usize],
    output_equivalences: Vec<kernel_types::SemanticId>,
) -> Result<(RelationValue, CanonicalRowEvidence), RelQueryError> {
    if rows.len() != input_keys_by_row.len() || columns.len() != output_equivalences.len() {
        return Err(RelQueryError::TypeMismatch);
    }
    let projected_rows = project_rows(rows, columns)?;
    let mut keyed_rows = Vec::with_capacity(projected_rows.len());
    for (row, input_key) in projected_rows.into_iter().zip(input_keys_by_row.iter()) {
        let key = columns
            .iter()
            .map(|&column| {
                input_key
                    .get(column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<CanonicalRowKey, RelQueryError>>()?;
        keyed_rows.push((key, row));
    }
    keyed_rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    keyed_rows.dedup_by(|left, right| left.0 == right.0);
    let mut rows = Vec::with_capacity(keyed_rows.len());
    let mut keys = Vec::with_capacity(keyed_rows.len());
    for (key, row) in keyed_rows {
        keys.push(key);
        rows.push(row);
    }
    Ok((
        RelationValue::Set {
            rows,
            column_equivalences: output_equivalences,
        },
        CanonicalRowEvidence::from_dense(keys),
    ))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Preserve the existing value-taking boundary contract."
)]
#[allow(
    clippy::too_many_arguments,
    reason = "Keep explicit semantic and durability inputs at this boundary."
)]
fn join_relation_values_with_canonical_keys(
    left: RelationValue,
    left_keys_by_row: CanonicalRowEvidence,
    right: RelationValue,
    right_keys_by_row: CanonicalRowEvidence,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(RelationValue, CanonicalRowEvidence), RelQueryError> {
    let (
        RelationValue::Set {
            rows: left_rows,
            column_equivalences: left_equivalences,
        },
        RelationValue::Set {
            rows: right_rows,
            column_equivalences: right_equivalences,
        },
    ) = (left, right)
    else {
        return Err(RelQueryError::CanonicalObservationUnavailable);
    };
    if left_keys_by_row.len() != left_rows.len() || right_keys_by_row.len() != right_rows.len() {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }

    let mut right_buckets =
        BTreeMap::<kernel_semantics::CanonicalEqKey, Vec<(usize, &CanonicalRowKey)>>::new();
    for ((right_index, right_row), right_full_key) in
        right_rows.iter().enumerate().zip(right_keys_by_row.iter())
    {
        let right_key = right_row
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let canonical = registry
            .canonical_equivalence_key(context, equivalence, right_key)
            .map_err(RelQueryError::from)?;
        right_buckets
            .entry(canonical)
            .or_default()
            .push((right_index, right_full_key));
    }

    let mut rows = Vec::new();
    let mut canonical_keys = Vec::new();
    for (left_row, left_full_key) in left_rows.iter().zip(left_keys_by_row.iter()) {
        let left_key = left_row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let canonical = registry
            .canonical_equivalence_key(context, equivalence, left_key)
            .map_err(RelQueryError::from)?;
        let Some(matches) = right_buckets.get(&canonical) else {
            continue;
        };
        for (right_index, right_full_key) in matches {
            let right_row = right_rows
                .get(*right_index)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let mut joined = Vec::with_capacity(left_row.len() + right_row.len());
            joined.extend(left_row.iter().cloned());
            joined.extend(right_row.iter().cloned());
            rows.push(joined);

            let mut joined_key = Vec::with_capacity(left_full_key.len() + right_full_key.len());
            joined_key.extend(left_full_key.iter().cloned());
            joined_key.extend(right_full_key.iter().cloned());
            canonical_keys.push(joined_key);
        }
    }

    Ok((
        RelationValue::Set {
            rows,
            column_equivalences: left_equivalences
                .into_iter()
                .chain(right_equivalences)
                .collect(),
        },
        CanonicalRowEvidence::from_dense(canonical_keys),
    ))
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Preserve the existing value-taking boundary contract."
)]
#[allow(
    clippy::too_many_arguments,
    reason = "Keep explicit semantic and durability inputs at this boundary."
)]
fn anti_join_relation_values_with_canonical_keys(
    left: RelationValue,
    left_keys_by_row: CanonicalRowEvidence,
    right: &RelationValue,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(RelationValue, CanonicalRowEvidence), RelQueryError> {
    let RelationValue::Set {
        rows: left_rows,
        column_equivalences,
    } = left
    else {
        return Err(RelQueryError::CanonicalObservationUnavailable);
    };
    if left_keys_by_row.len() != left_rows.len() {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }

    let mut blocked = BTreeSet::new();
    for row in right.rows() {
        let value = row
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        blocked.insert(
            registry
                .canonical_equivalence_key(context, equivalence, value)
                .map_err(RelQueryError::from)?,
        );
    }

    let mut rows = Vec::with_capacity(left_rows.len());
    let mut canonical_keys = Vec::with_capacity(left_rows.len());
    for (row, full_key) in left_rows.into_iter().zip(left_keys_by_row.iter()) {
        let value = row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let blocker_key = registry
            .canonical_equivalence_key(context, equivalence, value)
            .map_err(RelQueryError::from)?;
        if blocked.contains(&blocker_key) {
            continue;
        }
        rows.push(row);
        canonical_keys.push(full_key.clone());
    }

    Ok((
        RelationValue::Set {
            rows,
            column_equivalences,
        },
        CanonicalRowEvidence::from_dense(canonical_keys),
    ))
}

fn difference_relation_values_with_canonical_keys(
    left: RelationValue,
    right: RelationValue,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(RelationValue, CanonicalRowEvidence), RelQueryError> {
    let (
        RelationValue::Set {
            rows: left_rows,
            column_equivalences: left_equivalences,
        },
        RelationValue::Set {
            rows: right_rows,
            column_equivalences: right_equivalences,
        },
    ) = (left, right)
    else {
        return Err(RelQueryError::CanonicalObservationUnavailable);
    };
    if left_equivalences != column_equivalences || right_equivalences != column_equivalences {
        return Err(RelQueryError::TypeMismatch);
    }

    let mut blocked = BTreeSet::new();
    for row in right_rows {
        blocked.insert(canonical_row_key(
            &row,
            column_equivalences,
            context,
            registry,
        )?);
    }

    let mut rows = Vec::new();
    let mut canonical_keys = Vec::new();
    for row in left_rows {
        let key = canonical_row_key(&row, column_equivalences, context, registry)?;
        if blocked.contains(&key) {
            continue;
        }
        rows.push(row);
        canonical_keys.push(key);
    }
    Ok((
        RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.to_vec(),
        },
        CanonicalRowEvidence::from_dense(canonical_keys),
    ))
}

fn union_relation_values_with_canonical_keys(
    left: RelationValue,
    right: RelationValue,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(RelationValue, CanonicalRowEvidence), RelQueryError> {
    let (
        RelationValue::Set {
            rows: left_rows,
            column_equivalences: left_equivalences,
        },
        RelationValue::Set {
            rows: right_rows,
            column_equivalences: right_equivalences,
        },
    ) = (left, right)
    else {
        return Err(RelQueryError::CanonicalObservationUnavailable);
    };
    if left_equivalences != column_equivalences || right_equivalences != column_equivalences {
        return Err(RelQueryError::TypeMismatch);
    }

    let mut keyed_rows = Vec::with_capacity(left_rows.len() + right_rows.len());
    for row in left_rows.into_iter().chain(right_rows) {
        let key = canonical_row_key(&row, column_equivalences, context, registry)?;
        keyed_rows.push((key, row));
    }
    keyed_rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));

    let mut rows = Vec::with_capacity(keyed_rows.len());
    let mut canonical_keys = Vec::with_capacity(keyed_rows.len());
    for (key, row) in keyed_rows {
        if canonical_keys.last().is_some_and(|last| *last == key) {
            continue;
        }
        rows.push(row);
        canonical_keys.push(key);
    }

    Ok((
        RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.to_vec(),
        },
        CanonicalRowEvidence::from_dense(canonical_keys),
    ))
}

impl RelExpr {
    #[must_use]
    pub fn scan_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        let mut relations = BTreeSet::new();
        collect_rel_source_relations(self, &mut relations);
        relations
    }

    pub fn prepare(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PreparedRelExpr, RelQueryError> {
        let result_type = self.typecheck(context, registry)?;
        let mut ordering_ids = BTreeSet::new();
        collect_rel_orderings(self, &mut ordering_ids);
        let compiled_orderings = ordering_ids
            .into_iter()
            .map(|ordering| {
                registry
                    .compile_ordering(context, ordering)
                    .map(|compiled| (ordering, compiled))
                    .map_err(RelQueryError::from)
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(PreparedRelExpr::new(
            self.clone(),
            result_type,
            context.clone(),
            compiled_orderings,
        ))
    }

    pub fn typecheck(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        match self {
            Self::Scan(relation) => Self::typecheck_scan(*relation, context),
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => Self::typecheck_filter(input, *column, value, *equivalence, context, registry),
            Self::FilterOrderConst {
                input,
                column,
                value,
                ordering,
                ..
            } => Self::typecheck_filter_order_const(
                input, *column, value, *ordering, context, registry,
            ),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Self::typecheck_filter_columns(
                input,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Project { input, columns } => {
                Self::typecheck_project(input, columns, context, registry)
            }
            Self::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::typecheck_join(
                left,
                right,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Difference { left, right } => {
                Self::typecheck_difference(left, right, context, registry)
            }
            Self::Union { left, right } => Self::typecheck_union(left, right, context, registry),
            Self::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::typecheck_anti_join(
                left,
                right,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Distinct {
                input,
                column_equivalences,
            } => Self::typecheck_distinct(input, column_equivalences, context, registry),
            Self::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
            } => Self::typecheck_group(
                input,
                group_columns,
                group_equivalences,
                aggregate,
                context,
                registry,
            ),
            Self::TopKWithTies {
                input,
                column,
                ordering,
                ..
            } => Self::typecheck_top_k_with_ties(input, *column, *ordering, context, registry),
            Self::PromoteToBag(input) => Self::typecheck_promote_to_bag(input, context, registry),
        }
    }

    fn typecheck_promote_to_bag(
        input: &Self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let column_equivalences = match input_type.semantics {
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            }
            | kernel_schema::RelationSemantics::Bag {
                column_equivalences,
            } => column_equivalences,
        };
        Ok(RelType {
            columns: input_type.columns,
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences,
            },
        })
    }

    fn typecheck_union(
        left: &Self,
        right: &Self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let left_type = left.typecheck(context, registry)?;
        let right_type = right.typecheck(context, registry)?;
        if left_type != right_type {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(left_type)
    }

    fn typecheck_difference(
        left: &Self,
        right: &Self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let left_type = left.typecheck(context, registry)?;
        let right_type = right.typecheck(context, registry)?;
        if left_type != right_type {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(left_type)
    }

    fn typecheck_anti_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let left_type = left.typecheck(context, registry)?;
        let _ = Self::typecheck_join(
            left,
            right,
            left_column,
            right_column,
            equivalence,
            context,
            registry,
        )?;
        Ok(left_type)
    }

    pub(super) fn typecheck_scan(
        relation: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
    ) -> Result<RelType, RelQueryError> {
        let definition = context
            .schema
            .relation(relation)
            .ok_or(RelQueryError::UnknownRelation(relation))?;
        Ok(RelType {
            columns: definition.columns.clone(),
            semantics: definition.semantics.clone(),
        })
    }

    fn typecheck_filter(
        input: &Self,
        column: usize,
        value: &Value,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let column_type = input_type
            .columns
            .get(column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        validate_query_equivalence(equivalence, column_type, context, registry)?;
        let input_equivalence = relation_column_equivalence(&input_type, column)?;
        if !registry.equivalence_refines(context, input_equivalence, equivalence)? {
            return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
        }
        if !value_shape_matches_type(value, column_type) {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(input_type)
    }

    fn typecheck_filter_order_const(
        input: &Self,
        column: usize,
        value: &Value,
        ordering: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let column_type = input_type
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
        let equivalence = relation_column_equivalence(&input_type, column)?;
        if !registry.ordering_congruent_with_equivalence(context, ordering, equivalence)? {
            return Err(RelQueryError::OrderingNotCongruentWithEquality);
        }
        if !value_shape_matches_type(value, column_type) {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(input_type)
    }

    fn typecheck_filter_columns(
        input: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let left_type = input_type
            .columns
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let right_type = input_type
            .columns
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if !query_types_compatible(left_type, right_type, &context.schema) {
            return Err(RelQueryError::TypeMismatch);
        }
        validate_query_equivalence(equivalence, left_type, context, registry)?;
        validate_query_equivalence(equivalence, right_type, context, registry)?;
        let left_input_equivalence = relation_column_equivalence(&input_type, left_column)?;
        let right_input_equivalence = relation_column_equivalence(&input_type, right_column)?;
        if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
            || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
        {
            return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
        }
        Ok(input_type)
    }

    fn typecheck_project(
        input: &Self,
        columns: &[usize],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let projected_columns = columns
            .iter()
            .map(|column| {
                input_type
                    .columns
                    .get(*column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let semantics = match input_type.semantics {
            kernel_schema::RelationSemantics::Bag {
                column_equivalences,
            } => kernel_schema::RelationSemantics::Bag {
                column_equivalences: columns
                    .iter()
                    .map(|column| {
                        column_equivalences
                            .get(*column)
                            .copied()
                            .ok_or(RelQueryError::ColumnOutOfBounds)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            },
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => {
                let projected = columns
                    .iter()
                    .map(|column| {
                        column_equivalences
                            .get(*column)
                            .copied()
                            .ok_or(RelQueryError::ColumnOutOfBounds)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                kernel_schema::RelationSemantics::Set {
                    column_equivalences: projected,
                }
            }
        };
        Ok(RelType {
            columns: projected_columns,
            semantics,
        })
    }

    fn typecheck_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let left_type = left.typecheck(context, registry)?;
        let right_type = right.typecheck(context, registry)?;
        let left_key = left_type
            .columns
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let right_key = right_type
            .columns
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if !query_types_compatible(left_key, right_key, &context.schema) {
            return Err(RelQueryError::TypeMismatch);
        }
        validate_query_equivalence(equivalence, left_key, context, registry)?;
        validate_query_equivalence(equivalence, right_key, context, registry)?;
        let left_input_equivalence = relation_column_equivalence(&left_type, left_column)?;
        let right_input_equivalence = relation_column_equivalence(&right_type, right_column)?;
        if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
            || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
        {
            return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
        }
        let semantics = match (&left_type.semantics, &right_type.semantics) {
            (
                kernel_schema::RelationSemantics::Set {
                    column_equivalences: left,
                },
                kernel_schema::RelationSemantics::Set {
                    column_equivalences: right,
                },
            ) => kernel_schema::RelationSemantics::Set {
                column_equivalences: left.iter().chain(right).copied().collect(),
            },
            (left_semantics, right_semantics) => {
                let left = match left_semantics {
                    kernel_schema::RelationSemantics::Set {
                        column_equivalences,
                    }
                    | kernel_schema::RelationSemantics::Bag {
                        column_equivalences,
                    } => column_equivalences,
                };
                let right = match right_semantics {
                    kernel_schema::RelationSemantics::Set {
                        column_equivalences,
                    }
                    | kernel_schema::RelationSemantics::Bag {
                        column_equivalences,
                    } => column_equivalences,
                };
                kernel_schema::RelationSemantics::Bag {
                    column_equivalences: left.iter().chain(right).copied().collect(),
                }
            }
        };
        let columns = left_type
            .columns
            .into_iter()
            .chain(right_type.columns)
            .collect();
        Ok(RelType { columns, semantics })
    }

    fn typecheck_group(
        input: &Self,
        group_columns: &[usize],
        group_equivalences: &[kernel_types::SemanticId],
        aggregate: &AggregateSpec,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        if group_columns.len() != group_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        let mut columns = Vec::with_capacity(group_columns.len() + 1);
        for (column, equivalence) in group_columns.iter().zip(group_equivalences) {
            let ty = input_type
                .columns
                .get(*column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            validate_query_equivalence(*equivalence, ty, context, registry)?;
            let input_equivalence = relation_column_equivalence(&input_type, *column)?;
            if !registry.equivalence_refines(context, input_equivalence, *equivalence)? {
                return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
            }
            columns.push(ty.clone());
        }
        let (result_type, result_equivalence) = match aggregate {
            AggregateSpec::Count { result_equivalence } => (
                kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::I64),
                *result_equivalence,
            ),
            AggregateSpec::ExactF64Sum {
                value_column,
                result_equivalence,
            } => {
                let value_type = input_type
                    .columns
                    .get(*value_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                let f64_type = kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::F64);
                if value_type != &f64_type {
                    return Err(RelQueryError::TypeMismatch);
                }
                (f64_type, *result_equivalence)
            }
        };
        validate_query_equivalence(result_equivalence, &result_type, context, registry)?;
        columns.push(result_type);
        let mut column_equivalences = group_equivalences.to_vec();
        column_equivalences.push(result_equivalence);
        Ok(RelType {
            columns,
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences,
            },
        })
    }

    fn typecheck_distinct(
        input: &Self,
        column_equivalences: &[kernel_types::SemanticId],
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        if input_type.columns.len() != column_equivalences.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        for (equivalence, column) in column_equivalences.iter().zip(&input_type.columns) {
            validate_query_equivalence(*equivalence, column, context, registry)?;
        }
        for (index, equivalence) in column_equivalences.iter().enumerate() {
            let input_equivalence = relation_column_equivalence(&input_type, index)?;
            if !registry.equivalence_refines(context, input_equivalence, *equivalence)? {
                return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
            }
        }
        Ok(RelType {
            columns: input_type.columns,
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: column_equivalences.to_vec(),
            },
        })
    }

    fn typecheck_top_k_with_ties(
        input: &Self,
        column: usize,
        ordering: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelType, RelQueryError> {
        let input_type = input.typecheck(context, registry)?;
        let column_type = input_type
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
        let column_equivalences = match &input_type.semantics {
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            }
            | kernel_schema::RelationSemantics::Bag {
                column_equivalences,
            } => column_equivalences,
        };
        let equivalence = *column_equivalences
            .get(column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        if !registry.ordering_congruent_with_equivalence(context, ordering, equivalence)? {
            return Err(RelQueryError::OrderingNotCongruentWithEquality);
        }
        Ok(input_type)
    }

    pub fn evaluate(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        self.prepare(context, registry)?
            .evaluate(model, context, registry)
    }

    pub fn evaluate_with_occurrence_certificate(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(RelationValue, RelationOccurrenceCertificate), RelQueryError> {
        self.prepare(context, registry)?
            .evaluate_with_occurrence_certificate(model, context, registry)
    }

    fn evaluate_unchecked(
        &self,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        match self {
            Self::Scan(relation) => Self::eval_scan(*relation, eval),
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => Self::eval_filter(input, *column, value, *equivalence, eval),
            Self::FilterOrderConst {
                input,
                column,
                value,
                ordering,
                comparison,
            } => Self::eval_filter_order_const(input, *column, value, *ordering, *comparison, eval),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Self::eval_filter_columns(input, *left_column, *right_column, *equivalence, eval),
            Self::Project { input, columns } => Self::eval_project(input, columns, eval),
            Self::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::eval_join(left, right, *left_column, *right_column, *equivalence, eval),
            Self::Difference { left, right } => Self::eval_difference(left, right, eval),
            Self::Union { left, right } => Self::eval_union(left, right, eval),
            Self::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::eval_anti_join(left, right, *left_column, *right_column, *equivalence, eval),
            Self::Distinct {
                input,
                column_equivalences,
            } => Self::eval_distinct(input, column_equivalences, eval),
            Self::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
            } => Self::eval_group(input, group_columns, group_equivalences, aggregate, eval),
            Self::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            } => Self::eval_top_k_with_ties(input, *column, *ordering, *direction, *k, eval),
            Self::PromoteToBag(input) => Ok(RelationValue::Bag(
                input.evaluate_unchecked(eval)?.into_rows(),
            )),
        }
    }

    fn eval_scan(
        relation: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let definition = eval
            .semantic
            .schema
            .relation(relation)
            .ok_or(RelQueryError::UnknownRelation(relation))?;
        let rows = eval
            .model
            .relations
            .materialize_owned(&relation)
            .unwrap_or_default();
        Ok(match &definition.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        })
    }

    fn eval_top_k_with_ties(
        input: &Self,
        column: usize,
        ordering: kernel_types::SemanticId,
        direction: OrderDirection,
        k: usize,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let compiled = eval
            .compiled_orderings
            .get(&ordering)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        top_k_relation_value(input_value, column, compiled, direction, k)
    }

    fn eval_filter(
        input: &Self,
        column: usize,
        value: &Value,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let set_equivalences = match &input_value {
            RelationValue::Set {
                column_equivalences,
                ..
            } => Some(column_equivalences.clone()),
            RelationValue::Bag(_) => None,
        };
        let mut out = Vec::new();
        for row in input_value.into_rows() {
            let candidate = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
            if eval
                .registry
                .equivalent(eval.semantic, equivalence, candidate, value)?
            {
                out.push(row);
            }
        }
        Ok(match set_equivalences {
            Some(column_equivalences) => RelationValue::Set {
                rows: out,
                column_equivalences,
            },
            None => RelationValue::Bag(out),
        })
    }

    fn eval_filter_order_const(
        input: &Self,
        column: usize,
        value: &Value,
        ordering: kernel_types::SemanticId,
        comparison: OrderComparison,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let set_equivalences = match &input_value {
            RelationValue::Set {
                column_equivalences,
                ..
            } => Some(column_equivalences.clone()),
            RelationValue::Bag(_) => None,
        };
        let compiled = eval
            .compiled_orderings
            .get(&ordering)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let threshold = compiled.canonical_key(value).map_err(RelQueryError::from)?;
        let mut out = Vec::new();
        for row in input_value.into_rows() {
            let candidate = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
            let order = compiled
                .canonical_key(candidate)
                .map_err(RelQueryError::from)?
                .cmp(&threshold);
            let passes = match comparison {
                OrderComparison::Less => order.is_lt(),
                OrderComparison::LessOrEqual => order.is_le(),
                OrderComparison::Greater => order.is_gt(),
                OrderComparison::GreaterOrEqual => order.is_ge(),
            };
            if passes {
                out.push(row);
            }
        }
        Ok(match set_equivalences {
            Some(column_equivalences) => RelationValue::Set {
                rows: out,
                column_equivalences,
            },
            None => RelationValue::Bag(out),
        })
    }

    fn eval_filter_columns(
        input: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let set_equivalences = match &input_value {
            RelationValue::Set {
                column_equivalences,
                ..
            } => Some(column_equivalences.clone()),
            RelationValue::Bag(_) => None,
        };
        let mut out = Vec::new();
        for row in input_value.into_rows() {
            let left = row
                .get(left_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            let right = row
                .get(right_column)
                .ok_or(RelQueryError::ColumnOutOfBounds)?;
            if eval
                .registry
                .equivalent(eval.semantic, equivalence, left, right)?
            {
                out.push(row);
            }
        }
        Ok(match set_equivalences {
            Some(column_equivalences) => RelationValue::Set {
                rows: out,
                column_equivalences,
            },
            None => RelationValue::Bag(out),
        })
    }

    fn eval_project(
        input: &Self,
        columns: &[usize],
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        let input_equivalences = match &input_value {
            RelationValue::Set {
                column_equivalences,
                ..
            } => Some(column_equivalences.clone()),
            RelationValue::Bag(_) => None,
        };
        let rows = input_value
            .into_rows()
            .into_iter()
            .map(|row| {
                columns
                    .iter()
                    .map(|column| {
                        row.get(*column)
                            .cloned()
                            .ok_or(RelQueryError::ColumnOutOfBounds)
                    })
                    .collect()
            })
            .collect::<Result<Vec<Row>, _>>()?;
        let Some(equivalences) = input_equivalences else {
            return Ok(RelationValue::Bag(rows));
        };
        let projected_equivalences = columns
            .iter()
            .map(|column| {
                equivalences
                    .get(*column)
                    .copied()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let rows = distinct_rows(rows, &projected_equivalences, eval.semantic, eval.registry)?;
        Ok(RelationValue::Set {
            rows,
            column_equivalences: projected_equivalences,
        })
    }

    fn eval_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let left_value = left.evaluate_unchecked(eval)?;
        let right_value = right.evaluate_unchecked(eval)?;
        join_relation_values(
            left_value,
            right_value,
            left_column,
            right_column,
            equivalence,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_difference(
        left: &Self,
        right: &Self,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let left_type = left.typecheck(eval.semantic, eval.registry)?;
        let column_equivalences = relation_column_equivalences(&left_type);
        let left_value = left.evaluate_unchecked(eval)?;
        let right_value = right.evaluate_unchecked(eval)?;
        difference_relation_values(
            left_value,
            right_value,
            column_equivalences,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_union(
        left: &Self,
        right: &Self,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let left_type = left.typecheck(eval.semantic, eval.registry)?;
        let left_value = left.evaluate_unchecked(eval)?;
        let right_value = right.evaluate_unchecked(eval)?;
        union_relation_values(
            left_value,
            right_value,
            relation_column_equivalences(&left_type),
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_anti_join(
        left: &Self,
        right: &Self,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let left_value = left.evaluate_unchecked(eval)?;
        let right_value = right.evaluate_unchecked(eval)?;
        anti_join_relation_values(
            left_value,
            &right_value,
            left_column,
            right_column,
            equivalence,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_group(
        input: &Self,
        group_columns: &[usize],
        group_equivalences: &[kernel_types::SemanticId],
        aggregate: &AggregateSpec,
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let input_value = input.evaluate_unchecked(eval)?;
        group_relation_value(
            input_value,
            group_columns,
            group_equivalences,
            aggregate,
            eval.semantic,
            eval.registry,
        )
    }

    fn eval_distinct(
        input: &Self,
        column_equivalences: &[kernel_types::SemanticId],
        eval: &RelEvalContext<'_>,
    ) -> Result<RelationValue, RelQueryError> {
        let rows = input.evaluate_unchecked(eval)?.into_rows();
        let rows = distinct_rows(rows, column_equivalences, eval.semantic, eval.registry)?;
        Ok(RelationValue::Set {
            rows,
            column_equivalences: column_equivalences.to_vec(),
        })
    }
}

fn join_relation_values(
    left_value: RelationValue,
    right_value: RelationValue,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let output_set_equivalences = match (&left_value, &right_value) {
        (
            RelationValue::Set {
                column_equivalences: left,
                ..
            },
            RelationValue::Set {
                column_equivalences: right,
                ..
            },
        ) => Some(left.iter().chain(right).copied().collect::<Vec<_>>()),
        _ => None,
    };
    let left_rows = left_value.into_rows();
    let right_rows = right_value.into_rows();
    let mut right_buckets = BTreeMap::<kernel_semantics::CanonicalEqKey, Vec<Row>>::new();
    for right_row in right_rows {
        let right_key = right_row
            .get(right_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let canonical = registry
            .canonical_equivalence_key(context, equivalence, right_key)
            .map_err(RelQueryError::from)?;
        right_buckets.entry(canonical).or_default().push(right_row);
    }
    let mut out = Vec::new();
    for left_row in &left_rows {
        let left_key = left_row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let canonical = registry
            .canonical_equivalence_key(context, equivalence, left_key)
            .map_err(RelQueryError::from)?;
        let Some(matches) = right_buckets.get(&canonical) else {
            continue;
        };
        for right_row in matches {
            let mut joined = Vec::with_capacity(left_row.len() + right_row.len());
            joined.extend(left_row.iter().cloned());
            joined.extend(right_row.iter().cloned());
            out.push(joined);
        }
    }
    Ok(match output_set_equivalences {
        Some(column_equivalences) => RelationValue::Set {
            rows: out,
            column_equivalences,
        },
        None => RelationValue::Bag(out),
    })
}

fn top_k_relation_value(
    input_value: RelationValue,
    column: usize,
    ordering: &kernel_semantics::CompiledOrdering,
    direction: OrderDirection,
    k: usize,
) -> Result<RelationValue, RelQueryError> {
    let set_equivalences = match &input_value {
        RelationValue::Set {
            column_equivalences,
            ..
        } => Some(column_equivalences.clone()),
        RelationValue::Bag(_) => None,
    };
    let mut rows = input_value.into_rows();
    if k == 0 || rows.is_empty() {
        rows.clear();
    } else {
        let mut keyed_rows = rows
            .into_iter()
            .map(|row| {
                let value = row.get(column).ok_or(RelQueryError::ColumnOutOfBounds)?;
                let key = ordering.canonical_key(value).map_err(RelQueryError::from)?;
                Ok((key, row))
            })
            .collect::<Result<Vec<_>, RelQueryError>>()?;
        keyed_rows.sort_by(|(left, _), (right, _)| match direction {
            OrderDirection::Ascending => left.cmp(right),
            OrderDirection::Descending => right.cmp(left),
        });
        if k < keyed_rows.len() {
            let threshold = keyed_rows[k - 1].0.clone();
            let mut keep = k;
            while keep < keyed_rows.len() && keyed_rows[keep].0 == threshold {
                keep += 1;
            }
            keyed_rows.truncate(keep);
        }
        rows = keyed_rows.into_iter().map(|(_, row)| row).collect();
    }
    Ok(match set_equivalences {
        Some(column_equivalences) => RelationValue::Set {
            rows,
            column_equivalences,
        },
        None => RelationValue::Bag(rows),
    })
}
