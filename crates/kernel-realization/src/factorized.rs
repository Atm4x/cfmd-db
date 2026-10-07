use std::collections::{BTreeMap, BTreeSet};

use kernel_model::{DatabaseState, Value};
use kernel_persistent::{PersistentOrdMap, PersistentVec};
use kernel_query::{
    ExactQuery, PreparedRelationRewrite, RelationBaseWitness, RelationDelta,
    RelationScanOccurrenceSeed, RelationValue,
};
use kernel_schema::SemanticContext;
use kernel_semantics::SemanticRegistry;
use kernel_transport::{SchemaMigrationProgram, TransportError};
use kernel_types::{EntityId, RevisionId, SemanticId, StableRowHandle};

use crate::{
    PackedSumFieldSegment, PackedSumRelationSegment, PhysicalAtomId, PhysicalAtomPayload,
    PhysicalAtomStore, PhysicalCodec, RealizationError, atom_payload,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldColumnSegment {
    entities: Vec<EntityId>,
    values: Vec<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CarrierSegmentCoordinate {
    carrier: PhysicalAtomId,
    start_ordinal: usize,
    len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactorizedFieldNativeChunk {
    segment: CarrierSegmentCoordinate,
    first_entity: EntityId,
    last_entity: EntityId,
    atom: PhysicalAtomId,
}

impl FactorizedFieldNativeChunk {
    #[must_use]
    pub const fn segment(self) -> CarrierSegmentCoordinate {
        self.segment
    }

    #[must_use]
    pub const fn atom(self) -> PhysicalAtomId {
        self.atom
    }

    #[must_use]
    pub fn contains_entity(self, entity: EntityId) -> bool {
        entity >= self.first_entity && entity <= self.last_entity
    }
}

impl CarrierSegmentCoordinate {
    #[must_use]
    pub const fn new(carrier: PhysicalAtomId, start_ordinal: usize, len: usize) -> Self {
        Self {
            carrier,
            start_ordinal,
            len,
        }
    }

    #[must_use]
    pub const fn carrier(self) -> PhysicalAtomId {
        self.carrier
    }

    #[must_use]
    pub const fn start_ordinal(self) -> usize {
        self.start_ordinal
    }

    #[must_use]
    pub const fn len(self) -> usize {
        self.len
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub const fn end_ordinal(self) -> usize {
        self.start_ordinal + self.len
    }

    #[must_use]
    pub const fn contains_ordinal(self, ordinal: usize) -> bool {
        ordinal >= self.start_ordinal && ordinal < self.end_ordinal()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationColumnSegment {
    start_row: usize,
    values: Vec<Value>,
}

impl RelationColumnSegment {
    #[must_use]
    pub fn new(values: Vec<Value>) -> Self {
        Self::with_start_row(0, values)
    }

    #[must_use]
    pub fn with_start_row(start_row: usize, values: Vec<Value>) -> Self {
        Self { start_row, values }
    }

    #[must_use]
    pub const fn start_row(&self) -> usize {
        self.start_row
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub fn get(&self, row: usize) -> Option<&Value> {
        row.checked_sub(self.start_row)
            .and_then(|local| self.values.get(local))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.values.iter()
    }
}

impl FieldColumnSegment {
    pub fn new(
        entries: impl IntoIterator<Item = (EntityId, Value)>,
    ) -> Result<Self, RealizationError> {
        let mut entries = entries.into_iter().collect::<Vec<_>>();
        entries.sort_by_key(|(entity, _)| *entity);
        Self::from_sorted(entries)
    }

    fn from_sorted(
        entries: impl IntoIterator<Item = (EntityId, Value)>,
    ) -> Result<Self, RealizationError> {
        let entries = entries.into_iter().collect::<Vec<_>>();
        if let Some(pair) = entries.windows(2).find(|pair| pair[0].0 >= pair[1].0) {
            if pair[0].0 == pair[1].0 {
                return Err(RealizationError::DuplicateFieldColumnEntity(pair[0].0));
            }
            return Err(RealizationError::UnsortedFieldColumnEntities);
        }
        let (entities, values) = entries.into_iter().unzip();
        Ok(Self { entities, values })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    #[must_use]
    pub fn get(&self, entity: EntityId) -> Option<&Value> {
        self.entities
            .binary_search(&entity)
            .ok()
            .and_then(|index| self.values.get(index))
    }

    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &Value)> {
        self.entities.iter().copied().zip(self.values.iter())
    }

    fn aligned_values(&self, entities: &[EntityId]) -> Option<&[Value]> {
        if entities.is_empty() {
            return Some(&self.values[0..0]);
        }
        let start = self.entities.binary_search(&entities[0]).ok()?;
        let end = start.checked_add(entities.len())?;
        (end <= self.entities.len() && self.entities[start..end] == *entities)
            .then_some(&self.values[start..end])
    }
}

fn insert_field_column_atom(
    atoms: &mut PhysicalAtomStore,
    entries: Vec<(EntityId, Value)>,
) -> Result<PhysicalAtomId, RealizationError> {
    if entries
        .iter()
        .all(|(_, value)| matches!(value, Value::Variant { .. }))
    {
        return Ok(atoms.insert(PhysicalAtomPayload::PackedSumFieldSegment(
            PackedSumFieldSegment::new(entries)?,
        )));
    }
    Ok(atoms.insert(PhysicalAtomPayload::FieldColumnSegment(
        FieldColumnSegment::from_sorted(entries)?,
    )))
}

fn insert_relation_column_atom(
    atoms: &mut PhysicalAtomStore,
    start_row: usize,
    values: Vec<Value>,
) -> Result<PhysicalAtomId, RealizationError> {
    if values
        .iter()
        .all(|value| matches!(value, Value::Variant { .. }))
    {
        return Ok(atoms.insert(PhysicalAtomPayload::PackedSumRelationSegment(
            PackedSumRelationSegment::with_start_row(start_row, values)?,
        )));
    }
    Ok(atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
        RelationColumnSegment::with_start_row(start_row, values),
    )))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactorizedFieldExpr {
    Direct(PhysicalAtomId),
    Constant(Value),
    I64ToF64Direct(PhysicalAtomId),
    Product(BTreeMap<SemanticId, Self>),
    Transform {
        source: Box<Self>,
        query: ExactQuery,
    },
    ChunkOverlay {
        base: Box<Self>,
        carrier: PhysicalAtomId,
        entity_count: usize,
        chunk_entities: usize,
        native_chunks: Vec<FactorizedFieldNativeChunk>,
    },
}

impl FactorizedFieldExpr {
    #[allow(
        clippy::cast_precision_loss,
        reason = "This operation explicitly requests IEEE-754 rounding or a diagnostic ratio."
    )]
    pub fn evaluate_entity(
        &self,
        atoms: &PhysicalAtomStore,
        entity: EntityId,
    ) -> Result<Value, RealizationError> {
        match self {
            Self::Direct(atom) => {
                match atom_payload(atoms, *atom)? {
                    PhysicalAtomPayload::FieldColumnSegment(column) => column
                        .get(entity)
                        .cloned()
                        .ok_or(RealizationError::MissingFactorizedFieldValue {
                            atom: *atom,
                            entity,
                        }),
                    PhysicalAtomPayload::PackedSumFieldSegment(column) => column
                        .get(entity)?
                        .ok_or(RealizationError::MissingFactorizedFieldValue {
                            atom: *atom,
                            entity,
                        }),
                    payload => Err(RealizationError::CodecMismatch {
                        atom: *atom,
                        expected: PhysicalCodec::FieldColumnSegment,
                        actual: payload.codec(),
                    }),
                }
            }
            Self::Constant(value) => Ok(value.clone()),
            Self::I64ToF64Direct(atom) => match atom_payload(atoms, *atom)? {
                PhysicalAtomPayload::FieldColumnSegment(column) => match column.get(entity) {
                    Some(Value::I64(value)) => Ok(Value::F64Bits((*value as f64).to_bits())),
                    Some(_) => Err(RealizationError::Query(
                        kernel_query::QueryError::TypeMismatch,
                    )),
                    None => Err(RealizationError::MissingFactorizedFieldValue {
                        atom: *atom,
                        entity,
                    }),
                },
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::FieldColumnSegment,
                    actual: payload.codec(),
                }),
            },
            Self::Product(fields) => fields
                .iter()
                .map(|(&field, expr)| {
                    expr.evaluate_entity(atoms, entity)
                        .map(|value| (field, value))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
                .map(Value::Product),
            Self::Transform { source, query } => query
                .evaluate(&source.evaluate_entity(atoms, entity)?)
                .map_err(RealizationError::Query),
            Self::ChunkOverlay {
                base,
                carrier,
                entity_count,
                native_chunks,
                ..
            } => {
                if entity_order(atoms, *carrier)?.len() != *entity_count {
                    return Err(RealizationError::RealizationChunkShapeMismatch);
                }
                let native = if native_chunks.len() <= 8 {
                    native_chunks
                        .iter()
                        .find(|chunk| chunk.contains_entity(entity))
                } else {
                    let index = native_chunks.partition_point(|chunk| chunk.first_entity <= entity);
                    index
                        .checked_sub(1)
                        .and_then(|index| native_chunks.get(index))
                        .filter(|chunk| chunk.contains_entity(entity))
                };
                if let Some(chunk) = native {
                    let atom = chunk.atom;
                    return FactorizedFieldExpr::Direct(atom).evaluate_entity(atoms, entity);
                }
                base.evaluate_entity(atoms, entity)
            }
        }
    }

    fn collect_dependencies(&self, into: &mut BTreeSet<PhysicalAtomId>) {
        match self {
            Self::Constant(_) => {}
            Self::Direct(atom) | Self::I64ToF64Direct(atom) => {
                into.insert(*atom);
            }
            Self::Product(fields) => {
                for expr in fields.values() {
                    expr.collect_dependencies(into);
                }
            }
            Self::Transform { source, .. } => source.collect_dependencies(into),
            Self::ChunkOverlay {
                base,
                entity_count,
                chunk_entities,
                native_chunks,
                ..
            } => {
                into.extend(native_chunks.iter().map(|chunk| chunk.atom));
                let chunk_count = entity_count.div_ceil(*chunk_entities);
                if native_chunks.len() < chunk_count {
                    base.collect_dependencies(into);
                }
            }
        }
    }

    fn visit_direct_carrier_range<F>(
        atoms: &PhysicalAtomStore,
        atom: PhysicalAtomId,
        order: &[EntityId],
        start: usize,
        end: usize,
        visit: &mut F,
    ) -> Result<(), RealizationError>
    where
        F: FnMut(EntityId, Value),
    {
        match atom_payload(atoms, atom)? {
            PhysicalAtomPayload::FieldColumnSegment(column) => {
                let entities = &order[start..end];
                let values = column
                    .aligned_values(entities)
                    .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
                for (&entity, value) in entities.iter().zip(values) {
                    visit(entity, value.clone());
                }
                Ok(())
            }
            PhysicalAtomPayload::PackedSumFieldSegment(column) => {
                for &entity in &order[start..end] {
                    visit(
                        entity,
                        column.get(entity)?.ok_or(
                            RealizationError::MissingFactorizedFieldValue { atom, entity },
                        )?,
                    );
                }
                Ok(())
            }
            payload => Err(RealizationError::CodecMismatch {
                atom,
                expected: PhysicalCodec::FieldColumnSegment,
                actual: payload.codec(),
            }),
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "This operation explicitly requests IEEE-754 rounding or a diagnostic ratio."
    )]
    fn visit_carrier_range<F>(
        &self,
        atoms: &PhysicalAtomStore,
        carrier: PhysicalAtomId,
        start: usize,
        end: usize,
        visit: &mut F,
    ) -> Result<(), RealizationError>
    where
        F: FnMut(EntityId, Value),
    {
        let order = entity_order(atoms, carrier)?;
        if end > order.len() || start > end {
            return Err(RealizationError::RealizationChunkShapeMismatch);
        }
        match self {
            Self::Direct(atom) => {
                Self::visit_direct_carrier_range(atoms, *atom, order, start, end, visit)
            }
            Self::I64ToF64Direct(atom) => match atom_payload(atoms, *atom)? {
                PhysicalAtomPayload::FieldColumnSegment(column) => {
                    let entities = &order[start..end];
                    let values = column
                        .aligned_values(entities)
                        .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
                    for (&entity, value) in entities.iter().zip(values) {
                        let Value::I64(value) = value else {
                            return Err(RealizationError::Query(
                                kernel_query::QueryError::TypeMismatch,
                            ));
                        };
                        visit(entity, Value::F64Bits((*value as f64).to_bits()));
                    }
                    Ok(())
                }
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::FieldColumnSegment,
                    actual: payload.codec(),
                }),
            },
            Self::ChunkOverlay {
                base,
                carrier: overlay_carrier,
                entity_count,
                native_chunks,
                ..
            } => {
                if *overlay_carrier != carrier || *entity_count != order.len() {
                    return Err(RealizationError::RealizationChunkShapeMismatch);
                }
                let mut cursor = start;
                while cursor < end {
                    let native = native_chunks
                        .iter()
                        .find(|chunk| chunk.segment.contains_ordinal(cursor));
                    if let Some(chunk) = native {
                        let segment_end = usize::min(chunk.segment.end_ordinal(), end);
                        FactorizedFieldExpr::Direct(chunk.atom).visit_carrier_range(
                            atoms,
                            carrier,
                            cursor,
                            segment_end,
                            visit,
                        )?;
                        cursor = segment_end;
                        continue;
                    }
                    let next_native = native_chunks
                        .iter()
                        .map(|chunk| chunk.segment.start_ordinal())
                        .filter(|&ordinal| ordinal > cursor)
                        .min()
                        .unwrap_or(end);
                    let base_end = usize::min(next_native, end);
                    base.visit_carrier_range(atoms, carrier, cursor, base_end, visit)?;
                    cursor = base_end;
                }
                Ok(())
            }
            _ => {
                for &entity in &order[start..end] {
                    visit(entity, self.evaluate_entity(atoms, entity)?);
                }
                Ok(())
            }
        }
    }
}

fn entity_order(
    atoms: &PhysicalAtomStore,
    atom: PhysicalAtomId,
) -> Result<&[EntityId], RealizationError> {
    match atom_payload(atoms, atom)? {
        PhysicalAtomPayload::EntityOrder(entities) => Ok(entities),
        payload => Err(RealizationError::CodecMismatch {
            atom,
            expected: PhysicalCodec::EntityOrder,
            actual: payload.codec(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactorizedRelationColumnExpr {
    Direct(PhysicalAtomId),
    Constant(Value),
    I64ToF64Direct(PhysicalAtomId),
    Product(BTreeMap<SemanticId, Self>),
    Transform {
        source: Box<Self>,
        query: ExactQuery,
    },
    ChunkOverlay {
        base: Box<Self>,
        row_count: usize,
        chunk_rows: usize,
        native_chunks: Vec<(usize, PhysicalAtomId)>,
    },
}

impl FactorizedRelationColumnExpr {
    #[allow(
        clippy::cast_precision_loss,
        reason = "This operation explicitly requests IEEE-754 rounding or a diagnostic ratio."
    )]
    pub fn evaluate_row(
        &self,
        atoms: &PhysicalAtomStore,
        row: usize,
    ) -> Result<Value, RealizationError> {
        match self {
            Self::Direct(atom) => match atom_payload(atoms, *atom)? {
                PhysicalAtomPayload::RelationColumnSegment(column) => column
                    .get(row)
                    .cloned()
                    .ok_or(RealizationError::MissingFactorizedRelationRow { atom: *atom, row }),
                PhysicalAtomPayload::PackedSumRelationSegment(column) => column
                    .get(row)?
                    .ok_or(RealizationError::MissingFactorizedRelationRow { atom: *atom, row }),
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::RelationColumnSegment,
                    actual: payload.codec(),
                }),
            },
            Self::Constant(value) => Ok(value.clone()),
            Self::I64ToF64Direct(atom) => match atom_payload(atoms, *atom)? {
                PhysicalAtomPayload::RelationColumnSegment(column) => match column.get(row) {
                    Some(Value::I64(value)) => Ok(Value::F64Bits((*value as f64).to_bits())),
                    Some(_) => Err(RealizationError::Query(
                        kernel_query::QueryError::TypeMismatch,
                    )),
                    None => {
                        Err(RealizationError::MissingFactorizedRelationRow { atom: *atom, row })
                    }
                },
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::RelationColumnSegment,
                    actual: payload.codec(),
                }),
            },
            Self::Product(columns) => columns
                .iter()
                .map(|(&column, expr)| expr.evaluate_row(atoms, row).map(|value| (column, value)))
                .collect::<Result<BTreeMap<_, _>, _>>()
                .map(Value::Product),
            Self::Transform { source, query } => query
                .evaluate(&source.evaluate_row(atoms, row)?)
                .map_err(RealizationError::Query),
            Self::ChunkOverlay {
                base,
                row_count,
                chunk_rows,
                native_chunks,
            } => {
                if row >= *row_count {
                    return Err(RealizationError::FactorizedRelationRowOutOfBounds {
                        row,
                        row_count: *row_count,
                    });
                }
                let chunk = row / *chunk_rows;
                let native = if native_chunks.len() <= 8 {
                    native_chunks
                        .iter()
                        .find_map(|(index, atom)| (*index == chunk).then_some(*atom))
                } else {
                    native_chunks
                        .binary_search_by_key(&chunk, |(index, _)| *index)
                        .ok()
                        .map(|index| native_chunks[index].1)
                };
                if let Some(atom) = native {
                    return FactorizedRelationColumnExpr::Direct(atom).evaluate_row(atoms, row);
                }
                base.evaluate_row(atoms, row)
            }
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "This operation explicitly requests IEEE-754 rounding or a diagnostic ratio."
    )]
    fn visit_range<F>(
        &self,
        atoms: &PhysicalAtomStore,
        start: usize,
        end: usize,
        visit: &mut F,
    ) -> Result<(), RealizationError>
    where
        F: FnMut(Value),
    {
        if start > end {
            return Err(RealizationError::RealizationChunkShapeMismatch);
        }
        match self {
            Self::Direct(atom) => match atom_payload(atoms, *atom)? {
                PhysicalAtomPayload::RelationColumnSegment(column) => {
                    for row in start..end {
                        visit(column.get(row).cloned().ok_or(
                            RealizationError::MissingFactorizedRelationRow { atom: *atom, row },
                        )?);
                    }
                    Ok(())
                }
                PhysicalAtomPayload::PackedSumRelationSegment(column) => {
                    for row in start..end {
                        visit(column.get(row)?.ok_or(
                            RealizationError::MissingFactorizedRelationRow { atom: *atom, row },
                        )?);
                    }
                    Ok(())
                }
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::RelationColumnSegment,
                    actual: payload.codec(),
                }),
            },
            Self::I64ToF64Direct(atom) => match atom_payload(atoms, *atom)? {
                PhysicalAtomPayload::RelationColumnSegment(column) => {
                    for row in start..end {
                        let value = column.get(row).ok_or(
                            RealizationError::MissingFactorizedRelationRow { atom: *atom, row },
                        )?;
                        match value {
                            Value::I64(value) => visit(Value::F64Bits((*value as f64).to_bits())),
                            _ => {
                                return Err(RealizationError::Query(
                                    kernel_query::QueryError::TypeMismatch,
                                ));
                            }
                        }
                    }
                    Ok(())
                }
                payload => Err(RealizationError::CodecMismatch {
                    atom: *atom,
                    expected: PhysicalCodec::RelationColumnSegment,
                    actual: payload.codec(),
                }),
            },
            Self::Constant(value) => {
                for _ in start..end {
                    visit(value.clone());
                }
                Ok(())
            }
            Self::ChunkOverlay {
                base,
                row_count,
                chunk_rows,
                native_chunks,
            } => {
                if end > *row_count {
                    return Err(RealizationError::FactorizedRelationRowOutOfBounds {
                        row: end.saturating_sub(1),
                        row_count: *row_count,
                    });
                }
                let mut cursor = start;
                while cursor < end {
                    let chunk = cursor / *chunk_rows;
                    let chunk_end = usize::min((chunk + 1) * *chunk_rows, end);
                    let native = if native_chunks.len() <= 8 {
                        native_chunks
                            .iter()
                            .find_map(|(index, atom)| (*index == chunk).then_some(*atom))
                    } else {
                        native_chunks
                            .binary_search_by_key(&chunk, |(index, _)| *index)
                            .ok()
                            .map(|index| native_chunks[index].1)
                    };
                    if let Some(atom) = native {
                        FactorizedRelationColumnExpr::Direct(atom)
                            .visit_range(atoms, cursor, chunk_end, visit)?;
                    } else {
                        base.visit_range(atoms, cursor, chunk_end, visit)?;
                    }
                    cursor = chunk_end;
                }
                Ok(())
            }
            Self::Product(_) | Self::Transform { .. } => {
                for row in start..end {
                    visit(self.evaluate_row(atoms, row)?);
                }
                Ok(())
            }
        }
    }

    fn collect_dependencies(&self, into: &mut BTreeSet<PhysicalAtomId>) {
        match self {
            Self::Direct(atom) | Self::I64ToF64Direct(atom) => {
                into.insert(*atom);
            }
            Self::Constant(_) => {}
            Self::Product(columns) => {
                for expr in columns.values() {
                    expr.collect_dependencies(into);
                }
            }
            Self::Transform { source, .. } => source.collect_dependencies(into),
            Self::ChunkOverlay {
                base,
                row_count,
                chunk_rows,
                native_chunks,
            } => {
                into.extend(native_chunks.iter().map(|(_, atom)| *atom));
                let chunk_count = row_count.div_ceil(*chunk_rows);
                if native_chunks.len() < chunk_count {
                    base.collect_dependencies(into);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationDeltaOverlay {
    base_row_count: usize,
    live_handles: PersistentVec<StableRowHandle>,
    position_overrides: PersistentOrdMap<StableRowHandle, usize>,
    inserted_rows: PersistentOrdMap<StableRowHandle, (PhysicalAtomId, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationDeltaOverlayStats {
    pub base_rows: usize,
    pub live_rows: usize,
    pub inserted_rows: usize,
    pub displaced_rows: usize,
    pub delta_atoms: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationCompactionWorkloadCost {
    pub expected_uses: u64,
    pub overlay_cost: u64,
    pub native_cost: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationCompactionDecision {
    pub expected_savings: u128,
    pub expected_penalty: u128,
    pub compaction_cost: u128,
    pub maintenance_cost: u128,
    pub should_compact: bool,
}

pub fn relation_compaction_decision(
    workloads: &[RelationCompactionWorkloadCost],
    compaction_cost: u128,
    maintenance_cost: u128,
) -> Result<RelationCompactionDecision, RealizationError> {
    let mut expected_savings = 0_u128;
    let mut expected_penalty = 0_u128;
    for workload in workloads {
        let uses = u128::from(workload.expected_uses);
        if workload.overlay_cost >= workload.native_cost {
            expected_savings = expected_savings
                .checked_add(uses * u128::from(workload.overlay_cost - workload.native_cost))
                .ok_or(RealizationError::CompactionCostOverflow)?;
        } else {
            expected_penalty = expected_penalty
                .checked_add(uses * u128::from(workload.native_cost - workload.overlay_cost))
                .ok_or(RealizationError::CompactionCostOverflow)?;
        }
    }
    let total_cost = compaction_cost
        .checked_add(maintenance_cost)
        .and_then(|cost| cost.checked_add(expected_penalty))
        .ok_or(RealizationError::CompactionCostOverflow)?;
    Ok(RelationCompactionDecision {
        expected_savings,
        expected_penalty,
        compaction_cost,
        maintenance_cost,
        should_compact: expected_savings >= total_cost,
    })
}

impl RelationDeltaOverlay {
    fn stats(&self) -> RelationDeltaOverlayStats {
        RelationDeltaOverlayStats {
            base_rows: self.base_row_count,
            live_rows: self.live_handles.len(),
            inserted_rows: self.inserted_rows.len(),
            displaced_rows: self.position_overrides.len(),
            delta_atoms: self
                .inserted_rows
                .values()
                .map(|(atom, _)| *atom)
                .collect::<BTreeSet<_>>()
                .len(),
        }
    }

    fn from_base_row_count(row_count: usize) -> Self {
        let handles = (0..row_count)
            .map(|slot| StableRowHandle {
                slot,
                generation: 0,
            })
            .collect();
        Self {
            base_row_count: row_count,
            live_handles: PersistentVec::from_vec(handles),
            position_overrides: PersistentOrdMap::default(),
            inserted_rows: PersistentOrdMap::default(),
        }
    }

    fn row_count(&self) -> usize {
        self.live_handles.len()
    }

    fn is_identity(&self) -> bool {
        self.inserted_rows.is_empty()
            && self.position_overrides.is_empty()
            && self.live_handles.len() == self.base_row_count
            && self
                .live_handles
                .iter()
                .copied()
                .enumerate()
                .all(|(position, handle)| handle.slot == position && handle.generation == 0)
    }

    fn handle_at(&self, logical_row: usize) -> Option<StableRowHandle> {
        self.live_handles.get(logical_row).copied()
    }

    fn base_row(&self, handle: StableRowHandle) -> Option<usize> {
        (!self.inserted_rows.contains_key(&handle)
            && handle.generation == 0
            && handle.slot < self.base_row_count)
            .then_some(handle.slot)
    }

    fn logical_position(&self, handle: StableRowHandle) -> Option<usize> {
        let position = self
            .position_overrides
            .get(&handle)
            .copied()
            .unwrap_or(handle.slot);
        (self.live_handles.get(position).copied() == Some(handle)).then_some(position)
    }

    fn inserted_value(
        &self,
        atoms: &PhysicalAtomStore,
        handle: StableRowHandle,
        column_ordinal: usize,
    ) -> Result<Value, RealizationError> {
        let (atom, row_offset) = self
            .inserted_rows
            .get(&handle)
            .copied()
            .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
        match atom_payload(atoms, atom)? {
            PhysicalAtomPayload::RelationRows(rows) => rows
                .get(row_offset)
                .and_then(|row| row.get(column_ordinal))
                .cloned()
                .ok_or(RealizationError::RealizationChunkShapeMismatch),
            payload => Err(RealizationError::CodecMismatch {
                atom,
                expected: PhysicalCodec::RelationRows,
                actual: payload.codec(),
            }),
        }
    }

    fn remove_handle(&mut self, handle: StableRowHandle) -> Result<usize, RealizationError> {
        let position = self
            .logical_position(handle)
            .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
        self.position_overrides.remove(&handle);
        let removed = self.live_handles.swap_remove(position);
        if removed != handle {
            return Err(RealizationError::RealizationChunkShapeMismatch);
        }
        if let Some(&moved) = self.live_handles.get(position) {
            if moved.slot == position {
                self.position_overrides.remove(&moved);
            } else {
                self.position_overrides.insert(moved, position);
            }
        }
        self.inserted_rows.remove(&handle);
        Ok(position)
    }

    fn apply_prepared_delta<I>(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        prepared: &PreparedRelationRewrite<I>,
    ) -> Result<(Option<PhysicalAtomId>, Vec<usize>), RealizationError> {
        if prepared.removed_occurrence_handles().len() != prepared.delta().removed.len()
            || prepared.inserted_occurrence_handles().len() != prepared.delta().inserted.len()
        {
            return Err(RealizationError::RealizationChunkShapeMismatch);
        }
        let mut removed_positions = Vec::with_capacity(prepared.removed_occurrence_handles().len());
        for handle in prepared.removed_occurrence_handles().iter().copied() {
            removed_positions.push(self.remove_handle(handle)?);
        }

        let inserted_atom = if prepared.delta().inserted.is_empty() {
            None
        } else {
            Some(atoms.insert(PhysicalAtomPayload::RelationRows(
                prepared.delta().inserted.clone(),
            )))
        };
        if let Some(atom) = inserted_atom {
            for (row_offset, handle) in prepared
                .inserted_occurrence_handles()
                .iter()
                .copied()
                .enumerate()
            {
                if self.logical_position(handle).is_some()
                    || self.inserted_rows.contains_key(&handle)
                {
                    return Err(RealizationError::RealizationChunkShapeMismatch);
                }
                let position = self.live_handles.len();
                self.live_handles.push(handle);
                if handle.slot != position {
                    self.position_overrides.insert(handle, position);
                }
                self.inserted_rows.insert(handle, (atom, row_offset));
            }
        }
        Ok((inserted_atom, removed_positions))
    }

    fn collect_dependencies(&self, into: &mut BTreeSet<PhysicalAtomId>) {
        into.extend(self.inserted_rows.values().map(|(atom, _)| *atom));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorizedRelationRule {
    column_order: Vec<SemanticId>,
    columns: BTreeMap<SemanticId, FactorizedRelationColumnExpr>,
    row_count: usize,
    delta_overlay: Option<RelationDeltaOverlay>,
    base_witness: Option<RelationBaseWitness>,
    scan_seed: Option<RelationScanOccurrenceSeed>,
}

impl FactorizedRelationRule {
    #[must_use]
    pub fn column_order(&self) -> &[SemanticId] {
        &self.column_order
    }

    #[must_use]
    pub const fn columns(&self) -> &BTreeMap<SemanticId, FactorizedRelationColumnExpr> {
        &self.columns
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    fn value_at(
        &self,
        atoms: &PhysicalAtomStore,
        column: SemanticId,
        logical_row: usize,
    ) -> Result<Value, RealizationError> {
        if logical_row >= self.row_count {
            return Err(RealizationError::FactorizedRelationRowOutOfBounds {
                row: logical_row,
                row_count: self.row_count,
            });
        }
        let expr =
            self.columns
                .get(&column)
                .ok_or(RealizationError::MissingFactorizedRelationColumn {
                    relation: SemanticId::new(0),
                    column,
                })?;
        let Some(overlay) = &self.delta_overlay else {
            return expr.evaluate_row(atoms, logical_row);
        };
        let handle = overlay
            .handle_at(logical_row)
            .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
        if let Some(base_row) = overlay.base_row(handle) {
            expr.evaluate_row(atoms, base_row)
        } else {
            let ordinal = self
                .column_order
                .iter()
                .position(|candidate| *candidate == column)
                .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
            overlay.inserted_value(atoms, handle, ordinal)
        }
    }

    fn visit_column_range<F>(
        &self,
        atoms: &PhysicalAtomStore,
        column: SemanticId,
        start: usize,
        end: usize,
        visit: &mut F,
    ) -> Result<(), RealizationError>
    where
        F: FnMut(Value),
    {
        if start > end || end > self.row_count {
            return Err(RealizationError::FactorizedRelationRowOutOfBounds {
                row: end.saturating_sub(1),
                row_count: self.row_count,
            });
        }
        let expr =
            self.columns
                .get(&column)
                .ok_or(RealizationError::MissingFactorizedRelationColumn {
                    relation: SemanticId::new(0),
                    column,
                })?;
        let Some(overlay) = &self.delta_overlay else {
            return expr.visit_range(atoms, start, end, visit);
        };
        let ordinal = self
            .column_order
            .iter()
            .position(|candidate| *candidate == column)
            .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
        let mut cursor = start;
        while cursor < end {
            let handle = overlay.live_handles[cursor];
            let Some(base_row) = overlay.base_row(handle) else {
                visit(overlay.inserted_value(atoms, handle, ordinal)?);
                cursor += 1;
                continue;
            };
            let run_start_row = base_row;
            let mut run_end = cursor + 1;
            let mut next_row = base_row + 1;
            while run_end < end {
                let candidate = overlay.live_handles[run_end];
                if overlay.base_row(candidate) != Some(next_row) {
                    break;
                }
                run_end += 1;
                next_row += 1;
            }
            expr.visit_range(atoms, run_start_row, next_row, visit)?;
            cursor = run_end;
        }
        Ok(())
    }

    fn evaluate_rows(
        &self,
        atoms: &PhysicalAtomStore,
    ) -> Result<Vec<Vec<Value>>, RealizationError> {
        (0..self.row_count)
            .map(|row| {
                self.column_order
                    .iter()
                    .map(|column| self.value_at(atoms, *column, row))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect()
    }

    fn collect_dependencies(&self, into: &mut BTreeSet<PhysicalAtomId>) {
        for expr in self.columns.values() {
            expr.collect_dependencies(into);
        }
        if let Some(overlay) = &self.delta_overlay {
            overlay.collect_dependencies(into);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorizedFieldRule {
    owner: SemanticId,
    expr: FactorizedFieldExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectFactorizedFieldRoot {
    pub owner: SemanticId,
    pub atom: PhysicalAtomId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectFactorizedRelationRoot {
    pub column_order: Vec<SemanticId>,
    pub column_atoms: BTreeMap<SemanticId, PhysicalAtomId>,
    pub row_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectFactorizedRealizationRoot {
    pub lifecycle: PhysicalAtomId,
    pub carriers: BTreeMap<SemanticId, PhysicalAtomId>,
    pub fields: BTreeMap<SemanticId, DirectFactorizedFieldRoot>,
    pub relations: BTreeMap<SemanticId, DirectFactorizedRelationRoot>,
}

impl FactorizedFieldRule {
    #[must_use]
    pub const fn owner(&self) -> SemanticId {
        self.owner
    }

    #[must_use]
    pub const fn expr(&self) -> &FactorizedFieldExpr {
        &self.expr
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedFactorizedRelation {
    target_relation: SemanticId,
    source_relations: BTreeSet<SemanticId>,
    source_atoms: BTreeSet<PhysicalAtomId>,
    column_order: Vec<SemanticId>,
    column_atoms: BTreeMap<SemanticId, PhysicalAtomId>,
    row_count: usize,
    delta_overlay: RelationDeltaOverlay,
    base_witness: RelationBaseWitness,
    scan_seed: RelationScanOccurrenceSeed,
}

impl PreparedFactorizedRelation {
    #[must_use]
    pub const fn target_relation(&self) -> SemanticId {
        self.target_relation
    }

    #[must_use]
    pub const fn source_relations(&self) -> &BTreeSet<SemanticId> {
        &self.source_relations
    }

    #[must_use]
    pub const fn source_atoms(&self) -> &BTreeSet<PhysicalAtomId> {
        &self.source_atoms
    }

    #[must_use]
    pub fn prepared_atoms(&self) -> BTreeSet<PhysicalAtomId> {
        self.column_atoms.values().copied().collect()
    }

    #[must_use]
    pub const fn base_witness(&self) -> &RelationBaseWitness {
        &self.base_witness
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Keep explicit semantic and durability inputs at this boundary."
    )]
    pub(crate) fn from_execution_columns(
        atoms: &mut PhysicalAtomStore,
        target_relation: SemanticId,
        source_relations: BTreeSet<SemanticId>,
        source_atoms: BTreeSet<PhysicalAtomId>,
        column_order: Vec<SemanticId>,
        columns: Vec<Vec<Value>>,
        result_type: kernel_query::RelType,
        target_context: &SemanticContext,
        registry: &SemanticRegistry,
        occurrence_certificate: Option<kernel_query::RelationOccurrenceCertificate>,
    ) -> Result<Self, RealizationError> {
        if columns.len() != column_order.len() {
            return Err(RealizationError::FactorizedRelationColumnArityMismatch(
                target_relation,
            ));
        }
        let row_count = columns.first().map_or(0, Vec::len);
        if columns.iter().any(|column| column.len() != row_count) {
            return Err(RealizationError::FactorizedRelationColumnArityMismatch(
                target_relation,
            ));
        }
        let base_witness = if let Some(certificate) = occurrence_certificate {
            RelationBaseWitness::from_occurrence_certificate(
                RevisionId::new(0),
                target_relation,
                certificate,
                result_type,
                target_context,
                registry,
            )
            .map_err(RealizationError::RelationQuery)?
        } else {
            let column_refs = columns.iter().map(Vec::as_slice).collect::<Vec<_>>();
            RelationBaseWitness::build_columnar(
                RevisionId::new(0),
                target_relation,
                row_count,
                &column_refs,
                result_type,
                target_context,
                registry,
            )
            .map_err(RealizationError::RelationQuery)?
        };
        let column_atoms = column_order
            .iter()
            .copied()
            .zip(columns)
            .map(|(column, values)| {
                let atom = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
                    RelationColumnSegment::new(values),
                ));
                (column, atom)
            })
            .collect();
        let identity_handles = (0..row_count)
            .map(|slot| StableRowHandle {
                slot,
                generation: 0,
            })
            .collect::<Vec<_>>();
        let scan_seed = base_witness
            .scan_occurrence_seed(&identity_handles)
            .map_err(RealizationError::RelationQuery)?;
        Ok(Self {
            target_relation,
            source_relations,
            source_atoms,
            column_order,
            column_atoms,
            row_count,
            delta_overlay: RelationDeltaOverlay::from_base_row_count(row_count),
            base_witness,
            scan_seed,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactorizedRealizationRoot {
    lifecycle: PhysicalAtomId,
    carriers: BTreeMap<SemanticId, PhysicalAtomId>,
    fields: BTreeMap<SemanticId, FactorizedFieldRule>,
    relations: BTreeMap<SemanticId, FactorizedRelationRule>,
}

impl FactorizedRealizationRoot {
    pub fn validate_direct_physical_root(
        &self,
        atoms: &PhysicalAtomStore,
    ) -> Result<(), RealizationError> {
        self.to_direct_durable_root()?;
        match atom_payload(atoms, self.lifecycle)? {
            PhysicalAtomPayload::Lifecycle(_) => {}
            payload => {
                return Err(RealizationError::CodecMismatch {
                    atom: self.lifecycle,
                    expected: PhysicalCodec::Lifecycle,
                    actual: payload.codec(),
                });
            }
        }
        for (&owner, &carrier) in &self.carriers {
            let entities = match atom_payload(atoms, carrier)? {
                PhysicalAtomPayload::EntityOrder(entities) => entities,
                payload => {
                    return Err(RealizationError::CodecMismatch {
                        atom: carrier,
                        expected: PhysicalCodec::EntityOrder,
                        actual: payload.codec(),
                    });
                }
            };
            if entities.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(RealizationError::RealizationChunkShapeMismatch);
            }
            for rule in self.fields.values().filter(|rule| rule.owner == owner) {
                let FactorizedFieldExpr::Direct(atom) = rule.expr else {
                    return Err(RealizationError::NonDirectDurableRealization);
                };
                match atom_payload(atoms, atom)? {
                    PhysicalAtomPayload::FieldColumnSegment(column) => {
                        if column.len() != entities.len()
                            || column
                                .iter()
                                .zip(entities.iter().copied())
                                .any(|((entity, _), expected)| entity != expected)
                        {
                            return Err(RealizationError::RealizationChunkShapeMismatch);
                        }
                    }
                    PhysicalAtomPayload::PackedSumFieldSegment(column) => {
                        if column.entities() != entities.as_slice() {
                            return Err(RealizationError::RealizationChunkShapeMismatch);
                        }
                    }
                    payload => {
                        return Err(RealizationError::CodecMismatch {
                            atom,
                            expected: PhysicalCodec::FieldColumnSegment,
                            actual: payload.codec(),
                        });
                    }
                }
            }
        }
        for rule in self.fields.values() {
            if !self.carriers.contains_key(&rule.owner) {
                return Err(RealizationError::MissingFactorizedCarrier(rule.owner));
            }
        }
        for rule in self.relations.values() {
            if rule.columns.len() != rule.column_order.len() {
                return Err(RealizationError::RealizationChunkShapeMismatch);
            }
            for column in &rule.column_order {
                let FactorizedRelationColumnExpr::Direct(atom) = rule
                    .columns
                    .get(column)
                    .ok_or(RealizationError::RealizationChunkShapeMismatch)?
                else {
                    return Err(RealizationError::NonDirectDurableRealization);
                };
                match atom_payload(atoms, *atom)? {
                    PhysicalAtomPayload::RelationColumnSegment(segment)
                        if segment.start_row() == 0 && segment.len() == rule.row_count => {}
                    PhysicalAtomPayload::PackedSumRelationSegment(segment)
                        if segment.start_row() == 0 && segment.len() == rule.row_count => {}
                    PhysicalAtomPayload::RelationColumnSegment(_)
                    | PhysicalAtomPayload::PackedSumRelationSegment(_) => {
                        return Err(RealizationError::RealizationChunkShapeMismatch);
                    }
                    payload => {
                        return Err(RealizationError::CodecMismatch {
                            atom: *atom,
                            expected: PhysicalCodec::RelationColumnSegment,
                            actual: payload.codec(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    pub fn to_direct_durable_root(
        &self,
    ) -> Result<DirectFactorizedRealizationRoot, RealizationError> {
        let fields = self
            .fields
            .iter()
            .map(|(&field, rule)| match rule.expr {
                FactorizedFieldExpr::Direct(atom) => Ok((
                    field,
                    DirectFactorizedFieldRoot {
                        owner: rule.owner,
                        atom,
                    },
                )),
                _ => Err(RealizationError::NonDirectDurableRealization),
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let relations = self
            .relations
            .iter()
            .map(|(&relation, rule)| {
                if rule.delta_overlay.is_some()
                    || rule.base_witness.is_some()
                    || rule.scan_seed.is_some()
                {
                    return Err(RealizationError::NonDirectDurableRealization);
                }
                let column_atoms = rule
                    .columns
                    .iter()
                    .map(|(&column, expr)| match expr {
                        FactorizedRelationColumnExpr::Direct(atom) => Ok((column, *atom)),
                        _ => Err(RealizationError::NonDirectDurableRealization),
                    })
                    .collect::<Result<BTreeMap<_, _>, _>>()?;
                Ok((
                    relation,
                    DirectFactorizedRelationRoot {
                        column_order: rule.column_order.clone(),
                        column_atoms,
                        row_count: rule.row_count,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        Ok(DirectFactorizedRealizationRoot {
            lifecycle: self.lifecycle,
            carriers: self.carriers.clone(),
            fields,
            relations,
        })
    }

    #[must_use]
    pub fn from_direct_durable_root(root: DirectFactorizedRealizationRoot) -> Self {
        let fields = root
            .fields
            .into_iter()
            .map(|(field, direct)| {
                (
                    field,
                    FactorizedFieldRule {
                        owner: direct.owner,
                        expr: FactorizedFieldExpr::Direct(direct.atom),
                    },
                )
            })
            .collect();
        let relations = root
            .relations
            .into_iter()
            .map(|(relation, direct)| {
                (
                    relation,
                    FactorizedRelationRule {
                        column_order: direct.column_order,
                        columns: direct
                            .column_atoms
                            .into_iter()
                            .map(|(column, atom)| {
                                (column, FactorizedRelationColumnExpr::Direct(atom))
                            })
                            .collect(),
                        row_count: direct.row_count,
                        delta_overlay: None,
                        base_witness: None,
                        scan_seed: None,
                    },
                )
            })
            .collect();
        Self {
            lifecycle: root.lifecycle,
            carriers: root.carriers,
            fields,
            relations,
        }
    }
    #[must_use]
    pub const fn fields(&self) -> &BTreeMap<SemanticId, FactorizedFieldRule> {
        &self.fields
    }

    #[must_use]
    pub const fn relations(&self) -> &BTreeMap<SemanticId, FactorizedRelationRule> {
        &self.relations
    }

    pub fn read_field(
        &self,
        atoms: &PhysicalAtomStore,
        field: SemanticId,
        entity: EntityId,
    ) -> Result<Value, RealizationError> {
        self.fields
            .get(&field)
            .ok_or(RealizationError::MissingFactorizedField(field))?
            .expr
            .evaluate_entity(atoms, entity)
    }

    pub fn visit_field_carrier_range<F>(
        &self,
        atoms: &PhysicalAtomStore,
        field: SemanticId,
        start_ordinal: usize,
        end_ordinal: usize,
        mut visit: F,
    ) -> Result<(), RealizationError>
    where
        F: FnMut(EntityId, Value),
    {
        let rule = self
            .fields
            .get(&field)
            .ok_or(RealizationError::MissingFactorizedField(field))?;
        let carrier = *self
            .carriers
            .get(&rule.owner)
            .ok_or(RealizationError::MissingFactorizedCarrier(rule.owner))?;
        rule.expr
            .visit_carrier_range(atoms, carrier, start_ordinal, end_ordinal, &mut visit)
    }

    pub(crate) fn visit_execution_relation_rows(
        &self,
        atoms: &PhysicalAtomStore,
        relation: SemanticId,
        visit: &mut dyn FnMut(Vec<Value>) -> Result<(), RealizationError>,
    ) -> Result<(), RealizationError> {
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        for row in 0..rule.row_count {
            let values = rule
                .column_order
                .iter()
                .map(|column| rule.value_at(atoms, *column, row))
                .collect::<Result<Vec<_>, _>>()?;
            visit(values)?;
        }
        Ok(())
    }

    pub(crate) fn execution_relation_dependencies(
        &self,
        relation: SemanticId,
    ) -> Result<BTreeSet<PhysicalAtomId>, RealizationError> {
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        let mut dependencies = BTreeSet::new();
        rule.collect_dependencies(&mut dependencies);
        Ok(dependencies)
    }

    pub(crate) fn visit_execution_relation_column(
        &self,
        atoms: &PhysicalAtomStore,
        relation: SemanticId,
        ordinal: usize,
        visit: &mut dyn FnMut(Value),
    ) -> Result<(), RealizationError> {
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        let column = *rule.column_order.get(ordinal).ok_or(
            RealizationError::FactorizedRelationColumnArityMismatch(relation),
        )?;
        rule.visit_column_range(atoms, column, 0, rule.row_count, &mut |value| visit(value))
    }

    pub(crate) fn execution_relation_arity(
        &self,
        relation: SemanticId,
    ) -> Result<usize, RealizationError> {
        self.relations
            .get(&relation)
            .map(|rule| rule.column_order.len())
            .ok_or(RealizationError::MissingFactorizedRelation(relation))
    }

    pub fn read_relation_column(
        &self,
        atoms: &PhysicalAtomStore,
        relation: SemanticId,
        column: SemanticId,
        row: usize,
    ) -> Result<Value, RealizationError> {
        self.relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?
            .value_at(atoms, column, row)
    }

    pub fn visit_relation_column_range<F>(
        &self,
        atoms: &PhysicalAtomStore,
        relation: SemanticId,
        column: SemanticId,
        start: usize,
        end: usize,
        mut visit: F,
    ) -> Result<(), RealizationError>
    where
        F: FnMut(Value),
    {
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        if end > rule.row_count {
            return Err(RealizationError::FactorizedRelationRowOutOfBounds {
                row: end.saturating_sub(1),
                row_count: rule.row_count,
            });
        }
        rule.visit_column_range(atoms, column, start, end, &mut visit)
    }

    pub fn evaluate(&self, atoms: &PhysicalAtomStore) -> Result<DatabaseState, RealizationError> {
        let lifecycle = match atom_payload(atoms, self.lifecycle)? {
            PhysicalAtomPayload::Lifecycle(graph) => graph.clone(),
            payload => {
                return Err(RealizationError::CodecMismatch {
                    atom: self.lifecycle,
                    expected: PhysicalCodec::Lifecycle,
                    actual: payload.codec(),
                });
            }
        };
        let mut state = DatabaseState {
            lifecycle: lifecycle.into(),
            ..DatabaseState::default()
        };
        for (&semantic, &atom) in &self.carriers {
            match atom_payload(atoms, atom)? {
                PhysicalAtomPayload::EntityOrder(entities) => {
                    state
                        .model
                        .carriers
                        .insert(semantic, entities.iter().copied().collect());
                }
                payload => {
                    return Err(RealizationError::CodecMismatch {
                        atom,
                        expected: PhysicalCodec::EntityOrder,
                        actual: payload.codec(),
                    });
                }
            }
        }
        for (&field, rule) in &self.fields {
            let entities = state
                .model
                .carriers
                .get(&rule.owner)
                .ok_or(RealizationError::MissingFactorizedCarrier(rule.owner))?;
            for &entity in entities {
                state
                    .model
                    .fields
                    .insert((field, entity), rule.expr.evaluate_entity(atoms, entity)?);
            }
        }
        for (&semantic, rule) in &self.relations {
            state
                .model
                .relations
                .insert(semantic, rule.evaluate_rows(atoms)?);
        }
        Ok(state)
    }

    #[must_use]
    pub fn dependencies(&self) -> BTreeSet<PhysicalAtomId> {
        let mut dependencies = BTreeSet::from([self.lifecycle]);
        dependencies.extend(self.carriers.values().copied());
        for rule in self.fields.values() {
            rule.expr.collect_dependencies(&mut dependencies);
        }
        for rule in self.relations.values() {
            rule.collect_dependencies(&mut dependencies);
        }
        dependencies
    }

    pub fn materialize_field_column(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        field: SemanticId,
        entities: impl IntoIterator<Item = EntityId>,
    ) -> Result<PhysicalAtomId, RealizationError> {
        let expr = self
            .fields
            .get(&field)
            .ok_or(RealizationError::MissingFactorizedField(field))?;
        let expr = &expr.expr;
        let entries = entities
            .into_iter()
            .map(|entity| {
                expr.evaluate_entity(atoms, entity)
                    .map(|value| (entity, value))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let atom = insert_field_column_atom(atoms, entries)?;
        self.fields.get_mut(&field).expect("field exists").expr = FactorizedFieldExpr::Direct(atom);
        Ok(atom)
    }

    pub fn materialize_field_column_chunk(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        field: SemanticId,
        entity: EntityId,
        chunk_entities: usize,
    ) -> Result<PhysicalAtomId, RealizationError> {
        if chunk_entities == 0 {
            return Err(RealizationError::InvalidRealizationChunkSize);
        }
        let rule = self
            .fields
            .get(&field)
            .ok_or(RealizationError::MissingFactorizedField(field))?;
        let carrier = *self
            .carriers
            .get(&rule.owner)
            .ok_or(RealizationError::MissingFactorizedCarrier(rule.owner))?;
        let order = entity_order(atoms, carrier)?;
        let entity_count = order.len();
        let ordinal = order
            .binary_search(&entity)
            .map_err(|_| RealizationError::MissingFactorizedCarrierEntity { carrier, entity })?;
        let current = rule.expr.clone();
        let (base, native_chunks) = match current {
            FactorizedFieldExpr::ChunkOverlay {
                base,
                carrier: existing_carrier,
                entity_count: existing_count,
                chunk_entities: existing_chunk_entities,
                native_chunks,
            } => {
                if existing_carrier != carrier
                    || existing_count != entity_count
                    || existing_chunk_entities != chunk_entities
                {
                    return Err(RealizationError::RealizationChunkShapeMismatch);
                }
                (base, native_chunks)
            }
            other => (Box::new(other), Vec::new()),
        };
        let chunk = ordinal / chunk_entities;
        if let Some(atom) = native_chunks.iter().find_map(|native| {
            (native.segment.start_ordinal() / chunk_entities == chunk).then_some(native.atom)
        }) {
            return Ok(atom);
        }
        let start = chunk * chunk_entities;
        let end = usize::min(start + chunk_entities, entity_count);
        let entries = order[start..end]
            .iter()
            .copied()
            .map(|entity| {
                base.evaluate_entity(atoms, entity)
                    .map(|value| (entity, value))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let first_entity = order[start];
        let last_entity = order[end - 1];
        let atom = atoms.insert(PhysicalAtomPayload::FieldColumnSegment(
            FieldColumnSegment::from_sorted(entries)?,
        ));
        let segment = CarrierSegmentCoordinate::new(carrier, start, end - start);
        let native = FactorizedFieldNativeChunk {
            segment,
            first_entity,
            last_entity,
            atom,
        };
        let mut native_chunks = native_chunks;
        let insert_at = native_chunks
            .binary_search_by_key(&start, |chunk| chunk.segment.start_ordinal())
            .unwrap_or_else(|index| index);
        native_chunks.insert(insert_at, native);
        self.fields.get_mut(&field).expect("field exists").expr =
            FactorizedFieldExpr::ChunkOverlay {
                base,
                carrier,
                entity_count,
                chunk_entities,
                native_chunks,
            };
        Ok(atom)
    }

    /// Installs a current-schema value as a native physical overlay.
    ///
    /// This is a realization operation, not a semantic conflict engine: the
    /// caller must already hold the exact current-world write authority.  The
    /// old/derived base is never inverted, which keeps non-injective migration
    /// transforms writable without reviving the source schema.
    pub fn install_field_value_overlay(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        field: SemanticId,
        entity: EntityId,
        value: Value,
        chunk_entities: usize,
    ) -> Result<PhysicalAtomId, RealizationError> {
        self.materialize_field_column_chunk(atoms, field, entity, chunk_entities)?;
        let expr = &mut self
            .fields
            .get_mut(&field)
            .ok_or(RealizationError::MissingFactorizedField(field))?
            .expr;
        let FactorizedFieldExpr::ChunkOverlay { native_chunks, .. } = expr else {
            return Err(RealizationError::RealizationChunkShapeMismatch);
        };
        let chunk = native_chunks
            .iter_mut()
            .find(|chunk| chunk.contains_entity(entity))
            .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
        let replacement = match atom_payload(atoms, chunk.atom)? {
            PhysicalAtomPayload::FieldColumnSegment(column) => {
                let mut entries = column
                    .iter()
                    .map(|(id, v)| (id, v.clone()))
                    .collect::<Vec<_>>();
                let slot = entries
                    .binary_search_by_key(&entity, |(id, _)| *id)
                    .map_err(|_| RealizationError::MissingFactorizedFieldValue {
                        atom: chunk.atom,
                        entity,
                    })?;
                entries[slot].1 = value;
                FieldColumnSegment::from_sorted(entries)?
            }
            payload => {
                return Err(RealizationError::CodecMismatch {
                    atom: chunk.atom,
                    expected: PhysicalCodec::FieldColumnSegment,
                    actual: payload.codec(),
                });
            }
        };
        let atom = atoms.insert(PhysicalAtomPayload::FieldColumnSegment(replacement));
        chunk.atom = atom;
        Ok(atom)
    }

    pub fn field_native_chunk_count(&self, field: SemanticId) -> Result<usize, RealizationError> {
        let expr = &self
            .fields
            .get(&field)
            .ok_or(RealizationError::MissingFactorizedField(field))?
            .expr;
        Ok(match expr {
            FactorizedFieldExpr::ChunkOverlay { native_chunks, .. } => native_chunks.len(),
            FactorizedFieldExpr::Direct(_) => 1,
            _ => 0,
        })
    }

    pub fn materialize_relation_column_chunk(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
        column: SemanticId,
        row: usize,
        chunk_rows: usize,
    ) -> Result<PhysicalAtomId, RealizationError> {
        if chunk_rows == 0 {
            return Err(RealizationError::InvalidRealizationChunkSize);
        }
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        if rule
            .delta_overlay
            .as_ref()
            .is_some_and(|overlay| !overlay.is_identity())
        {
            return Err(RealizationError::RelationDeltaOverlayUnavailable(relation));
        }
        let row_count = rule.row_count;
        if row >= row_count {
            return Err(RealizationError::FactorizedRelationRowOutOfBounds { row, row_count });
        }
        let current = rule
            .columns
            .get(&column)
            .ok_or(RealizationError::MissingFactorizedRelationColumn { relation, column })?
            .clone();
        let (base, native_chunks) = match current {
            FactorizedRelationColumnExpr::ChunkOverlay {
                base,
                row_count: overlay_row_count,
                chunk_rows: existing,
                native_chunks,
            } => {
                if existing != chunk_rows || overlay_row_count != row_count {
                    return Err(RealizationError::RealizationChunkShapeMismatch);
                }
                (base, native_chunks)
            }
            other => (Box::new(other), Vec::new()),
        };
        let chunk = row / chunk_rows;
        if let Some(atom) = native_chunks
            .iter()
            .find_map(|(index, atom)| (*index == chunk).then_some(*atom))
        {
            return Ok(atom);
        }
        let start = chunk * chunk_rows;
        let end = usize::min(start + chunk_rows, row_count);
        let values = (start..end)
            .map(|row| base.evaluate_row(atoms, row))
            .collect::<Result<Vec<_>, _>>()?;
        let atom = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
            RelationColumnSegment::with_start_row(start, values),
        ));
        let mut native_chunks = native_chunks;
        let insert_at = native_chunks
            .binary_search_by_key(&chunk, |(index, _)| *index)
            .unwrap_or_else(|index| index);
        native_chunks.insert(insert_at, (chunk, atom));
        self.relations
            .get_mut(&relation)
            .expect("relation exists")
            .columns
            .insert(
                column,
                FactorizedRelationColumnExpr::ChunkOverlay {
                    base,
                    row_count,
                    chunk_rows,
                    native_chunks,
                },
            );
        Ok(atom)
    }

    /// Installs a current-schema relation cell as a native physical overlay.
    /// The semantic write/conflict certificate remains owned by kernel-change;
    /// this layer only realizes its already-certified endpoint.
    pub fn install_relation_cell_overlay(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
        column: SemanticId,
        row: usize,
        value: Value,
        chunk_rows: usize,
    ) -> Result<PhysicalAtomId, RealizationError> {
        self.materialize_relation_column_chunk(atoms, relation, column, row, chunk_rows)?;
        let expr = self
            .relations
            .get_mut(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?
            .columns
            .get_mut(&column)
            .ok_or(RealizationError::MissingFactorizedRelationColumn { relation, column })?;
        let FactorizedRelationColumnExpr::ChunkOverlay {
            chunk_rows: existing_chunk_rows,
            native_chunks,
            ..
        } = expr
        else {
            return Err(RealizationError::RealizationChunkShapeMismatch);
        };
        let chunk_index = row / *existing_chunk_rows;
        let (_, old_atom) = native_chunks
            .iter_mut()
            .find(|(index, _)| *index == chunk_index)
            .ok_or(RealizationError::RealizationChunkShapeMismatch)?;
        let replacement = match atom_payload(atoms, *old_atom)? {
            PhysicalAtomPayload::RelationColumnSegment(segment) => {
                let mut values = segment.values.clone();
                let offset = row
                    .checked_sub(segment.start_row)
                    .filter(|offset| *offset < values.len())
                    .ok_or(RealizationError::FactorizedRelationRowOutOfBounds {
                        row,
                        row_count: segment.start_row + values.len(),
                    })?;
                values[offset] = value;
                RelationColumnSegment::with_start_row(segment.start_row, values)
            }
            payload => {
                return Err(RealizationError::CodecMismatch {
                    atom: *old_atom,
                    expected: PhysicalCodec::RelationColumnSegment,
                    actual: payload.codec(),
                });
            }
        };
        let atom = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(replacement));
        *old_atom = atom;
        Ok(atom)
    }

    /// Detaches one current-schema relation from any derived physical ancestry
    /// by installing the exact endpoint of an already prepared B-semantic
    /// relation rewrite as native B columns.
    ///
    /// The prepared rewrite is bound to the exact current relation support by
    /// `kernel-query`; this layer only materializes that certified endpoint.
    /// No inverse migration or source-schema row identity participates.
    /// Applies one already-authoritative current-schema relation delta without
    /// materializing the surrounding logical `DatabaseState`. Only the touched
    /// relation is decoded from factorized columns; the exact Γ delta law is
    /// still owned by kernel-query. Untouched physical atoms remain shared.
    pub fn apply_exact_relation_delta_endpoint(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
        delta: &RelationDelta,
        context: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Result<Vec<PhysicalAtomId>, RealizationError> {
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        let old_rows = rule.evaluate_rows(atoms)?;
        let old = match &delta.result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(old_rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows: old_rows,
                column_equivalences: column_equivalences.clone(),
            },
        };
        let endpoint = delta
            .apply_to_value(old, context, registry)
            .map_err(RealizationError::RelationQuery)?;
        let rows = endpoint.into_rows();
        let row_count = rows.len();
        let column_order = rule.column_order.clone();
        let mut column_values = vec![Vec::with_capacity(row_count); column_order.len()];
        for row in rows {
            if row.len() != column_order.len() {
                return Err(RealizationError::FactorizedRelationColumnArityMismatch(
                    relation,
                ));
            }
            for (ordinal, value) in row.into_iter().enumerate() {
                column_values[ordinal].push(value);
            }
        }
        let mut columns = BTreeMap::new();
        let mut native_atoms = Vec::with_capacity(column_order.len());
        for (column, values) in column_order.iter().copied().zip(column_values) {
            let atom = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
                RelationColumnSegment::new(values),
            ));
            native_atoms.push(atom);
            columns.insert(column, FactorizedRelationColumnExpr::Direct(atom));
        }
        self.relations.insert(
            relation,
            FactorizedRelationRule {
                column_order,
                columns,
                row_count,
                delta_overlay: None,
                base_witness: None,
                scan_seed: None,
            },
        );
        Ok(native_atoms)
    }

    pub fn install_prepared_relation_endpoint<I>(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
        prepared: &PreparedRelationRewrite<I>,
        registry: &SemanticRegistry,
    ) -> Result<Vec<PhysicalAtomId>, RealizationError> {
        if prepared.relation() != relation {
            return Err(RealizationError::PreparedRelationMismatch(relation));
        }
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        let old_rows = rule.evaluate_rows(atoms)?;
        let old = match &prepared.delta().result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(old_rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows: old_rows,
                column_equivalences: column_equivalences.clone(),
            },
        };
        let endpoint = prepared
            .apply_structural(&old, registry)
            .map_err(RealizationError::RelationQuery)?;
        let rows = endpoint.into_rows();
        let row_count = rows.len();
        let column_order = rule.column_order.clone();
        let mut column_values = vec![Vec::with_capacity(rows.len()); column_order.len()];
        for row in rows {
            if row.len() != column_order.len() {
                return Err(RealizationError::FactorizedRelationColumnArityMismatch(
                    relation,
                ));
            }
            for (ordinal, value) in row.into_iter().enumerate() {
                column_values[ordinal].push(value);
            }
        }
        let mut columns = BTreeMap::new();
        let mut native_atoms = Vec::with_capacity(column_order.len());
        for (column, values) in column_order.iter().copied().zip(column_values) {
            let atom = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
                RelationColumnSegment::new(values),
            ));
            native_atoms.push(atom);
            columns.insert(column, FactorizedRelationColumnExpr::Direct(atom));
        }
        self.relations.insert(
            relation,
            FactorizedRelationRule {
                column_order,
                columns,
                row_count,
                delta_overlay: None,
                base_witness: None,
                scan_seed: None,
            },
        );
        Ok(native_atoms)
    }

    pub fn relation_base_witness(
        &self,
        relation: SemanticId,
    ) -> Result<&RelationBaseWitness, RealizationError> {
        self.relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?
            .base_witness
            .as_ref()
            .ok_or(RealizationError::RelationDeltaOverlayUnavailable(relation))
    }

    pub fn relation_scan_occurrence_seed(
        &self,
        relation: SemanticId,
    ) -> Result<RelationScanOccurrenceSeed, RealizationError> {
        self.relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?
            .scan_seed
            .clone()
            .ok_or(RealizationError::RelationDeltaOverlayUnavailable(relation))
    }

    pub fn relation_delta_overlay_stats(
        &self,
        relation: SemanticId,
    ) -> Result<Option<RelationDeltaOverlayStats>, RealizationError> {
        Ok(self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?
            .delta_overlay
            .as_ref()
            .map(RelationDeltaOverlay::stats))
    }

    /// Installs an exact current-B relation delta as an immutable physical
    /// overlay. The semantic endpoint is already sealed by `kernel-query`;
    /// this method only resolves touched Γ classes to stable physical slots.
    /// No source-schema provenance or whole-relation reconstruction occurs.
    pub fn install_prepared_relation_delta_overlay<I>(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
        prepared: &PreparedRelationRewrite<I>,
        context: &SemanticContext,
        _registry: &SemanticRegistry,
    ) -> Result<Option<PhysicalAtomId>, RealizationError> {
        if prepared.relation() != relation {
            return Err(RealizationError::PreparedRelationMismatch(relation));
        }
        let rule = self
            .relations
            .get_mut(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        let witness = rule
            .base_witness
            .as_ref()
            .ok_or(RealizationError::RelationDeltaOverlayUnavailable(relation))?;
        if witness.semantic_context() != context {
            return Err(RealizationError::SemanticMismatch);
        }
        if !prepared.base_witness().certifies_same_base(witness) {
            return Err(RealizationError::RelationQuery(
                kernel_query::RelQueryError::StructuralRewriteBaseMismatch,
            ));
        }
        let overlay = rule
            .delta_overlay
            .as_mut()
            .ok_or(RealizationError::RelationDeltaOverlayUnavailable(relation))?;
        let (atom, removed_positions) = overlay.apply_prepared_delta(atoms, prepared)?;
        let seed = rule
            .scan_seed
            .as_mut()
            .ok_or(RealizationError::RelationDeltaOverlayUnavailable(relation))?;
        seed.apply_prepared_storage_transition(prepared, &removed_positions)
            .map_err(RealizationError::RelationQuery)?;
        let next_witness = prepared.advanced_base_witness(RevisionId::new(0));
        rule.row_count = overlay.row_count();
        rule.base_witness = Some(next_witness);
        Ok(atom)
    }

    /// Compacts a bounded relation-delta overlay back into native factorized
    /// columns while preserving the same semantic relation witness.
    pub fn compact_relation_delta_overlay(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
        _context: &SemanticContext,
        _registry: &SemanticRegistry,
    ) -> Result<Vec<PhysicalAtomId>, RealizationError> {
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        if rule.delta_overlay.is_none() {
            return Ok(Vec::new());
        }
        let rows = rule.evaluate_rows(atoms)?;
        let row_count = rows.len();
        let column_order = rule.column_order.clone();
        let scan_seed = rule
            .scan_seed
            .clone()
            .ok_or(RealizationError::RelationDeltaOverlayUnavailable(relation))?;
        let witness_value = rule
            .base_witness
            .as_ref()
            .ok_or(RealizationError::RelationDeltaOverlayUnavailable(relation))?
            .rebind_dense_storage_identity_from_seed(RevisionId::new(0), &scan_seed)
            .map_err(RealizationError::RelationQuery)?;
        if witness_value.result_type().columns.len() != column_order.len() {
            return Err(RealizationError::FactorizedRelationColumnArityMismatch(
                relation,
            ));
        }
        let witness = Some(witness_value);
        let next_overlay = RelationDeltaOverlay::from_base_row_count(row_count);
        let mut column_values = vec![Vec::with_capacity(row_count); column_order.len()];
        for row in rows {
            for (ordinal, value) in row.into_iter().enumerate() {
                column_values[ordinal].push(value);
            }
        }
        let mut columns = BTreeMap::new();
        let mut native_atoms = Vec::with_capacity(column_order.len());
        for (column, values) in column_order.iter().copied().zip(column_values) {
            let atom = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
                RelationColumnSegment::new(values),
            ));
            native_atoms.push(atom);
            columns.insert(column, FactorizedRelationColumnExpr::Direct(atom));
        }
        self.relations.insert(
            relation,
            FactorizedRelationRule {
                column_order,
                columns,
                row_count,
                delta_overlay: Some(next_overlay),
                base_witness: witness,
                scan_seed: Some(scan_seed),
            },
        );
        Ok(native_atoms)
    }

    pub fn relation_column_native_chunk_count(
        &self,
        relation: SemanticId,
        column: SemanticId,
    ) -> Result<usize, RealizationError> {
        let expr = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?
            .columns
            .get(&column)
            .ok_or(RealizationError::MissingFactorizedRelationColumn { relation, column })?;
        Ok(match expr {
            FactorizedRelationColumnExpr::ChunkOverlay { native_chunks, .. } => native_chunks.len(),
            FactorizedRelationColumnExpr::Direct(_) => 1,
            _ => 0,
        })
    }

    pub fn materialize_relation_column(
        &mut self,
        atoms: &mut PhysicalAtomStore,
        relation: SemanticId,
        column: SemanticId,
    ) -> Result<PhysicalAtomId, RealizationError> {
        let rule = self
            .relations
            .get(&relation)
            .ok_or(RealizationError::MissingFactorizedRelation(relation))?;
        if rule
            .delta_overlay
            .as_ref()
            .is_some_and(|overlay| !overlay.is_identity())
        {
            return Err(RealizationError::RelationDeltaOverlayUnavailable(relation));
        }
        if !rule.columns.contains_key(&column) {
            return Err(RealizationError::MissingFactorizedRelationColumn { relation, column });
        }
        let values = (0..rule.row_count)
            .map(|row| rule.value_at(atoms, column, row))
            .collect::<Result<Vec<_>, _>>()?;
        let atom = insert_relation_column_atom(atoms, 0, values)?;
        self.relations
            .get_mut(&relation)
            .expect("relation exists")
            .columns
            .insert(column, FactorizedRelationColumnExpr::Direct(atom));
        Ok(atom)
    }
}

pub fn realize_database_state_factorized(
    state: &DatabaseState,
    context: &SemanticContext,
) -> Result<(PhysicalAtomStore, FactorizedRealizationRoot), RealizationError> {
    let mut atoms = PhysicalAtomStore::default();
    let lifecycle = atoms.insert(PhysicalAtomPayload::Lifecycle((*state.lifecycle).clone()));
    let carriers = state
        .model
        .carriers
        .iter()
        .map(|(&semantic, entities)| {
            let atom = atoms.insert(PhysicalAtomPayload::EntityOrder(
                entities.iter().copied().collect(),
            ));
            (semantic, atom)
        })
        .collect::<BTreeMap<_, _>>();

    let mut grouped = BTreeMap::<SemanticId, Vec<(EntityId, Value)>>::new();
    for (&(field, entity), value) in &state.model.fields {
        grouped
            .entry(field)
            .or_default()
            .push((entity, value.clone()));
    }
    let mut fields = BTreeMap::new();
    for (field, entries) in grouped {
        let atom = insert_field_column_atom(&mut atoms, entries)?;
        let owner = context
            .schema
            .field(field)
            .ok_or(RealizationError::MissingFactorizedField(field))?
            .owner;
        fields.insert(
            field,
            FactorizedFieldRule {
                owner,
                expr: FactorizedFieldExpr::Direct(atom),
            },
        );
    }
    let mut relations = BTreeMap::new();
    for (&semantic, rows) in &state.model.relations {
        let column_order = context
            .schema
            .relation_column_ids(semantic)
            .ok_or(RealizationError::MissingFactorizedRelation(semantic))?
            .to_vec();
        let mut column_values = vec![Vec::with_capacity(rows.len()); column_order.len()];
        for row in rows {
            if row.len() != column_order.len() {
                return Err(RealizationError::FactorizedRelationColumnArityMismatch(
                    semantic,
                ));
            }
            for (ordinal, value) in row.iter().enumerate() {
                column_values[ordinal].push(value.clone());
            }
        }
        let mut columns = BTreeMap::new();
        for (column, values) in column_order.iter().copied().zip(column_values) {
            let atom = insert_relation_column_atom(&mut atoms, 0, values)?;
            columns.insert(column, FactorizedRelationColumnExpr::Direct(atom));
        }
        relations.insert(
            semantic,
            FactorizedRelationRule {
                column_order,
                columns,
                row_count: rows.len(),
                delta_overlay: None,
                base_witness: None,
                scan_seed: None,
            },
        );
    }

    Ok((
        atoms,
        FactorizedRealizationRoot {
            lifecycle,
            carriers,
            fields,
            relations,
        },
    ))
}

fn normalize_factorized_field_rewrite(
    source_root: &FactorizedRealizationRoot,
    rewrite: &kernel_transport::MigrationFieldRewrite,
) -> Result<FactorizedFieldExpr, RealizationError> {
    use kernel_query::Expr;

    if rewrite.source_fields.len() == 1 {
        let source_id = rewrite.source_fields[0];
        let source_expr = source_root
            .fields
            .get(&source_id)
            .ok_or(RealizationError::MissingFactorizedField(source_id))?
            .expr
            .clone();
        if let FactorizedFieldExpr::Direct(atom) = source_expr {
            match rewrite.transform.root() {
                Expr::ProductField { input, field }
                    if matches!(input.as_ref(), Expr::Input) && *field == source_id =>
                {
                    return Ok(FactorizedFieldExpr::Direct(atom));
                }
                Expr::I64ToF64(inner) => {
                    if let Expr::ProductField { input, field } = inner.as_ref()
                        && matches!(input.as_ref(), Expr::Input)
                        && *field == source_id
                    {
                        return Ok(FactorizedFieldExpr::I64ToF64Direct(atom));
                    }
                }
                _ => {}
            }
        }
    }

    if rewrite.source_fields.is_empty()
        && let Expr::Const(value) | Expr::TypedConst { value, .. } = rewrite.transform.root()
    {
        return Ok(FactorizedFieldExpr::Constant(value.clone()));
    }

    let product = rewrite
        .source_fields
        .iter()
        .map(|source| {
            source_root
                .fields
                .get(source)
                .map(|rule| (*source, rule.expr.clone()))
                .ok_or(RealizationError::MissingFactorizedField(*source))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(FactorizedFieldExpr::Transform {
        source: Box::new(FactorizedFieldExpr::Product(product)),
        query: rewrite.transform.clone(),
    })
}

fn normalize_factorized_relation_column_rewrite(
    source_rule: &FactorizedRelationRule,
    rewrite: &kernel_transport::MigrationColumnRewrite,
) -> Result<FactorizedRelationColumnExpr, RealizationError> {
    use kernel_query::Expr;

    if rewrite.source_columns.len() == 1 {
        let source_id = rewrite.source_columns[0];
        let source_expr = source_rule.columns.get(&source_id).ok_or(
            RealizationError::MissingFactorizedRelationColumn {
                relation: SemanticId::new(0),
                column: source_id,
            },
        )?;
        if let FactorizedRelationColumnExpr::Direct(atom) = source_expr {
            match rewrite.transform.root() {
                Expr::ProductField { input, field }
                    if matches!(input.as_ref(), Expr::Input) && *field == source_id =>
                {
                    return Ok(FactorizedRelationColumnExpr::Direct(*atom));
                }
                Expr::I64ToF64(inner) => {
                    if let Expr::ProductField { input, field } = inner.as_ref()
                        && matches!(input.as_ref(), Expr::Input)
                        && *field == source_id
                    {
                        return Ok(FactorizedRelationColumnExpr::I64ToF64Direct(*atom));
                    }
                }
                _ => {}
            }
        }
    }

    if rewrite.source_columns.is_empty()
        && let Expr::Const(value) | Expr::TypedConst { value, .. } = rewrite.transform.root()
    {
        return Ok(FactorizedRelationColumnExpr::Constant(value.clone()));
    }

    let product = rewrite
        .source_columns
        .iter()
        .map(|source| {
            source_rule
                .columns
                .get(source)
                .map(|expr| (*source, expr.clone()))
                .ok_or(RealizationError::MissingFactorizedRelationColumn {
                    relation: SemanticId::new(0),
                    column: *source,
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    Ok(FactorizedRelationColumnExpr::Transform {
        source: Box::new(FactorizedRelationColumnExpr::Product(product)),
        query: rewrite.transform.clone(),
    })
}

pub fn prepare_general_relation_factorized(
    source_root: &FactorizedRealizationRoot,
    atoms: &mut PhysicalAtomStore,
    source_context: &SemanticContext,
    registry: &SemanticRegistry,
    program: &SchemaMigrationProgram,
    target_relation: SemanticId,
) -> Result<PreparedFactorizedRelation, RealizationError> {
    let verified = program
        .verify(source_context, registry)
        .map_err(RealizationError::Migration)?;
    let rewrite = program
        .relation_rewrites()
        .iter()
        .find_map(|rewrite| match rewrite {
            kernel_transport::MigrationRelationRewrite::Query(rewrite)
                if rewrite.target_relation == target_relation =>
            {
                Some(rewrite)
            }
            _ => None,
        })
        .ok_or(
            RealizationError::GeneralRelationRewriteRequiresPreparedRealization(target_relation),
        )?;
    crate::one_shot::prepare_one_shot_relation(
        source_root,
        atoms,
        source_context,
        registry,
        verified.target(),
        target_relation,
        &rewrite.transform,
    )
}

pub fn compose_schema_migration_factorized_with_prepared(
    source_root: &FactorizedRealizationRoot,
    atoms: &PhysicalAtomStore,
    source_context: &SemanticContext,
    registry: &SemanticRegistry,
    program: &SchemaMigrationProgram,
    prepared: &[PreparedFactorizedRelation],
) -> Result<FactorizedRealizationRoot, RealizationError> {
    compose_schema_migration_factorized_impl(
        source_root,
        Some(atoms),
        source_context,
        registry,
        program,
        prepared,
    )
}

pub fn compose_schema_migration_factorized(
    source_root: &FactorizedRealizationRoot,
    source_context: &SemanticContext,
    registry: &SemanticRegistry,
    program: &SchemaMigrationProgram,
) -> Result<FactorizedRealizationRoot, RealizationError> {
    compose_schema_migration_factorized_impl(
        source_root,
        None,
        source_context,
        registry,
        program,
        &[],
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn compose_schema_migration_factorized_impl(
    source_root: &FactorizedRealizationRoot,
    atoms: Option<&PhysicalAtomStore>,
    source_context: &SemanticContext,
    registry: &SemanticRegistry,
    program: &SchemaMigrationProgram,
    prepared: &[PreparedFactorizedRelation],
) -> Result<FactorizedRealizationRoot, RealizationError> {
    let verified = program
        .verify(source_context, registry)
        .map_err(RealizationError::Migration)?;
    let target_context = verified.target();
    let rewrites = program
        .field_rewrites()
        .iter()
        .map(|rewrite| (rewrite.target_field, rewrite))
        .collect::<BTreeMap<_, _>>();
    let mut fields = BTreeMap::new();
    for target_field in target_context.schema.fields() {
        if source_context.schema.field(target_field.id) == Some(target_field) {
            let expr = source_root
                .fields
                .get(&target_field.id)
                .ok_or(RealizationError::MissingFactorizedField(target_field.id))?;
            fields.insert(target_field.id, expr.clone());
            continue;
        }
        let rewrite = rewrites
            .get(&target_field.id)
            .ok_or(RealizationError::Migration(
                TransportError::UnknownTargetField(target_field.id),
            ))?;
        fields.insert(
            target_field.id,
            FactorizedFieldRule {
                owner: target_field.owner,
                expr: normalize_factorized_field_rewrite(source_root, rewrite)?,
            },
        );
    }

    let relation_rewrites = program
        .relation_rewrites()
        .iter()
        .map(|rewrite| {
            let target = match rewrite {
                kernel_transport::MigrationRelationRewrite::Rows(row) => row.target_relation,
                kernel_transport::MigrationRelationRewrite::Query(query) => query.target_relation,
            };
            (target, rewrite)
        })
        .collect::<BTreeMap<_, _>>();
    let prepared = prepared
        .iter()
        .map(|relation| (relation.target_relation, relation))
        .collect::<BTreeMap<_, _>>();
    let mut relations = BTreeMap::new();
    for target_relation in target_context.schema.relations() {
        let target_id = target_relation.id;
        let target_column_order = target_context
            .schema
            .relation_column_ids(target_id)
            .ok_or(RealizationError::MissingFactorizedRelation(target_id))?
            .to_vec();

        if source_context.schema.relation(target_id) == Some(target_relation)
            && source_context.schema.relation_column_ids(target_id)
                == Some(target_column_order.as_slice())
        {
            let source_rule = source_root
                .relations
                .get(&target_id)
                .ok_or(RealizationError::MissingFactorizedRelation(target_id))?;
            relations.insert(target_id, source_rule.clone());
            continue;
        }

        let rewrite = relation_rewrites
            .get(&target_id)
            .ok_or(RealizationError::Migration(
                TransportError::UnknownTargetRelation(target_id),
            ))?;
        match rewrite {
            kernel_transport::MigrationRelationRewrite::Rows(rewrite) => {
                let source_rule = source_root.relations.get(&rewrite.source_relation).ok_or(
                    RealizationError::MissingFactorizedRelation(rewrite.source_relation),
                )?;
                let mut columns = BTreeMap::new();
                for column in &rewrite.columns {
                    columns.insert(
                        column.target_column,
                        normalize_factorized_relation_column_rewrite(source_rule, column)?,
                    );
                }
                if columns.len() != target_column_order.len()
                    || target_column_order
                        .iter()
                        .any(|column| !columns.contains_key(column))
                {
                    return Err(RealizationError::FactorizedRelationColumnArityMismatch(
                        target_id,
                    ));
                }
                relations.insert(
                    target_id,
                    FactorizedRelationRule {
                        column_order: target_column_order,
                        columns,
                        row_count: source_rule.row_count,
                        delta_overlay: source_rule.delta_overlay.clone(),
                        base_witness: source_rule.base_witness.clone(),
                        scan_seed: source_rule.scan_seed.clone(),
                    },
                );
            }
            kernel_transport::MigrationRelationRewrite::Query(rewrite) => {
                let prepared = prepared.get(&target_id).ok_or(
                    RealizationError::GeneralRelationRewriteRequiresPreparedRealization(target_id),
                )?;
                let expected_relations = rewrite.transform.scan_relations();
                let mut expected_source_atoms = BTreeSet::new();
                for relation in &expected_relations {
                    source_root
                        .relations
                        .get(relation)
                        .ok_or(RealizationError::MissingFactorizedRelation(*relation))?
                        .collect_dependencies(&mut expected_source_atoms);
                }
                if prepared.source_relations != expected_relations
                    || prepared.source_atoms != expected_source_atoms
                    || prepared.column_order != target_column_order
                {
                    return Err(RealizationError::PreparedRelationMismatch(target_id));
                }
                let atoms = atoms.ok_or(
                    RealizationError::GeneralRelationRewriteRequiresPreparedRealization(target_id),
                )?;
                for column in &target_column_order {
                    let atom = *prepared
                        .column_atoms
                        .get(column)
                        .ok_or(RealizationError::PreparedRelationMismatch(target_id))?;
                    match atom_payload(atoms, atom)? {
                        PhysicalAtomPayload::RelationColumnSegment(segment)
                            if segment.start_row == 0
                                && segment.values.len() == prepared.row_count => {}
                        PhysicalAtomPayload::RelationColumnSegment(_) => {
                            return Err(RealizationError::PreparedRelationMismatch(target_id));
                        }
                        payload => {
                            return Err(RealizationError::CodecMismatch {
                                atom,
                                expected: PhysicalCodec::RelationColumnSegment,
                                actual: payload.codec(),
                            });
                        }
                    }
                }
                let columns = prepared
                    .column_atoms
                    .iter()
                    .map(|(&column, &atom)| (column, FactorizedRelationColumnExpr::Direct(atom)))
                    .collect::<BTreeMap<_, _>>();
                relations.insert(
                    target_id,
                    FactorizedRelationRule {
                        column_order: target_column_order,
                        columns,
                        row_count: prepared.row_count,
                        delta_overlay: Some(prepared.delta_overlay.clone()),
                        base_witness: Some(prepared.base_witness.clone()),
                        scan_seed: Some(prepared.scan_seed.clone()),
                    },
                );
            }
        }
    }

    Ok(FactorizedRealizationRoot {
        lifecycle: source_root.lifecycle,
        carriers: source_root.carriers.clone(),
        fields,
        relations,
    })
}
