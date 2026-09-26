#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeColumn {
    Unit(usize),
    Bool(PersistentPhysicalVec<bool>),
    I64(PersistentPhysicalVec<i64>),
    F64Bits(PersistentPhysicalVec<u64>),
    Text(PersistentPhysicalVec<String>),
    Algebraic(Box<algebraic_native::AlgebraicNativeColumn>),
    LiveEntityIds {
        entity_type: SemanticId,
        values: PersistentPhysicalVec<kernel_types::EntityId>,
    },
    DenseLiveEntityIds {
        entity_type: SemanticId,
        ids: Arc<DenseEntityIds>,
        values: PersistentPhysicalVec<LocalEntityId>,
    },
    HistoricalEntityIds {
        entity_type: SemanticId,
        values: PersistentPhysicalVec<kernel_types::EntityId>,
    },
}

impl NativeColumn {
    fn try_from_untyped_scalar_values(
        values: &[Value],
    ) -> Result<Option<Self>, PhysicalExecutionError> {
        let Some(first) = values.first() else {
            return Ok(None);
        };
        let column = match first {
            Value::Unit if values.iter().all(|value| matches!(value, Value::Unit)) => {
                Self::Unit(values.len())
            }
            Value::Bool(_) => Self::Bool(
                values
                    .iter()
                    .map(|value| match value {
                        Value::Bool(value) => Ok(*value),
                        _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            ),
            Value::I64(_) => Self::I64(
                values
                    .iter()
                    .map(|value| match value {
                        Value::I64(value) => Ok(*value),
                        _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            ),
            Value::F64Bits(_) => Self::F64Bits(
                values
                    .iter()
                    .map(|value| match value {
                        Value::F64Bits(value) => Ok(*value),
                        _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            ),
            Value::Text(_) => Self::Text(
                values
                    .iter()
                    .map(|value| match value {
                        Value::Text(value) => Ok(value.clone()),
                        _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            ),
            Value::LiveEntityRef { entity_type, .. } => Self::LiveEntityIds {
                entity_type: *entity_type,
                values: values
                    .iter()
                    .map(|value| match value {
                        Value::LiveEntityRef {
                            entity_type: value_type,
                            id,
                        } if value_type == entity_type => Ok(*id),
                        _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            },
            Value::HistoricalEntityId { entity_type, .. } => Self::HistoricalEntityIds {
                entity_type: *entity_type,
                values: values
                    .iter()
                    .map(|value| match value {
                        Value::HistoricalEntityId {
                            entity_type: value_type,
                            id,
                        } if value_type == entity_type => Ok(*id),
                        _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            },
            _ => return Ok(None),
        };
        Ok(Some(column))
    }

    pub fn dense_live_entity_ids(
        entity_type: SemanticId,
        ids: Arc<DenseEntityIds>,
        values: Vec<kernel_types::EntityId>,
    ) -> Result<Self, PhysicalExecutionError> {
        let values = values
            .into_iter()
            .map(|id| {
                ids.local(id)
                    .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::DenseLiveEntityIds {
            entity_type,
            ids,
            values: values.into(),
        })
    }

    fn len(&self) -> usize {
        match self {
            Self::Unit(len) => *len,
            Self::Bool(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::F64Bits(values) => values.len(),
            Self::Text(values) => values.len(),
            Self::Algebraic(column) => column.len(),
            Self::LiveEntityIds { values, .. } | Self::HistoricalEntityIds { values, .. } => {
                values.len()
            }
            Self::DenseLiveEntityIds { values, .. } => values.len(),
        }
    }

    fn estimated_heap_bytes(&self) -> usize {
        match self {
            Self::Unit(_) => 0,
            Self::Bool(values) => values.capacity().div_ceil(8),
            Self::I64(values) => values.capacity().saturating_mul(std::mem::size_of::<i64>()),
            Self::F64Bits(values) => values.capacity().saturating_mul(std::mem::size_of::<u64>()),
            Self::Text(values) => values
                .capacity()
                .saturating_mul(std::mem::size_of::<String>())
                .saturating_add(saturating_usize_sum(values.iter().map(String::capacity))),
            Self::Algebraic(column) => {
                std::mem::size_of::<algebraic_native::AlgebraicNativeColumn>()
                    .saturating_add(column.estimated_heap_bytes())
            }
            Self::LiveEntityIds { values, .. } | Self::HistoricalEntityIds { values, .. } => values
                .capacity()
                .saturating_mul(std::mem::size_of::<kernel_types::EntityId>()),
            Self::DenseLiveEntityIds { values, .. } => values
                .capacity()
                .saturating_mul(std::mem::size_of::<LocalEntityId>()),
        }
    }

    fn semantic_work_units_at(&self, index: usize) -> Result<usize, PhysicalExecutionError> {
        if index >= self.len() {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        match self {
            Self::Unit(_)
            | Self::Bool(_)
            | Self::I64(_)
            | Self::F64Bits(_)
            | Self::LiveEntityIds { .. }
            | Self::DenseLiveEntityIds { .. }
            | Self::HistoricalEntityIds { .. } => Ok(1),
            Self::Text(values) => Ok(1_usize.saturating_add(values[index].len())),
            Self::Algebraic(column) => column.semantic_work_units_at(index),
        }
    }

    fn value_at(&self, index: usize) -> Value {
        match self {
            Self::Unit(_) => Value::Unit,
            Self::Bool(values) => Value::Bool(values[index]),
            Self::I64(values) => Value::I64(values[index]),
            Self::F64Bits(values) => Value::F64Bits(values[index]),
            Self::Text(values) => Value::Text(values[index].clone()),
            Self::Algebraic(column) => column
                .value_at(index)
                .expect("validated algebraic native column index"),
            Self::LiveEntityIds {
                entity_type,
                values,
            } => Value::LiveEntityRef {
                entity_type: *entity_type,
                id: values[index],
            },
            Self::DenseLiveEntityIds {
                entity_type,
                ids,
                values,
            } => Value::LiveEntityRef {
                entity_type: *entity_type,
                id: ids
                    .external(values[index])
                    .expect("dense live entity column shares its revision identity map"),
            },
            Self::HistoricalEntityIds {
                entity_type,
                values,
            } => Value::HistoricalEntityId {
                entity_type: *entity_type,
                id: values[index],
            },
        }
    }

    pub fn from_typed_values(
        values: &[Value],
        ty: &kernel_schema::TypeExpr,
    ) -> Result<Self, PhysicalExecutionError> {
        use kernel_schema::{ScalarType, TypeExpr};
        let template = match ty {
            TypeExpr::Scalar(ScalarType::Unit) => Some(Self::Unit(0)),
            TypeExpr::Scalar(ScalarType::Bool) => {
                Some(Self::Bool(PersistentPhysicalVec::default()))
            }
            TypeExpr::Scalar(ScalarType::I64) => Some(Self::I64(PersistentPhysicalVec::default())),
            TypeExpr::Scalar(ScalarType::F64) => {
                Some(Self::F64Bits(PersistentPhysicalVec::default()))
            }
            TypeExpr::Scalar(ScalarType::Text) => {
                Some(Self::Text(PersistentPhysicalVec::default()))
            }
            TypeExpr::Scalar(ScalarType::LiveEntityRef(entity_type)) => Some(Self::LiveEntityIds {
                entity_type: *entity_type,
                values: PersistentPhysicalVec::default(),
            }),
            TypeExpr::Scalar(ScalarType::HistoricalEntityId(entity_type)) => {
                Some(Self::HistoricalEntityIds {
                    entity_type: *entity_type,
                    values: PersistentPhysicalVec::default(),
                })
            }
            _ => None,
        };
        if let Some(template) = template {
            return Self::from_values_like(&template, values.to_vec());
        }
        Self::algebraic(values, ty)
    }

    pub fn algebraic(
        values: &[Value],
        ty: &kernel_schema::TypeExpr,
    ) -> Result<Self, PhysicalExecutionError> {
        algebraic_native::AlgebraicNativeColumn::from_values(values, ty)
            .map(Box::new)
            .map(Self::Algebraic)
    }

    fn from_values_like(
        template: &Self,
        values: Vec<Value>,
    ) -> Result<Self, PhysicalExecutionError> {
        match template {
            Self::Unit(_) => {
                if values.iter().all(|value| matches!(value, Value::Unit)) {
                    Ok(Self::Unit(values.len()))
                } else {
                    Err(PhysicalExecutionError::PhysicalTypeMismatch)
                }
            }
            Self::Bool(_) => values
                .into_iter()
                .map(|value| match value {
                    Value::Bool(value) => Ok(value),
                    _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|values| Self::Bool(values.into())),
            Self::I64(_) => values
                .into_iter()
                .map(|value| match value {
                    Value::I64(value) => Ok(value),
                    _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|values| Self::I64(values.into())),
            Self::F64Bits(_) => values
                .into_iter()
                .map(|value| match value {
                    Value::F64Bits(value) => Ok(value),
                    _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|values| Self::F64Bits(values.into())),
            Self::Text(_) => values
                .into_iter()
                .map(|value| match value {
                    Value::Text(value) => Ok(value),
                    _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|values| Self::Text(values.into())),
            Self::Algebraic(template) => {
                algebraic_native::AlgebraicNativeColumn::from_values(&values, template.ty())
                    .map(Box::new)
                    .map(Self::Algebraic)
            }
            Self::LiveEntityIds { entity_type, .. } => values
                .into_iter()
                .map(|value| match value {
                    Value::LiveEntityRef {
                        entity_type: value_type,
                        id,
                    } if value_type == *entity_type => Ok(id),
                    _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|values| Self::LiveEntityIds {
                    entity_type: *entity_type,
                    values: values.into(),
                }),
            Self::DenseLiveEntityIds {
                entity_type, ids, ..
            } => values
                .into_iter()
                .map(|value| match value {
                    Value::LiveEntityRef {
                        entity_type: value_type,
                        id,
                    } if value_type == *entity_type => ids
                        .local(id)
                        .ok_or(PhysicalExecutionError::PhysicalTypeMismatch),
                    _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|values| Self::DenseLiveEntityIds {
                    entity_type: *entity_type,
                    ids: Arc::clone(ids),
                    values: values.into(),
                }),
            Self::HistoricalEntityIds { entity_type, .. } => values
                .into_iter()
                .map(|value| match value {
                    Value::HistoricalEntityId {
                        entity_type: value_type,
                        id,
                    } if value_type == *entity_type => Ok(id),
                    _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(|values| Self::HistoricalEntityIds {
                    entity_type: *entity_type,
                    values: values.into(),
                }),
        }
    }

    fn select_positions(&self, positions: &[usize]) -> Result<Self, PhysicalExecutionError> {
        Ok(match self {
            Self::Unit(_) => Self::Unit(positions.len()),
            Self::Bool(values) => Self::Bool(select_persistent_positions(values, positions)?),
            Self::I64(values) => Self::I64(select_persistent_positions(values, positions)?),
            Self::F64Bits(values) => Self::F64Bits(select_persistent_positions(values, positions)?),
            Self::Text(values) => Self::Text(select_persistent_positions(values, positions)?),
            Self::Algebraic(column) => {
                Self::Algebraic(Box::new(column.select_positions(positions)?))
            }
            Self::LiveEntityIds {
                entity_type,
                values,
            } => Self::LiveEntityIds {
                entity_type: *entity_type,
                values: select_persistent_positions(values, positions)?,
            },
            Self::DenseLiveEntityIds {
                entity_type,
                ids,
                values,
            } => Self::DenseLiveEntityIds {
                entity_type: *entity_type,
                ids: Arc::clone(ids),
                values: select_persistent_positions(values, positions)?,
            },
            Self::HistoricalEntityIds {
                entity_type,
                values,
            } => Self::HistoricalEntityIds {
                entity_type: *entity_type,
                values: select_persistent_positions(values, positions)?,
            },
        })
    }

    fn swap_remove_at(&mut self, index: usize) -> Result<(), PhysicalExecutionError> {
        match self {
            Self::Unit(len) => *len -= 1,
            Self::Bool(values) => {
                values.swap_remove(index);
            }
            Self::I64(values) => {
                values.swap_remove(index);
            }
            Self::F64Bits(values) => {
                values.swap_remove(index);
            }
            Self::Text(values) => {
                values.swap_remove(index);
            }
            Self::Algebraic(column) => column.swap_remove_at(index)?,
            Self::LiveEntityIds { values, .. } | Self::HistoricalEntityIds { values, .. } => {
                values.swap_remove(index);
            }
            Self::DenseLiveEntityIds { values, .. } => {
                values.swap_remove(index);
            }
        }
        Ok(())
    }

    fn push_value(&mut self, value: &Value) -> Result<(), PhysicalExecutionError> {
        match (self, value) {
            (Self::Unit(len), Value::Unit) => *len += 1,
            (Self::Bool(values), Value::Bool(value)) => values.push(*value),
            (Self::I64(values), Value::I64(value)) => values.push(*value),
            (Self::F64Bits(values), Value::F64Bits(value)) => values.push(*value),
            (Self::Text(values), Value::Text(value)) => values.push(value.clone()),
            (Self::Algebraic(column), value) => column.push_value(value)?,
            (
                Self::LiveEntityIds {
                    entity_type,
                    values,
                },
                Value::LiveEntityRef {
                    entity_type: value_type,
                    id,
                },
            ) if entity_type == value_type => values.push(*id),
            (
                Self::DenseLiveEntityIds {
                    entity_type,
                    ids,
                    values,
                },
                Value::LiveEntityRef {
                    entity_type: value_type,
                    id,
                },
            ) if entity_type == value_type => values.push(
                ids.local(*id)
                    .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?,
            ),
            (
                Self::HistoricalEntityIds {
                    entity_type,
                    values,
                },
                Value::HistoricalEntityId {
                    entity_type: value_type,
                    id,
                },
            ) if entity_type == value_type => values.push(*id),
            _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
        }
        Ok(())
    }
}

fn select_persistent_positions<T: Clone>(
    values: &PersistentPhysicalVec<T>,
    positions: &[usize],
) -> Result<PersistentPhysicalVec<T>, PhysicalExecutionError> {
    positions
        .iter()
        .map(|&position| {
            values
                .get(position)
                .cloned()
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch)
        })
        .collect()
}

