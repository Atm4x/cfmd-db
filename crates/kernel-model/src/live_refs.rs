use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use kernel_identity::{DenseEntityIds, LocalEntityId};
use kernel_types::{EntityId, SemanticId};

use crate::{FiniteModel, Value};

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
    pub(super) field_by_target: Arc<BTreeMap<LocalEntityId, BTreeSet<(SemanticId, EntityId)>>>,
    pub(super) field_unresolved: Arc<BTreeMap<EntityId, BTreeSet<(SemanticId, EntityId)>>>,
    pub(super) relations: BTreeMap<SemanticId, Arc<RelationLiveRefSensitivity>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct RelationLiveRefSensitivity {
    pub(super) by_target: BTreeMap<LocalEntityId, BTreeSet<usize>>,
    pub(super) unresolved: BTreeMap<EntityId, BTreeSet<usize>>,
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
