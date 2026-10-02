mod live_refs;
mod state;
mod storage;
mod value;

pub use live_refs::{LiveRefConsumers, LiveRefSensitivityIndex};
pub use state::{DatabaseState, FiniteModel, ModelError, NormalizedDatabaseState};
pub use storage::{CowMap, CowValue, RelationStore, SharedRelationRows};
pub use value::Value;

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };

    use kernel_identity::DenseEntityIds;
    use kernel_types::{EntityId, SemanticId};

    use super::*;
    use crate::storage::SharedRelationRowsRepr;

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
            SharedRelationRowsRepr::DeltaRoot(_) => panic!("unexpected patch"),
        };
        let candidate_stable = match &candidate.model.relations.0[&stable_relation].0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::DeltaRoot(_) => panic!("unexpected patch"),
        };
        let original_changed = match &original.model.relations.0[&changed_relation].0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::DeltaRoot(_) => panic!("unexpected patch"),
        };
        let candidate_changed = match &candidate.model.relations.0[&changed_relation].0 {
            SharedRelationRowsRepr::Materialized(rows) => rows,
            SharedRelationRowsRepr::DeltaRoot(_) => panic!("unexpected patch"),
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

        let SharedRelationRowsRepr::DeltaRoot(_) = &appended.0 else {
            panic!("append must create a persistent delta root");
        };
        assert!(!appended.has_materialized_projection());
        assert_eq!(base.to_vec(), vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
        assert_eq!(
            appended.to_vec(),
            vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(3)]
            ]
        );
        assert!(!appended.has_materialized_projection());
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
