use super::{NativeColumn, NativeRelation, PersistentPhysicalVec, PhysicalExecutionError, Value};

// HOSTILE[P178][ACTIVE][CLEAN]: representation capabilities are owned beside NativeRelation,
// not by execution or storage policy modules.
#[derive(Debug, Clone, Copy)]
pub(crate) enum NativeI64ColumnView<'a> {
    Contiguous(&'a [i64]),
    Persistent(&'a PersistentPhysicalVec<i64>),
}

impl NativeI64ColumnView<'_> {
    pub(crate) fn len(self) -> usize {
        match self {
            Self::Contiguous(values) => values.len(),
            Self::Persistent(values) => values.len(),
        }
    }

    pub(crate) fn value(self, index: usize) -> i64 {
        match self {
            Self::Contiguous(values) => values[index],
            Self::Persistent(values) => values[index],
        }
    }

    pub(crate) fn get(&self, index: usize) -> Option<&i64> {
        match self {
            Self::Contiguous(values) => values.get(index),
            Self::Persistent(values) => values.get(index),
        }
    }
}

impl std::ops::Index<usize> for NativeI64ColumnView<'_> {
    type Output = i64;

    fn index(&self, index: usize) -> &Self::Output {
        self.get(index)
            .expect("native i64 column index out of bounds")
    }
}

pub(super) fn native_all_i64_columns(
    data: &NativeRelation,
) -> Option<Vec<NativeI64ColumnView<'_>>> {
    match data {
        NativeRelation::I64Columnar { columns, .. } => Some(
            columns
                .iter()
                .map(|values| NativeI64ColumnView::Contiguous(values.as_slice()))
                .collect(),
        ),
        NativeRelation::TypedColumnar { columns, .. } => columns
            .iter()
            .map(|column| match column {
                NativeColumn::I64(values) => Some(NativeI64ColumnView::Persistent(values)),
                _ => None,
            })
            .collect(),
        NativeRelation::RowStore(_) | NativeRelation::Columnar { .. } => None,
    }
}

pub(super) fn append_i64_row_values(
    row: &mut kernel_query::Row,
    columns: &[NativeI64ColumnView<'_>],
    row_index: usize,
) {
    row.extend(
        columns
            .iter()
            .map(|column| Value::I64(column.value(row_index))),
    );
}

pub(super) fn native_i64_column(
    data: &NativeRelation,
    column: usize,
) -> Option<NativeI64ColumnView<'_>> {
    match data {
        NativeRelation::I64Columnar { columns, .. } => columns
            .get(column)
            .map(|values| NativeI64ColumnView::Contiguous(values.as_slice())),
        NativeRelation::TypedColumnar { columns, .. } => match columns.get(column)? {
            NativeColumn::I64(values) => Some(NativeI64ColumnView::Persistent(values)),
            _ => None,
        },
        NativeRelation::RowStore(_) | NativeRelation::Columnar { .. } => None,
    }
}

pub(super) fn native_column_count(data: &NativeRelation) -> usize {
    match data {
        NativeRelation::I64Columnar { columns, .. } => columns.len(),
        NativeRelation::Columnar { columns, .. } => columns.len(),
        NativeRelation::TypedColumnar { columns, .. } => columns.len(),
        NativeRelation::RowStore(rows) => rows.first().map_or(0, Vec::len),
    }
}

pub(super) fn append_native_row_values(
    row: &mut kernel_query::Row,
    data: &NativeRelation,
    row_index: usize,
) -> Result<(), PhysicalExecutionError> {
    match data {
        NativeRelation::I64Columnar { columns, row_count } => {
            if row_index >= *row_count {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            row.extend(columns.iter().map(|column| Value::I64(column[row_index])));
            Ok(())
        }
        NativeRelation::TypedColumnar { columns, row_count } => {
            if row_index >= *row_count {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            row.extend(columns.iter().map(|column| column.value_at(row_index)));
            Ok(())
        }
        NativeRelation::RowStore(_) | NativeRelation::Columnar { .. } => {
            Err(PhysicalExecutionError::UnsupportedPhysicalPlan)
        }
    }
}

pub(super) fn native_row_count(data: &NativeRelation) -> usize {
    match data {
        NativeRelation::RowStore(rows) => rows.len(),
        NativeRelation::Columnar { row_count, .. }
        | NativeRelation::I64Columnar { row_count, .. }
        | NativeRelation::TypedColumnar { row_count, .. } => *row_count,
    }
}

pub(super) fn materialize_native_row(
    data: &NativeRelation,
    row_index: usize,
) -> Result<kernel_query::Row, PhysicalExecutionError> {
    match data {
        NativeRelation::RowStore(rows) => rows
            .get(row_index)
            .cloned()
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch),
        NativeRelation::Columnar { columns, row_count } => {
            if row_index >= *row_count {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            Ok(columns
                .iter()
                .map(|column| column[row_index].clone())
                .collect())
        }
        NativeRelation::I64Columnar { .. } | NativeRelation::TypedColumnar { .. } => {
            let mut row = Vec::with_capacity(native_column_count(data));
            append_native_row_values(&mut row, data, row_index)?;
            Ok(row)
        }
    }
}

pub(super) fn remove_native_row(
    data: &mut NativeRelation,
    row_index: usize,
) -> Result<(), PhysicalExecutionError> {
    match data {
        NativeRelation::RowStore(rows) => {
            if row_index >= rows.len() {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            rows.swap_remove(row_index);
        }
        NativeRelation::Columnar { columns, row_count } => {
            if row_index >= *row_count {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            for column in columns {
                column.swap_remove(row_index);
            }
            *row_count -= 1;
        }
        NativeRelation::I64Columnar { columns, row_count } => {
            if row_index >= *row_count {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            for column in columns {
                column.swap_remove(row_index);
            }
            *row_count -= 1;
        }
        NativeRelation::TypedColumnar { columns, row_count } => {
            if row_index >= *row_count {
                return Err(PhysicalExecutionError::ColumnShapeMismatch);
            }
            for column in columns {
                column.swap_remove_at(row_index)?;
            }
            *row_count -= 1;
        }
    }
    Ok(())
}

pub(super) fn push_native_row(
    data: &mut NativeRelation,
    row: &kernel_query::Row,
) -> Result<(), PhysicalExecutionError> {
    if row.len() != native_column_count(data) {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    match data {
        NativeRelation::RowStore(rows) => rows.push(row.clone()),
        NativeRelation::Columnar { columns, row_count } => {
            for (column, value) in columns.iter_mut().zip(row) {
                column.push(value.clone());
            }
            *row_count += 1;
        }
        NativeRelation::I64Columnar { columns, row_count } => {
            for (column, value) in columns.iter_mut().zip(row) {
                let Value::I64(value) = value else {
                    return Err(PhysicalExecutionError::PhysicalTypeMismatch);
                };
                column.push(*value);
            }
            *row_count += 1;
        }
        NativeRelation::TypedColumnar { columns, row_count } => {
            for (column, value) in columns.iter_mut().zip(row) {
                column.push_value(value)?;
            }
            *row_count += 1;
        }
    }
    Ok(())
}

pub(super) fn validate_native_row(
    data: &NativeRelation,
    row: &kernel_query::Row,
) -> Result<(), PhysicalExecutionError> {
    if row.len() != native_column_count(data) {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    match data {
        NativeRelation::RowStore(_) | NativeRelation::Columnar { .. } => Ok(()),
        NativeRelation::I64Columnar { .. } => row
            .iter()
            .all(|value| matches!(value, Value::I64(_)))
            .then_some(())
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch),
        NativeRelation::TypedColumnar { columns, .. } => columns
            .iter()
            .zip(row)
            .all(|(column, value)| native_column_accepts_value(column, value))
            .then_some(())
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch),
    }
}

fn native_column_accepts_value(column: &NativeColumn, value: &Value) -> bool {
    match (column, value) {
        (NativeColumn::Unit(_), Value::Unit)
        | (NativeColumn::Bool(_), Value::Bool(_))
        | (NativeColumn::I64(_), Value::I64(_))
        | (NativeColumn::F64Bits(_), Value::F64Bits(_))
        | (NativeColumn::Text(_), Value::Text(_)) => true,
        (NativeColumn::Algebraic(column), value) => column.accepts_value(value),
        (
            NativeColumn::LiveEntityIds { entity_type, .. },
            Value::LiveEntityRef {
                entity_type: value_type,
                ..
            },
        )
        | (
            NativeColumn::DenseLiveEntityIds { entity_type, .. },
            Value::LiveEntityRef {
                entity_type: value_type,
                ..
            },
        )
        | (
            NativeColumn::HistoricalEntityIds { entity_type, .. },
            Value::HistoricalEntityId {
                entity_type: value_type,
                ..
            },
        ) => entity_type == value_type,
        _ => false,
    }
}

pub(super) fn validate_i64_columnar_schema(
    context: &kernel_schema::SemanticContext,
    relation: super::SemanticId,
    column_count: usize,
) -> Result<(), PhysicalExecutionError> {
    let definition = context
        .schema
        .relation(relation)
        .ok_or(super::RelQueryError::UnknownRelation(relation))?;
    if definition.columns.len() != column_count
        || definition.columns.iter().any(|column| {
            !matches!(
                column,
                kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::I64)
            )
        })
    {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    Ok(())
}

pub(super) fn validate_typed_columnar_schema(
    context: &kernel_schema::SemanticContext,
    relation: super::SemanticId,
    columns: &[NativeColumn],
) -> Result<(), PhysicalExecutionError> {
    let definition = context
        .schema
        .relation(relation)
        .ok_or(super::RelQueryError::UnknownRelation(relation))?;
    if definition.columns.len() != columns.len() {
        return Err(PhysicalExecutionError::PhysicalTypeMismatch);
    }
    for (logical, physical) in definition.columns.iter().zip(columns) {
        if !native_column_matches_type(logical, physical) {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
    }
    Ok(())
}

fn native_column_matches_type(logical: &kernel_schema::TypeExpr, physical: &NativeColumn) -> bool {
    match (logical, physical) {
        (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Unit),
            NativeColumn::Unit(_),
        )
        | (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Bool),
            NativeColumn::Bool(_),
        )
        | (kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::I64), NativeColumn::I64(_))
        | (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::F64),
            NativeColumn::F64Bits(_),
        )
        | (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::Text),
            NativeColumn::Text(_),
        ) => true,
        (logical, NativeColumn::Algebraic(column)) => logical == column.ty(),
        (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::LiveEntityRef(logical_type)),
            NativeColumn::LiveEntityIds {
                entity_type: physical_type,
                ..
            }
            | NativeColumn::DenseLiveEntityIds {
                entity_type: physical_type,
                ..
            },
        )
        | (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::HistoricalEntityId(
                logical_type,
            )),
            NativeColumn::HistoricalEntityIds {
                entity_type: physical_type,
                ..
            },
        ) => logical_type == physical_type,
        _ => false,
    }
}

pub(super) fn validate_indexable_i64_relation(
    context: &kernel_schema::SemanticContext,
    relation: super::SemanticId,
    data: &NativeRelation,
) -> Result<(), PhysicalExecutionError> {
    match data {
        NativeRelation::I64Columnar { columns, .. } => {
            validate_i64_columnar_schema(context, relation, columns.len())
        }
        NativeRelation::TypedColumnar { columns, .. } => {
            validate_typed_columnar_schema(context, relation, columns)
        }
        NativeRelation::RowStore(_) | NativeRelation::Columnar { .. } => Ok(()),
    }
}
