use super::{
    BTreeMap, CanonicalRowKey, ExactDeltaSink, ExactDeltaView, RelQueryError, RelType, Row, Value,
    canonical_row_key, project_row, query_types_compatible, relation_column_equivalence,
    relation_column_equivalences, validate_query_equivalence, value_shape_matches_type,
};
use crate::MaintainedDelta;

#[derive(Clone, Copy)]
pub(super) struct OrderFilterSpec<'a> {
    pub(super) column: usize,
    pub(super) value: &'a Value,
    pub(super) ordering: kernel_types::SemanticId,
    pub(super) comparison: crate::OrderComparison,
}

pub(super) fn filter_delta_view(
    input_delta: &impl ExactDeltaView<Row>,
    input_type: &RelType,
    column: usize,
    value: &Value,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaintainedDelta, RelQueryError> {
    let column_type = input_type
        .columns
        .get(column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    validate_query_equivalence(equivalence, column_type, context, registry)?;
    let input_equivalence = relation_column_equivalence(input_type, column)?;
    if !registry.equivalence_refines(context, input_equivalence, equivalence)? {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }
    if !value_shape_matches_type(value, column_type) {
        return Err(RelQueryError::TypeMismatch);
    }

    let mut output = MaintainedDelta::default();
    let mut error = None;
    input_delta.visit_exact(|weight, row| {
        if weight.is_zero() || error.is_some() {
            return;
        }
        let Some(candidate) = row.get(column) else {
            error = Some(RelQueryError::ColumnOutOfBounds);
            return;
        };
        match registry.equivalent(context, equivalence, candidate, value) {
            Ok(true) => output.push_exact(weight.clone(), row.clone()),
            Ok(false) => {}
            Err(cause) => error = Some(cause.into()),
        }
    });
    error.map_or(Ok(output), Err)
}

pub(super) fn filter_order_delta_view(
    input_delta: &impl ExactDeltaView<Row>,
    input_type: &RelType,
    spec: OrderFilterSpec<'_>,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaintainedDelta, RelQueryError> {
    let column_type = input_type
        .columns
        .get(spec.column)
        .ok_or(RelQueryError::ColumnOutOfBounds)?;
    let ordering_domain = registry.ordering_domain(context, spec.ordering)?;
    let expected = kernel_semantics::domain_for_type(column_type)
        .map(kernel_semantics::OrderingDomain::from)
        .ok_or(RelQueryError::TypeMismatch)?;
    if ordering_domain != expected {
        return Err(RelQueryError::TypeMismatch);
    }
    let equivalence = relation_column_equivalence(input_type, spec.column)?;
    if !registry.ordering_congruent_with_equivalence(context, spec.ordering, equivalence)? {
        return Err(RelQueryError::OrderingNotCongruentWithEquality);
    }
    if !value_shape_matches_type(spec.value, column_type) {
        return Err(RelQueryError::TypeMismatch);
    }
    let mut output = MaintainedDelta::default();
    let mut error = None;
    input_delta.visit_exact(|weight, row| {
        if weight.is_zero() || error.is_some() {
            return;
        }
        let Some(candidate) = row.get(spec.column) else {
            error = Some(RelQueryError::ColumnOutOfBounds);
            return;
        };
        match registry.compare(context, spec.ordering, candidate, spec.value) {
            Ok(order) => {
                let passes = match spec.comparison {
                    crate::OrderComparison::Less => order.is_lt(),
                    crate::OrderComparison::LessOrEqual => order.is_le(),
                    crate::OrderComparison::Greater => order.is_gt(),
                    crate::OrderComparison::GreaterOrEqual => order.is_ge(),
                };
                if passes {
                    output.push_exact(weight.clone(), row.clone());
                }
            }
            Err(cause) => error = Some(cause.into()),
        }
    });
    error.map_or(Ok(output), Err)
}

pub(super) fn filter_columns_delta_view(
    input_delta: &impl ExactDeltaView<Row>,
    input_type: &RelType,
    left_column: usize,
    right_column: usize,
    equivalence: kernel_types::SemanticId,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaintainedDelta, RelQueryError> {
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
    let left_input_equivalence = relation_column_equivalence(input_type, left_column)?;
    let right_input_equivalence = relation_column_equivalence(input_type, right_column)?;
    if !registry.equivalence_refines(context, left_input_equivalence, equivalence)?
        || !registry.equivalence_refines(context, right_input_equivalence, equivalence)?
    {
        return Err(RelQueryError::EquivalenceNotCongruentWithInputEquality);
    }

    let mut output = MaintainedDelta::default();
    let mut error = None;
    input_delta.visit_exact(|weight, row| {
        if weight.is_zero() || error.is_some() {
            return;
        }
        let Some(left) = row.get(left_column) else {
            error = Some(RelQueryError::ColumnOutOfBounds);
            return;
        };
        let Some(right) = row.get(right_column) else {
            error = Some(RelQueryError::ColumnOutOfBounds);
            return;
        };
        match registry.equivalent(context, equivalence, left, right) {
            Ok(true) => output.push_exact(weight.clone(), row.clone()),
            Ok(false) => {}
            Err(cause) => error = Some(cause.into()),
        }
    });
    error.map_or(Ok(output), Err)
}

pub(super) fn project_bag_delta_view(
    input_delta: &impl ExactDeltaView<Row>,
    columns: &[usize],
    result_type: &RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<MaintainedDelta, RelQueryError> {
    let equivalences = relation_column_equivalences(result_type);
    let mut canonical = BTreeMap::<CanonicalRowKey, usize>::new();
    let mut classes =
        Vec::<(Row, kernel_exact::ExactInteger)>::with_capacity(input_delta.support_len());
    let mut error = None;
    input_delta.visit_exact(|weight, row| {
        if weight.is_zero() || error.is_some() {
            return;
        }
        let projected = match project_row(row, columns) {
            Ok(row) => row,
            Err(cause) => {
                error = Some(cause);
                return;
            }
        };
        let key = match canonical_row_key(&projected, equivalences, context, registry) {
            Ok(key) => key,
            Err(cause) => {
                error = Some(cause);
                return;
            }
        };
        if let Some(index) = canonical.get(&key).copied() {
            classes[index].1.add_assign(weight);
        } else {
            canonical.insert(key, classes.len());
            classes.push((projected, weight.clone()));
        }
    });
    if let Some(error) = error {
        return Err(error);
    }

    let mut output = MaintainedDelta::default();
    for (row, weight) in classes {
        output.push_exact(weight, row);
    }
    Ok(output)
}

pub(super) fn project_delta_view(
    input_delta: &impl ExactDeltaView<Row>,
    columns: &[usize],
) -> Result<MaintainedDelta, RelQueryError> {
    let mut projected = MaintainedDelta::default();
    let mut error = None;
    input_delta.visit_exact(|weight, row| {
        if weight.is_zero() || error.is_some() {
            return;
        }
        match project_row(row, columns) {
            Ok(row) => projected.push_exact(weight.clone(), row),
            Err(cause) => error = Some(cause),
        }
    });
    error.map_or(Ok(projected), Err)
}
