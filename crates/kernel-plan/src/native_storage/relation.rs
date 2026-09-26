#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeRelation {
    RowStore(PersistentPhysicalVec<kernel_query::Row>),
    Columnar {
        columns: Vec<PersistentPhysicalVec<Value>>,
        row_count: usize,
    },
    I64Columnar {
        columns: Vec<PersistentPhysicalVec<i64>>,
        row_count: usize,
    },
    TypedColumnar {
        columns: Vec<NativeColumn>,
        row_count: usize,
    },
}

impl NativeRelation {
    pub fn columnar(columns: Vec<Vec<Value>>) -> Result<Self, PhysicalExecutionError> {
        let row_count = columns.first().map_or(0, Vec::len);
        if columns.iter().any(|column| column.len() != row_count) {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        if !columns.is_empty() && row_count > 0 {
            let typed = columns
                .iter()
                .map(|column| NativeColumn::try_from_untyped_scalar_values(column))
                .collect::<Result<Vec<_>, _>>()?;
            if typed.iter().all(Option::is_some) {
                return Self::typed_columnar(typed.into_iter().flatten().collect());
            }
        }
        Ok(Self::Columnar {
            columns: columns
                .into_iter()
                .map(PersistentPhysicalVec::from_vec)
                .collect(),
            row_count,
        })
    }

    pub fn i64_columnar(columns: Vec<Vec<i64>>) -> Result<Self, PhysicalExecutionError> {
        let row_count = columns.first().map_or(0, Vec::len);
        if columns.iter().any(|column| column.len() != row_count) {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        Self::typed_columnar(
            columns
                .into_iter()
                .map(|values| NativeColumn::I64(values.into()))
                .collect(),
        )
    }

    pub fn typed_from_rows(
        rows: &[kernel_query::Row],
        column_types: &[kernel_schema::TypeExpr],
    ) -> Result<Self, PhysicalExecutionError> {
        if rows.iter().any(|row| row.len() != column_types.len()) {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        let mut columns = Vec::with_capacity(column_types.len());
        for (column_index, ty) in column_types.iter().enumerate() {
            let values = rows
                .iter()
                .map(|row| row[column_index].clone())
                .collect::<Vec<_>>();
            columns.push(NativeColumn::from_typed_values(&values, ty)?);
        }
        Self::typed_columnar(columns)
    }

    pub fn typed_columnar(columns: Vec<NativeColumn>) -> Result<Self, PhysicalExecutionError> {
        let row_count = columns.first().map_or(0, NativeColumn::len);
        if columns.iter().any(|column| column.len() != row_count) {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        Ok(Self::TypedColumnar { columns, row_count })
    }

    #[must_use]
    pub fn row_store(rows: Vec<kernel_query::Row>) -> Self {
        Self::RowStore(PersistentPhysicalVec::from_vec(rows))
    }
}

fn value_heap_bytes(value: &Value) -> usize {
    match value {
        Value::Unit
        | Value::Bool(_)
        | Value::I64(_)
        | Value::F64Bits(_)
        | Value::LiveEntityRef { .. }
        | Value::HistoricalEntityId { .. } => 0,
        Value::Text(value) => value.capacity(),
        Value::Product(values) => values.values().fold(0, |bytes, value| {
            bytes
                .saturating_add(std::mem::size_of::<(SemanticId, Value)>())
                .saturating_add(value_heap_bytes(value))
        }),
        Value::Option(value) => value.as_deref().map_or(0, |value| {
            std::mem::size_of::<Value>().saturating_add(value_heap_bytes(value))
        }),
        Value::Variant { value, .. } => {
            std::mem::size_of::<Value>().saturating_add(value_heap_bytes(value))
        }
        Value::Seq(values)
        | Value::Set {
            elements: values, ..
        } => values
            .capacity()
            .saturating_mul(std::mem::size_of::<Value>())
            .saturating_add(saturating_usize_sum(values.iter().map(value_heap_bytes))),
        Value::Bag { entries, .. } => entries
            .capacity()
            .saturating_mul(std::mem::size_of::<(Value, u64)>())
            .saturating_add(saturating_usize_sum(
                entries.iter().map(|(value, _)| value_heap_bytes(value)),
            )),
        Value::Map { entries, .. } => entries
            .capacity()
            .saturating_mul(std::mem::size_of::<(Value, Value)>())
            .saturating_add(saturating_usize_sum(entries.iter().map(|(key, value)| {
                value_heap_bytes(key).saturating_add(value_heap_bytes(value))
            }))),
    }
}

fn native_relation_estimated_heap_bytes(relation: &NativeRelation) -> usize {
    match relation {
        NativeRelation::RowStore(rows) => {
            rows.estimated_heap_bytes()
                .saturating_add(saturating_usize_sum(rows.iter().map(|row| {
                    row.capacity()
                        .saturating_mul(std::mem::size_of::<Value>())
                        .saturating_add(saturating_usize_sum(row.iter().map(value_heap_bytes)))
                })))
        }
        NativeRelation::Columnar { columns, .. } => columns
            .capacity()
            .saturating_mul(std::mem::size_of::<PersistentPhysicalVec<Value>>())
            .saturating_add(saturating_usize_sum(columns.iter().map(|column| {
                column
                    .estimated_heap_bytes()
                    .saturating_add(saturating_usize_sum(column.iter().map(value_heap_bytes)))
            }))),
        NativeRelation::I64Columnar { columns, .. } => columns
            .capacity()
            .saturating_mul(std::mem::size_of::<PersistentPhysicalVec<i64>>())
            .saturating_add(saturating_usize_sum(
                columns
                    .iter()
                    .map(PersistentPhysicalVec::estimated_heap_bytes),
            )),
        NativeRelation::TypedColumnar { columns, .. } => columns
            .capacity()
            .saturating_mul(std::mem::size_of::<NativeColumn>())
            .saturating_add(saturating_usize_sum(
                columns.iter().map(NativeColumn::estimated_heap_bytes),
            )),
    }
}

