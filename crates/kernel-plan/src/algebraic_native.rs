use std::collections::BTreeMap;

use kernel_model::Value;
use kernel_schema::{ScalarType, TypeExpr, TypeVar};
use kernel_types::SemanticId;

use super::{NativeColumn, PhysicalExecutionError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgebraicNativeColumn {
    ty: TypeExpr,
    storage: AlgebraicStorage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AlgebraicStorage {
    Empty,
    Scalar(NativeColumn),
    Product {
        fields: BTreeMap<SemanticId, AlgebraicNativeColumn>,
        row_count: usize,
    },
    Sum {
        tags: Vec<SemanticId>,
        payload_index: Vec<u32>,
        variants: BTreeMap<SemanticId, AlgebraicNativeColumn>,
    },
    Option {
        payload_index: Vec<Option<u32>>,
        payload: Box<AlgebraicNativeColumn>,
    },
    Seq {
        offsets: Vec<u32>,
        payload: Box<AlgebraicNativeColumn>,
    },
    Set {
        equivalence: SemanticId,
        offsets: Vec<u32>,
        payload: Box<AlgebraicNativeColumn>,
    },
    Bag {
        equivalence: SemanticId,
        offsets: Vec<u32>,
        payload: Box<AlgebraicNativeColumn>,
        counts: Vec<u64>,
    },
    Map {
        key_equivalence: SemanticId,
        offsets: Vec<u32>,
        keys: Box<AlgebraicNativeColumn>,
        values: Box<AlgebraicNativeColumn>,
    },
    Recursive(Box<AlgebraicNativeColumn>),
}

impl AlgebraicNativeColumn {
    pub fn from_values(values: &[Value], ty: &TypeExpr) -> Result<Self, PhysicalExecutionError> {
        ty.validate()
            .map_err(|_| PhysicalExecutionError::PhysicalTypeMismatch)?;
        Self::from_values_inner(values, ty, &BTreeMap::new())
    }

    fn from_values_inner(
        values: &[Value],
        ty: &TypeExpr,
        recursive: &BTreeMap<TypeVar, TypeExpr>,
    ) -> Result<Self, PhysicalExecutionError> {
        if values.is_empty() {
            return Ok(Self {
                ty: ty.clone(),
                storage: AlgebraicStorage::Empty,
            });
        }
        if let TypeExpr::Var(var) = ty {
            let resolved = recursive
                .get(var)
                .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
            return Self::from_values_inner(values, resolved, recursive);
        }
        let storage = build_storage(values, ty, recursive)?;
        Ok(Self {
            ty: ty.clone(),
            storage,
        })
    }

    fn from_validated_value(value: &Value, ty: &TypeExpr) -> Result<Self, PhysicalExecutionError> {
        Self::from_values_inner(std::slice::from_ref(value), ty, &BTreeMap::new())
    }

    #[must_use]
    pub fn ty(&self) -> &TypeExpr {
        &self.ty
    }

    #[must_use]
    pub fn len(&self) -> usize {
        match &self.storage {
            AlgebraicStorage::Empty => 0,
            AlgebraicStorage::Scalar(column) => column.len(),
            AlgebraicStorage::Product { row_count, .. } => *row_count,
            AlgebraicStorage::Sum { tags, .. } => tags.len(),
            AlgebraicStorage::Option { payload_index, .. } => payload_index.len(),
            AlgebraicStorage::Seq { offsets, .. }
            | AlgebraicStorage::Set { offsets, .. }
            | AlgebraicStorage::Bag { offsets, .. }
            | AlgebraicStorage::Map { offsets, .. } => offsets.len().saturating_sub(1),
            AlgebraicStorage::Recursive(child) => child.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn estimated_heap_bytes(&self) -> usize {
        match &self.storage {
            AlgebraicStorage::Empty => 0,
            AlgebraicStorage::Scalar(column) => column.estimated_heap_bytes(),
            AlgebraicStorage::Product { fields, .. } => {
                fields.iter().fold(0, |bytes, (_, child)| {
                    bytes
                        .saturating_add(std::mem::size_of::<(SemanticId, AlgebraicNativeColumn)>())
                        .saturating_add(child.estimated_heap_bytes())
                })
            }
            AlgebraicStorage::Sum {
                tags,
                payload_index,
                variants,
            } => {
                let mut bytes = tags
                    .capacity()
                    .saturating_mul(std::mem::size_of::<SemanticId>())
                    .saturating_add(
                        payload_index
                            .capacity()
                            .saturating_mul(std::mem::size_of::<u32>()),
                    );
                for child in variants.values() {
                    bytes = bytes
                        .saturating_add(std::mem::size_of::<(SemanticId, AlgebraicNativeColumn)>())
                        .saturating_add(child.estimated_heap_bytes());
                }
                bytes
            }
            AlgebraicStorage::Option {
                payload_index,
                payload,
            } => payload_index
                .capacity()
                .saturating_mul(std::mem::size_of::<Option<u32>>())
                .saturating_add(std::mem::size_of::<AlgebraicNativeColumn>())
                .saturating_add(payload.estimated_heap_bytes()),
            AlgebraicStorage::Seq { offsets, payload }
            | AlgebraicStorage::Set {
                offsets, payload, ..
            } => offsets
                .capacity()
                .saturating_mul(std::mem::size_of::<u32>())
                .saturating_add(std::mem::size_of::<AlgebraicNativeColumn>())
                .saturating_add(payload.estimated_heap_bytes()),
            AlgebraicStorage::Bag {
                offsets,
                payload,
                counts,
                ..
            } => offsets
                .capacity()
                .saturating_mul(std::mem::size_of::<u32>())
                .saturating_add(counts.capacity().saturating_mul(std::mem::size_of::<u64>()))
                .saturating_add(std::mem::size_of::<AlgebraicNativeColumn>())
                .saturating_add(payload.estimated_heap_bytes()),
            AlgebraicStorage::Map {
                offsets,
                keys,
                values,
                ..
            } => offsets
                .capacity()
                .saturating_mul(std::mem::size_of::<u32>())
                .saturating_add(2 * std::mem::size_of::<AlgebraicNativeColumn>())
                .saturating_add(keys.estimated_heap_bytes())
                .saturating_add(values.estimated_heap_bytes()),
            AlgebraicStorage::Recursive(child) => std::mem::size_of::<AlgebraicNativeColumn>()
                .saturating_add(child.estimated_heap_bytes()),
        }
    }

    pub(crate) fn semantic_work_units_at(
        &self,
        index: usize,
    ) -> Result<usize, PhysicalExecutionError> {
        match &self.storage {
            AlgebraicStorage::Empty => Err(PhysicalExecutionError::ColumnShapeMismatch),
            AlgebraicStorage::Scalar(column) => column.semantic_work_units_at(index),
            AlgebraicStorage::Product { fields, row_count } => {
                if index >= *row_count {
                    return Err(PhysicalExecutionError::ColumnShapeMismatch);
                }
                fields.values().try_fold(1_usize, |work, child| {
                    Ok(work.saturating_add(child.semantic_work_units_at(index)?))
                })
            }
            AlgebraicStorage::Sum {
                tags,
                payload_index,
                variants,
            } => {
                let tag = *tags
                    .get(index)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                let payload_index = *payload_index
                    .get(index)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                let payload = variants
                    .get(&tag)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                Ok(1_usize.saturating_add(payload.semantic_work_units_at(payload_index as usize)?))
            }
            AlgebraicStorage::Option {
                payload_index,
                payload,
            } => {
                let payload_index = payload_index
                    .get(index)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                payload_index.map_or(Ok(1), |payload_index| {
                    Ok(1_usize
                        .saturating_add(payload.semantic_work_units_at(payload_index as usize)?))
                })
            }
            AlgebraicStorage::Seq { offsets, payload }
            | AlgebraicStorage::Set {
                offsets, payload, ..
            }
            | AlgebraicStorage::Bag {
                offsets, payload, ..
            } => {
                let (start, end) = range(offsets, index)?;
                (start..end).try_fold(1_usize, |work, child_index| {
                    Ok(work.saturating_add(payload.semantic_work_units_at(child_index)?))
                })
            }
            AlgebraicStorage::Map {
                offsets,
                keys,
                values,
                ..
            } => {
                let (start, end) = range(offsets, index)?;
                (start..end).try_fold(1_usize, |work, child_index| {
                    Ok(work
                        .saturating_add(keys.semantic_work_units_at(child_index)?)
                        .saturating_add(values.semantic_work_units_at(child_index)?))
                })
            }
            AlgebraicStorage::Recursive(child) => child.semantic_work_units_at(index),
        }
    }

    pub fn value_at(&self, index: usize) -> Result<Value, PhysicalExecutionError> {
        match &self.storage {
            AlgebraicStorage::Empty => Err(PhysicalExecutionError::ColumnShapeMismatch),
            AlgebraicStorage::Scalar(column) => (index < column.len())
                .then(|| column.value_at(index))
                .ok_or(PhysicalExecutionError::ColumnShapeMismatch),
            AlgebraicStorage::Product { fields, row_count } => {
                if index >= *row_count {
                    return Err(PhysicalExecutionError::ColumnShapeMismatch);
                }
                let mut values = BTreeMap::new();
                for (&field, child) in fields {
                    values.insert(field, child.value_at(index)?);
                }
                Ok(Value::Product(values))
            }
            AlgebraicStorage::Sum {
                tags,
                payload_index,
                variants,
            } => {
                let tag = *tags
                    .get(index)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                let payload_index = *payload_index
                    .get(index)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                let payload = variants
                    .get(&tag)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?
                    .value_at(payload_index as usize)?;
                Ok(Value::Variant {
                    tag,
                    value: Box::new(payload),
                })
            }
            AlgebraicStorage::Option {
                payload_index,
                payload,
            } => {
                let payload_index = payload_index
                    .get(index)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
                Ok(Value::Option(
                    payload_index
                        .map(|payload_index| payload.value_at(payload_index as usize))
                        .transpose()?
                        .map(Box::new),
                ))
            }
            AlgebraicStorage::Seq { offsets, payload } => Ok(Value::Seq(
                values_in_range(offsets, payload, index)?.collect::<Result<Vec<_>, _>>()?,
            )),
            AlgebraicStorage::Set {
                equivalence,
                offsets,
                payload,
            } => Ok(Value::Set {
                equivalence: *equivalence,
                elements: values_in_range(offsets, payload, index)?
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            AlgebraicStorage::Bag {
                equivalence,
                offsets,
                payload,
                counts,
            } => {
                let (start, end) = range(offsets, index)?;
                let mut entries = Vec::with_capacity(end.saturating_sub(start));
                for position in start..end {
                    entries.push((
                        payload.value_at(position)?,
                        *counts
                            .get(position)
                            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?,
                    ));
                }
                Ok(Value::Bag {
                    equivalence: *equivalence,
                    entries,
                })
            }
            AlgebraicStorage::Map {
                key_equivalence,
                offsets,
                keys,
                values,
            } => {
                let (start, end) = range(offsets, index)?;
                let mut entries = Vec::with_capacity(end.saturating_sub(start));
                for position in start..end {
                    entries.push((keys.value_at(position)?, values.value_at(position)?));
                }
                Ok(Value::Map {
                    key_equivalence: *key_equivalence,
                    entries,
                })
            }
            AlgebraicStorage::Recursive(child) => child.value_at(index),
        }
    }

    pub(crate) fn select_positions(
        &self,
        positions: &[usize],
    ) -> Result<Self, PhysicalExecutionError> {
        let storage = match &self.storage {
            AlgebraicStorage::Empty => {
                if positions.is_empty() {
                    AlgebraicStorage::Empty
                } else {
                    return Err(PhysicalExecutionError::ColumnShapeMismatch);
                }
            }
            AlgebraicStorage::Scalar(column) => {
                AlgebraicStorage::Scalar(column.select_positions(positions)?)
            }
            AlgebraicStorage::Product { fields, .. } => AlgebraicStorage::Product {
                fields: fields
                    .iter()
                    .map(|(&field, child)| Ok((field, child.select_positions(positions)?)))
                    .collect::<Result<BTreeMap<_, _>, PhysicalExecutionError>>()?,
                row_count: positions.len(),
            },
            AlgebraicStorage::Sum {
                tags,
                payload_index,
                variants,
            } => select_sum(positions, tags, payload_index, variants)?,
            AlgebraicStorage::Option {
                payload_index,
                payload,
            } => select_option(positions, payload_index, payload)?,
            AlgebraicStorage::Seq { offsets, payload } => {
                let (offsets, flat_positions) = select_ranges(offsets, positions)?;
                AlgebraicStorage::Seq {
                    offsets,
                    payload: Box::new(payload.select_positions(&flat_positions)?),
                }
            }
            AlgebraicStorage::Set {
                equivalence,
                offsets,
                payload,
            } => {
                let (offsets, flat_positions) = select_ranges(offsets, positions)?;
                AlgebraicStorage::Set {
                    equivalence: *equivalence,
                    offsets,
                    payload: Box::new(payload.select_positions(&flat_positions)?),
                }
            }
            AlgebraicStorage::Bag {
                equivalence,
                offsets,
                payload,
                counts,
            } => {
                let (offsets, flat_positions) = select_ranges(offsets, positions)?;
                let selected_counts = flat_positions
                    .iter()
                    .map(|&position| {
                        counts
                            .get(position)
                            .copied()
                            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                AlgebraicStorage::Bag {
                    equivalence: *equivalence,
                    offsets,
                    payload: Box::new(payload.select_positions(&flat_positions)?),
                    counts: selected_counts,
                }
            }
            AlgebraicStorage::Map {
                key_equivalence,
                offsets,
                keys,
                values,
            } => {
                let (offsets, flat_positions) = select_ranges(offsets, positions)?;
                AlgebraicStorage::Map {
                    key_equivalence: *key_equivalence,
                    offsets,
                    keys: Box::new(keys.select_positions(&flat_positions)?),
                    values: Box::new(values.select_positions(&flat_positions)?),
                }
            }
            AlgebraicStorage::Recursive(child) => {
                AlgebraicStorage::Recursive(Box::new(child.select_positions(positions)?))
            }
        };
        Ok(Self {
            ty: self.ty.clone(),
            storage,
        })
    }

    pub(crate) fn swap_remove_at(&mut self, index: usize) -> Result<(), PhysicalExecutionError> {
        if index >= self.len() {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        match &mut self.storage {
            AlgebraicStorage::Scalar(column) => column.swap_remove_at(index),
            AlgebraicStorage::Product { fields, row_count } => {
                for child in fields.values_mut() {
                    child.swap_remove_at(index)?;
                }
                *row_count -= 1;
                Ok(())
            }
            AlgebraicStorage::Recursive(child) => child.swap_remove_at(index),
            _ => {
                let mut positions = (0..self.len()).collect::<Vec<_>>();
                positions.swap_remove(index);
                *self = self.select_positions(&positions)?;
                Ok(())
            }
        }
    }

    pub(crate) fn push_value(&mut self, value: &Value) -> Result<(), PhysicalExecutionError> {
        if !self.accepts_value(value) {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        match (&mut self.storage, value) {
            (AlgebraicStorage::Empty, value) => {
                *self = Self::from_validated_value(value, &self.ty)?;
                Ok(())
            }
            (AlgebraicStorage::Scalar(column), value) => column.push_value(value),
            (AlgebraicStorage::Product { fields, row_count }, Value::Product(values)) => {
                for (&field, child) in fields.iter_mut() {
                    child.push_value(
                        values
                            .get(&field)
                            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?,
                    )?;
                }
                *row_count += 1;
                Ok(())
            }
            (
                AlgebraicStorage::Sum {
                    tags,
                    payload_index,
                    variants,
                },
                Value::Variant { tag, value },
            ) => {
                let child = variants
                    .get_mut(tag)
                    .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
                let next = u32_len(child.len())?;
                child.push_value(value)?;
                tags.push(*tag);
                payload_index.push(next);
                Ok(())
            }
            (
                AlgebraicStorage::Option {
                    payload_index,
                    payload,
                },
                Value::Option(value),
            ) => {
                if let Some(value) = value.as_deref() {
                    let next = u32_len(payload.len())?;
                    payload.push_value(value)?;
                    payload_index.push(Some(next));
                } else {
                    payload_index.push(None);
                }
                Ok(())
            }
            (AlgebraicStorage::Seq { offsets, payload }, Value::Seq(elements))
            | (
                AlgebraicStorage::Set {
                    offsets, payload, ..
                },
                Value::Set { elements, .. },
            ) => {
                push_payload_values(payload, elements)?;
                offsets.push(u32_len(payload.len())?);
                Ok(())
            }
            (
                AlgebraicStorage::Bag {
                    offsets,
                    payload,
                    counts,
                    ..
                },
                Value::Bag { entries, .. },
            ) => {
                for (entry, count) in entries {
                    payload.push_value(entry)?;
                    counts.push(*count);
                }
                offsets.push(u32_len(payload.len())?);
                Ok(())
            }
            (
                AlgebraicStorage::Map {
                    offsets,
                    keys,
                    values,
                    ..
                },
                Value::Map { entries, .. },
            ) => {
                for (key, value) in entries {
                    keys.push_value(key)?;
                    values.push_value(value)?;
                }
                offsets.push(u32_len(keys.len())?);
                Ok(())
            }
            (AlgebraicStorage::Recursive(child), value) => child.push_value(value),
            _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
        }
    }

    #[must_use]
    pub(crate) fn accepts_value(&self, value: &Value) -> bool {
        Self::from_validated_value(value, &self.ty).is_ok()
    }

    #[must_use]
    pub fn field(&self, field: SemanticId) -> Option<&Self> {
        let AlgebraicStorage::Product { fields, .. } = &self.storage else {
            return None;
        };
        fields.get(&field)
    }

    #[must_use]
    pub fn as_i64(&self) -> Option<&[i64]> {
        let AlgebraicStorage::Scalar(NativeColumn::I64(values)) = &self.storage else {
            return None;
        };
        Some(values)
    }

    #[must_use]
    pub fn sum_tags(&self) -> Option<&[SemanticId]> {
        let AlgebraicStorage::Sum { tags, .. } = &self.storage else {
            return None;
        };
        Some(tags)
    }

    #[must_use]
    pub fn option_payload_index(&self) -> Option<&[Option<u32>]> {
        let AlgebraicStorage::Option { payload_index, .. } = &self.storage else {
            return None;
        };
        Some(payload_index)
    }

    #[must_use]
    pub fn option_payload(&self) -> Option<&Self> {
        let AlgebraicStorage::Option { payload, .. } = &self.storage else {
            return None;
        };
        Some(payload)
    }

    pub(crate) fn canonical_key_at_compiled(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
    ) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
        if index >= self.len() {
            return Err(PhysicalExecutionError::ColumnShapeMismatch);
        }
        self.canonical_compiled_node_at(index, compiled, compiled.root_node())
    }

    fn canonical_compiled_node_at(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
        node_index: usize,
    ) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
        use kernel_semantics::{CanonicalEqKey, CompiledEquivalenceNodeRef};
        let (_, node) = compiled
            .node(node_index)
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        match node {
            CompiledEquivalenceNodeRef::Primitive(resolved) => {
                let AlgebraicStorage::Scalar(column) = &self.storage else {
                    return Err(PhysicalExecutionError::PhysicalTypeMismatch);
                };
                resolved
                    .canonical_key(&column.value_at(index))
                    .map_err(Into::into)
            }
            CompiledEquivalenceNodeRef::Mu { body } => {
                let AlgebraicStorage::Recursive(child) = &self.storage else {
                    return Err(PhysicalExecutionError::PhysicalTypeMismatch);
                };
                child.canonical_compiled_node_at(index, compiled, body)
            }
            CompiledEquivalenceNodeRef::Var { binder } => {
                let AlgebraicStorage::Recursive(child) = &self.storage else {
                    return Err(PhysicalExecutionError::PhysicalTypeMismatch);
                };
                child.canonical_compiled_node_at(index, compiled, binder)
            }
            CompiledEquivalenceNodeRef::Product(fields) => {
                self.canonical_compiled_product_at(index, compiled, fields)
            }
            CompiledEquivalenceNodeRef::Option { inner } => {
                self.canonical_compiled_option_at(index, compiled, inner)
            }
            CompiledEquivalenceNodeRef::Sum(variants) => {
                self.canonical_compiled_sum_at(index, compiled, variants)
            }
            CompiledEquivalenceNodeRef::Seq { element } => self
                .canonical_compiled_range_at(index, compiled, element)
                .map(CanonicalEqKey::Seq),
            CompiledEquivalenceNodeRef::Set { element } => self
                .canonical_compiled_range_at(index, compiled, element)
                .map(kernel_semantics::finite_measure_from_atoms)
                .map(CanonicalEqKey::Set),
            CompiledEquivalenceNodeRef::Bag { element } => {
                self.canonical_compiled_bag_at(index, compiled, element)
            }
            CompiledEquivalenceNodeRef::Map { key, value } => {
                self.canonical_compiled_map_at(index, compiled, key, value)
            }
        }
    }

    fn canonical_compiled_product_at(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
        compiled_fields: &[(SemanticId, usize)],
    ) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
        let AlgebraicStorage::Product { fields, row_count } = &self.storage else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        if index >= *row_count || fields.len() != compiled_fields.len() {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        compiled_fields
            .iter()
            .map(|(field, child_node)| {
                let child = fields
                    .get(field)
                    .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
                Ok((
                    *field,
                    child.canonical_compiled_node_at(index, compiled, *child_node)?,
                ))
            })
            .collect::<Result<Vec<_>, PhysicalExecutionError>>()
            .map(kernel_semantics::CanonicalEqKey::Product)
    }

    fn canonical_compiled_option_at(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
        inner: usize,
    ) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
        use kernel_semantics::CanonicalEqKey;
        let AlgebraicStorage::Option {
            payload_index,
            payload,
        } = &self.storage
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        match payload_index
            .get(index)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?
        {
            None => Ok(CanonicalEqKey::OptionNone),
            Some(payload_index) => payload
                .canonical_compiled_node_at(*payload_index as usize, compiled, inner)
                .map(Box::new)
                .map(CanonicalEqKey::OptionSome),
        }
    }

    fn canonical_compiled_sum_at(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
        variants: &BTreeMap<SemanticId, usize>,
    ) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
        let AlgebraicStorage::Sum {
            tags,
            payload_index,
            variants: columns,
        } = &self.storage
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        let tag = *tags
            .get(index)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
        let payload_index = *payload_index
            .get(index)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
        let child_node = *variants
            .get(&tag)
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        let child = columns
            .get(&tag)
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        Ok(kernel_semantics::CanonicalEqKey::Variant {
            tag,
            value: Box::new(child.canonical_compiled_node_at(
                payload_index as usize,
                compiled,
                child_node,
            )?),
        })
    }

    fn canonical_compiled_bag_at(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
        element: usize,
    ) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
        let AlgebraicStorage::Bag {
            offsets,
            payload,
            counts,
            ..
        } = &self.storage
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        let (start, end) = range(offsets, index)?;
        let mut atoms = Vec::with_capacity(end.saturating_sub(start));
        for position in start..end {
            atoms.push(kernel_semantics::CanonicalBagAtom {
                value: payload.canonical_compiled_node_at(position, compiled, element)?,
                stored_count: *counts
                    .get(position)
                    .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?,
            });
        }
        Ok(kernel_semantics::CanonicalEqKey::Bag(
            kernel_semantics::finite_measure_from_atoms(atoms),
        ))
    }

    fn canonical_compiled_map_at(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
        key: usize,
        value: usize,
    ) -> Result<kernel_semantics::CanonicalEqKey, PhysicalExecutionError> {
        let AlgebraicStorage::Map {
            offsets,
            keys,
            values,
            ..
        } = &self.storage
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        let (start, end) = range(offsets, index)?;
        let mut atoms = Vec::with_capacity(end.saturating_sub(start));
        for position in start..end {
            atoms.push(kernel_semantics::CanonicalMapAtom {
                key: keys.canonical_compiled_node_at(position, compiled, key)?,
                value: values.canonical_compiled_node_at(position, compiled, value)?,
            });
        }
        Ok(kernel_semantics::CanonicalEqKey::Map(
            kernel_semantics::finite_measure_from_atoms(atoms),
        ))
    }

    fn canonical_compiled_range_at(
        &self,
        index: usize,
        compiled: &kernel_semantics::CompiledEquivalence,
        element: usize,
    ) -> Result<Vec<kernel_semantics::CanonicalEqKey>, PhysicalExecutionError> {
        let (offsets, payload) = match &self.storage {
            AlgebraicStorage::Seq { offsets, payload }
            | AlgebraicStorage::Set {
                offsets, payload, ..
            } => (offsets, payload.as_ref()),
            _ => return Err(PhysicalExecutionError::PhysicalTypeMismatch),
        };
        let (start, end) = range(offsets, index)?;
        let keys = (start..end)
            .map(|position| payload.canonical_compiled_node_at(position, compiled, element))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(keys)
    }
}

fn select_sum(
    positions: &[usize],
    tags: &[SemanticId],
    payload_index: &[u32],
    variants: &BTreeMap<SemanticId, AlgebraicNativeColumn>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut selected_tags = Vec::with_capacity(positions.len());
    let mut selected_payload_index = Vec::with_capacity(positions.len());
    let mut per_variant_positions = variants
        .keys()
        .copied()
        .map(|tag| (tag, Vec::<usize>::new()))
        .collect::<BTreeMap<_, _>>();
    for &position in positions {
        let tag = *tags
            .get(position)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
        let payload_position = *payload_index
            .get(position)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?
            as usize;
        let selected = per_variant_positions
            .get_mut(&tag)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?;
        selected_payload_index.push(u32_len(selected.len())?);
        selected_tags.push(tag);
        selected.push(payload_position);
    }
    let selected_variants = variants
        .iter()
        .map(|(&tag, child)| {
            Ok((
                tag,
                child.select_positions(
                    per_variant_positions
                        .get(&tag)
                        .expect("variant selection bucket exists"),
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, PhysicalExecutionError>>()?;
    Ok(AlgebraicStorage::Sum {
        tags: selected_tags,
        payload_index: selected_payload_index,
        variants: selected_variants,
    })
}

fn select_option(
    positions: &[usize],
    payload_index: &[Option<u32>],
    payload: &AlgebraicNativeColumn,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut selected_payload_index = Vec::with_capacity(positions.len());
    let mut payload_positions = Vec::new();
    for &position in positions {
        match payload_index
            .get(position)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)?
        {
            Some(payload_position) => {
                selected_payload_index.push(Some(u32_len(payload_positions.len())?));
                payload_positions.push(*payload_position as usize);
            }
            None => selected_payload_index.push(None),
        }
    }
    Ok(AlgebraicStorage::Option {
        payload_index: selected_payload_index,
        payload: Box::new(payload.select_positions(&payload_positions)?),
    })
}

fn select_ranges(
    offsets: &[u32],
    positions: &[usize],
) -> Result<(Vec<u32>, Vec<usize>), PhysicalExecutionError> {
    let mut selected_offsets = Vec::with_capacity(positions.len() + 1);
    let mut flat_positions = Vec::new();
    selected_offsets.push(0);
    for &position in positions {
        let (start, end) = range(offsets, position)?;
        flat_positions.extend(start..end);
        selected_offsets.push(u32_len(flat_positions.len())?);
    }
    Ok((selected_offsets, flat_positions))
}

fn push_payload_values(
    payload: &mut AlgebraicNativeColumn,
    values: &[Value],
) -> Result<(), PhysicalExecutionError> {
    for value in values {
        payload.push_value(value)?;
    }
    Ok(())
}

fn build_storage(
    values: &[Value],
    ty: &TypeExpr,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    match ty {
        TypeExpr::Scalar(scalar) => build_scalar(values, scalar).map(AlgebraicStorage::Scalar),
        TypeExpr::Product(field_types) => build_product(values, field_types, recursive),
        TypeExpr::Sum(variant_types) => build_sum(values, variant_types, recursive),
        TypeExpr::Option(inner) => build_option(values, inner, recursive),
        TypeExpr::Seq(inner) => {
            let (offsets, payload_values) = flatten_sequences(values)?;
            Ok(AlgebraicStorage::Seq {
                offsets,
                payload: Box::new(AlgebraicNativeColumn::from_values_inner(
                    &payload_values,
                    inner,
                    recursive,
                )?),
            })
        }
        TypeExpr::Set {
            element,
            equivalence,
        } => build_set(values, element, *equivalence, recursive),
        TypeExpr::Bag {
            element,
            equivalence,
        } => build_bag(values, element, *equivalence, recursive),
        TypeExpr::Map {
            key,
            value,
            key_equivalence,
        } => build_map(values, key, value, *key_equivalence, recursive),
        TypeExpr::Mu { binder, body } => {
            let mut nested = recursive.clone();
            nested.insert(*binder, ty.clone());
            Ok(AlgebraicStorage::Recursive(Box::new(
                AlgebraicNativeColumn::from_values_inner(values, body, &nested)?,
            )))
        }
        TypeExpr::Var(_) => unreachable!("TypeExpr::Var is resolved before build_storage"),
    }
}

fn build_product(
    values: &[Value],
    field_types: &BTreeMap<SemanticId, TypeExpr>,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut fields = BTreeMap::new();
    for (&field, field_type) in field_types {
        let mut child_values = Vec::with_capacity(values.len());
        for value in values {
            let Value::Product(product) = value else {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            };
            if product.len() != field_types.len() {
                return Err(PhysicalExecutionError::PhysicalTypeMismatch);
            }
            child_values.push(
                product
                    .get(&field)
                    .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?
                    .clone(),
            );
        }
        fields.insert(
            field,
            AlgebraicNativeColumn::from_values_inner(&child_values, field_type, recursive)?,
        );
    }
    Ok(AlgebraicStorage::Product {
        fields,
        row_count: values.len(),
    })
}

fn build_sum(
    values: &[Value],
    variant_types: &BTreeMap<SemanticId, TypeExpr>,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut by_tag: BTreeMap<SemanticId, Vec<Value>> = variant_types
        .keys()
        .copied()
        .map(|tag| (tag, Vec::new()))
        .collect();
    let mut tags = Vec::with_capacity(values.len());
    let mut payload_index = Vec::with_capacity(values.len());
    for value in values {
        let Value::Variant { tag, value } = value else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        let payloads = by_tag
            .get_mut(tag)
            .ok_or(PhysicalExecutionError::PhysicalTypeMismatch)?;
        payload_index.push(u32_len(payloads.len())?);
        tags.push(*tag);
        payloads.push((**value).clone());
    }
    let mut variants = BTreeMap::new();
    for (&tag, variant_type) in variant_types {
        variants.insert(
            tag,
            AlgebraicNativeColumn::from_values_inner(
                by_tag.get(&tag).expect("variant bucket exists"),
                variant_type,
                recursive,
            )?,
        );
    }
    Ok(AlgebraicStorage::Sum {
        tags,
        payload_index,
        variants,
    })
}

fn build_option(
    values: &[Value],
    inner: &TypeExpr,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut payload_index = Vec::with_capacity(values.len());
    let mut payload_values = Vec::new();
    for value in values {
        let Value::Option(value) = value else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        if let Some(value) = value.as_deref() {
            payload_index.push(Some(u32_len(payload_values.len())?));
            payload_values.push(value.clone());
        } else {
            payload_index.push(None);
        }
    }
    Ok(AlgebraicStorage::Option {
        payload_index,
        payload: Box::new(AlgebraicNativeColumn::from_values_inner(
            &payload_values,
            inner,
            recursive,
        )?),
    })
}

fn build_set(
    values: &[Value],
    element: &TypeExpr,
    equivalence: SemanticId,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut offsets = vec![0];
    let mut payload_values = Vec::new();
    for value in values {
        let Value::Set {
            equivalence: actual,
            elements,
        } = value
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        if *actual != equivalence {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        payload_values.extend(elements.iter().cloned());
        offsets.push(u32_len(payload_values.len())?);
    }
    Ok(AlgebraicStorage::Set {
        equivalence,
        offsets,
        payload: Box::new(AlgebraicNativeColumn::from_values_inner(
            &payload_values,
            element,
            recursive,
        )?),
    })
}

fn build_bag(
    values: &[Value],
    element: &TypeExpr,
    equivalence: SemanticId,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut offsets = vec![0];
    let mut payload_values = Vec::new();
    let mut counts = Vec::new();
    for value in values {
        let Value::Bag {
            equivalence: actual,
            entries,
        } = value
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        if *actual != equivalence {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        for (entry, count) in entries {
            payload_values.push(entry.clone());
            counts.push(*count);
        }
        offsets.push(u32_len(payload_values.len())?);
    }
    Ok(AlgebraicStorage::Bag {
        equivalence,
        offsets,
        payload: Box::new(AlgebraicNativeColumn::from_values_inner(
            &payload_values,
            element,
            recursive,
        )?),
        counts,
    })
}

fn build_map(
    inputs: &[Value],
    key_type: &TypeExpr,
    value_type: &TypeExpr,
    key_equivalence: SemanticId,
    recursive: &BTreeMap<TypeVar, TypeExpr>,
) -> Result<AlgebraicStorage, PhysicalExecutionError> {
    let mut offsets = vec![0];
    let mut keys = Vec::new();
    let mut values = Vec::new();
    for input in inputs {
        let Value::Map {
            key_equivalence: actual,
            entries,
        } = input
        else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        if *actual != key_equivalence {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        }
        for (key, value) in entries {
            keys.push(key.clone());
            values.push(value.clone());
        }
        offsets.push(u32_len(keys.len())?);
    }
    Ok(AlgebraicStorage::Map {
        key_equivalence,
        offsets,
        keys: Box::new(AlgebraicNativeColumn::from_values_inner(
            &keys, key_type, recursive,
        )?),
        values: Box::new(AlgebraicNativeColumn::from_values_inner(
            &values, value_type, recursive,
        )?),
    })
}

fn flatten_sequences(values: &[Value]) -> Result<(Vec<u32>, Vec<Value>), PhysicalExecutionError> {
    let mut offsets = Vec::with_capacity(values.len() + 1);
    let mut payload_values = Vec::new();
    offsets.push(0);
    for value in values {
        let Value::Seq(elements) = value else {
            return Err(PhysicalExecutionError::PhysicalTypeMismatch);
        };
        payload_values.extend(elements.iter().cloned());
        offsets.push(u32_len(payload_values.len())?);
    }
    Ok((offsets, payload_values))
}

fn u32_len(len: usize) -> Result<u32, PhysicalExecutionError> {
    u32::try_from(len).map_err(|_| PhysicalExecutionError::ColumnShapeMismatch)
}

fn range(offsets: &[u32], index: usize) -> Result<(usize, usize), PhysicalExecutionError> {
    Ok((
        *offsets
            .get(index)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)? as usize,
        *offsets
            .get(index + 1)
            .ok_or(PhysicalExecutionError::ColumnShapeMismatch)? as usize,
    ))
}

fn values_in_range<'a>(
    offsets: &'a [u32],
    payload: &'a AlgebraicNativeColumn,
    index: usize,
) -> Result<impl Iterator<Item = Result<Value, PhysicalExecutionError>> + 'a, PhysicalExecutionError>
{
    let (start, end) = range(offsets, index)?;
    Ok((start..end).map(|position| payload.value_at(position)))
}

fn build_scalar(
    values: &[Value],
    scalar: &ScalarType,
) -> Result<NativeColumn, PhysicalExecutionError> {
    match scalar {
        ScalarType::Unit if values.iter().all(|value| matches!(value, Value::Unit)) => {
            Ok(NativeColumn::Unit(values.len()))
        }
        ScalarType::Bool => values
            .iter()
            .map(|value| match value {
                Value::Bool(value) => Ok(*value),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(NativeColumn::Bool),
        ScalarType::I64 => values
            .iter()
            .map(|value| match value {
                Value::I64(value) => Ok(*value),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(NativeColumn::I64),
        ScalarType::F64 => values
            .iter()
            .map(|value| match value {
                Value::F64Bits(value) => Ok(*value),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(NativeColumn::F64Bits),
        ScalarType::Text => values
            .iter()
            .map(|value| match value {
                Value::Text(value) => Ok(value.clone()),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(NativeColumn::Text),
        ScalarType::LiveEntityRef(entity_type) => values
            .iter()
            .map(|value| match value {
                Value::LiveEntityRef {
                    entity_type: actual,
                    id,
                } if actual == entity_type => Ok(*id),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| NativeColumn::LiveEntityIds {
                entity_type: *entity_type,
                values,
            }),
        ScalarType::HistoricalEntityId(entity_type) => values
            .iter()
            .map(|value| match value {
                Value::HistoricalEntityId {
                    entity_type: actual,
                    id,
                } if actual == entity_type => Ok(*id),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| NativeColumn::HistoricalEntityIds {
                entity_type: *entity_type,
                values,
            }),
        ScalarType::Unit => Err(PhysicalExecutionError::PhysicalTypeMismatch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_non_recursive_algebraic_constructors_round_trip() {
        let a = SemanticId::new(1);
        let b = SemanticId::new(2);
        let left = SemanticId::new(3);
        let right = SemanticId::new(4);
        let eq = SemanticId::new(50);

        let cases = vec![
            (
                TypeExpr::Product(BTreeMap::from([
                    (a, TypeExpr::Scalar(ScalarType::I64)),
                    (
                        b,
                        TypeExpr::Option(Box::new(TypeExpr::Scalar(ScalarType::Text))),
                    ),
                ])),
                vec![Value::Product(BTreeMap::from([
                    (a, Value::I64(7)),
                    (b, Value::Option(Some(Box::new(Value::Text("x".into()))))),
                ]))],
            ),
            (
                TypeExpr::Sum(BTreeMap::from([
                    (left, TypeExpr::Scalar(ScalarType::I64)),
                    (right, TypeExpr::Scalar(ScalarType::Text)),
                ])),
                vec![
                    Value::Variant {
                        tag: left,
                        value: Box::new(Value::I64(9)),
                    },
                    Value::Variant {
                        tag: right,
                        value: Box::new(Value::Text("r".into())),
                    },
                ],
            ),
            (
                TypeExpr::Seq(Box::new(TypeExpr::Scalar(ScalarType::I64))),
                vec![Value::Seq(vec![Value::I64(1), Value::I64(2)])],
            ),
            (
                TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    equivalence: eq,
                },
                vec![Value::Set {
                    equivalence: eq,
                    elements: vec![Value::I64(1), Value::I64(2)],
                }],
            ),
            (
                TypeExpr::Bag {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: eq,
                },
                vec![Value::Bag {
                    equivalence: eq,
                    entries: vec![(Value::Text("a".into()), 3)],
                }],
            ),
            (
                TypeExpr::Map {
                    key: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    value: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    key_equivalence: eq,
                },
                vec![Value::Map {
                    key_equivalence: eq,
                    entries: vec![(Value::Text("a".into()), Value::I64(3))],
                }],
            ),
        ];

        for (ty, rows) in cases {
            let native = AlgebraicNativeColumn::from_values(&rows, &ty).unwrap();
            assert_eq!(native.len(), rows.len());
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(native.value_at(index).unwrap(), *row);
            }
        }
    }

    fn assert_variable_cardinality_mutation(ty: &TypeExpr, rows: &[Value]) {
        let native = AlgebraicNativeColumn::from_values(rows, ty).unwrap();
        let mut native = native.select_positions(&[2, 0]).unwrap();
        native.push_value(&rows[1]).unwrap();
        native.swap_remove_at(1).unwrap();
        assert_eq!(native.len(), 2);
        assert_eq!(native.value_at(0).unwrap(), rows[2]);
        assert_eq!(native.value_at(1).unwrap(), rows[1]);
    }

    #[test]
    fn option_sum_and_seq_mutation_stay_column_native() {
        let left = SemanticId::new(10);
        let right = SemanticId::new(11);
        let cases = vec![
            (
                TypeExpr::Option(Box::new(TypeExpr::Scalar(ScalarType::I64))),
                vec![
                    Value::Option(Some(Box::new(Value::I64(1)))),
                    Value::Option(None),
                    Value::Option(Some(Box::new(Value::I64(3)))),
                ],
            ),
            (
                TypeExpr::Sum(BTreeMap::from([
                    (left, TypeExpr::Scalar(ScalarType::I64)),
                    (right, TypeExpr::Scalar(ScalarType::Text)),
                ])),
                vec![
                    Value::Variant {
                        tag: left,
                        value: Box::new(Value::I64(1)),
                    },
                    Value::Variant {
                        tag: right,
                        value: Box::new(Value::Text("two".into())),
                    },
                    Value::Variant {
                        tag: left,
                        value: Box::new(Value::I64(3)),
                    },
                ],
            ),
            (
                TypeExpr::Seq(Box::new(TypeExpr::Scalar(ScalarType::I64))),
                vec![
                    Value::Seq(vec![Value::I64(1)]),
                    Value::Seq(vec![Value::I64(2), Value::I64(20)]),
                    Value::Seq(vec![Value::I64(3), Value::I64(30), Value::I64(300)]),
                ],
            ),
        ];
        for (ty, rows) in cases {
            assert_variable_cardinality_mutation(&ty, &rows);
        }
    }

    #[test]
    fn set_bag_and_map_mutation_stay_column_native() {
        let eq = SemanticId::new(12);
        let cases = vec![
            (
                TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    equivalence: eq,
                },
                vec![
                    Value::Set {
                        equivalence: eq,
                        elements: vec![Value::I64(1)],
                    },
                    Value::Set {
                        equivalence: eq,
                        elements: vec![Value::I64(2), Value::I64(20)],
                    },
                    Value::Set {
                        equivalence: eq,
                        elements: vec![Value::I64(3)],
                    },
                ],
            ),
            (
                TypeExpr::Bag {
                    element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    equivalence: eq,
                },
                vec![
                    Value::Bag {
                        equivalence: eq,
                        entries: vec![(Value::I64(1), 1)],
                    },
                    Value::Bag {
                        equivalence: eq,
                        entries: vec![(Value::I64(2), 2), (Value::I64(20), 1)],
                    },
                    Value::Bag {
                        equivalence: eq,
                        entries: vec![(Value::I64(3), 3)],
                    },
                ],
            ),
            (
                TypeExpr::Map {
                    key: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    value: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    key_equivalence: eq,
                },
                vec![
                    Value::Map {
                        key_equivalence: eq,
                        entries: vec![(Value::I64(1), Value::Text("one".into()))],
                    },
                    Value::Map {
                        key_equivalence: eq,
                        entries: vec![(Value::I64(2), Value::Text("two".into()))],
                    },
                    Value::Map {
                        key_equivalence: eq,
                        entries: vec![(Value::I64(3), Value::Text("three".into()))],
                    },
                ],
            ),
        ];

        for (ty, rows) in cases {
            assert_variable_cardinality_mutation(&ty, &rows);
        }
    }

    #[test]
    fn guarded_recursive_sum_round_trips() {
        let binder = TypeVar(1);
        let nil = SemanticId::new(1);
        let cons = SemanticId::new(2);
        let head = SemanticId::new(3);
        let tail = SemanticId::new(4);
        let ty = TypeExpr::Mu {
            binder,
            body: Box::new(TypeExpr::Sum(BTreeMap::from([
                (nil, TypeExpr::Scalar(ScalarType::Unit)),
                (
                    cons,
                    TypeExpr::Product(BTreeMap::from([
                        (head, TypeExpr::Scalar(ScalarType::I64)),
                        (tail, TypeExpr::Option(Box::new(TypeExpr::Var(binder)))),
                    ])),
                ),
            ]))),
        };
        let nil_value = Value::Variant {
            tag: nil,
            value: Box::new(Value::Unit),
        };
        let one = Value::Variant {
            tag: cons,
            value: Box::new(Value::Product(BTreeMap::from([
                (head, Value::I64(1)),
                (tail, Value::Option(Some(Box::new(nil_value.clone())))),
            ]))),
        };
        let native =
            AlgebraicNativeColumn::from_values(&[nil_value.clone(), one.clone()], &ty).unwrap();
        assert_eq!(native.value_at(0).unwrap(), nil_value);
        assert_eq!(native.value_at(1).unwrap(), one);
    }

    #[test]
    fn hostile_rejects_wrong_semantic_tags_and_empty_recursive_is_finite() {
        let eq = SemanticId::new(10);
        let wrong_eq = SemanticId::new(11);
        let set_ty = TypeExpr::Set {
            element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
            equivalence: eq,
        };
        assert_eq!(
            AlgebraicNativeColumn::from_values(
                &[Value::Set {
                    equivalence: wrong_eq,
                    elements: vec![Value::I64(1)],
                }],
                &set_ty,
            ),
            Err(PhysicalExecutionError::PhysicalTypeMismatch)
        );

        let known = SemanticId::new(20);
        let unknown = SemanticId::new(21);
        let sum_ty = TypeExpr::Sum(BTreeMap::from([(known, TypeExpr::Scalar(ScalarType::I64))]));
        assert_eq!(
            AlgebraicNativeColumn::from_values(
                &[Value::Variant {
                    tag: unknown,
                    value: Box::new(Value::I64(1)),
                }],
                &sum_ty,
            ),
            Err(PhysicalExecutionError::PhysicalTypeMismatch)
        );

        let binder = TypeVar(9);
        let recursive = TypeExpr::Mu {
            binder,
            body: Box::new(TypeExpr::Seq(Box::new(TypeExpr::Var(binder)))),
        };
        let empty = AlgebraicNativeColumn::from_values(&[], &recursive).unwrap();
        assert!(empty.is_empty());

        let unguarded = TypeExpr::Mu {
            binder,
            body: Box::new(TypeExpr::Var(binder)),
        };
        assert_eq!(
            AlgebraicNativeColumn::from_values(&[Value::Seq(Vec::new())], &unguarded),
            Err(PhysicalExecutionError::PhysicalTypeMismatch)
        );
    }

    struct ComposedAlgebraFixture {
        context: kernel_schema::SemanticContext,
        registry: kernel_semantics::SemanticRegistry,
        product_equivalence: SemanticId,
        element_equivalence: SemanticId,
        fields: [SemanticId; 6],
        text_tag: SemanticId,
    }

    fn composed_algebra_fixture() -> ComposedAlgebraFixture {
        use kernel_schema::{Schema, SemanticEnvironment, StructuralEquivalenceDef};
        use kernel_semantics::{EquivalenceModule, SemanticRegistry};
        use kernel_types::{SchemaRevisionId, SemanticEnvId};

        let ci = SemanticId::new(90_000);
        let option_eq = SemanticId::new(90_001);
        let sequence_eq = SemanticId::new(90_002);
        let set_equivalence = SemanticId::new(90_003);
        let bag_eq = SemanticId::new(90_004);
        let map_eq = SemanticId::new(90_005);
        let sum_eq = SemanticId::new(90_006);
        let product_eq = SemanticId::new(90_007);
        let fields = std::array::from_fn(|index| SemanticId::new(90_010 + index as u128));
        let text_tag = SemanticId::new(90_020);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(90_000));
        environment.pin_module(ci, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(90_000));
        for (id, definition) in [
            (option_eq, StructuralEquivalenceDef::Option { inner: ci }),
            (sequence_eq, StructuralEquivalenceDef::Seq { element: ci }),
            (
                set_equivalence,
                StructuralEquivalenceDef::Set { element: ci },
            ),
            (bag_eq, StructuralEquivalenceDef::Bag { element: ci }),
            (map_eq, StructuralEquivalenceDef::Map { key: ci, value: ci }),
            (
                sum_eq,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(text_tag, ci)]),
                },
            ),
        ] {
            schema
                .define_structural_equivalence(id, definition)
                .unwrap();
        }
        schema
            .define_structural_equivalence(
                product_eq,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([
                        (fields[0], option_eq),
                        (fields[1], sequence_eq),
                        (fields[2], set_equivalence),
                        (fields[3], bag_eq),
                        (fields[4], map_eq),
                        (fields[5], sum_eq),
                    ]),
                },
            )
            .unwrap();
        ComposedAlgebraFixture {
            context: kernel_schema::SemanticContext {
                schema,
                environment,
            },
            registry,
            product_equivalence: product_eq,
            element_equivalence: ci,
            fields,
            text_tag,
        }
    }

    fn composed_algebra_type(fixture: &ComposedAlgebraFixture) -> TypeExpr {
        let text = TypeExpr::Scalar(ScalarType::Text);
        TypeExpr::Product(BTreeMap::from([
            (fixture.fields[0], TypeExpr::Option(Box::new(text.clone()))),
            (fixture.fields[1], TypeExpr::Seq(Box::new(text.clone()))),
            (
                fixture.fields[2],
                TypeExpr::Set {
                    element: Box::new(text.clone()),
                    equivalence: fixture.element_equivalence,
                },
            ),
            (
                fixture.fields[3],
                TypeExpr::Bag {
                    element: Box::new(text.clone()),
                    equivalence: fixture.element_equivalence,
                },
            ),
            (
                fixture.fields[4],
                TypeExpr::Map {
                    key: Box::new(text.clone()),
                    value: Box::new(text.clone()),
                    key_equivalence: fixture.element_equivalence,
                },
            ),
            (
                fixture.fields[5],
                TypeExpr::Sum(BTreeMap::from([(fixture.text_tag, text)])),
            ),
        ]))
    }

    fn composed_algebra_row(fixture: &ComposedAlgebraFixture, upper: bool) -> Value {
        let (a, b) = if upper {
            ("ALPHA", "BETA")
        } else {
            ("alpha", "beta")
        };
        let eq = fixture.element_equivalence;
        Value::Product(BTreeMap::from([
            (
                fixture.fields[0],
                Value::Option(Some(Box::new(Value::Text(a.into())))),
            ),
            (
                fixture.fields[1],
                Value::Seq(vec![Value::Text(a.into()), Value::Text(b.into())]),
            ),
            (
                fixture.fields[2],
                Value::Set {
                    equivalence: eq,
                    elements: vec![Value::Text(b.into()), Value::Text(a.into())],
                },
            ),
            (
                fixture.fields[3],
                Value::Bag {
                    equivalence: eq,
                    entries: vec![(Value::Text(b.into()), 1), (Value::Text(a.into()), 2)],
                },
            ),
            (
                fixture.fields[4],
                Value::Map {
                    key_equivalence: eq,
                    entries: vec![(Value::Text(a.into()), Value::Text(b.into()))],
                },
            ),
            (
                fixture.fields[5],
                Value::Variant {
                    tag: fixture.text_tag,
                    value: Box::new(Value::Text(a.into())),
                },
            ),
        ]))
    }

    #[test]
    fn native_structural_canonical_key_matches_registry_for_composed_algebra() {
        let fixture = composed_algebra_fixture();
        let rows = vec![
            composed_algebra_row(&fixture, false),
            composed_algebra_row(&fixture, true),
        ];
        let native =
            AlgebraicNativeColumn::from_values(&rows, &composed_algebra_type(&fixture)).unwrap();
        let compiled = fixture
            .registry
            .compile_equivalence(&fixture.context, fixture.product_equivalence)
            .unwrap();
        let mut keys = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let native_key = native.canonical_key_at_compiled(index, &compiled).unwrap();
            let registry_key = fixture
                .registry
                .canonical_equivalence_key(&fixture.context, fixture.product_equivalence, row)
                .unwrap();
            assert_eq!(native_key, registry_key);
            keys.push(native_key);
        }
        assert_eq!(keys[0], keys[1]);
    }

    #[test]
    fn selection_push_and_swap_remove_preserve_exact_values() {
        let field = SemanticId::new(1);
        let ty = TypeExpr::Product(BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::I64))]));
        let row = |value| Value::Product(BTreeMap::from([(field, Value::I64(value))]));
        let mut native =
            AlgebraicNativeColumn::from_values(&[row(1), row(2), row(3)], &ty).unwrap();
        let selected = native.select_positions(&[2, 0]).unwrap();
        assert_eq!(selected.value_at(0).unwrap(), row(3));
        assert_eq!(selected.value_at(1).unwrap(), row(1));
        native.swap_remove_at(1).unwrap();
        assert_eq!(native.value_at(0).unwrap(), row(1));
        assert_eq!(native.value_at(1).unwrap(), row(3));
        native.push_value(&row(4)).unwrap();
        assert_eq!(native.value_at(2).unwrap(), row(4));
    }

    #[test]
    fn typed_relation_compiler_keeps_scalars_native_and_structures_algebraic() {
        let field = SemanticId::new(1);
        let product_ty =
            TypeExpr::Product(BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::I64))]));
        let column_types = vec![product_ty, TypeExpr::Scalar(ScalarType::I64)];
        let product = |value| Value::Product(BTreeMap::from([(field, Value::I64(value))]));
        let rows = vec![
            vec![product(1), Value::I64(10)],
            vec![product(2), Value::I64(20)],
        ];
        let mut relation = crate::NativeRelation::typed_from_rows(&rows, &column_types).unwrap();
        let crate::NativeRelation::TypedColumnar { columns, .. } = &relation else {
            panic!("typed compiler must produce TypedColumnar");
        };
        assert!(matches!(columns[0], crate::NativeColumn::Algebraic(_)));
        assert!(matches!(columns[1], crate::NativeColumn::I64(_)));
        assert_eq!(
            crate::materialize_native_row(&relation, 0).unwrap(),
            rows[0]
        );
        crate::remove_native_row(&mut relation, 0).unwrap();
        assert_eq!(
            crate::materialize_native_row(&relation, 0).unwrap(),
            rows[1]
        );
        crate::push_native_row(&mut relation, &vec![product(3), Value::I64(30)]).unwrap();
        assert_eq!(
            crate::materialize_native_row(&relation, 1).unwrap(),
            vec![product(3), Value::I64(30)]
        );
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_sum_tag_filter_against_boxed_logical_variants() {
        use std::hint::black_box;
        use std::time::Instant;

        let left = SemanticId::new(1);
        let right = SemanticId::new(2);
        let ty = TypeExpr::Sum(BTreeMap::from([
            (left, TypeExpr::Scalar(ScalarType::I64)),
            (right, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let rows = (0_i64..100_000)
            .map(|value| Value::Variant {
                tag: if value % 3 == 0 { left } else { right },
                value: Box::new(Value::I64(value)),
            })
            .collect::<Vec<_>>();
        let native = AlgebraicNativeColumn::from_values(&rows, &ty).unwrap();
        let tags = native.sum_tags().unwrap();

        let start = Instant::now();
        let mut logical_hits = 0_usize;
        for _ in 0..10 {
            logical_hits += rows
                .iter()
                .filter(|row| matches!(row, Value::Variant { tag, .. } if *tag == left))
                .count();
        }
        black_box(logical_hits);
        let logical_ns = start.elapsed().as_nanos();

        let start = Instant::now();
        let mut native_hits = 0_usize;
        for _ in 0..10 {
            native_hits += tags.iter().filter(|&&tag| tag == left).count();
        }
        black_box(native_hits);
        let native_ns = start.elapsed().as_nanos();
        assert_eq!(logical_hits, native_hits);
        println!(
            "logical_ns={logical_ns} native_ns={native_ns} ratio_milli={}",
            logical_ns.saturating_mul(1_000) / native_ns.max(1)
        );
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_product_field_filter_against_logical_btree_rows() {
        use std::hint::black_box;
        use std::time::Instant;

        let a = SemanticId::new(1);
        let b = SemanticId::new(2);
        let c = SemanticId::new(3);
        let ty = TypeExpr::Product(BTreeMap::from([
            (a, TypeExpr::Scalar(ScalarType::I64)),
            (b, TypeExpr::Scalar(ScalarType::I64)),
            (c, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let rows = (0_i64..100_000)
            .map(|value| {
                Value::Product(BTreeMap::from([
                    (a, Value::I64(value)),
                    (b, Value::I64(value * 2)),
                    (c, Value::I64(value * 3)),
                ]))
            })
            .collect::<Vec<_>>();
        let native = AlgebraicNativeColumn::from_values(&rows, &ty).unwrap();
        let native_a = native.field(a).unwrap().as_i64().unwrap();

        let start = Instant::now();
        let mut logical_hits = 0_usize;
        for _ in 0..10 {
            for row in &rows {
                let Value::Product(fields) = row else {
                    unreachable!()
                };
                if matches!(fields.get(&a), Some(Value::I64(value)) if *value >= 50_000) {
                    logical_hits += 1;
                }
            }
        }
        black_box(logical_hits);
        let logical_ns = start.elapsed().as_nanos();

        let start = Instant::now();
        let mut native_hits = 0_usize;
        for _ in 0..10 {
            native_hits += native_a.iter().filter(|&&value| value >= 50_000).count();
        }
        black_box(native_hits);
        let native_ns = start.elapsed().as_nanos();
        assert_eq!(logical_hits, native_hits);
        println!(
            "logical_ns={logical_ns} native_ns={native_ns} ratio_milli={}",
            logical_ns.saturating_mul(1_000) / native_ns.max(1)
        );
    }
}
