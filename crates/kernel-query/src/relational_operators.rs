use super::{
    AggregateSpec, BTreeMap, BTreeSet, CanonicalRowKey, RelQueryError, RelType, RelationValue, Row,
    Value, canonical_row_key, relation_column_equivalences,
};

/// Exact semantic union under the supplied row equivalences.
///
/// Set inputs use Γ-support union. Bag inputs use additive multiset union.
/// The operator never delegates duplicate semantics to host-language equality.
pub fn union_relation_values(
    left: RelationValue,
    right: RelationValue,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    match (left, right) {
        (
            RelationValue::Set {
                rows: left_rows,
                column_equivalences: left_equivalences,
            },
            RelationValue::Set {
                rows: right_rows,
                column_equivalences: right_equivalences,
            },
        ) => {
            if left_equivalences != column_equivalences || right_equivalences != column_equivalences
            {
                return Err(RelQueryError::TypeMismatch);
            }
            let mut seen = BTreeSet::new();
            let mut rows = Vec::with_capacity(left_rows.len() + right_rows.len());
            for row in left_rows.into_iter().chain(right_rows) {
                let key = canonical_row_key(&row, column_equivalences, context, registry)?;
                if seen.insert(key) {
                    rows.push(row);
                }
            }
            Ok(RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.to_vec(),
            })
        }
        (RelationValue::Bag(mut left_rows), RelationValue::Bag(right_rows)) => {
            left_rows.extend(right_rows);
            Ok(RelationValue::Bag(left_rows))
        }
        _ => Err(RelQueryError::TypeMismatch),
    }
}

/// Exact semantic difference under the supplied row equivalences.
///
/// Set inputs use support subtraction. Bag inputs use truncated natural
/// subtraction (monus) per Γ-canonical row class, preserving a representative
/// from the left input for every surviving occurrence.
pub fn difference_relation_values(
    left: RelationValue,
    right: RelationValue,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    match (left, right) {
        (
            RelationValue::Set {
                rows: left_rows,
                column_equivalences: left_equivalences,
            },
            RelationValue::Set {
                rows: right_rows,
                column_equivalences: right_equivalences,
            },
        ) => {
            if left_equivalences != column_equivalences || right_equivalences != column_equivalences
            {
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
            for row in left_rows {
                let key = canonical_row_key(&row, column_equivalences, context, registry)?;
                if !blocked.contains(&key) {
                    rows.push(row);
                }
            }
            Ok(RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.to_vec(),
            })
        }
        (RelationValue::Bag(left_rows), RelationValue::Bag(right_rows)) => {
            let mut blockers = BTreeMap::<CanonicalRowKey, usize>::new();
            for row in right_rows {
                let key = canonical_row_key(&row, column_equivalences, context, registry)?;
                *blockers.entry(key).or_default() += 1;
            }
            let mut rows = Vec::new();
            for row in left_rows {
                let key = canonical_row_key(&row, column_equivalences, context, registry)?;
                if let Some(count) = blockers.get_mut(&key)
                    && *count > 0
                {
                    *count -= 1;
                    continue;
                }
                rows.push(row);
            }
            Ok(RelationValue::Bag(rows))
        }
        _ => Err(RelQueryError::TypeMismatch),
    }
}

/// Exact anti-semi join. Right multiplicity is a blocker predicate only:
/// one or more Γ-equivalent right keys suppress the complete left-key fiber.
/// Left multiplicity is otherwise preserved unchanged.
pub fn anti_join_relation_values(
    left: RelationValue,
    right: &RelationValue,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let left_equivalences = match &left {
        RelationValue::Set {
            column_equivalences,
            ..
        } => Some(column_equivalences.clone()),
        RelationValue::Bag(_) => None,
    };
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
    let mut rows = Vec::new();
    for row in left.into_rows() {
        let value = row
            .get(left_column)
            .ok_or(RelQueryError::ColumnOutOfBounds)?;
        let key = registry
            .canonical_equivalence_key(context, equivalence, value)
            .map_err(RelQueryError::from)?;
        if !blocked.contains(&key) {
            rows.push(row);
        }
    }
    Ok(match left_equivalences {
        Some(column_equivalences) => RelationValue::Set {
            rows,
            column_equivalences,
        },
        None => RelationValue::Bag(rows),
    })
}

pub(super) fn group_relation_value(
    input_value: RelationValue,
    group_columns: &[usize],
    group_equivalences: &[kernel_types::SemanticId],
    aggregate: &AggregateSpec,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    enum State {
        Count(kernel_aggregate::ExactCount),
        ExactF64Sum(kernel_aggregate::ExactF64Sum),
    }

    let rows = input_value.into_rows();
    let mut groups: Vec<(Vec<Value>, State)> = Vec::new();
    let mut group_lookup = BTreeMap::<CanonicalRowKey, usize>::new();
    if rows.is_empty() && group_columns.is_empty() {
        let state = match aggregate {
            AggregateSpec::Count { .. } => State::Count(kernel_aggregate::ExactCount::default()),
            AggregateSpec::ExactF64Sum { .. } => {
                State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
            }
        };
        groups.push((Vec::new(), state));
    }
    for row in rows {
        let key = group_columns
            .iter()
            .map(|column| {
                row.get(*column)
                    .cloned()
                    .ok_or(RelQueryError::ColumnOutOfBounds)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let canonical_key = canonical_row_key(&key, group_equivalences, context, registry)?;
        let index = if let Some(index) = group_lookup.get(&canonical_key).copied() {
            index
        } else {
            let state = match aggregate {
                AggregateSpec::Count { .. } => {
                    State::Count(kernel_aggregate::ExactCount::default())
                }
                AggregateSpec::ExactF64Sum { .. } => {
                    State::ExactF64Sum(kernel_aggregate::ExactF64Sum::default())
                }
            };
            let index = groups.len();
            groups.push((key, state));
            group_lookup.insert(canonical_key, index);
            index
        };
        match (&mut groups[index].1, aggregate) {
            (State::Count(count), AggregateSpec::Count { .. }) => count.add_one(),
            (State::ExactF64Sum(sum), AggregateSpec::ExactF64Sum { value_column, .. }) => {
                let value = row
                    .get(*value_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                let Value::F64Bits(bits) = value else {
                    return Err(RelQueryError::TypeMismatch);
                };
                sum.add(f64::from_bits(*bits))?;
            }
            _ => unreachable!("aggregate state is created from the same AggregateSpec"),
        }
    }
    let mut rows = Vec::with_capacity(groups.len());
    for (mut key, state) in groups {
        let value = match state {
            State::Count(count) => Value::I64(count.finish_i64()?),
            State::ExactF64Sum(sum) => Value::F64Bits(sum.finish().to_bits()),
        };
        key.push(value);
        rows.push(key);
    }
    let result_equivalence = match aggregate {
        AggregateSpec::Count { result_equivalence }
        | AggregateSpec::ExactF64Sum {
            result_equivalence, ..
        } => *result_equivalence,
    };
    let mut column_equivalences = group_equivalences.to_vec();
    column_equivalences.push(result_equivalence);
    Ok(RelationValue::Set {
        rows,
        column_equivalences,
    })
}

pub(super) fn value_shape_matches_type(value: &Value, ty: &kernel_schema::TypeExpr) -> bool {
    value_shape_matches_type_inner(value, ty, &std::collections::BTreeMap::new())
}

fn value_shape_matches_type_inner<'a>(
    value: &Value,
    ty: &'a kernel_schema::TypeExpr,
    recursive: &std::collections::BTreeMap<kernel_schema::TypeVar, &'a kernel_schema::TypeExpr>,
) -> bool {
    match (value, ty) {
        (Value::Unit, kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Unit))
        | (Value::Bool(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Bool))
        | (Value::I64(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::I64))
        | (Value::F64Bits(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::F64))
        | (Value::Text(_), kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Text)) => {
            true
        }
        (
            Value::LiveEntityRef { entity_type, .. },
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::LiveEntityRef(expected)),
        )
        | (
            Value::HistoricalEntityId { entity_type, .. },
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::HistoricalEntityId(
                expected,
            )),
        ) => entity_type == expected,
        (Value::Product(values), kernel_schema::TypeExpr::Product(fields)) => {
            values.len() == fields.len()
                && fields.iter().all(|(field, field_type)| {
                    values.get(field).is_some_and(|value| {
                        value_shape_matches_type_inner(value, field_type, recursive)
                    })
                })
        }
        (Value::Option(value), kernel_schema::TypeExpr::Option(inner)) => value
            .as_deref()
            .is_none_or(|value| value_shape_matches_type_inner(value, inner, recursive)),
        (Value::Variant { tag, value }, kernel_schema::TypeExpr::Sum(variants)) => variants
            .get(tag)
            .is_some_and(|variant| value_shape_matches_type_inner(value, variant, recursive)),
        (Value::Seq(values), kernel_schema::TypeExpr::Seq(element)) => values
            .iter()
            .all(|value| value_shape_matches_type_inner(value, element, recursive)),
        (
            Value::Set {
                equivalence,
                elements,
            },
            kernel_schema::TypeExpr::Set {
                element,
                equivalence: expected,
            },
        ) => {
            equivalence == expected
                && elements
                    .iter()
                    .all(|value| value_shape_matches_type_inner(value, element, recursive))
        }
        (
            Value::Bag {
                equivalence,
                entries,
            },
            kernel_schema::TypeExpr::Bag {
                element,
                equivalence: expected,
            },
        ) => {
            equivalence == expected
                && entries
                    .iter()
                    .all(|(value, _)| value_shape_matches_type_inner(value, element, recursive))
        }
        (
            Value::Map {
                key_equivalence,
                entries,
            },
            kernel_schema::TypeExpr::Map {
                key,
                value,
                key_equivalence: expected,
            },
        ) => {
            key_equivalence == expected
                && entries.iter().all(|(entry_key, entry_value)| {
                    value_shape_matches_type_inner(entry_key, key, recursive)
                        && value_shape_matches_type_inner(entry_value, value, recursive)
                })
        }
        (_, kernel_schema::TypeExpr::Mu { binder, body }) => {
            let mut next = recursive.clone();
            next.insert(*binder, ty);
            value_shape_matches_type_inner(value, body, &next)
        }
        (_, kernel_schema::TypeExpr::Var(var)) => recursive
            .get(var)
            .is_some_and(|bound| value_shape_matches_type_inner(value, bound, recursive)),
        _ => false,
    }
}

pub(super) fn query_types_compatible(
    left: &kernel_schema::TypeExpr,
    right: &kernel_schema::TypeExpr,
    schema: &kernel_schema::Schema,
) -> bool {
    if left == right {
        return true;
    }
    match (left, right) {
        (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::LiveEntityRef(left)),
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::LiveEntityRef(right)),
        )
        | (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::HistoricalEntityId(left)),
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::HistoricalEntityId(right)),
        ) => schema.is_subtype(*left, *right) || schema.is_subtype(*right, *left),
        _ => false,
    }
}

pub(super) fn validate_query_equivalence(
    equivalence: kernel_types::SemanticId,
    ty: &kernel_schema::TypeExpr,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), RelQueryError> {
    let expected = kernel_semantics::domain_for_type(ty).ok_or(RelQueryError::TypeMismatch)?;
    let actual = registry.equivalence_domain(context, equivalence)?;
    if expected == actual {
        Ok(())
    } else {
        Err(RelQueryError::Semantic(
            kernel_semantics::SemanticError::EquivalenceDomainMismatch {
                equivalence,
                expected,
                actual,
            },
        ))
    }
}

pub(super) fn relation_column_equivalence(
    relation_type: &RelType,
    column: usize,
) -> Result<kernel_types::SemanticId, RelQueryError> {
    relation_column_equivalences(relation_type)
        .get(column)
        .copied()
        .ok_or(RelQueryError::ColumnOutOfBounds)
}

pub(super) fn distinct_rows(
    rows: Vec<Row>,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<Vec<Row>, RelQueryError> {
    let mut seen = BTreeSet::<CanonicalRowKey>::new();
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let key = canonical_row_key(&row, column_equivalences, context, registry)?;
        if seen.insert(key) {
            out.push(row);
        }
    }
    Ok(out)
}

pub(super) fn distinct_rows_with_canonical_keys(
    rows: Vec<Row>,
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(Vec<Row>, Vec<CanonicalRowKey>), RelQueryError> {
    let mut keyed_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let key = canonical_row_key(&row, column_equivalences, context, registry)?;
        keyed_rows.push((key, row));
    }
    keyed_rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));

    let mut out = Vec::with_capacity(keyed_rows.len());
    let mut canonical_keys = Vec::with_capacity(keyed_rows.len());
    for (key, row) in keyed_rows {
        if canonical_keys.last().is_some_and(|last| *last == key) {
            continue;
        }
        out.push(row);
        canonical_keys.push(key);
    }
    Ok((out, canonical_keys))
}
