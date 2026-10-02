use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use kernel_identity::{DenseEntityIds, DenseEntitySet};
use kernel_lifecycle::LifecycleGraph;
use kernel_schema::ContextError;
use kernel_types::{EntityId, SemanticId};

use crate::{CowMap, CowValue, LiveRefSensitivityIndex, RelationStore, Value};

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
    pub fn detach_relation_materialized_projections(&mut self) {
        self.model.relations.detach_materialized_projections();
    }

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
