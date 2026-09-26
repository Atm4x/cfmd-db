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

    pub(crate) fn swap_remove_requires_rebuild(&self) -> bool {
        match &self.storage {
            AlgebraicStorage::Empty | AlgebraicStorage::Scalar(_) => false,
            AlgebraicStorage::Product { fields, .. } => fields
                .values()
                .any(AlgebraicNativeColumn::swap_remove_requires_rebuild),
            AlgebraicStorage::Recursive(child) => child.swap_remove_requires_rebuild(),
            AlgebraicStorage::Sum { .. }
            | AlgebraicStorage::Option { .. }
            | AlgebraicStorage::Seq { .. }
            | AlgebraicStorage::Set { .. }
            | AlgebraicStorage::Bag { .. }
            | AlgebraicStorage::Map { .. } => true,
        }
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
                    offsets: offsets.into(),
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
                    offsets: offsets.into(),
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
                    offsets: offsets.into(),
                    payload: Box::new(payload.select_positions(&flat_positions)?),
                    counts: selected_counts.into(),
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
                    offsets: offsets.into(),
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

