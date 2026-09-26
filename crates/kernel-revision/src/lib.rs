use std::{collections::BTreeSet, sync::Arc};

use kernel_identity::DenseEntityIds;
use kernel_lifecycle::DenseLifecycleProjection;
use kernel_model::{DatabaseState, LiveRefSensitivityIndex, ModelError};
use kernel_schema::{RelationSemantics, SemanticContext};
use kernel_semantics::{SemanticError, SemanticRegistry};
use kernel_types::{RevisionId, SemanticRevision};
use kernel_validation::{
    DenseTypeExtents, ValidationError, validate_relations_with_extents, validate_state_with_extents,
};

#[derive(Debug, Clone)]
struct RelationOnlyProvenance {
    identity: Arc<()>,
    source_identity: Option<Arc<()>>,
    touched_relations: Arc<BTreeSet<kernel_types::SemanticId>>,
}

impl RelationOnlyProvenance {
    fn root() -> Self {
        Self {
            identity: Arc::new(()),
            source_identity: None,
            touched_relations: Arc::new(BTreeSet::new()),
        }
    }

    fn derived(source: &Self, touched_relations: BTreeSet<kernel_types::SemanticId>) -> Self {
        Self {
            identity: Arc::new(()),
            source_identity: Some(Arc::clone(&source.identity)),
            touched_relations: Arc::new(touched_relations),
        }
    }

    fn certifies_from(
        &self,
        source: &Self,
        touched_relations: &BTreeSet<kernel_types::SemanticId>,
    ) -> bool {
        self.source_identity.as_ref().is_some_and(|identity| {
            Arc::ptr_eq(identity, &source.identity)
                && self.touched_relations.as_ref() == touched_relations
        })
    }
}

// Provenance is non-semantic runtime evidence. Two Revisions compare by their
// logical contents, not by which certified construction path produced them.
impl PartialEq for RelationOnlyProvenance {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for RelationOnlyProvenance {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionError {
    InvalidModel(ModelError),
    InvalidSemantics(SemanticError),
    InvalidTypedModel(ValidationError),
    InvalidRelationOnlyTransition,
}

impl From<ModelError> for RevisionError {
    fn from(value: ModelError) -> Self {
        Self::InvalidModel(value)
    }
}

/// Dense revision-local validation basis whose value depends only on the
/// entity universe, lifecycle graph and pinned schema -- never on relation
/// tuples. Relation-data-only revisions therefore share this root exactly;
/// full/schema/lifecycle reconstruction builds a new root.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RevisionDenseBasis {
    entities: Arc<DenseEntityIds>,
    lifecycle: DenseLifecycleProjection,
    type_extents: DenseTypeExtents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    id: RevisionId,
    semantics: SemanticRevision,
    semantic_context: SemanticContext,
    state: DatabaseState,
    dense_basis: Arc<RevisionDenseBasis>,
    live_ref_sensitivity: LiveRefSensitivityIndex,
    relation_only_provenance: RelationOnlyProvenance,
}

#[derive(Debug)]
pub struct RelationUpdateCandidate<'a> {
    source: &'a Revision,
    state: DatabaseState,
    touched_relations: BTreeSet<kernel_types::SemanticId>,
}

impl RelationUpdateCandidate<'_> {
    #[must_use]
    pub const fn state(&self) -> &DatabaseState {
        &self.state
    }

    pub fn replace_relation_rows(
        &mut self,
        relation: kernel_types::SemanticId,
        rows: Vec<Vec<kernel_model::Value>>,
    ) {
        self.state.model.relations.insert(relation, rows);
        self.touched_relations.insert(relation);
    }

    pub fn build(
        mut self,
        id: RevisionId,
        registry: &SemanticRegistry,
    ) -> Result<Revision, RevisionError> {
        registry
            .validate_context(&self.source.semantic_context)
            .map_err(RevisionError::InvalidSemantics)?;
        let live = &self.source.state.lifecycle.entities;
        for relation in &self.touched_relations {
            let Some(rows) = self.state.model.relations.get_mut(relation) else {
                continue;
            };
            rows.retain(|row| {
                row.iter()
                    .all(|value| value.first_dangling_live_ref(live).is_none())
            });
        }
        validate_relations_with_extents(
            &self.source.semantic_context,
            registry,
            &self.state,
            &self.source.dense_basis.type_extents,
            &self.touched_relations,
        )
        .map_err(RevisionError::InvalidTypedModel)?;
        let live_ref_sensitivity = self.source.live_ref_sensitivity.with_relations_recompiled(
            &self.state.model,
            &self.source.dense_basis.entities,
            &self.touched_relations,
        );
        Ok(Revision {
            id,
            semantics: self.source.semantics,
            semantic_context: self.source.semantic_context.clone(),
            state: self.state,
            dense_basis: Arc::clone(&self.source.dense_basis),
            live_ref_sensitivity,
            relation_only_provenance: RelationOnlyProvenance::derived(
                &self.source.relation_only_provenance,
                self.touched_relations,
            ),
        })
    }
}

impl Revision {
    /// Builds the exact endpoint of an append-only Bag transition without
    /// materializing the source relation.
    ///
    /// This path is deliberately narrow.  It is valid only when the source
    /// relation has no live-reference sensitivity and the appended rows add no
    /// live references.  Bag semantics has no uniqueness witness, so a valid
    /// source plus individually validated appended rows preserves relation-local
    /// validity without rescanning the old support.
    pub fn build_append_only_bag_relations(
        id: RevisionId,
        source: &Self,
        registry: &SemanticRegistry,
        appends: &[(kernel_types::SemanticId, Vec<Vec<kernel_model::Value>>)],
    ) -> Result<Self, RevisionError> {
        registry
            .validate_context(&source.semantic_context)
            .map_err(RevisionError::InvalidSemantics)?;
        let mut validation_state = source.state.clone();
        let mut touched = BTreeSet::new();
        for (relation, inserted) in appends {
            if !touched.insert(*relation) {
                return Err(RevisionError::InvalidRelationOnlyTransition);
            }
            let definition = source
                .semantic_context
                .schema
                .relation(*relation)
                .ok_or(RevisionError::InvalidRelationOnlyTransition)?;
            if !matches!(definition.semantics, RelationSemantics::Bag { .. })
                || source
                    .live_ref_sensitivity
                    .relation_has_live_refs(*relation)
                || inserted
                    .iter()
                    .flatten()
                    .any(kernel_model::Value::contains_live_ref)
            {
                return Err(RevisionError::InvalidRelationOnlyTransition);
            }
            validation_state
                .model
                .relations
                .insert(*relation, inserted.clone());
        }
        validate_relations_with_extents(
            &source.semantic_context,
            registry,
            &validation_state,
            &source.dense_basis.type_extents,
            &touched,
        )
        .map_err(RevisionError::InvalidTypedModel)?;

        let mut state = source.state.clone();
        for (relation, inserted) in appends {
            state
                .model
                .relations
                .append_persistent(*relation, inserted.clone());
        }
        Ok(Self {
            id,
            semantics: source.semantics,
            semantic_context: source.semantic_context.clone(),
            state,
            dense_basis: Arc::clone(&source.dense_basis),
            live_ref_sensitivity: source.live_ref_sensitivity.clone(),
            relation_only_provenance: RelationOnlyProvenance::derived(
                &source.relation_only_provenance,
                touched,
            ),
        })
    }

    #[must_use]
    pub fn relation_update_candidate(&self) -> RelationUpdateCandidate<'_> {
        RelationUpdateCandidate {
            source: self,
            state: self.state.clone(),
            touched_relations: BTreeSet::new(),
        }
    }

    pub fn build(
        id: RevisionId,
        context: &SemanticContext,
        registry: &SemanticRegistry,
        state: DatabaseState,
    ) -> Result<Self, RevisionError> {
        context
            .validate()
            .map_err(ModelError::InvalidSemanticContext)?;
        registry
            .validate_context(context)
            .map_err(RevisionError::InvalidSemantics)?;
        let normalized = state.normalize_certified()?;
        let state = normalized.state;
        let dense_entities = normalized.dense_entities;
        let dense_type_extents =
            DenseTypeExtents::compile_with_ids(&state.model, &context.schema, &dense_entities);
        validate_state_with_extents(context, registry, &state, &dense_type_extents)
            .map_err(RevisionError::InvalidTypedModel)?;
        let dense_lifecycle =
            DenseLifecycleProjection::compile_with_ids(&state.lifecycle, &dense_entities);
        let live_ref_sensitivity = normalized.live_ref_sensitivity;
        Ok(Self {
            id,
            semantics: context.revision(),
            semantic_context: context.clone(),
            state,
            dense_basis: Arc::new(RevisionDenseBasis {
                entities: dense_entities,
                lifecycle: dense_lifecycle,
                type_extents: dense_type_extents,
            }),
            live_ref_sensitivity,
            relation_only_provenance: RelationOnlyProvenance::root(),
        })
    }

    pub fn build_relation_update(
        id: RevisionId,
        source: &Self,
        registry: &SemanticRegistry,
        state: DatabaseState,
        touched_relations: &BTreeSet<kernel_types::SemanticId>,
    ) -> Result<Self, RevisionError> {
        registry
            .validate_context(&source.semantic_context)
            .map_err(RevisionError::InvalidSemantics)?;
        if state.lifecycle != source.state.lifecycle
            || state.model.carriers != source.state.model.carriers
            || state.model.fields != source.state.model.fields
        {
            return Err(RevisionError::InvalidRelationOnlyTransition);
        }
        let source_untouched = source
            .state
            .model
            .relations
            .iter()
            .filter(|(relation, _)| !touched_relations.contains(relation));
        let target_untouched = state
            .model
            .relations
            .iter()
            .filter(|(relation, _)| !touched_relations.contains(relation));
        if !source_untouched.eq(target_untouched) {
            return Err(RevisionError::InvalidRelationOnlyTransition);
        }
        RelationUpdateCandidate {
            source,
            state,
            touched_relations: touched_relations.clone(),
        }
        .build(id, registry)
    }

    #[must_use]
    pub const fn id(&self) -> RevisionId {
        self.id
    }

    #[must_use]
    pub const fn semantic_revision(&self) -> SemanticRevision {
        self.semantics
    }

    #[must_use]
    pub fn semantic_context(&self) -> &SemanticContext {
        &self.semantic_context
    }

    #[must_use]
    pub const fn state(&self) -> &DatabaseState {
        &self.state
    }

    /// Returns whether this revision was constructed by the certified
    /// relation-only builder from this exact source revision lineage, touching
    /// exactly the named relations. This proves that lifecycle/carriers/fields
    /// and every untouched relation were inherited from `source`; callers must
    /// still validate any claimed relation delta against the touched endpoint.
    #[must_use]
    pub fn certifies_relation_only_from(
        &self,
        source: &Self,
        touched_relations: &BTreeSet<kernel_types::SemanticId>,
    ) -> bool {
        self.relation_only_provenance
            .certifies_from(&source.relation_only_provenance, touched_relations)
    }

    #[must_use]
    pub fn dense_entity_ids(&self) -> Arc<DenseEntityIds> {
        Arc::clone(&self.dense_basis.entities)
    }

    #[must_use]
    pub fn dense_lifecycle(&self) -> &DenseLifecycleProjection {
        &self.dense_basis.lifecycle
    }

    #[must_use]
    pub fn dense_type_extents(&self) -> &DenseTypeExtents {
        &self.dense_basis.type_extents
    }

    #[must_use]
    pub const fn live_ref_sensitivity(&self) -> &LiveRefSensitivityIndex {
        &self.live_ref_sensitivity
    }
}

#[cfg(test)]
mod tests {
    use kernel_model::{DatabaseState, FiniteModel, Value};
    use kernel_schema::{FieldDef, ScalarType, Schema, SemanticEnvironment, TypeExpr};
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{EntityId, SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    fn empty_context() -> SemanticContext {
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(1)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        }
    }

    #[test]
    fn construction_always_runs_registry_and_typed_validation() {
        let entity_type = SemanticId::new(1);
        let field = SemanticId::new(2);
        let entity = EntityId::new(10);
        let mut context = empty_context();
        context
            .schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let mut state = DatabaseState {
            model: FiniteModel::default(),
            ..DatabaseState::default()
        };
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
        state.model.carriers.insert(entity_type, [entity].into());
        state
            .model
            .fields
            .insert((field, entity), Value::Text("wrong".into()));

        let error = Revision::build(
            RevisionId::new(1),
            &context,
            &SemanticRegistry::default(),
            state,
        )
        .unwrap_err();
        assert!(matches!(error, RevisionError::InvalidTypedModel(_)));
    }

    #[test]
    fn revision_pins_full_context_not_only_numeric_ids() {
        let context = empty_context();
        let revision = Revision::build(
            RevisionId::new(1),
            &context,
            &SemanticRegistry::default(),
            DatabaseState::default(),
        )
        .unwrap();
        assert_eq!(revision.semantic_context(), &context);
        assert_eq!(revision.semantic_revision(), context.revision());
    }

    #[test]
    fn revision_owns_one_shared_dense_identity_projection_for_physical_derivatives() {
        let context = empty_context();
        let first = EntityId::new(10);
        let second = EntityId::new(20);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([first, second]);
        state.lifecycle.roots.insert(first);
        state
            .lifecycle
            .keeps_alive
            .entry(first)
            .or_default()
            .insert(second);
        let revision = Revision::build(
            RevisionId::new(1),
            &context,
            &SemanticRegistry::default(),
            state,
        )
        .unwrap();

        let left = revision.dense_entity_ids();
        let right = revision.dense_entity_ids();
        assert!(Arc::ptr_eq(&left, &right));
        assert_eq!(left.local(first).unwrap().index(), 0);
        assert_eq!(left.local(second).unwrap().index(), 1);
        assert_eq!(revision.dense_lifecycle().live_count(), 2);
    }

    #[test]
    fn relation_update_reuses_revision_local_derivatives_and_rejects_static_changes() {
        let relation = SemanticId::new(90);
        let equivalence = SemanticId::new(91);
        let mut context = empty_context();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        context.environment.pin_module(equivalence, digest);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
        let mut state = DatabaseState::default();
        state
            .model
            .relations
            .insert(relation, vec![vec![Value::I64(1)]]);
        let source = Revision::build(RevisionId::new(1), &context, &registry, state).unwrap();
        let source_ids = source.dense_entity_ids();

        let mut target_state = source.state().clone();
        target_state
            .model
            .relations
            .insert(relation, vec![vec![Value::I64(2)]]);
        let target = Revision::build_relation_update(
            RevisionId::new(2),
            &source,
            &registry,
            target_state,
            &BTreeSet::from([relation]),
        )
        .unwrap();
        assert!(Arc::ptr_eq(&source_ids, &target.dense_entity_ids()));
        assert!(Arc::ptr_eq(&source.dense_basis, &target.dense_basis));
        assert!(target.certifies_relation_only_from(&source, &BTreeSet::from([relation])));
        assert!(!target.certifies_relation_only_from(&source, &BTreeSet::new()));
        assert_eq!(
            source.dense_type_extents().entity_count(),
            target.dense_type_extents().entity_count(),
        );

        let append_target = Revision::build_append_only_bag_relations(
            RevisionId::new(4),
            &target,
            &registry,
            &[(relation, vec![vec![Value::I64(3)]])],
        )
        .unwrap();
        assert!(Arc::ptr_eq(&target.dense_basis, &append_target.dense_basis));
        assert!(append_target.certifies_relation_only_from(&target, &BTreeSet::from([relation])));
        assert!(!append_target.certifies_relation_only_from(&source, &BTreeSet::from([relation])));

        let independently_rebuilt = Revision::build(
            RevisionId::new(2),
            &context,
            &registry,
            target.state().clone(),
        )
        .unwrap();
        assert!(
            !independently_rebuilt
                .certifies_relation_only_from(&source, &BTreeSet::from([relation]))
        );

        let mut invalid = target.state().clone();
        invalid.lifecycle.entities.insert(EntityId::new(999));
        assert_eq!(
            Revision::build_relation_update(
                RevisionId::new(3),
                &target,
                &registry,
                invalid,
                &BTreeSet::from([relation]),
            ),
            Err(RevisionError::InvalidRelationOnlyTransition),
        );
    }

    #[test]
    fn certified_relation_update_revalidates_the_pinned_semantic_registry() {
        let relation = SemanticId::new(190);
        let equivalence = SemanticId::new(191);
        let mut context = empty_context();
        let mut build_registry = SemanticRegistry::default();
        let digest = build_registry.install_equivalence(EquivalenceModule::I64Exact);
        context.environment.pin_module(equivalence, digest);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
        let mut state = DatabaseState::default();
        state
            .model
            .relations
            .insert(relation, vec![vec![Value::I64(1)]]);
        let source =
            Revision::build(RevisionId::new(10), &context, &build_registry, state).unwrap();

        let mut candidate = source.relation_update_candidate();
        candidate.replace_relation_rows(relation, vec![vec![Value::I64(2)]]);
        let error = candidate
            .build(RevisionId::new(11), &SemanticRegistry::default())
            .unwrap_err();

        assert!(matches!(error, RevisionError::InvalidSemantics(_)));
    }

    #[test]
    fn certified_relation_update_matches_full_normalization_for_dangling_live_ref_rows() {
        let person = SemanticId::new(290);
        let relation = SemanticId::new(291);
        let equivalence = SemanticId::new(292);
        let live = EntityId::new(10);
        let dangling = EntityId::new(20);
        let mut context = empty_context();
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        context.environment.pin_module(equivalence, digest);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(person))],
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(live);
        state.lifecycle.roots.insert(live);
        state.model.carriers.insert(person, BTreeSet::from([live]));
        state.model.relations.insert(relation, Vec::new());
        let source = Revision::build(RevisionId::new(20), &context, &registry, state).unwrap();
        let dangling_row = vec![Value::LiveEntityRef {
            entity_type: person,
            id: dangling,
        }];

        let mut full_state = source.state().clone();
        full_state
            .model
            .relations
            .insert(relation, vec![dangling_row.clone()]);
        let full = Revision::build(RevisionId::new(21), &context, &registry, full_state).unwrap();

        let mut candidate = source.relation_update_candidate();
        candidate.replace_relation_rows(relation, vec![dangling_row]);
        let incremental = candidate.build(RevisionId::new(21), &registry).unwrap();

        assert_eq!(incremental.state(), full.state());
        assert!(incremental.state().model.relations[&relation].is_empty());
    }
}
