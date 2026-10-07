use std::collections::{BTreeMap, BTreeSet};

use kernel_model::Value;
use kernel_types::{EntityId, SemanticId};

use crate::RealizationError;

const NO_PAYLOAD_ORDINAL: u32 = u32::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedSumColumn {
    variants: Vec<SemanticId>,
    bits_per_tag: u8,
    len: usize,
    packed_tags: Vec<u8>,
    payload_ordinals: Option<Vec<u32>>,
    payloads: BTreeMap<SemanticId, Vec<Value>>,
}

impl PackedSumColumn {
    pub fn new(values: impl IntoIterator<Item = Value>) -> Result<Self, RealizationError> {
        let values = values.into_iter().collect::<Vec<_>>();
        let variants = values
            .iter()
            .map(|value| match value {
                Value::Variant { tag, .. } => Ok(*tag),
                _ => Err(RealizationError::PackedSumRequiresVariants),
            })
            .collect::<Result<BTreeSet<_>, _>>()?
            .into_iter()
            .collect::<Vec<_>>();
        if variants.is_empty() && !values.is_empty() {
            return Err(RealizationError::PackedSumRequiresVariants);
        }
        let bits_per_tag = tag_width(variants.len())?;
        let mut local_tags = Vec::with_capacity(values.len());
        let mut payload_ordinals = Vec::with_capacity(values.len());
        let mut payloads = BTreeMap::<SemanticId, Vec<Value>>::new();
        let mut unit_shape = BTreeMap::<SemanticId, bool>::new();
        for value in values {
            let Value::Variant { tag, value } = value else {
                return Err(RealizationError::PackedSumRequiresVariants);
            };
            let local = variants
                .binary_search(&tag)
                .expect("collected variant must exist");
            local_tags.push(
                u32::try_from(local).map_err(|_| RealizationError::PackedSumTooManyVariants)?,
            );
            let is_unit = matches!(value.as_ref(), Value::Unit);
            if let Some(previous) = unit_shape.insert(tag, is_unit)
                && previous != is_unit
            {
                return Err(RealizationError::PackedSumPayloadShapeMismatch(tag));
            }
            if is_unit {
                payload_ordinals.push(NO_PAYLOAD_ORDINAL);
            } else {
                let payload = payloads.entry(tag).or_default();
                let ordinal = u32::try_from(payload.len())
                    .map_err(|_| RealizationError::PackedSumPayloadTooLarge(tag))?;
                payload_ordinals.push(ordinal);
                payload.push(*value);
            }
        }
        let packed_tags = pack_tags(&local_tags, bits_per_tag);
        let payload_ordinals = (!payloads.is_empty()).then_some(payload_ordinals);
        Ok(Self {
            variants,
            bits_per_tag,
            len: local_tags.len(),
            packed_tags,
            payload_ordinals,
            payloads,
        })
    }

    pub fn from_physical_parts(
        variants: Vec<SemanticId>,
        bits_per_tag: u8,
        len: usize,
        packed_tags: Vec<u8>,
        payload_ordinals: Option<Vec<u32>>,
        payloads: BTreeMap<SemanticId, Vec<Value>>,
    ) -> Result<Self, RealizationError> {
        if variants.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RealizationError::PackedSumVariantLayoutInvalid);
        }
        if bits_per_tag != tag_width(variants.len())?
            || payload_ordinals
                .as_ref()
                .is_some_and(|ordinals| ordinals.len() != len)
            || (!payloads.is_empty() && payload_ordinals.is_none())
            || (payloads.is_empty() && payload_ordinals.is_some())
        {
            return Err(RealizationError::PackedSumVariantLayoutInvalid);
        }
        let expected_bytes = len
            .checked_mul(usize::from(bits_per_tag))
            .and_then(|bits| bits.checked_add(7))
            .map(|bits| bits / 8)
            .ok_or(RealizationError::PackedSumVariantLayoutInvalid)?;
        if packed_tags.len() != expected_bytes {
            return Err(RealizationError::PackedSumVariantLayoutInvalid);
        }
        let mut payload_counts = BTreeMap::<SemanticId, usize>::new();
        for row in 0..len {
            let local = unpack_tag(&packed_tags, bits_per_tag, row)?;
            let tag = *variants
                .get(local)
                .ok_or(RealizationError::PackedSumVariantLayoutInvalid)?;
            match payloads.get(&tag) {
                Some(values) => {
                    let ordinal = payload_ordinals
                        .as_ref()
                        .ok_or(RealizationError::PackedSumVariantLayoutInvalid)?[row];
                    let expected = payload_counts.entry(tag).or_default();
                    if usize::try_from(ordinal).ok() != Some(*expected) || *expected >= values.len()
                    {
                        return Err(RealizationError::PackedSumVariantLayoutInvalid);
                    }
                    *expected += 1;
                }
                None => {
                    if payload_ordinals
                        .as_ref()
                        .is_some_and(|ordinals| ordinals[row] != NO_PAYLOAD_ORDINAL)
                    {
                        return Err(RealizationError::PackedSumVariantLayoutInvalid);
                    }
                }
            }
        }
        if payloads
            .iter()
            .any(|(tag, values)| payload_counts.get(tag).copied().unwrap_or(0) != values.len())
        {
            return Err(RealizationError::PackedSumVariantLayoutInvalid);
        }
        Ok(Self {
            variants,
            bits_per_tag,
            len,
            packed_tags,
            payload_ordinals,
            payloads,
        })
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn variants(&self) -> &[SemanticId] {
        &self.variants
    }

    #[must_use]
    pub const fn bits_per_tag(&self) -> u8 {
        self.bits_per_tag
    }

    #[must_use]
    pub fn packed_tags(&self) -> &[u8] {
        &self.packed_tags
    }

    #[must_use]
    pub fn payload_ordinals(&self) -> Option<&[u32]> {
        self.payload_ordinals.as_deref()
    }

    #[must_use]
    pub const fn payloads(&self) -> &BTreeMap<SemanticId, Vec<Value>> {
        &self.payloads
    }

    #[must_use]
    pub fn payload_value_count(&self) -> usize {
        self.payloads.values().map(Vec::len).sum()
    }

    pub fn semantic_tag_at(&self, row: usize) -> Result<SemanticId, RealizationError> {
        let local = unpack_tag(&self.packed_tags, self.bits_per_tag, row)?;
        self.variants
            .get(local)
            .copied()
            .ok_or(RealizationError::PackedSumVariantLayoutInvalid)
    }

    pub fn matches_variant(&self, row: usize, tag: SemanticId) -> Result<bool, RealizationError> {
        let Ok(expected) = self.variants.binary_search(&tag) else {
            return Ok(false);
        };
        Ok(unpack_tag(&self.packed_tags, self.bits_per_tag, row)? == expected)
    }

    pub fn value_at(&self, row: usize) -> Result<Value, RealizationError> {
        if row >= self.len {
            return Err(RealizationError::PackedSumRowOutOfBounds { row, len: self.len });
        }
        let tag = self.semantic_tag_at(row)?;
        let value = match self.payloads.get(&tag) {
            Some(values) => {
                let ordinal = usize::try_from(
                    self.payload_ordinals
                        .as_ref()
                        .ok_or(RealizationError::PackedSumVariantLayoutInvalid)?[row],
                )
                .map_err(|_| RealizationError::PackedSumVariantLayoutInvalid)?;
                values
                    .get(ordinal)
                    .cloned()
                    .ok_or(RealizationError::PackedSumVariantLayoutInvalid)?
            }
            None => Value::Unit,
        };
        Ok(Value::Variant {
            tag,
            value: Box::new(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedSumFieldSegment {
    entities: Vec<EntityId>,
    column: PackedSumColumn,
}

impl PackedSumFieldSegment {
    pub fn new(
        entries: impl IntoIterator<Item = (EntityId, Value)>,
    ) -> Result<Self, RealizationError> {
        let mut entries = entries.into_iter().collect::<Vec<_>>();
        entries.sort_by_key(|(entity, _)| *entity);
        if entries.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err(RealizationError::UnsortedFieldColumnEntities);
        }
        let (entities, values): (Vec<_>, Vec<_>) = entries.into_iter().unzip();
        Ok(Self {
            entities,
            column: PackedSumColumn::new(values)?,
        })
    }

    pub fn from_physical_parts(
        entities: Vec<EntityId>,
        column: PackedSumColumn,
    ) -> Result<Self, RealizationError> {
        if entities.len() != column.len() || entities.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(RealizationError::RealizationChunkShapeMismatch);
        }
        Ok(Self { entities, column })
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entities.len()
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
    #[must_use]
    pub fn entities(&self) -> &[EntityId] {
        &self.entities
    }
    #[must_use]
    pub const fn column(&self) -> &PackedSumColumn {
        &self.column
    }

    pub fn get(&self, entity: EntityId) -> Result<Option<Value>, RealizationError> {
        match self.entities.binary_search(&entity) {
            Ok(index) => self.column.value_at(index).map(Some),
            Err(_) => Ok(None),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Result<(EntityId, Value), RealizationError>> + '_ {
        self.entities
            .iter()
            .copied()
            .enumerate()
            .map(|(index, entity)| self.column.value_at(index).map(|value| (entity, value)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedSumRelationSegment {
    start_row: usize,
    column: PackedSumColumn,
}

impl PackedSumRelationSegment {
    pub fn new(values: Vec<Value>) -> Result<Self, RealizationError> {
        Self::with_start_row(0, values)
    }

    pub fn with_start_row(start_row: usize, values: Vec<Value>) -> Result<Self, RealizationError> {
        Ok(Self {
            start_row,
            column: PackedSumColumn::new(values)?,
        })
    }

    #[must_use]
    pub fn from_physical_parts(start_row: usize, column: PackedSumColumn) -> Self {
        Self { start_row, column }
    }

    #[must_use]
    pub const fn start_row(&self) -> usize {
        self.start_row
    }
    #[must_use]
    pub const fn len(&self) -> usize {
        self.column.len()
    }
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.column.is_empty()
    }
    #[must_use]
    pub const fn column(&self) -> &PackedSumColumn {
        &self.column
    }

    pub fn get(&self, row: usize) -> Result<Option<Value>, RealizationError> {
        let Some(local) = row.checked_sub(self.start_row) else {
            return Ok(None);
        };
        if local >= self.column.len() {
            return Ok(None);
        }
        self.column.value_at(local).map(Some)
    }
}

fn tag_width(variant_count: usize) -> Result<u8, RealizationError> {
    Ok(match variant_count {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        17..=256 => 8,
        257..=65_536 => 16,
        _ if u32::try_from(variant_count).is_ok() => 32,
        _ => return Err(RealizationError::PackedSumTooManyVariants),
    })
}

fn pack_tags(tags: &[u32], width: u8) -> Vec<u8> {
    let width = usize::from(width);
    let mut bytes = vec![0_u8; tags.len().saturating_mul(width).div_ceil(8)];
    for (row, tag) in tags.iter().copied().enumerate() {
        for bit in 0..width {
            if (tag >> bit) & 1 != 0 {
                let offset = row * width + bit;
                bytes[offset / 8] |= 1 << (offset % 8);
            }
        }
    }
    bytes
}

fn unpack_tag(bytes: &[u8], width: u8, row: usize) -> Result<usize, RealizationError> {
    let width = usize::from(width);
    let end = row
        .checked_add(1)
        .and_then(|next| next.checked_mul(width))
        .ok_or(RealizationError::PackedSumVariantLayoutInvalid)?;
    if end > bytes.len().saturating_mul(8) {
        return Err(RealizationError::PackedSumVariantLayoutInvalid);
    }
    let mut value = 0_u32;
    for bit in 0..width {
        let offset = row * width + bit;
        let set = (bytes[offset / 8] >> (offset % 8)) & 1;
        value |= u32::from(set) << bit;
    }
    usize::try_from(value).map_err(|_| RealizationError::PackedSumVariantLayoutInvalid)
}
