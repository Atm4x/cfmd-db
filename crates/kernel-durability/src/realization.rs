use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use kernel_change::RevisionEffectId;
use kernel_model::DatabaseState;
use kernel_realization::{
    DirectFactorizedFieldRoot, DirectFactorizedRealizationRoot, DirectFactorizedRelationRoot,
    FactorizedRealizationRoot, FieldColumnSegment, PhysicalAtomId, PhysicalAtomPayload,
    PhysicalAtomStore, RelationColumnSegment,
};
use kernel_revision::Revision;
use kernel_schema::SemanticContext;
use kernel_semantics::SemanticRegistry;
use kernel_types::{EntityId, RevisionId, SemanticId};

use crate::binary_codec::{
    BinarySink, BinarySource, CountingBinarySink, Cursor, ReadBinarySource, StreamingBinarySink,
    encode_rows, encode_value, push_bytes, push_len, push_u64, push_u128,
};
use crate::checkpoint::{decode_state, encode_state};
use crate::runtime::{CodecError, DurabilityError};
use crate::{DurableModelDelta, SingleFileContainer, SingleFileSectionKind};

const MAGIC: [u8; 4] = *b"CFPR";
const VERSION: u16 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableHistoricalRealizationRoot {
    revision: RevisionId,
    semantic_context: Option<SemanticContext>,
    root: FactorizedRealizationRoot,
}

impl DurableHistoricalRealizationRoot {
    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn semantic_context(&self) -> Option<&SemanticContext> {
        self.semantic_context.as_ref()
    }

    #[must_use]
    pub const fn root(&self) -> &FactorizedRealizationRoot {
        &self.root
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableFactorizedReadSnapshot {
    revision: RevisionId,
    semantic_context: SemanticContext,
    atoms: PhysicalAtomStore,
    root: FactorizedRealizationRoot,
}

impl DurableFactorizedReadSnapshot {
    #[must_use]
    pub const fn revision(&self) -> RevisionId { self.revision }
    #[must_use]
    pub const fn semantic_context(&self) -> &SemanticContext { &self.semantic_context }
    #[must_use]
    pub const fn atoms(&self) -> &PhysicalAtomStore { &self.atoms }
    #[must_use]
    pub const fn root(&self) -> &FactorizedRealizationRoot { &self.root }

    pub fn advance_relation_delta(
        &self,
        target_revision: RevisionId,
        relation: SemanticId,
        delta: &kernel_query::RelationDelta,
        registry: &SemanticRegistry,
    ) -> Result<Self, DurabilityError> {
        let mut atoms = self.atoms.clone();
        let mut root = self.root.clone();
        root.apply_exact_relation_delta_endpoint(
            &mut atoms,
            relation,
            delta,
            &self.semantic_context,
            registry,
        )
        .map_err(|_| protocol("historical factorized relation delta is invalid"))?;
        Ok(Self {
            revision: target_revision,
            semantic_context: self.semantic_context.clone(),
            atoms,
            root,
        })
    }

    pub fn advance_model_delta(
        &self,
        target_revision: RevisionId,
        delta: &DurableModelDelta,
    ) -> Result<Self, DurabilityError> {
        let mut atoms = self.atoms.clone();
        let mut direct = self
            .root
            .to_direct_durable_root()
            .map_err(|_| protocol("historical factorized model source is not direct"))?;

        if !delta.lifecycle_entities_inserted.is_empty()
            || !delta.lifecycle_entities_removed.is_empty()
            || !delta.lifecycle_roots_inserted.is_empty()
            || !delta.lifecycle_roots_removed.is_empty()
            || !delta.lifecycle_keeps_alive.is_empty()
        {
            let mut lifecycle = match atoms.get(direct.lifecycle).map(|atom| atom.payload()) {
                Some(PhysicalAtomPayload::Lifecycle(graph)) => graph.clone(),
                _ => return Err(protocol("historical factorized lifecycle atom is invalid")),
            };
            for entity in &delta.lifecycle_entities_removed {
                lifecycle.entities.remove(entity);
            }
            lifecycle
                .entities
                .extend(delta.lifecycle_entities_inserted.iter().copied());
            for entity in &delta.lifecycle_roots_removed {
                lifecycle.roots.remove(entity);
            }
            lifecycle
                .roots
                .extend(delta.lifecycle_roots_inserted.iter().copied());
            for patch in &delta.lifecycle_keeps_alive {
                let children = lifecycle.keeps_alive.entry(patch.parent).or_default();
                for child in &patch.removed {
                    children.remove(child);
                }
                children.extend(patch.inserted.iter().copied());
                if !patch.target_present {
                    lifecycle.keeps_alive.remove(&patch.parent);
                }
            }
            direct.lifecycle = atoms.insert(PhysicalAtomPayload::Lifecycle(lifecycle));
        }

        for patch in &delta.carriers {
            let mut entities = match direct.carriers.get(&patch.carrier).copied() {
                Some(atom) => match atoms.get(atom).map(|atom| atom.payload()) {
                    Some(PhysicalAtomPayload::EntityOrder(entities)) => {
                        entities.iter().copied().collect::<BTreeSet<_>>()
                    }
                    _ => return Err(protocol("historical factorized carrier atom is invalid")),
                },
                None => BTreeSet::new(),
            };
            for entity in &patch.removed {
                entities.remove(entity);
            }
            entities.extend(patch.inserted.iter().copied());
            if patch.target_present {
                let atom = atoms.insert(PhysicalAtomPayload::EntityOrder(
                    entities.into_iter().collect(),
                ));
                direct.carriers.insert(patch.carrier, atom);
            } else {
                direct.carriers.remove(&patch.carrier);
            }
        }

        let mut field_patches = BTreeMap::<SemanticId, Vec<_>>::new();
        for patch in &delta.fields {
            field_patches.entry(patch.field).or_default().push(patch);
        }
        for (field, patches) in field_patches {
            let mut entries = match direct.fields.get(&field) {
                Some(field_root) => match atoms.get(field_root.atom).map(|atom| atom.payload()) {
                    Some(PhysicalAtomPayload::FieldColumnSegment(column)) => column
                        .iter()
                        .map(|(entity, value)| (entity, value.clone()))
                        .collect::<BTreeMap<_, _>>(),
                    _ => return Err(protocol("historical factorized field atom is invalid")),
                },
                None => BTreeMap::new(),
            };
            for patch in patches {
                match &patch.value {
                    Some(value) => {
                        entries.insert(patch.owner, value.clone());
                    }
                    None => {
                        entries.remove(&patch.owner);
                    }
                }
            }
            if entries.is_empty() {
                direct.fields.remove(&field);
                continue;
            }
            let owner = self
                .semantic_context
                .schema
                .field(field)
                .ok_or_else(|| protocol("historical factorized field is absent from schema"))?
                .owner;
            let column = FieldColumnSegment::new(entries)
                .map_err(|_| protocol("historical factorized field patch is invalid"))?;
            let atom = atoms.insert(PhysicalAtomPayload::FieldColumnSegment(column));
            direct.fields.insert(
                field,
                DirectFactorizedFieldRoot {
                    owner,
                    atom,
                },
            );
        }

        let root = FactorizedRealizationRoot::from_direct_durable_root(direct);
        root.validate_direct_physical_root(&atoms)
            .map_err(|_| protocol("historical factorized model target topology is invalid"))?;
        let reachable = root.dependencies();
        atoms.retain(&reachable);
        Ok(Self {
            revision: target_revision,
            semantic_context: self.semantic_context.clone(),
            atoms,
            root,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableFactorizedRealization {
    revision: RevisionId,
    atoms: PhysicalAtomStore,
    root: FactorizedRealizationRoot,
    historical_roots: BTreeMap<RevisionEffectId, DurableHistoricalRealizationRoot>,
}

impl DurableFactorizedRealization {
    pub fn new(
        revision: RevisionId,
        atoms: PhysicalAtomStore,
        root: FactorizedRealizationRoot,
    ) -> Result<Self, DurabilityError> {
        root.validate_direct_physical_root(&atoms)
            .map_err(|_| protocol("durable realization physical topology is invalid"))?;
        root.to_direct_durable_root()
            .map_err(|_| protocol("durable realization requires a direct factorized root"))?;
        Ok(Self {
            revision,
            atoms,
            root,
            historical_roots: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn atoms(&self) -> &PhysicalAtomStore {
        &self.atoms
    }

    #[must_use]
    pub const fn root(&self) -> &FactorizedRealizationRoot {
        &self.root
    }

    #[must_use]
    pub const fn historical_roots(
        &self,
    ) -> &BTreeMap<RevisionEffectId, DurableHistoricalRealizationRoot> {
        &self.historical_roots
    }

    #[must_use]
    pub fn historical_root(
        &self,
        effect_id: RevisionEffectId,
    ) -> Option<&DurableHistoricalRealizationRoot> {
        self.historical_roots.get(&effect_id)
    }

    pub fn retain_historical_root(
        &mut self,
        effect_id: RevisionEffectId,
        historical: &Self,
    ) -> Result<(), DurabilityError> {
        self.retain_historical_root_inner(effect_id, historical, None)
    }

    pub fn retain_historical_root_with_context(
        &mut self,
        effect_id: RevisionEffectId,
        historical: &Self,
        semantic_context: &SemanticContext,
    ) -> Result<(), DurabilityError> {
        self.retain_historical_root_inner(effect_id, historical, Some(semantic_context.clone()))
    }

    fn retain_historical_root_inner(
        &mut self,
        effect_id: RevisionEffectId,
        historical: &Self,
        semantic_context: Option<SemanticContext>,
    ) -> Result<(), DurabilityError> {
        if let Some(existing) = self.historical_roots.get(&effect_id) {
            if existing.revision == historical.revision
                && existing.root == historical.root
                && (semantic_context.is_none() || existing.semantic_context == semantic_context)
            {
                return Ok(());
            }
            return Err(protocol("historical realization effect identity conflicts with retained root"));
        }

        let mut merged = self.atoms.clone();
        merged
            .merge_exact_from(&historical.atoms)
            .map_err(|_| protocol("historical realization atom union is invalid"))?;
        historical
            .root
            .validate_direct_physical_root(&merged)
            .map_err(|_| protocol("historical realization root topology is invalid"))?;
        historical
            .root
            .to_direct_durable_root()
            .map_err(|_| protocol("historical realization requires a direct factorized root"))?;
        self.atoms = merged;
        self.historical_roots.insert(
            effect_id,
            DurableHistoricalRealizationRoot {
                revision: historical.revision,
                semantic_context,
                root: historical.root.clone(),
            },
        );
        self.prune_unreachable_atoms();
        Ok(())
    }

    pub fn inherit_retained_historical_roots(
        &mut self,
        previous: &Self,
        previous_current_context: &SemanticContext,
        retained: impl IntoIterator<Item = (RevisionEffectId, RevisionId)>,
    ) -> Result<(), DurabilityError> {
        for (effect_id, source_revision) in retained {
            if let Some(root) = previous.historical_roots.get(&effect_id) {
                let source = Self {
                    revision: root.revision,
                    atoms: previous.atoms.clone(),
                    root: root.root.clone(),
                    historical_roots: BTreeMap::new(),
                };
                self.retain_historical_root_inner(
                    effect_id,
                    &source,
                    root.semantic_context.clone(),
                )?;
            } else if previous.revision == source_revision {
                self.retain_historical_root_with_context(
                    effect_id,
                    previous,
                    previous_current_context,
                )?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn historical_read_snapshot(
        &self,
        revision: RevisionId,
    ) -> Option<DurableFactorizedReadSnapshot> {
        self.historical_roots.values().find_map(|historical| {
            if historical.revision != revision {
                return None;
            }
            let semantic_context = historical.semantic_context.clone()?;
            Some(DurableFactorizedReadSnapshot {
                revision,
                semantic_context,
                atoms: self.atoms.clone(),
                root: historical.root.clone(),
            })
        })
    }

    pub fn historical_revision(
        &self,
        effect_id: RevisionEffectId,
        registry: &SemanticRegistry,
    ) -> Result<Option<Revision>, DurabilityError> {
        let Some(historical) = self.historical_roots.get(&effect_id) else {
            return Ok(None);
        };
        let Some(context) = historical.semantic_context.as_ref() else {
            return Ok(None);
        };
        let state = historical
            .root
            .evaluate(&self.atoms)
            .map_err(|_| protocol("historical realization root cannot materialize its logical state"))?;
        Revision::build(historical.revision, context, registry, state)
            .map(Some)
            .map_err(|_| protocol("historical realization root is invalid under its semantic context"))
    }

    pub fn release_historical_root(
        &mut self,
        effect_id: RevisionEffectId,
    ) -> bool {
        let removed = self.historical_roots.remove(&effect_id).is_some();
        if removed {
            self.prune_unreachable_atoms();
        }
        removed
    }

    #[must_use]
    pub fn reachable_atoms(&self) -> BTreeSet<PhysicalAtomId> {
        let mut reachable = self.root.dependencies();
        for historical in self.historical_roots.values() {
            reachable.extend(historical.root.dependencies());
        }
        reachable
    }

    fn prune_unreachable_atoms(&mut self) {
        let reachable = self.reachable_atoms();
        self.atoms.retain(&reachable);
    }

    pub fn encode(&self) -> Result<Vec<u8>, DurabilityError> {
        let mut out = Vec::new();
        encode_durable_factorized_realization_into(&mut out, self)?;
        Ok(out)
    }

    pub fn encoded_len(&self) -> Result<u64, DurabilityError> {
        let mut sink = CountingBinarySink::default();
        encode_durable_factorized_realization_into(&mut sink, self)?;
        sink.len().map_err(Into::into)
    }

    pub fn stream(
        &self,
        emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
    ) -> Result<(), DurabilityError> {
        let mut sink = StreamingBinarySink::new(emit);
        encode_durable_factorized_realization_into(&mut sink, self)?;
        sink.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DurabilityError> {
        let mut cursor = Cursor::new(bytes);
        decode_durable_factorized_realization_from_source(&mut cursor)
    }

    pub fn decode_from_reader(reader: &mut dyn Read, len: u64) -> Result<Self, DurabilityError> {
        let mut cursor = ReadBinarySource::new(reader, len);
        decode_durable_factorized_realization_from_source(&mut cursor)
    }
}

fn corrupt(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

fn protocol(reason: &'static str) -> DurabilityError {
    DurabilityError::Protocol { offset: 0, reason }
}

fn encode_atom_payload(
    out: &mut impl BinarySink,
    payload: &PhysicalAtomPayload,
) -> Result<(), CodecError> {
    match payload {
        PhysicalAtomPayload::Value(value) => {
            out.push(0);
            encode_value(out, value, 0)?;
        }
        PhysicalAtomPayload::EntitySet(entities) => {
            out.push(1);
            push_len(out, entities.len())?;
            for entity in entities {
                push_u128(out, entity.raw());
            }
        }
        PhysicalAtomPayload::EntityOrder(entities) => {
            out.push(2);
            push_len(out, entities.len())?;
            for entity in entities {
                push_u128(out, entity.raw());
            }
        }
        PhysicalAtomPayload::RelationRows(rows) => {
            out.push(3);
            encode_rows(out, rows)?;
        }
        PhysicalAtomPayload::FieldColumnSegment(column) => {
            out.push(4);
            push_len(out, column.len())?;
            for (entity, value) in column.iter() {
                push_u128(out, entity.raw());
                encode_value(out, value, 0)?;
            }
        }
        PhysicalAtomPayload::RelationColumnSegment(column) => {
            out.push(5);
            push_u64(
                out,
                u64::try_from(column.start_row()).map_err(|_| CodecError::LengthOverflow)?,
            );
            push_len(out, column.len())?;
            for value in column.iter() {
                encode_value(out, value, 0)?;
            }
        }
        PhysicalAtomPayload::Lifecycle(graph) => {
            out.push(6);
            let mut state = DatabaseState::default();
            state.lifecycle = graph.clone().into();
            let mut encoded = Vec::new();
            encode_state(&mut encoded, &state)?;
            push_bytes(out, &encoded)?;
        }
    }
    Ok(())
}

fn decode_atom_payload(cursor: &mut impl BinarySource) -> Result<PhysicalAtomPayload, DurabilityError> {
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(PhysicalAtomPayload::Value(cursor.value(0).map_err(corrupt)?)),
        1 => {
            let count = cursor.len().map_err(corrupt)?;
            let mut entities = BTreeSet::new();
            let mut previous = None;
            for _ in 0..count {
                let entity = EntityId::new(cursor.u128().map_err(corrupt)?);
                if previous.is_some_and(|p| p >= entity) {
                    return Err(corrupt("physical entity set is not strictly sorted"));
                }
                previous = Some(entity);
                entities.insert(entity);
            }
            Ok(PhysicalAtomPayload::EntitySet(entities))
        }
        2 => {
            let count = cursor.len().map_err(corrupt)?;
            let mut entities = Vec::with_capacity(cursor.bounded_capacity(count));
            for _ in 0..count {
                entities.push(EntityId::new(cursor.u128().map_err(corrupt)?));
            }
            Ok(PhysicalAtomPayload::EntityOrder(entities))
        }
        3 => Ok(PhysicalAtomPayload::RelationRows(cursor.rows(0).map_err(corrupt)?)),
        4 => {
            let count = cursor.len().map_err(corrupt)?;
            let mut entries = Vec::with_capacity(cursor.bounded_capacity(count));
            let mut previous = None;
            for _ in 0..count {
                let entity = EntityId::new(cursor.u128().map_err(corrupt)?);
                if previous.is_some_and(|p| p >= entity) {
                    return Err(corrupt("field column entities are not strictly sorted"));
                }
                previous = Some(entity);
                entries.push((entity, cursor.value(0).map_err(corrupt)?));
            }
            let column = FieldColumnSegment::new(entries)
                .map_err(|_| corrupt("invalid durable field column segment"))?;
            Ok(PhysicalAtomPayload::FieldColumnSegment(column))
        }
        5 => {
            let start_row = usize::try_from(cursor.u64().map_err(corrupt)?)
                .map_err(|_| corrupt("relation column start row overflow"))?;
            let count = cursor.len().map_err(corrupt)?;
            let mut values = Vec::with_capacity(cursor.bounded_capacity(count));
            for _ in 0..count {
                values.push(cursor.value(0).map_err(corrupt)?);
            }
            Ok(PhysicalAtomPayload::RelationColumnSegment(
                RelationColumnSegment::with_start_row(start_row, values),
            ))
        }
        6 => {
            let len = cursor.len().map_err(corrupt)?;
            let bytes = cursor.take_owned(len).map_err(corrupt)?;
            let mut lifecycle_cursor = Cursor::new(&bytes);
            let state = decode_state(&mut lifecycle_cursor)?;
            lifecycle_cursor.finish().map_err(corrupt)?;
            if !state.model.carriers.is_empty()
                || !state.model.fields.is_empty()
                || !state.model.relations.is_empty()
            {
                return Err(corrupt("durable lifecycle atom contains model state"));
            }
            Ok(PhysicalAtomPayload::Lifecycle((*state.lifecycle).clone()))
        }
        _ => Err(corrupt("unknown durable physical atom payload tag")),
    }
}

pub fn encode_factorized_realization(
    revision: RevisionId,
    atoms: &PhysicalAtomStore,
    root: &FactorizedRealizationRoot,
) -> Result<Vec<u8>, DurabilityError> {
    let mut out = Vec::new();
    encode_factorized_realization_into(&mut out, revision, atoms, root)?;
    Ok(out)
}

fn encode_direct_root(
    out: &mut impl BinarySink,
    direct: DirectFactorizedRealizationRoot,
) -> Result<(), DurabilityError> {
    push_u128(out, direct.lifecycle.raw());
    push_len(out, direct.carriers.len()).map_err(DurabilityError::from)?;
    for (semantic, atom) in direct.carriers {
        push_u128(out, semantic.raw());
        push_u128(out, atom.raw());
    }
    push_len(out, direct.fields.len()).map_err(DurabilityError::from)?;
    for (field, direct) in direct.fields {
        push_u128(out, field.raw());
        push_u128(out, direct.owner.raw());
        push_u128(out, direct.atom.raw());
    }
    push_len(out, direct.relations.len()).map_err(DurabilityError::from)?;
    for (relation, direct) in direct.relations {
        push_u128(out, relation.raw());
        push_u64(
            out,
            u64::try_from(direct.row_count).map_err(|_| DurabilityError::PayloadTooLarge)?,
        );
        push_len(out, direct.column_order.len()).map_err(DurabilityError::from)?;
        for column in direct.column_order {
            let atom = direct
                .column_atoms
                .get(&column)
                .copied()
                .ok_or_else(|| protocol("direct relation root is missing a column atom"))?;
            push_u128(out, column.raw());
            push_u128(out, atom.raw());
        }
    }
    Ok(())
}

fn encode_factorized_realization_authority_into(
    out: &mut impl BinarySink,
    revision: RevisionId,
    atoms: &PhysicalAtomStore,
    root: &FactorizedRealizationRoot,
    historical_roots: &BTreeMap<RevisionEffectId, DurableHistoricalRealizationRoot>,
) -> Result<(), DurabilityError> {
    root.validate_direct_physical_root(atoms)
        .map_err(|_| protocol("durable realization physical topology is invalid"))?;
    let direct = root
        .to_direct_durable_root()
        .map_err(|_| protocol("durable realization requires a direct factorized root"))?;
    let mut reachable = root.dependencies();
    for historical in historical_roots.values() {
        historical
            .root
            .validate_direct_physical_root(atoms)
            .map_err(|_| protocol("historical durable realization topology is invalid"))?;
        historical
            .root
            .to_direct_durable_root()
            .map_err(|_| protocol("historical realization requires a direct factorized root"))?;
        reachable.extend(historical.root.dependencies());
    }
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0_u16.to_le_bytes());
    push_u64(out, revision.raw());
    push_len(out, reachable.len()).map_err(DurabilityError::from)?;
    for id in reachable {
        push_u128(out, id.raw());
        let atom = atoms
            .get(id)
            .ok_or_else(|| protocol("realization root references a missing physical atom"))?;
        encode_atom_payload(out, atom.payload()).map_err(DurabilityError::from)?;
    }
    encode_direct_root(out, direct)?;
    push_len(out, historical_roots.len()).map_err(DurabilityError::from)?;
    for (effect_id, historical) in historical_roots {
        push_u128(out, effect_id.0);
        push_u64(out, historical.revision.raw());
        match historical.semantic_context.as_ref() {
            Some(context) => {
                out.push(1);
                out.extend_from_slice(&crate::checkpoint::CHECKPOINT_CODEC_VERSION.to_le_bytes());
                crate::checkpoint::encode_context(out, context).map_err(DurabilityError::from)?;
            }
            None => out.push(0),
        }
        encode_direct_root(
            out,
            historical
                .root
                .to_direct_durable_root()
                .map_err(|_| protocol("historical realization requires a direct factorized root"))?,
        )?;
    }
    Ok(())
}

fn encode_durable_factorized_realization_into(
    out: &mut impl BinarySink,
    realization: &DurableFactorizedRealization,
) -> Result<(), DurabilityError> {
    encode_factorized_realization_authority_into(
        out,
        realization.revision,
        &realization.atoms,
        &realization.root,
        &realization.historical_roots,
    )
}

fn encode_factorized_realization_into(
    out: &mut impl BinarySink,
    revision: RevisionId,
    atoms: &PhysicalAtomStore,
    root: &FactorizedRealizationRoot,
) -> Result<(), DurabilityError> {
    encode_factorized_realization_authority_into(out, revision, atoms, root, &BTreeMap::new())
}

fn decode_direct_root(
    cursor: &mut impl BinarySource,
) -> Result<FactorizedRealizationRoot, DurabilityError> {
    let lifecycle = PhysicalAtomId::new(cursor.u128().map_err(corrupt)?);
    let carrier_count = cursor.len().map_err(corrupt)?;
    let mut carriers = BTreeMap::new();
    let mut previous_semantic = None;
    for _ in 0..carrier_count {
        let semantic = SemanticId::new(cursor.u128().map_err(corrupt)?);
        if previous_semantic.is_some_and(|previous| previous >= semantic) {
            return Err(corrupt("durable carriers are not strictly sorted and unique"));
        }
        previous_semantic = Some(semantic);
        carriers.insert(
            semantic,
            PhysicalAtomId::new(cursor.u128().map_err(corrupt)?),
        );
    }
    let field_count = cursor.len().map_err(corrupt)?;
    let mut fields = BTreeMap::new();
    previous_semantic = None;
    for _ in 0..field_count {
        let field = SemanticId::new(cursor.u128().map_err(corrupt)?);
        if previous_semantic.is_some_and(|previous| previous >= field) {
            return Err(corrupt("durable fields are not strictly sorted and unique"));
        }
        previous_semantic = Some(field);
        fields.insert(
            field,
            DirectFactorizedFieldRoot {
                owner: SemanticId::new(cursor.u128().map_err(corrupt)?),
                atom: PhysicalAtomId::new(cursor.u128().map_err(corrupt)?),
            },
        );
    }
    let relation_count = cursor.len().map_err(corrupt)?;
    let mut relations = BTreeMap::new();
    previous_semantic = None;
    for _ in 0..relation_count {
        let relation = SemanticId::new(cursor.u128().map_err(corrupt)?);
        if previous_semantic.is_some_and(|previous| previous >= relation) {
            return Err(corrupt("durable relations are not strictly sorted and unique"));
        }
        previous_semantic = Some(relation);
        let row_count = usize::try_from(cursor.u64().map_err(corrupt)?)
            .map_err(|_| corrupt("durable relation row count overflow"))?;
        let column_count = cursor.len().map_err(corrupt)?;
        let mut column_order = Vec::with_capacity(cursor.bounded_capacity(column_count));
        let mut column_atoms = BTreeMap::new();
        let mut seen_columns = BTreeSet::new();
        for _ in 0..column_count {
            let column = SemanticId::new(cursor.u128().map_err(corrupt)?);
            if !seen_columns.insert(column) {
                return Err(corrupt("durable relation column order contains a duplicate"));
            }
            column_order.push(column);
            column_atoms.insert(
                column,
                PhysicalAtomId::new(cursor.u128().map_err(corrupt)?),
            );
        }
        relations.insert(
            relation,
            DirectFactorizedRelationRoot {
                column_order,
                column_atoms,
                row_count,
            },
        );
    }
    Ok(FactorizedRealizationRoot::from_direct_durable_root(
        DirectFactorizedRealizationRoot {
            lifecycle,
            carriers,
            fields,
            relations,
        },
    ))
}

fn decode_durable_factorized_realization_from_source(
    cursor: &mut impl BinarySource,
) -> Result<DurableFactorizedRealization, DurabilityError> {
    if cursor.take_owned(4).map_err(corrupt)?.as_slice() != MAGIC {
        return Err(corrupt("durable physical realization magic mismatch"));
    }
    let version = cursor.u16().map_err(corrupt)?;
    if !matches!(version, 1 | 2 | VERSION) || cursor.u16().map_err(corrupt)? != 0 {
        return Err(corrupt("unsupported durable physical realization version"));
    }
    let revision = RevisionId::new(cursor.u64().map_err(corrupt)?);
    let atom_count = cursor.len().map_err(corrupt)?;
    let mut atoms = Vec::with_capacity(cursor.bounded_capacity(atom_count));
    let mut previous_atom = None;
    for _ in 0..atom_count {
        let id = PhysicalAtomId::new(cursor.u128().map_err(corrupt)?);
        if previous_atom.is_some_and(|previous| previous >= id) {
            return Err(corrupt("physical atoms are not strictly sorted and unique"));
        }
        previous_atom = Some(id);
        atoms.push((id, decode_atom_payload(cursor)?));
    }
    let root = decode_direct_root(cursor)?;
    let mut historical_roots = BTreeMap::new();
    if version >= 2 {
        let historical_count = cursor.len().map_err(corrupt)?;
        let mut previous_effect = None;
        for _ in 0..historical_count {
            let effect_id = RevisionEffectId(cursor.u128().map_err(corrupt)?);
            if previous_effect.is_some_and(|previous| previous >= effect_id) {
                return Err(corrupt("historical realization roots are not strictly sorted and unique"));
            }
            previous_effect = Some(effect_id);
            let historical_revision = RevisionId::new(cursor.u64().map_err(corrupt)?);
            let semantic_context = if version >= 3 {
                match cursor.u8().map_err(corrupt)? {
                    0 => None,
                    1 => {
                        let context_version = cursor.u16().map_err(corrupt)?;
                        if context_version > crate::checkpoint::CHECKPOINT_CODEC_VERSION {
                            return Err(corrupt("historical semantic context codec is unsupported"));
                        }
                        Some(crate::checkpoint::decode_context(cursor, context_version)?)
                    }
                    _ => {
                        return Err(corrupt(
                            "historical semantic context presence tag is invalid",
                        ));
                    }
                }
            } else {
                None
            };
            let historical_root = decode_direct_root(cursor)?;
            historical_roots.insert(
                effect_id,
                DurableHistoricalRealizationRoot {
                    revision: historical_revision,
                    semantic_context,
                    root: historical_root,
                },
            );
        }
    }
    cursor.finish().map_err(corrupt)?;
    let atoms = PhysicalAtomStore::from_exact_atoms(atoms)
        .map_err(|_| corrupt("invalid durable physical atom store"))?;
    root.validate_direct_physical_root(&atoms)
        .map_err(|_| corrupt("durable realization root topology is invalid"))?;
    for historical in historical_roots.values() {
        historical
            .root
            .validate_direct_physical_root(&atoms)
            .map_err(|_| corrupt("historical durable realization topology is invalid"))?;
    }
    let mut realization = DurableFactorizedRealization {
        revision,
        atoms,
        root,
        historical_roots,
    };
    realization.prune_unreachable_atoms();
    Ok(realization)
}

impl SingleFileContainer {
    pub fn read_active_factorized_realization(
        &mut self,
    ) -> Result<Option<(RevisionId, PhysicalAtomStore, FactorizedRealizationRoot)>, DurabilityError>
    {
        self.with_section_reader(SingleFileSectionKind::PhysicalArtifact, 0, |reader, len| {
            decode_factorized_realization_from_reader(reader, len)
        })
    }
}

pub fn decode_factorized_realization(
    bytes: &[u8],
) -> Result<(RevisionId, PhysicalAtomStore, FactorizedRealizationRoot), DurabilityError> {
    let realization = DurableFactorizedRealization::decode(bytes)?;
    if !realization.historical_roots.is_empty() {
        return Err(protocol(
            "current-root projection cannot discard retained historical realization roots",
        ));
    }
    Ok((realization.revision, realization.atoms, realization.root))
}

pub fn decode_factorized_realization_from_reader(
    reader: &mut dyn Read,
    len: u64,
) -> Result<(RevisionId, PhysicalAtomStore, FactorizedRealizationRoot), DurabilityError> {
    let realization = DurableFactorizedRealization::decode_from_reader(reader, len)?;
    if !realization.historical_roots.is_empty() {
        return Err(protocol(
            "current-root projection cannot discard retained historical realization roots",
        ));
    }
    Ok((realization.revision, realization.atoms, realization.root))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::OpenOptions;
    use std::io::Write;

    use kernel_change::RevisionEffectId;
    use kernel_model::{DatabaseState, Value};
    use kernel_realization::{
        DirectFactorizedFieldRoot, DirectFactorizedRealizationRoot, DirectFactorizedRelationRoot,
        FactorizedRealizationRoot, FieldColumnSegment, PhysicalAtomPayload, PhysicalAtomStore,
        RelationColumnSegment, realize_database_state_factorized,
    };
    use kernel_query::{RelExpr, RelationDelta};
    use kernel_schema::{
        FieldDef, RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext,
        SemanticEnvironment, TypeExpr,
    };
    use kernel_semantics::SemanticRegistry;
    use kernel_types::{EntityId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

    use crate::{
        DurableModelDelta, SingleFileContainer, SingleFileSectionInput, StorageEncryption,
        StorageEncryptionKey,
    };

    use super::{
        DurableFactorizedRealization, decode_factorized_realization,
        encode_factorized_realization,
    };

    fn fixture() -> (
        PhysicalAtomStore,
        FactorizedRealizationRoot,
        SemanticId,
        EntityId,
    ) {
        let entity_type = SemanticId::new(11);
        let field = SemanticId::new(12);
        let entity = EntityId::new(101);
        let mut atoms = PhysicalAtomStore::default();
        let lifecycle = atoms.insert(PhysicalAtomPayload::Lifecycle(
            (*DatabaseState::default().lifecycle).clone(),
        ));
        let carrier = atoms.insert(PhysicalAtomPayload::EntityOrder(vec![entity]));
        let value = atoms.insert(PhysicalAtomPayload::FieldColumnSegment(
            FieldColumnSegment::new([(entity, Value::I64(7))]).unwrap(),
        ));
        let root = FactorizedRealizationRoot::from_direct_durable_root(
            DirectFactorizedRealizationRoot {
                lifecycle,
                carriers: BTreeMap::from([(entity_type, carrier)]),
                fields: BTreeMap::from([(
                    field,
                    DirectFactorizedFieldRoot {
                        owner: entity_type,
                        atom: value,
                    },
                )]),
                relations: BTreeMap::new(),
            },
        );
        (atoms, root, field, entity)
    }

    #[test]
    fn factorized_model_delta_rewrites_only_model_authorities_without_logical_rebuild() {
        let entity_type = SemanticId::new(70_001);
        let field = SemanticId::new(70_002);
        let first = EntityId::new(700_001);
        let child = EntityId::new(700_002);
        let replacement = EntityId::new(700_003);
        let mut schema = Schema::new(SchemaRevisionId::new(700));
        schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(700)),
        };

        let mut source = DatabaseState::default();
        source.lifecycle.entities.extend([first, child]);
        source.lifecycle.roots.insert(first);
        source
            .lifecycle
            .keeps_alive
            .insert(first, [child].into_iter().collect());
        source
            .model
            .carriers
            .insert(entity_type, [first].into_iter().collect());
        source.model.fields.insert((field, first), Value::I64(7));

        let mut target = DatabaseState::default();
        target.lifecycle.entities.extend([child, replacement]);
        target.lifecycle.roots.insert(replacement);
        target
            .lifecycle
            .keeps_alive
            .insert(replacement, [child].into_iter().collect());
        target
            .model
            .carriers
            .insert(entity_type, [replacement].into_iter().collect());
        target
            .model
            .fields
            .insert((field, replacement), Value::I64(9));

        let (atoms, root) = realize_database_state_factorized(&source, &context).unwrap();
        let snapshot = super::DurableFactorizedReadSnapshot {
            revision: RevisionId::new(700),
            semantic_context: context,
            atoms,
            root,
        };
        let delta = DurableModelDelta::between(&source, &target);
        let advanced = snapshot
            .advance_model_delta(RevisionId::new(701), &delta)
            .unwrap();

        assert_eq!(advanced.revision(), RevisionId::new(701));
        assert_eq!(advanced.root().evaluate(advanced.atoms()).unwrap(), target);
        assert_eq!(advanced.atoms().len(), advanced.root().dependencies().len());
    }

    #[test]
    fn factorized_mixed_relation_and_model_delta_matches_exact_target() {
        let entity_type = SemanticId::new(71_001);
        let field = SemanticId::new(71_002);
        let relation = SemanticId::new(71_003);
        let first = EntityId::new(710_001);
        let second = EntityId::new(710_002);
        let mut schema = Schema::new(SchemaRevisionId::new(710));
        schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: Vec::new(),
                semantics: RelationSemantics::Bag {
                    column_equivalences: Vec::new(),
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(710)),
        };
        let registry = SemanticRegistry::default();

        let mut source = DatabaseState::default();
        source.lifecycle.entities.insert(first);
        source.lifecycle.roots.insert(first);
        source
            .model
            .carriers
            .insert(entity_type, [first].into_iter().collect());
        source.model.fields.insert((field, first), Value::I64(1));
        source.model.relations.insert(relation, vec![vec![]].into());

        let mut target = source.clone();
        target.lifecycle.entities.insert(second);
        target.lifecycle.roots.insert(second);
        target.model.carriers.get_mut(&entity_type).unwrap().insert(second);
        target.model.fields.insert((field, second), Value::I64(2));
        target
            .model
            .relations
            .insert(relation, vec![vec![], vec![]].into());

        let (atoms, root) = realize_database_state_factorized(&source, &context).unwrap();
        let snapshot = super::DurableFactorizedReadSnapshot {
            revision: RevisionId::new(710),
            semantic_context: context.clone(),
            atoms,
            root,
        };
        let result_type = RelExpr::Scan(relation).typecheck(&context, &registry).unwrap();
        let relation_delta = RelationDelta {
            inserted: vec![vec![]],
            removed: Vec::new(),
            result_type,
        };
        let model_delta = DurableModelDelta::between(&source, &target);
        let advanced = snapshot
            .advance_relation_delta(RevisionId::new(711), relation, &relation_delta, &registry)
            .unwrap()
            .advance_model_delta(RevisionId::new(711), &model_delta)
            .unwrap();

        assert_eq!(advanced.root().evaluate(advanced.atoms()).unwrap(), target);
    }

    #[test]
    fn direct_realization_roundtrips_without_logical_state_reconstruction() {
        let (atoms, root, field, entity) = fixture();
        let bytes = encode_factorized_realization(RevisionId::new(42), &atoms, &root).unwrap();
        let (revision, decoded_atoms, decoded_root) = decode_factorized_realization(&bytes).unwrap();
        assert_eq!(revision, RevisionId::new(42));
        assert_eq!(
            decoded_root.read_field(&decoded_atoms, field, entity).unwrap(),
            Value::I64(7)
        );
        assert_eq!(decoded_root.dependencies(), root.dependencies());
    }

    #[test]
    fn retained_historical_root_shares_atoms_roundtrips_and_reclaims_on_release() {
        let (historical_atoms, historical_root, field, entity) = fixture();
        let historical = DurableFactorizedRealization::new(
            RevisionId::new(41),
            historical_atoms.clone(),
            historical_root.clone(),
        )
        .unwrap();
        let old_atom = historical_root
            .to_direct_durable_root()
            .unwrap()
            .fields[&field]
            .atom;

        let mut current_atoms = historical_atoms;
        let new_atom = current_atoms.insert(PhysicalAtomPayload::FieldColumnSegment(
            FieldColumnSegment::new([(entity, Value::I64(9))]).unwrap(),
        ));
        let mut current_direct = historical_root.to_direct_durable_root().unwrap();
        current_direct.fields.get_mut(&field).unwrap().atom = new_atom;
        let current_root = FactorizedRealizationRoot::from_direct_durable_root(current_direct);
        let mut current = DurableFactorizedRealization::new(
            RevisionId::new(42),
            current_atoms,
            current_root,
        )
        .unwrap();
        let effect_id = RevisionEffectId(700);
        current
            .retain_historical_root(effect_id, &historical)
            .unwrap();

        assert_eq!(current.atoms().len(), 4);
        assert_eq!(
            current
                .historical_root(effect_id)
                .unwrap()
                .root()
                .read_field(current.atoms(), field, entity)
                .unwrap(),
            Value::I64(7)
        );
        assert_eq!(
            current.root().read_field(current.atoms(), field, entity).unwrap(),
            Value::I64(9)
        );

        let encoded = current.encode().unwrap();
        let mut decoded = DurableFactorizedRealization::decode(&encoded).unwrap();
        assert_eq!(decoded.historical_roots().len(), 1);
        assert_eq!(decoded.atoms().len(), 4);
        assert!(decoded.release_historical_root(effect_id));
        assert_eq!(decoded.atoms().len(), 3);
        assert!(decoded.atoms().get(old_atom).is_none());
        assert!(decoded.atoms().get(new_atom).is_some());
        assert!(!decoded.release_historical_root(effect_id));
    }

    #[test]
    fn many_retained_historical_roots_share_one_atom_graph_instead_of_copying_each_root() {
        let (mut lineage_atoms, base_root, field, entity) = fixture();
        let mut roots = Vec::new();

        let mut base_atoms = lineage_atoms.clone();
        base_atoms.retain(&base_root.dependencies());
        roots.push(
            DurableFactorizedRealization::new(RevisionId::new(10_000), base_atoms, base_root.clone())
                .unwrap(),
        );

        let mut direct = base_root.to_direct_durable_root().unwrap();
        for index in 0..64_u64 {
            let atom = lineage_atoms.insert(PhysicalAtomPayload::FieldColumnSegment(
                FieldColumnSegment::new([(entity, Value::I64(100 + index as i64))]).unwrap(),
            ));
            direct.fields.get_mut(&field).unwrap().atom = atom;
            let root = FactorizedRealizationRoot::from_direct_durable_root(direct.clone());
            let mut exact_atoms = lineage_atoms.clone();
            exact_atoms.retain(&root.dependencies());
            roots.push(
                DurableFactorizedRealization::new(RevisionId::new(10_001 + index), exact_atoms, root)
                    .unwrap(),
            );
        }

        let mut current = roots.pop().unwrap();
        let naive_root_atom_slots = (roots.len() + 1) * 3;
        for (index, historical) in roots.iter().enumerate() {
            current
                .retain_historical_root(RevisionEffectId(20_000 + index as u128), historical)
                .unwrap();
        }

        assert_eq!(current.historical_roots().len(), 64);
        assert_eq!(naive_root_atom_slots, 195);
        assert_eq!(current.atoms().len(), 67);
        assert_eq!(current.reachable_atoms().len(), 67);

        let bytes = current.encode().unwrap();
        let decoded = DurableFactorizedRealization::decode(&bytes).unwrap();
        assert_eq!(decoded.historical_roots().len(), 64);
        assert_eq!(decoded.atoms().len(), 67);
    }

    #[test]
    fn durable_realization_streaming_encoding_is_byte_and_length_exact() {
        let (atoms, root, _, _) = fixture();
        let revision = RevisionId::new(43);
        let bytes = encode_factorized_realization(revision, &atoms, &root).unwrap();
        let physical = DurableFactorizedRealization::new(revision, atoms, root).unwrap();
        assert_eq!(physical.encoded_len().unwrap(), bytes.len() as u64);
        let mut streamed = Vec::new();
        physical
            .stream(&mut |chunk| {
                assert!(chunk.len() <= 64 * 1024);
                streamed.extend_from_slice(chunk);
                Ok(())
            })
            .unwrap();
        assert_eq!(streamed, bytes);
    }

    #[test]
    fn durable_relation_column_order_is_not_semantic_id_sort_order() {
        let relation = SemanticId::new(30);
        let first_column = SemanticId::new(20);
        let second_column = SemanticId::new(19);
        let mut atoms = PhysicalAtomStore::default();
        let lifecycle = atoms.insert(PhysicalAtomPayload::Lifecycle(
            (*DatabaseState::default().lifecycle).clone(),
        ));
        let first = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
            RelationColumnSegment::new(vec![Value::I64(1)]),
        ));
        let second = atoms.insert(PhysicalAtomPayload::RelationColumnSegment(
            RelationColumnSegment::new(vec![Value::I64(2)]),
        ));
        let root = FactorizedRealizationRoot::from_direct_durable_root(
            DirectFactorizedRealizationRoot {
                lifecycle,
                carriers: BTreeMap::new(),
                fields: BTreeMap::new(),
                relations: BTreeMap::from([(
                    relation,
                    DirectFactorizedRelationRoot {
                        column_order: vec![first_column, second_column],
                        column_atoms: BTreeMap::from([
                            (first_column, first),
                            (second_column, second),
                        ]),
                        row_count: 1,
                    },
                )]),
            },
        );
        let bytes = encode_factorized_realization(RevisionId::new(7), &atoms, &root).unwrap();
        let (_, decoded_atoms, decoded_root) = decode_factorized_realization(&bytes).unwrap();
        assert_eq!(
            decoded_root.relations()[&relation].column_order(),
            &[first_column, second_column]
        );
        assert_eq!(
            decoded_root
                .read_relation_column(&decoded_atoms, relation, first_column, 0)
                .unwrap(),
            Value::I64(1)
        );
    }

    #[test]
    fn malformed_direct_root_is_rejected_before_durable_encoding() {
        let entity_type = SemanticId::new(51);
        let field = SemanticId::new(52);
        let entity = EntityId::new(501);
        let mut atoms = PhysicalAtomStore::default();
        let lifecycle = atoms.insert(PhysicalAtomPayload::Lifecycle(
            (*DatabaseState::default().lifecycle).clone(),
        ));
        let carrier = atoms.insert(PhysicalAtomPayload::EntityOrder(vec![entity]));
        let root = FactorizedRealizationRoot::from_direct_durable_root(
            DirectFactorizedRealizationRoot {
                lifecycle,
                carriers: BTreeMap::from([(entity_type, carrier)]),
                fields: BTreeMap::from([(
                    field,
                    DirectFactorizedFieldRoot {
                        owner: entity_type,
                        atom: carrier,
                    },
                )]),
                relations: BTreeMap::new(),
            },
        );
        assert!(encode_factorized_realization(RevisionId::new(1), &atoms, &root).is_err());
    }

    #[test]
    fn encrypted_single_file_reopens_realization_without_logical_checkpoint_decode() {
        let (atoms, root, field, entity) = fixture();
        let revision = RevisionId::new(88);
        let bytes = encode_factorized_realization(revision, &atoms, &root).unwrap();
        let path = std::env::temp_dir().join(format!(
            "cfmd-p429-encrypted-realization-{}-{}.db",
            std::process::id(),
            entity.raw()
        ));
        let _ = std::fs::remove_file(&path);
        let encryption = StorageEncryption::aes256_gcm_siv(
            StorageEncryptionKey::try_new([0x6d; 32]).unwrap(),
        );
        let container = SingleFileContainer::create_with_encryption(
            &path,
            &[SingleFileSectionInput::physical_realization(&bytes)],
            &encryption,
        )
        .unwrap();
        drop(container);
        let mut reopened = SingleFileContainer::open_with_encryption(&path, &encryption).unwrap();
        let (opened_revision, opened_atoms, opened_root) = reopened
            .read_active_factorized_realization()
            .unwrap()
            .unwrap();
        assert_eq!(opened_revision, revision);
        assert_eq!(
            opened_root.read_field(&opened_atoms, field, entity).unwrap(),
            Value::I64(7)
        );
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn single_file_root_switch_is_same_revision_and_ignores_unpublished_tail() {
        let (mut atoms, mut root, field, entity) = fixture();
        let revision = RevisionId::new(42);
        let first = encode_factorized_realization(revision, &atoms, &root).unwrap();
        let path = std::env::temp_dir().join(format!(
            "cfmd-p429-realization-{}-{}.db",
            std::process::id(),
            entity.raw()
        ));
        let _ = std::fs::remove_file(&path);
        let container = SingleFileContainer::create(
            &path,
            &[SingleFileSectionInput::physical_realization(&first)],
        )
        .unwrap();
        let first_generation = container.generation();
        let committed_len = std::fs::metadata(&path).unwrap().len();

        let old_dependencies = root.dependencies();
        root.materialize_field_column(&mut atoms, field, [entity]).unwrap();
        assert_ne!(root.dependencies(), old_dependencies);
        assert_eq!(root.read_field(&atoms, field, entity).unwrap(), Value::I64(7));
        let second = encode_factorized_realization(revision, &atoms, &root).unwrap();
        drop(container);

        // Prepared bytes without a published generation/root are not authority.
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&second)
            .unwrap();
        let mut reopened = SingleFileContainer::open(&path).unwrap();
        assert_eq!(reopened.generation(), first_generation);
        let (active_revision, active_atoms, active_root) = reopened
            .read_active_factorized_realization()
            .unwrap()
            .unwrap();
        assert_eq!(active_revision, revision);
        assert_eq!(
            active_root.read_field(&active_atoms, field, entity).unwrap(),
            Value::I64(7)
        );
        assert_eq!(active_root.dependencies(), old_dependencies);
        drop(reopened);

        // Remove the simulated unpublished tail before performing a legal retry.
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(committed_len)
            .unwrap();
        let mut reopened = SingleFileContainer::open(&path).unwrap();

        // A generation/root switch changes only physical authority, not semantic revision.
        reopened
            .publish_generation(&[SingleFileSectionInput::physical_realization(&second)])
            .unwrap();
        let switched_generation = reopened.generation();
        assert!(switched_generation > first_generation);
        drop(reopened);

        let mut final_open = SingleFileContainer::open(&path).unwrap();
        let (active_revision, active_atoms, active_root) = final_open
            .read_active_factorized_realization()
            .unwrap()
            .unwrap();
        assert_eq!(active_revision, revision);
        assert_eq!(
            active_root.read_field(&active_atoms, field, entity).unwrap(),
            Value::I64(7)
        );
        assert_eq!(active_root.dependencies(), root.dependencies());
        assert_ne!(active_root.dependencies(), old_dependencies);
        assert_eq!(active_atoms.len(), active_root.dependencies().len());
        drop(final_open);
        let _ = std::fs::remove_file(&path);
    }
}
