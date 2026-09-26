use std::{
    collections::{BTreeMap, BTreeSet},
    ops::{Deref, DerefMut},
    sync::{Arc, OnceLock},
};

use kernel_identity::{DenseEntityIds, DenseEntitySet, LocalEntityId};
use kernel_lifecycle::LifecycleGraph;
use kernel_schema::ContextError;
use kernel_types::{EntityId, SemanticId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CowValue<T>(Arc<T>);

impl<T: Default> Default for CowValue<T> {
    fn default() -> Self {
        Self(Arc::new(T::default()))
    }
}

impl<T> From<T> for CowValue<T> {
    fn from(value: T) -> Self {
        Self(Arc::new(value))
    }
}

impl<T> Deref for CowValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Clone> DerefMut for CowValue<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CowMap<K, V>(Arc<BTreeMap<K, V>>);

impl<K, V> Default for CowMap<K, V> {
    fn default() -> Self {
        Self(Arc::new(BTreeMap::new()))
    }
}

impl<K, V> From<BTreeMap<K, V>> for CowMap<K, V> {
    fn from(value: BTreeMap<K, V>) -> Self {
        Self(Arc::new(value))
    }
}

impl<K, V> Deref for CowMap<K, V> {
    type Target = BTreeMap<K, V>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<K: Clone + Ord, V: Clone> DerefMut for CowMap<K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

impl<'a, K, V> IntoIterator for &'a CowMap<K, V> {
    type Item = (&'a K, &'a V);
    type IntoIter = std::collections::btree_map::Iter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(Debug)]
struct RelationRowsAppendPatch {
    base: SharedRelationRows,
    inserted: Arc<Vec<Vec<Value>>>,
    materialized: OnceLock<Vec<Vec<Value>>>,
}

#[derive(Debug, Clone)]
enum SharedRelationRowsRepr {
    Materialized(Arc<Vec<Vec<Value>>>),
    AppendPatch(Arc<RelationRowsAppendPatch>),
}

#[derive(Debug, Clone)]
pub struct SharedRelationRows(SharedRelationRowsRepr);

impl Default for SharedRelationRows {
    fn default() -> Self {
        Self::from(Vec::new())
    }
}

impl From<Vec<Vec<Value>>> for SharedRelationRows {
    fn from(rows: Vec<Vec<Value>>) -> Self {
        Self(SharedRelationRowsRepr::Materialized(Arc::new(rows)))
    }
}

impl SharedRelationRows {
    #[must_use]
    pub fn append_persistent(&self, inserted: Vec<Vec<Value>>) -> Self {
        if inserted.is_empty() {
            return self.clone();
        }
        Self(SharedRelationRowsRepr::AppendPatch(Arc::new(
            RelationRowsAppendPatch {
                base: self.clone(),
                inserted: Arc::new(inserted),
                materialized: OnceLock::new(),
            },
        )))
    }

    fn materialized(&self) -> &Vec<Vec<Value>> {
        match &self.0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::AppendPatch(patch) => patch.materialized.get_or_init(|| {
                let mut segments = Vec::new();
                patch.base.collect_materialized_segments(&mut segments);
                segments.push(patch.inserted.as_slice());
                let capacity = segments
                    .iter()
                    .fold(0_usize, |len, segment| len.saturating_add(segment.len()));
                let mut rows = Vec::with_capacity(capacity);
                for segment in segments {
                    rows.extend(segment.iter().cloned());
                }
                rows
            }),
        }
    }

    fn collect_materialized_segments<'a>(&'a self, output: &mut Vec<&'a [Vec<Value>]>) {
        let mut tail = Vec::new();
        let mut cursor = self;
        loop {
            match &cursor.0 {
                SharedRelationRowsRepr::Materialized(rows) => {
                    output.push(rows.as_slice());
                    break;
                }
                SharedRelationRowsRepr::AppendPatch(patch) => {
                    if let Some(rows) = patch.materialized.get() {
                        output.push(rows.as_slice());
                        break;
                    }
                    tail.push(patch.inserted.as_slice());
                    cursor = &patch.base;
                }
            }
        }
        output.extend(tail.into_iter().rev());
    }
}

impl PartialEq for SharedRelationRows {
    fn eq(&self, other: &Self) -> bool {
        self.materialized() == other.materialized()
    }
}

impl Eq for SharedRelationRows {}

impl Deref for SharedRelationRows {
    type Target = Vec<Vec<Value>>;

    fn deref(&self) -> &Self::Target {
        self.materialized()
    }
}

impl DerefMut for SharedRelationRows {
    fn deref_mut(&mut self) -> &mut Self::Target {
        if let SharedRelationRowsRepr::AppendPatch(_) = &self.0 {
            let rows = self.materialized().clone();
            self.0 = SharedRelationRowsRepr::Materialized(Arc::new(rows));
        }
        match &mut self.0 {
            SharedRelationRowsRepr::Materialized(rows) => Arc::make_mut(rows),
            SharedRelationRowsRepr::AppendPatch(_) => unreachable!(),
        }
    }
}

impl<'a> IntoIterator for &'a SharedRelationRows {
    type Item = &'a Vec<Value>;
    type IntoIter = std::slice::Iter<'a, Vec<Value>>;

    fn into_iter(self) -> Self::IntoIter {
        self.materialized().iter()
    }
}

impl PartialEq<Vec<Vec<Value>>> for SharedRelationRows {
    fn eq(&self, other: &Vec<Vec<Value>>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelationStore(CowMap<SemanticId, SharedRelationRows>);

impl RelationStore {
    pub fn insert(
        &mut self,
        relation: SemanticId,
        rows: Vec<Vec<Value>>,
    ) -> Option<SharedRelationRows> {
        self.0.insert(relation, rows.into())
    }

    #[must_use]
    pub fn get(&self, relation: &SemanticId) -> Option<&Vec<Vec<Value>>> {
        self.0.get(relation).map(|rows| &**rows)
    }

    pub fn get_mut(&mut self, relation: &SemanticId) -> Option<&mut Vec<Vec<Value>>> {
        self.0.get_mut(relation).map(SharedRelationRows::deref_mut)
    }

    pub fn append_persistent(&mut self, relation: SemanticId, inserted: Vec<Vec<Value>>) {
        let next = match self.0.get(&relation) {
            Some(rows) => rows.append_persistent(inserted),
            None => SharedRelationRows::from(inserted),
        };
        self.0.insert(relation, next);
    }

    pub fn remove(&mut self, relation: &SemanticId) -> Option<SharedRelationRows> {
        self.0.remove(relation)
    }
}

impl Deref for RelationStore {
    type Target = BTreeMap<SemanticId, SharedRelationRows>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for RelationStore {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<'a> IntoIterator for &'a RelationStore {
    type Item = (&'a SemanticId, &'a SharedRelationRows);
    type IntoIter = std::collections::btree_map::Iter<'a, SemanticId, SharedRelationRows>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Unit,
    Bool(bool),
    I64(i64),
    F64Bits(u64),
    Text(String),
    LiveEntityRef {
        entity_type: SemanticId,
        id: EntityId,
    },
    HistoricalEntityId {
        entity_type: SemanticId,
        id: EntityId,
    },
    Product(BTreeMap<SemanticId, Self>),
    Option(Option<Box<Self>>),
    Variant {
        tag: SemanticId,
        value: Box<Self>,
    },
    Seq(Vec<Self>),
    Set {
        equivalence: SemanticId,
        elements: Vec<Self>,
    },
    Bag {
        equivalence: SemanticId,
        entries: Vec<(Self, u64)>,
    },
    Map {
        key_equivalence: SemanticId,
        entries: Vec<(Self, Self)>,
    },
}

impl Value {
    #[must_use]
    pub fn contains_live_ref(&self) -> bool {
        match self {
            Self::LiveEntityRef { .. } => true,
            Self::HistoricalEntityId { .. }
            | Self::Unit
            | Self::Bool(_)
            | Self::I64(_)
            | Self::F64Bits(_)
            | Self::Text(_) => false,
            Self::Product(values) => values.values().any(Self::contains_live_ref),
            Self::Seq(values)
            | Self::Set {
                elements: values, ..
            } => values.iter().any(Self::contains_live_ref),
            Self::Variant { value, .. } => value.contains_live_ref(),
            Self::Option(value) => value.as_deref().is_some_and(Self::contains_live_ref),
            Self::Bag { entries, .. } => entries.iter().any(|(value, _)| value.contains_live_ref()),
            Self::Map { entries, .. } => entries
                .iter()
                .any(|(key, value)| key.contains_live_ref() || value.contains_live_ref()),
        }
    }

    #[must_use]
    pub fn first_dangling_live_ref(&self, live: &BTreeSet<EntityId>) -> Option<EntityId> {
        match self {
            Self::LiveEntityRef { id, .. } => (!live.contains(id)).then_some(*id),
            Self::HistoricalEntityId { .. }
            | Self::Unit
            | Self::Bool(_)
            | Self::I64(_)
            | Self::F64Bits(_)
            | Self::Text(_) => None,
            Self::Product(values) => values
                .values()
                .find_map(|value| value.first_dangling_live_ref(live)),
            Self::Seq(values) => values
                .iter()
                .find_map(|value| value.first_dangling_live_ref(live)),
            Self::Variant { value, .. } => value.first_dangling_live_ref(live),
            Self::Option(value) => value
                .as_deref()
                .and_then(|value| value.first_dangling_live_ref(live)),
            Self::Set { elements, .. } => elements
                .iter()
                .find_map(|value| value.first_dangling_live_ref(live)),
            Self::Bag { entries, .. } => entries
                .iter()
                .find_map(|(value, _)| value.first_dangling_live_ref(live)),
            Self::Map { entries, .. } => entries.iter().find_map(|(key, value)| {
                key.first_dangling_live_ref(live)
                    .or_else(|| value.first_dangling_live_ref(live))
            }),
        }
    }

    fn collect_live_refs(&self, output: &mut Vec<EntityId>) {
        match self {
            Self::LiveEntityRef { id, .. } => output.push(*id),
            Self::HistoricalEntityId { .. }
            | Self::Unit
            | Self::Bool(_)
            | Self::I64(_)
            | Self::F64Bits(_)
            | Self::Text(_) => {}
            Self::Product(values) => {
                for value in values.values() {
                    value.collect_live_refs(output);
                }
            }
            Self::Seq(values)
            | Self::Set {
                elements: values, ..
            } => {
                for value in values {
                    value.collect_live_refs(output);
                }
            }
            Self::Variant { value, .. } => value.collect_live_refs(output),
            Self::Option(value) => {
                if let Some(value) = value.as_deref() {
                    value.collect_live_refs(output);
                }
            }
            Self::Bag { entries, .. } => {
                for (value, _) in entries {
                    value.collect_live_refs(output);
                }
            }
            Self::Map { entries, .. } => {
                for (key, value) in entries {
                    key.collect_live_refs(output);
                    value.collect_live_refs(output);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveRefConsumers {
    fields: BTreeSet<(SemanticId, EntityId)>,
    relation_rows: BTreeMap<SemanticId, BTreeSet<usize>>,
}

impl LiveRefConsumers {
    #[must_use]
    pub fn fields(&self) -> &BTreeSet<(SemanticId, EntityId)> {
        &self.fields
    }

    #[must_use]
    pub fn relation_rows(&self) -> &BTreeMap<SemanticId, BTreeSet<usize>> {
        &self.relation_rows
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveRefSensitivityIndex {
    field_by_target: Arc<BTreeMap<LocalEntityId, BTreeSet<(SemanticId, EntityId)>>>,
    field_unresolved: Arc<BTreeMap<EntityId, BTreeSet<(SemanticId, EntityId)>>>,
    relations: BTreeMap<SemanticId, Arc<RelationLiveRefSensitivity>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RelationLiveRefSensitivity {
    by_target: BTreeMap<LocalEntityId, BTreeSet<usize>>,
    unresolved: BTreeMap<EntityId, BTreeSet<usize>>,
}

impl LiveRefSensitivityIndex {
    #[must_use]
    pub fn relation_has_live_refs(&self, relation: SemanticId) -> bool {
        self.relations.get(&relation).is_some_and(|sensitivity| {
            !sensitivity.by_target.is_empty() || !sensitivity.unresolved.is_empty()
        })
    }

    #[must_use]
    pub fn compile(model: &FiniteModel, ids: &DenseEntityIds) -> Self {
        let mut field_by_target = BTreeMap::<LocalEntityId, BTreeSet<_>>::new();
        let mut field_unresolved = BTreeMap::<EntityId, BTreeSet<_>>::new();
        for (&field, value) in &model.fields {
            let mut targets = Vec::new();
            value.collect_live_refs(&mut targets);
            for target in targets {
                if let Some(local) = ids.local(target) {
                    field_by_target.entry(local).or_default().insert(field);
                } else {
                    field_unresolved.entry(target).or_default().insert(field);
                }
            }
        }
        let relations = model
            .relations
            .iter()
            .map(|(&relation, rows)| (relation, Arc::new(Self::compile_relation(rows, ids))))
            .collect();
        Self {
            field_by_target: Arc::new(field_by_target),
            field_unresolved: Arc::new(field_unresolved),
            relations,
        }
    }

    fn compile_relation(rows: &[Vec<Value>], ids: &DenseEntityIds) -> RelationLiveRefSensitivity {
        let mut sensitivity = RelationLiveRefSensitivity::default();
        for (row_index, row) in rows.iter().enumerate() {
            let mut targets = Vec::new();
            for value in row {
                value.collect_live_refs(&mut targets);
            }
            targets.sort_unstable();
            targets.dedup();
            for target in targets {
                if let Some(local) = ids.local(target) {
                    sensitivity
                        .by_target
                        .entry(local)
                        .or_default()
                        .insert(row_index);
                } else {
                    sensitivity
                        .unresolved
                        .entry(target)
                        .or_default()
                        .insert(row_index);
                }
            }
        }
        sensitivity
    }

    #[must_use]
    pub fn consumers(&self, target: LocalEntityId) -> Option<LiveRefConsumers> {
        let mut consumers = LiveRefConsumers::default();
        if let Some(fields) = self.field_by_target.get(&target) {
            consumers.fields.clone_from(fields);
        }
        for (&relation, sensitivity) in &self.relations {
            if let Some(rows) = sensitivity.by_target.get(&target) {
                consumers.relation_rows.insert(relation, rows.clone());
            }
        }
        (!consumers.fields.is_empty() || !consumers.relation_rows.is_empty()).then_some(consumers)
    }

    #[must_use]
    pub fn unresolved(&self, target: EntityId) -> Option<LiveRefConsumers> {
        let mut consumers = LiveRefConsumers::default();
        if let Some(fields) = self.field_unresolved.get(&target) {
            consumers.fields.clone_from(fields);
        }
        for (&relation, sensitivity) in &self.relations {
            if let Some(rows) = sensitivity.unresolved.get(&target) {
                consumers.relation_rows.insert(relation, rows.clone());
            }
        }
        (!consumers.fields.is_empty() || !consumers.relation_rows.is_empty()).then_some(consumers)
    }

    #[must_use]
    pub fn with_relations_recompiled(
        &self,
        model: &FiniteModel,
        ids: &DenseEntityIds,
        relations: &BTreeSet<SemanticId>,
    ) -> Self {
        let mut next_relations = self.relations.clone();
        for &relation in relations {
            if let Some(rows) = model.relations.get(&relation) {
                next_relations.insert(relation, Arc::new(Self::compile_relation(rows, ids)));
            } else {
                next_relations.remove(&relation);
            }
        }
        Self {
            field_by_target: Arc::clone(&self.field_by_target),
            field_unresolved: Arc::clone(&self.field_unresolved),
            relations: next_relations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FiniteModel {
    pub carriers: CowMap<SemanticId, BTreeSet<EntityId>>,
    pub fields: CowMap<(SemanticId, EntityId), Value>,
    pub relations: RelationStore,
}

impl FiniteModel {
    fn all_entities(&self) -> BTreeSet<EntityId> {
        self.carriers
            .values()
            .flat_map(|carrier| carrier.iter().copied())
            .collect()
    }

    fn restrict_to_live_indexed(
        &mut self,
        live: &BTreeSet<EntityId>,
        ids: &DenseEntityIds,
        refs: &LiveRefSensitivityIndex,
    ) -> Result<bool, ModelError> {
        let carrier_entries_before = self.carriers.values().map(BTreeSet::len).sum::<usize>();
        let fields_before = self.fields.len();
        for carrier in self.carriers.values_mut() {
            carrier.retain(|entity| live.contains(entity));
        }
        self.fields.retain(|(_, owner), _| live.contains(owner));

        let mut live_dense = DenseEntitySet::with_capacity(ids.len());
        for entity in live {
            if let Some(local) = ids.local(*entity) {
                live_dense.insert(local);
            }
        }

        let mut relation_rows_to_remove = BTreeMap::<SemanticId, BTreeSet<usize>>::new();
        for (&target, fields) in refs.field_by_target.as_ref() {
            if live_dense.contains(target) {
                continue;
            }
            if fields.iter().any(|(_, owner)| live.contains(owner)) {
                let external = ids
                    .external(target)
                    .expect("sensitivity target comes from dense identity map");
                return Err(ModelError::DanglingLiveReference(external));
            }
        }
        for (&target, fields) in refs.field_unresolved.as_ref() {
            if fields.iter().any(|(_, owner)| live.contains(owner)) {
                return Err(ModelError::DanglingLiveReference(target));
            }
        }
        for (&relation, sensitivity) in &refs.relations {
            for (&target, rows) in &sensitivity.by_target {
                if !live_dense.contains(target) {
                    relation_rows_to_remove
                        .entry(relation)
                        .or_default()
                        .extend(rows.iter().copied());
                }
            }
            for rows in sensitivity.unresolved.values() {
                relation_rows_to_remove
                    .entry(relation)
                    .or_default()
                    .extend(rows.iter().copied());
            }
        }

        let removed_relation_rows = relation_rows_to_remove
            .values()
            .map(BTreeSet::len)
            .sum::<usize>();
        for (relation, rows_to_remove) in relation_rows_to_remove {
            let Some(rows) = self.relations.get_mut(&relation) else {
                continue;
            };
            let mut row_index = 0_usize;
            rows.retain(|_| {
                let keep = !rows_to_remove.contains(&row_index);
                row_index += 1;
                keep
            });
        }
        Ok(
            carrier_entries_before != self.carriers.values().map(BTreeSet::len).sum::<usize>()
                || fields_before != self.fields.len()
                || removed_relation_rows != 0,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    EntityMissingFromLifecycle(EntityId),
    DanglingLiveReference(EntityId),
    DenseIdentityCapacityExceeded,
    InvalidSemanticContext(ContextError),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DatabaseState {
    pub model: FiniteModel,
    pub lifecycle: CowValue<LifecycleGraph>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedDatabaseState {
    pub state: DatabaseState,
    pub dense_entities: Arc<DenseEntityIds>,
    pub live_ref_sensitivity: LiveRefSensitivityIndex,
}

impl DatabaseState {
    pub fn normalize(self) -> Result<Self, ModelError> {
        Ok(self.normalize_certified()?.state)
    }

    pub fn normalize_certified(mut self) -> Result<NormalizedDatabaseState, ModelError> {
        for entity in self.model.all_entities() {
            if !self.lifecycle.entities.contains(&entity) {
                return Err(ModelError::EntityMissingFromLifecycle(entity));
            }
        }

        let source_entities = self.lifecycle.entities.clone();
        let ids = DenseEntityIds::compile(&source_entities)
            .map_err(|_| ModelError::DenseIdentityCapacityExceeded)?;
        let refs = LiveRefSensitivityIndex::compile(&self.model, &ids);
        self.lifecycle = self.lifecycle.normalize().into();
        let model_changed =
            self.model
                .restrict_to_live_indexed(&self.lifecycle.entities, &ids, &refs)?;
        if self.lifecycle.entities == source_entities && !model_changed {
            return Ok(NormalizedDatabaseState {
                state: self,
                dense_entities: Arc::new(ids),
                live_ref_sensitivity: refs,
            });
        }
        let final_ids = Arc::new(
            DenseEntityIds::compile(&self.lifecycle.entities)
                .map_err(|_| ModelError::DenseIdentityCapacityExceeded)?,
        );
        let final_refs = LiveRefSensitivityIndex::compile(&self.model, &final_ids);
        Ok(NormalizedDatabaseState {
            state: self,
            dense_entities: final_ids,
            live_ref_sensitivity: final_refs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_values_are_identity_free_until_reference_is_explicit() {
        let left = Value::Product(BTreeMap::from([
            (SemanticId::new(1), Value::I64(1)),
            (SemanticId::new(2), Value::Text("x".into())),
        ]));
        let right = left.clone();
        assert_eq!(left, right);
    }

    #[test]
    fn set_and_map_equality_are_explicit_semantic_inputs() {
        let text_equivalence = SemanticId::new(900);
        let set = Value::Set {
            equivalence: text_equivalence,
            elements: vec![Value::Text("A".into()), Value::Text("a".into())],
        };
        let map = Value::Map {
            key_equivalence: text_equivalence,
            entries: vec![(Value::Text("A".into()), Value::I64(1))],
        };
        assert!(matches!(
            set,
            Value::Set {
                equivalence,
                ..
            } if equivalence == text_equivalence
        ));
        assert!(matches!(
            map,
            Value::Map {
                key_equivalence,
                ..
            } if key_equivalence == text_equivalence
        ));
    }

    #[test]
    fn normalization_removes_dead_entities_from_carriers_and_owned_fields() {
        let live = EntityId::new(1);
        let dead = EntityId::new(2);
        let entity_type = SemanticId::new(10);
        let field = SemanticId::new(11);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([live, dead]);
        state.lifecycle.roots.insert(live);
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([live, dead]));
        state.model.fields.insert((field, dead), Value::I64(7));

        let normalized = state.normalize().unwrap();
        assert_eq!(
            normalized.model.carriers[&entity_type],
            BTreeSet::from([live])
        );
        assert!(!normalized.model.fields.contains_key(&(field, dead)));
    }

    #[test]
    fn surviving_entity_cannot_keep_dangling_live_reference() {
        let live = EntityId::new(1);
        let dead = EntityId::new(2);
        let entity_type = SemanticId::new(10);
        let field = SemanticId::new(11);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([live, dead]);
        state.lifecycle.roots.insert(live);
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([live, dead]));
        state.model.fields.insert(
            (field, live),
            Value::LiveEntityRef {
                entity_type,
                id: dead,
            },
        );

        assert_eq!(
            state.normalize(),
            Err(ModelError::DanglingLiveReference(dead))
        );
    }

    #[test]
    fn historical_identity_may_outlive_entity() {
        let live = EntityId::new(1);
        let dead = EntityId::new(2);
        let entity_type = SemanticId::new(10);
        let field = SemanticId::new(11);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([live, dead]);
        state.lifecycle.roots.insert(live);
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([live, dead]));
        state.model.fields.insert(
            (field, live),
            Value::HistoricalEntityId {
                entity_type,
                id: dead,
            },
        );

        let normalized = state.normalize().unwrap();
        assert_eq!(
            normalized.model.fields[&(field, live)],
            Value::HistoricalEntityId {
                entity_type,
                id: dead
            }
        );
    }

    #[test]
    fn reverse_live_ref_sensitivity_indexes_nested_fields_and_relation_rows() {
        let owner = EntityId::new(1);
        let target = EntityId::new(2);
        let entity_type = SemanticId::new(10);
        let field = SemanticId::new(11);
        let relation = SemanticId::new(12);
        let ids = DenseEntityIds::compile(&BTreeSet::from([owner, target])).unwrap();
        let mut model = FiniteModel::default();
        model.fields.insert(
            (field, owner),
            Value::Product(BTreeMap::from([(
                SemanticId::new(99),
                Value::Option(Some(Box::new(Value::LiveEntityRef {
                    entity_type,
                    id: target,
                }))),
            )])),
        );
        model.relations.insert(
            relation,
            vec![
                vec![Value::I64(1)],
                vec![Value::LiveEntityRef {
                    entity_type,
                    id: target,
                }],
            ],
        );

        let sensitivity = LiveRefSensitivityIndex::compile(&model, &ids);
        let consumers = sensitivity.consumers(ids.local(target).unwrap()).unwrap();
        assert!(consumers.fields().contains(&(field, owner)));
        assert_eq!(
            consumers.relation_rows().get(&relation),
            Some(&BTreeSet::from([1]))
        );
    }

    #[test]
    fn relation_sensitivity_recompile_shares_unaffected_partitions() {
        let target = EntityId::new(2);
        let entity_type = SemanticId::new(10);
        let changed_relation = SemanticId::new(12);
        let stable_relation = SemanticId::new(13);
        let ids = DenseEntityIds::compile(&BTreeSet::from([target])).unwrap();
        let mut model = FiniteModel::default();
        model.relations.insert(
            changed_relation,
            vec![vec![Value::LiveEntityRef {
                entity_type,
                id: target,
            }]],
        );
        model.relations.insert(
            stable_relation,
            vec![vec![Value::LiveEntityRef {
                entity_type,
                id: target,
            }]],
        );

        let original = LiveRefSensitivityIndex::compile(&model, &ids);
        let original_stable = Arc::clone(&original.relations[&stable_relation]);
        let original_changed = Arc::clone(&original.relations[&changed_relation]);
        model
            .relations
            .insert(changed_relation, vec![vec![Value::I64(7)]]);
        let updated =
            original.with_relations_recompiled(&model, &ids, &BTreeSet::from([changed_relation]));

        assert!(Arc::ptr_eq(
            &original.field_by_target,
            &updated.field_by_target
        ));
        assert!(Arc::ptr_eq(
            &original.field_unresolved,
            &updated.field_unresolved
        ));
        assert!(Arc::ptr_eq(
            &original_stable,
            &updated.relations[&stable_relation]
        ));
        assert!(!Arc::ptr_eq(
            &original_changed,
            &updated.relations[&changed_relation]
        ));
        assert!(
            updated
                .consumers(ids.local(target).unwrap())
                .unwrap()
                .relation_rows()
                .get(&changed_relation)
                .is_none()
        );
    }

    #[test]
    fn database_state_clone_path_copies_only_the_mutated_relation() {
        let stable_relation = SemanticId::new(700);
        let changed_relation = SemanticId::new(701);
        let entity_type = SemanticId::new(702);
        let field = SemanticId::new(703);
        let entity = EntityId::new(1);
        let mut original = DatabaseState::default();
        original
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([entity]));
        original.model.fields.insert((field, entity), Value::I64(9));
        original
            .model
            .relations
            .insert(stable_relation, vec![vec![Value::I64(1)]]);
        original
            .model
            .relations
            .insert(changed_relation, vec![vec![Value::I64(2)]]);
        original.lifecycle.entities.insert(entity);

        let mut candidate = original.clone();
        assert!(Arc::ptr_eq(
            &original.model.carriers.0,
            &candidate.model.carriers.0
        ));
        assert!(Arc::ptr_eq(
            &original.model.fields.0,
            &candidate.model.fields.0
        ));
        assert!(Arc::ptr_eq(&original.lifecycle.0, &candidate.lifecycle.0));
        assert!(Arc::ptr_eq(
            &original.model.relations.0.0,
            &candidate.model.relations.0.0
        ));

        candidate
            .model
            .relations
            .get_mut(&changed_relation)
            .unwrap()
            .push(vec![Value::I64(3)]);

        assert!(Arc::ptr_eq(
            &original.model.carriers.0,
            &candidate.model.carriers.0
        ));
        assert!(Arc::ptr_eq(
            &original.model.fields.0,
            &candidate.model.fields.0
        ));
        assert!(Arc::ptr_eq(&original.lifecycle.0, &candidate.lifecycle.0));
        assert!(!Arc::ptr_eq(
            &original.model.relations.0.0,
            &candidate.model.relations.0.0
        ));
        let original_stable = match &original.model.relations.0[&stable_relation].0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::AppendPatch(_) => panic!("unexpected patch"),
        };
        let candidate_stable = match &candidate.model.relations.0[&stable_relation].0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::AppendPatch(_) => panic!("unexpected patch"),
        };
        let original_changed = match &original.model.relations.0[&changed_relation].0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::AppendPatch(_) => panic!("unexpected patch"),
        };
        let candidate_changed = match &candidate.model.relations.0[&changed_relation].0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::AppendPatch(_) => panic!("unexpected patch"),
        };
        assert!(Arc::ptr_eq(original_stable, candidate_stable));
        assert!(!Arc::ptr_eq(original_changed, candidate_changed));
        assert_eq!(original.model.relations[&changed_relation].len(), 1);
        assert_eq!(candidate.model.relations[&changed_relation].len(), 2);
    }

    #[test]
    fn persistent_relation_append_defers_materialization_and_preserves_source() {
        let base = SharedRelationRows::from(vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
        let appended = base.append_persistent(vec![vec![Value::I64(3)]]);

        let SharedRelationRowsRepr::AppendPatch(patch) = &appended.0 else {
            panic!("append must create a persistent patch");
        };
        assert!(patch.materialized.get().is_none());
        assert_eq!(base.as_slice(), &[vec![Value::I64(1)], vec![Value::I64(2)]]);
        assert_eq!(
            appended.as_slice(),
            &[
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(3)]
            ]
        );
        assert!(patch.materialized.get().is_some());
    }

    #[test]
    fn indexed_lifecycle_restriction_preserves_previous_dangling_reference_semantics() {
        let live = EntityId::new(1);
        let dead = EntityId::new(2);
        let entity_type = SemanticId::new(10);
        let relation = SemanticId::new(12);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([live, dead]);
        state.lifecycle.roots.insert(live);
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([live, dead]));
        state.model.relations.insert(
            relation,
            vec![
                vec![Value::I64(7)],
                vec![Value::LiveEntityRef {
                    entity_type,
                    id: dead,
                }],
            ],
        );

        let normalized = state.normalize().unwrap();
        assert_eq!(
            normalized.model.relations[&relation],
            vec![vec![Value::I64(7)]]
        );
    }
}
