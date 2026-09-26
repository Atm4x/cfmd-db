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
        tags: selected_tags.into(),
        payload_index: selected_payload_index.into(),
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
        payload_index: selected_payload_index.into(),
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
                offsets: offsets.into(),
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
        tags: tags.into(),
        payload_index: payload_index.into(),
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
        payload_index: payload_index.into(),
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
        offsets: offsets.into(),
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
        offsets: offsets.into(),
        payload: Box::new(AlgebraicNativeColumn::from_values_inner(
            &payload_values,
            element,
            recursive,
        )?),
        counts: counts.into(),
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
        offsets: offsets.into(),
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
            .map(|values| NativeColumn::Bool(values.into())),
        ScalarType::I64 => values
            .iter()
            .map(|value| match value {
                Value::I64(value) => Ok(*value),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| NativeColumn::I64(values.into())),
        ScalarType::F64 => values
            .iter()
            .map(|value| match value {
                Value::F64Bits(value) => Ok(*value),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| NativeColumn::F64Bits(values.into())),
        ScalarType::Text => values
            .iter()
            .map(|value| match value {
                Value::Text(value) => Ok(value.clone()),
                _ => Err(PhysicalExecutionError::PhysicalTypeMismatch),
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|values| NativeColumn::Text(values.into())),
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
                values: values.into(),
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
                values: values.into(),
            }),
        ScalarType::Unit => Err(PhysicalExecutionError::PhysicalTypeMismatch),
    }
}

