use std::collections::{BTreeMap, BTreeSet, VecDeque};

use kernel_identity::{IdentityTransport, IdentityTransportError};
use kernel_lifecycle::{LifecycleConflict, LifecycleGraph, LifecycleIntent};
use kernel_model::{DatabaseState, ModelError};
use kernel_revision::{Revision, RevisionError};
use kernel_schema::SemanticContext;
use kernel_semantics::{SemanticError, SemanticRegistry};
use kernel_transport::{
    BijectiveIdentityRevisionTransport, ConservativeSemanticEnvironmentExtension,
    DefinitionalTransport, EquivalentSemanticEnvironmentTransport, SemanticLawMigration,
    TransportError, TypedFieldTransport, TypedRelationTransport, transport_database_state,
    transport_lifecycle_graph, transport_lifecycle_intent,
};
use kernel_types::{RevisionId, SemanticRevision};
use kernel_validation::ValidationError;

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParentRewrite {
    Lifecycle(LifecycleIntent),
    DefinitionalTransport,
    EquivalentSemanticEnvironmentTransport,
    ConservativeSemanticEnvironmentExtension,
    BijectiveIdentityRevisionTransport(Box<BijectiveIdentityRevisionTransport>),
    TypedFieldTransport,
    TypedRelationTransport,
    SemanticLawMigration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParentIntent {
    parent: RevisionId,
    rewrite: ParentRewrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RevisionNode {
    revision: Revision,
    parents: BTreeSet<RevisionId>,
    parent_intent: Option<ParentIntent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LifecycleTrace {
    base_graph: LifecycleGraph,
    intent: LifecycleIntent,
    base_to_descendant: IdentityTransport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    UnknownRevision(RevisionId),
    NoRevision,
    AmbiguousMergeBase(BTreeSet<RevisionId>),
    NonLinearIntentPath,
    NonIdentityRewriteInLifecyclePath,
    AmbiguousIdentityCoordinate,
    CoordinateRevisionNotBranch(RevisionId),
    IdentityHistoryTransport(IdentityTransportError),
    LifecycleConflict(LifecycleConflict),
    InvalidModel(ModelError),
    InvalidSemantics(SemanticError),
    InvalidTypedModel(ValidationError),
    InvalidRevision(RevisionError),
    InvalidTransport(TransportError),
    SemanticTransportRequired {
        from: SemanticRevision,
        to: SemanticRevision,
    },
}

impl From<ModelError> for StoreError {
    fn from(value: ModelError) -> Self {
        Self::InvalidModel(value)
    }
}

impl From<LifecycleConflict> for StoreError {
    fn from(value: LifecycleConflict) -> Self {
        Self::LifecycleConflict(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStore {
    semantic_registry: SemanticRegistry,
    revisions: BTreeMap<RevisionId, RevisionNode>,
    head: Option<RevisionId>,
    next_revision: u64,
}

impl Default for MemoryStore {
    fn default() -> Self {
        Self {
            semantic_registry: SemanticRegistry::default(),
            revisions: BTreeMap::new(),
            head: None,
            next_revision: 1,
        }
    }
}

impl MemoryStore {
    #[must_use]
    pub fn with_semantic_registry(semantic_registry: SemanticRegistry) -> Self {
        Self {
            semantic_registry,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn semantic_registry(&self) -> &SemanticRegistry {
        &self.semantic_registry
    }

    #[must_use]
    pub fn head(&self) -> Option<&Revision> {
        self.head.and_then(|id| self.revision(id))
    }

    pub fn bootstrap(
        &mut self,
        context: &SemanticContext,
        state: DatabaseState,
    ) -> Result<RevisionId, StoreError> {
        self.insert_revision(BTreeSet::new(), None, context, state)
    }

    pub fn commit_snapshot_from(
        &mut self,
        parents: BTreeSet<RevisionId>,
        context: &SemanticContext,
        state: DatabaseState,
    ) -> Result<RevisionId, StoreError> {
        self.require_revisions(&parents)?;
        for &parent in &parents {
            let parent_revision = self
                .revision(parent)
                .ok_or(StoreError::UnknownRevision(parent))?;
            if parent_revision.semantic_context() != context {
                return Err(StoreError::SemanticTransportRequired {
                    from: parent_revision.semantic_revision(),
                    to: context.revision(),
                });
            }
        }
        self.insert_revision(parents, None, context, state)
    }

    pub fn commit_lifecycle(
        &mut self,
        parent: RevisionId,
        context: &SemanticContext,
        intent: LifecycleIntent,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != context {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: context.revision(),
            });
        }
        let mut state = parent_revision.state().clone();
        state.lifecycle.apply_intent(&intent);
        let parent_intent = ParentIntent {
            parent,
            rewrite: ParentRewrite::Lifecycle(intent),
        };
        self.insert_revision(
            BTreeSet::from([parent]),
            Some(parent_intent),
            context,
            state,
        )
    }

    pub fn commit_definitional_transport(
        &mut self,
        parent: RevisionId,
        transport: &DefinitionalTransport,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != transport.source() {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: transport.target().revision(),
            });
        }
        let verified = DefinitionalTransport::verify(
            transport.source(),
            transport.target(),
            &self.semantic_registry,
        )
        .map_err(StoreError::InvalidTransport)?;
        let state = verified.transport_state(parent_revision.state());
        self.insert_revision(
            BTreeSet::from([parent]),
            Some(ParentIntent {
                parent,
                rewrite: ParentRewrite::DefinitionalTransport,
            }),
            verified.target(),
            state,
        )
    }

    pub fn commit_typed_field_transport(
        &mut self,
        parent: RevisionId,
        transport: &TypedFieldTransport,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != transport.source() {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: transport.target().revision(),
            });
        }
        let id = RevisionId::new(self.next_revision);
        let revision = transport
            .transport_revision(parent_revision, id, &self.semantic_registry)
            .map_err(StoreError::InvalidTransport)?;
        self.next_revision += 1;
        self.revisions.insert(
            id,
            RevisionNode {
                revision,
                parents: BTreeSet::from([parent]),
                parent_intent: Some(ParentIntent {
                    parent,
                    rewrite: ParentRewrite::TypedFieldTransport,
                }),
            },
        );
        self.head = Some(id);
        Ok(id)
    }

    pub fn commit_typed_relation_transport(
        &mut self,
        parent: RevisionId,
        transport: &TypedRelationTransport,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != transport.source() {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: transport.target().revision(),
            });
        }
        let id = RevisionId::new(self.next_revision);
        let revision = transport
            .transport_revision(parent_revision, id, &self.semantic_registry)
            .map_err(StoreError::InvalidTransport)?;
        self.next_revision += 1;
        self.revisions.insert(
            id,
            RevisionNode {
                revision,
                parents: BTreeSet::from([parent]),
                parent_intent: Some(ParentIntent {
                    parent,
                    rewrite: ParentRewrite::TypedRelationTransport,
                }),
            },
        );
        self.head = Some(id);
        Ok(id)
    }

    pub fn commit_bijective_identity_transport(
        &mut self,
        parent: RevisionId,
        transport: &BijectiveIdentityRevisionTransport,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != transport.source() {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: transport.target().revision(),
            });
        }
        let id = RevisionId::new(self.next_revision);
        let revision = transport
            .transport_revision(parent_revision, id, &self.semantic_registry)
            .map_err(StoreError::InvalidTransport)?;
        self.next_revision += 1;
        self.revisions.insert(
            id,
            RevisionNode {
                revision,
                parents: BTreeSet::from([parent]),
                parent_intent: Some(ParentIntent {
                    parent,
                    rewrite: ParentRewrite::BijectiveIdentityRevisionTransport(Box::new(
                        transport.clone(),
                    )),
                }),
            },
        );
        self.head = Some(id);
        Ok(id)
    }

    pub fn commit_equivalent_semantic_environment_transport(
        &mut self,
        parent: RevisionId,
        transport: &EquivalentSemanticEnvironmentTransport,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != transport.source() {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: transport.target().revision(),
            });
        }
        let verified = EquivalentSemanticEnvironmentTransport::verify(
            transport.source(),
            transport.target(),
            &self.semantic_registry,
        )
        .map_err(StoreError::InvalidTransport)?;
        let id = RevisionId::new(self.next_revision);
        let revision = verified
            .transport_revision(parent_revision, id, &self.semantic_registry)
            .map_err(StoreError::InvalidTransport)?;
        self.next_revision += 1;
        self.revisions.insert(
            id,
            RevisionNode {
                revision,
                parents: BTreeSet::from([parent]),
                parent_intent: Some(ParentIntent {
                    parent,
                    rewrite: ParentRewrite::EquivalentSemanticEnvironmentTransport,
                }),
            },
        );
        self.head = Some(id);
        Ok(id)
    }

    pub fn commit_conservative_semantic_environment_extension(
        &mut self,
        parent: RevisionId,
        transport: &ConservativeSemanticEnvironmentExtension,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != transport.source() {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: transport.target().revision(),
            });
        }
        let verified = ConservativeSemanticEnvironmentExtension::verify(
            transport.source(),
            transport.target(),
            &self.semantic_registry,
        )
        .map_err(StoreError::InvalidTransport)?;
        let id = RevisionId::new(self.next_revision);
        let revision = verified
            .transport_revision(parent_revision, id, &self.semantic_registry)
            .map_err(StoreError::InvalidTransport)?;
        self.next_revision += 1;
        self.revisions.insert(
            id,
            RevisionNode {
                revision,
                parents: BTreeSet::from([parent]),
                parent_intent: Some(ParentIntent {
                    parent,
                    rewrite: ParentRewrite::ConservativeSemanticEnvironmentExtension,
                }),
            },
        );
        self.head = Some(id);
        Ok(id)
    }

    pub fn commit_semantic_law_migration(
        &mut self,
        parent: RevisionId,
        migration: &SemanticLawMigration,
    ) -> Result<RevisionId, StoreError> {
        let parent_revision = self
            .revision(parent)
            .ok_or(StoreError::UnknownRevision(parent))?;
        if parent_revision.semantic_context() != migration.source() {
            return Err(StoreError::SemanticTransportRequired {
                from: parent_revision.semantic_revision(),
                to: migration.target().revision(),
            });
        }
        let verified = SemanticLawMigration::verify(
            migration.source(),
            migration.target(),
            &self.semantic_registry,
        )
        .map_err(StoreError::InvalidTransport)?;
        let id = RevisionId::new(self.next_revision);
        let revision = verified
            .transport_revision(parent_revision, id, &self.semantic_registry)
            .map_err(StoreError::InvalidTransport)?;
        self.next_revision += 1;
        self.revisions.insert(
            id,
            RevisionNode {
                revision,
                parents: BTreeSet::from([parent]),
                parent_intent: Some(ParentIntent {
                    parent,
                    rewrite: ParentRewrite::SemanticLawMigration,
                }),
            },
        );
        self.head = Some(id);
        Ok(id)
    }

    pub fn merge_lifecycle_branches(
        &mut self,
        left: RevisionId,
        right: RevisionId,
        context: &SemanticContext,
    ) -> Result<RevisionId, StoreError> {
        self.merge_lifecycle_branches_inner(left, right, context, None)
    }

    pub fn merge_lifecycle_branches_in_revision_space(
        &mut self,
        left: RevisionId,
        right: RevisionId,
        context: &SemanticContext,
        coordinate_revision: RevisionId,
    ) -> Result<RevisionId, StoreError> {
        self.merge_lifecycle_branches_inner(left, right, context, Some(coordinate_revision))
    }

    fn merge_lifecycle_branches_inner(
        &mut self,
        left: RevisionId,
        right: RevisionId,
        context: &SemanticContext,
        coordinate_revision: Option<RevisionId>,
    ) -> Result<RevisionId, StoreError> {
        let base = self.unique_merge_base(left, right)?;
        for revision_id in [base, left, right] {
            let revision = self
                .revision(revision_id)
                .ok_or(StoreError::UnknownRevision(revision_id))?;
            if revision.semantic_context() != context
                && !self
                    .semantic_registry
                    .context_conservatively_extends(revision.semantic_context(), context)
                    .map_err(StoreError::InvalidSemantics)?
            {
                return Err(StoreError::SemanticTransportRequired {
                    from: revision.semantic_revision(),
                    to: context.revision(),
                });
            }
        }

        let left_trace = self.lifecycle_trace_since(base, left)?;
        let right_trace = self.lifecycle_trace_since(base, right)?;
        let left_revision = self
            .revision(left)
            .ok_or(StoreError::UnknownRevision(left))?;
        let right_revision = self
            .revision(right)
            .ok_or(StoreError::UnknownRevision(right))?;

        let target_coordinates = if let Some(coordinate_revision) = coordinate_revision {
            if coordinate_revision == left {
                left_trace.base_to_descendant.clone()
            } else if coordinate_revision == right {
                right_trace.base_to_descendant.clone()
            } else {
                return Err(StoreError::CoordinateRevisionNotBranch(coordinate_revision));
            }
        } else {
            match (
                left_revision.semantic_context() == context,
                right_revision.semantic_context() == context,
            ) {
                (true, false) => left_trace.base_to_descendant.clone(),
                (false, true) => right_trace.base_to_descendant.clone(),
                (true, true) | (false, false) => {
                    if left_trace.base_to_descendant == right_trace.base_to_descendant {
                        left_trace.base_to_descendant.clone()
                    } else {
                        return Err(StoreError::AmbiguousIdentityCoordinate);
                    }
                }
            }
        };

        let align =
            |trace: LifecycleTrace| -> Result<(LifecycleGraph, LifecycleIntent), StoreError> {
                let to_target = trace
                    .base_to_descendant
                    .inverse()
                    .then(&target_coordinates)
                    .map_err(StoreError::IdentityHistoryTransport)?;
                Ok((
                    transport_lifecycle_graph(&to_target, &trace.base_graph)
                        .map_err(StoreError::InvalidTransport)?,
                    transport_lifecycle_intent(&to_target, &trace.intent)
                        .map_err(StoreError::InvalidTransport)?,
                ))
            };
        let (left_base, left_intent) = align(left_trace)?;
        let (right_base, right_intent) = align(right_trace)?;
        if left_base != right_base {
            return Err(StoreError::AmbiguousIdentityCoordinate);
        }

        let merged_lifecycle =
            LifecycleGraph::merge_from_lca(&left_base, &left_intent, &right_intent)?;
        let base_revision = self
            .revision(base)
            .ok_or(StoreError::UnknownRevision(base))?;
        let mut state = transport_database_state(&target_coordinates, base_revision.state())
            .map_err(StoreError::InvalidTransport)?;
        state.lifecycle = merged_lifecycle.into();
        self.insert_revision(BTreeSet::from([left, right]), None, context, state)
    }

    fn require_revisions(&self, revisions: &BTreeSet<RevisionId>) -> Result<(), StoreError> {
        for &revision in revisions {
            if !self.revisions.contains_key(&revision) {
                return Err(StoreError::UnknownRevision(revision));
            }
        }
        Ok(())
    }

    fn insert_revision(
        &mut self,
        parents: BTreeSet<RevisionId>,
        parent_intent: Option<ParentIntent>,
        context: &SemanticContext,
        state: DatabaseState,
    ) -> Result<RevisionId, StoreError> {
        self.semantic_registry
            .validate_context(context)
            .map_err(StoreError::InvalidSemantics)?;
        let id = RevisionId::new(self.next_revision);
        let revision = Revision::build(id, context, &self.semantic_registry, state)
            .map_err(StoreError::InvalidRevision)?;
        self.next_revision += 1;
        let node = RevisionNode {
            revision,
            parents,
            parent_intent,
        };
        self.revisions.insert(id, node);
        self.head = Some(id);
        Ok(id)
    }

    #[must_use]
    pub fn revision(&self, id: RevisionId) -> Option<&Revision> {
        self.revisions.get(&id).map(|node| &node.revision)
    }

    #[must_use]
    pub fn parents(&self, id: RevisionId) -> Option<&BTreeSet<RevisionId>> {
        self.revisions.get(&id).map(|node| &node.parents)
    }

    #[must_use]
    pub fn ancestors_including(&self, start: RevisionId) -> BTreeSet<RevisionId> {
        let mut result = BTreeSet::new();
        let mut queue = VecDeque::from([start]);
        while let Some(current) = queue.pop_front() {
            if !result.insert(current) {
                continue;
            }
            if let Some(node) = self.revisions.get(&current) {
                queue.extend(node.parents.iter().copied());
            }
        }
        result
    }

    #[must_use]
    pub fn is_ancestor(&self, possible_ancestor: RevisionId, revision: RevisionId) -> bool {
        self.ancestors_including(revision)
            .contains(&possible_ancestor)
    }

    #[must_use]
    pub fn lowest_common_ancestors(
        &self,
        left: RevisionId,
        right: RevisionId,
    ) -> BTreeSet<RevisionId> {
        let left_ancestors = self.ancestors_including(left);
        let right_ancestors = self.ancestors_including(right);
        let common: BTreeSet<_> = left_ancestors
            .intersection(&right_ancestors)
            .copied()
            .collect();

        common
            .iter()
            .copied()
            .filter(|candidate| {
                !common
                    .iter()
                    .copied()
                    .any(|other| other != *candidate && self.is_ancestor(*candidate, other))
            })
            .collect()
    }

    pub fn unique_merge_base(
        &self,
        left: RevisionId,
        right: RevisionId,
    ) -> Result<RevisionId, StoreError> {
        let lcas = self.lowest_common_ancestors(left, right);
        match (lcas.len(), lcas.iter().next().copied()) {
            (1, Some(only)) => Ok(only),
            (0, _) => Err(StoreError::NoRevision),
            _ => Err(StoreError::AmbiguousMergeBase(lcas)),
        }
    }

    fn lifecycle_trace_since(
        &self,
        ancestor: RevisionId,
        descendant: RevisionId,
    ) -> Result<LifecycleTrace, StoreError> {
        let ancestor_revision = self
            .revision(ancestor)
            .ok_or(StoreError::UnknownRevision(ancestor))?;
        let base_ids = ancestor_revision.state().lifecycle.entities.clone();
        let mut trace = LifecycleTrace {
            base_graph: (*ancestor_revision.state().lifecycle).clone(),
            intent: LifecycleIntent::default(),
            base_to_descendant: IdentityTransport::identity(&base_ids),
        };
        if ancestor == descendant {
            return Ok(trace);
        }

        let mut current = descendant;
        let mut reverse_path = Vec::new();
        while current != ancestor {
            let node = self
                .revisions
                .get(&current)
                .ok_or(StoreError::UnknownRevision(current))?;
            let edge = node
                .parent_intent
                .as_ref()
                .ok_or(StoreError::NonLinearIntentPath)?;
            reverse_path.push(edge.rewrite.clone());
            current = edge.parent;
        }

        for rewrite in reverse_path.into_iter().rev() {
            match rewrite {
                ParentRewrite::Lifecycle(intent) => {
                    trace.intent = trace.intent.then(&intent);
                }
                ParentRewrite::DefinitionalTransport
                | ParentRewrite::EquivalentSemanticEnvironmentTransport
                | ParentRewrite::ConservativeSemanticEnvironmentExtension => {}
                ParentRewrite::BijectiveIdentityRevisionTransport(transport) => {
                    trace.base_graph = transport
                        .transport_lifecycle_graph(&trace.base_graph)
                        .map_err(StoreError::InvalidTransport)?;
                    trace.intent = transport
                        .transport_lifecycle_intent(&trace.intent)
                        .map_err(StoreError::InvalidTransport)?;
                    trace.base_to_descendant = trace
                        .base_to_descendant
                        .then(transport.identity())
                        .map_err(StoreError::IdentityHistoryTransport)?;
                }
                ParentRewrite::TypedFieldTransport
                | ParentRewrite::TypedRelationTransport
                | ParentRewrite::SemanticLawMigration => {
                    return Err(StoreError::NonIdentityRewriteInLifecyclePath);
                }
            }
        }
        Ok(trace)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use kernel_lifecycle::{LifecycleFact, LifecycleIntent};
    use kernel_model::{DatabaseState, Value};
    use kernel_query::{ExactQuery, Expr};
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_transport::{FieldRewrite, TypedFieldTransport};
    use kernel_types::{EntityId, SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    fn context() -> SemanticContext {
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(2)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(4)),
        }
    }

    fn id(raw: u128) -> EntityId {
        EntityId::new(raw)
    }

    fn identity_rescue_contexts() -> (SemanticContext, SemanticContext, SemanticId, SemanticId) {
        let entity_type = SemanticId::new(9700);
        let field = SemanticId::new(9701);
        let mut schema = Schema::new(SchemaRevisionId::new(970));
        schema
            .define_field(kernel_schema::FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let source = SemanticContext {
            schema: schema.clone(),
            environment: SemanticEnvironment::new(SemanticEnvId::new(970)),
        };
        let target = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(971)),
        };
        (source, target, entity_type, field)
    }

    fn identity_rescue_state(entity_type: SemanticId, field: SemanticId) -> DatabaseState {
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([id(1), id(2), id(3)]);
        state.lifecycle.roots.extend([id(1), id(3)]);
        state
            .lifecycle
            .keeps_alive
            .insert(id(1), BTreeSet::from([id(2)]));
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([id(2)]));
        state.model.fields.insert((field, id(2)), Value::I64(42));
        state
    }

    #[test]
    fn commit_normalizes_lifecycle_and_pins_semantics() {
        let context = context();
        let mut state = DatabaseState::default();
        let root = id(1);
        let dead = id(2);
        state.lifecycle.entities = BTreeSet::from([root, dead]);
        state.lifecycle.roots.insert(root);

        let mut store = MemoryStore::default();
        let revision_id = store.bootstrap(&context, state).unwrap();
        let revision = store.revision(revision_id).unwrap();
        assert_eq!(revision.semantic_revision(), context.revision());
        assert_eq!(revision.state().lifecycle.entities, BTreeSet::from([root]));
        assert_eq!(
            store.head().map(kernel_revision::Revision::id),
            Some(revision_id)
        );
    }

    #[test]
    fn diamond_has_unique_merge_base() {
        let context = context();
        let state = DatabaseState::default();
        let mut store = MemoryStore::default();
        let root = store.bootstrap(&context, state.clone()).unwrap();
        let left = store
            .commit_snapshot_from(BTreeSet::from([root]), &context, state.clone())
            .unwrap();
        let right = store
            .commit_snapshot_from(BTreeSet::from([root]), &context, state)
            .unwrap();
        assert_eq!(store.unique_merge_base(left, right), Ok(root));
    }

    #[test]
    fn criss_cross_history_exposes_ambiguous_merge_base_instead_of_guessing() {
        let context = context();
        let state = DatabaseState::default();
        let mut store = MemoryStore::default();
        let root = store.bootstrap(&context, state.clone()).unwrap();
        let a = store
            .commit_snapshot_from(BTreeSet::from([root]), &context, state.clone())
            .unwrap();
        let b = store
            .commit_snapshot_from(BTreeSet::from([root]), &context, state.clone())
            .unwrap();
        let left = store
            .commit_snapshot_from(BTreeSet::from([a, b]), &context, state.clone())
            .unwrap();
        let right = store
            .commit_snapshot_from(BTreeSet::from([a, b]), &context, state)
            .unwrap();

        let expected = BTreeSet::from([a, b]);
        assert_eq!(store.lowest_common_ancestors(left, right), expected);
        assert_eq!(
            store.unique_merge_base(left, right),
            Err(StoreError::AmbiguousMergeBase(expected))
        );
    }

    #[test]
    fn lifecycle_merge_uses_semantic_intents_from_lca_not_pruned_snapshots() {
        let context = context();
        let mut initial = DatabaseState::default();
        initial.lifecycle.entities.extend([id(1), id(2), id(3)]);
        initial.lifecycle.roots.extend([id(1), id(3)]);
        initial
            .lifecycle
            .keeps_alive
            .insert(id(1), BTreeSet::from([id(2)]));

        let mut store = MemoryStore::default();
        let base = store.bootstrap(&context, initial).unwrap();

        let mut left_intent = LifecycleIntent::default();
        left_intent.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );
        let left = store.commit_lifecycle(base, &context, left_intent).unwrap();
        assert!(
            !store
                .revision(left)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(2))
        );

        let mut right_intent = LifecycleIntent::default();
        right_intent.set(
            LifecycleFact::KeepsAlive {
                parent: id(3),
                child: id(2),
            },
            true,
        );
        let right = store
            .commit_lifecycle(base, &context, right_intent)
            .unwrap();

        let merged = store
            .merge_lifecycle_branches(left, right, &context)
            .unwrap();
        assert!(
            store
                .revision(merged)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(2))
        );
    }

    #[test]
    fn lifecycle_rewrite_cannot_silently_cross_semantic_revision() {
        let base_context = context();
        let mut changed_context = base_context.clone();
        changed_context.environment.revision = SemanticEnvId::new(99);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));

        let mut store = MemoryStore::default();
        let base = store.bootstrap(&base_context, state).unwrap();
        let result = store.commit_lifecycle(base, &changed_context, LifecycleIntent::default());
        assert_eq!(
            result,
            Err(StoreError::SemanticTransportRequired {
                from: base_context.revision(),
                to: changed_context.revision(),
            })
        );
    }

    #[test]
    fn snapshot_commit_also_requires_explicit_semantic_transport() {
        let base_context = context();
        let mut changed_context = base_context.clone();
        changed_context.schema.revision = SchemaRevisionId::new(77);
        let state = DatabaseState::default();
        let mut store = MemoryStore::default();
        let base = store.bootstrap(&base_context, state.clone()).unwrap();
        let result = store.commit_snapshot_from(BTreeSet::from([base]), &changed_context, state);
        assert_eq!(
            result,
            Err(StoreError::SemanticTransportRequired {
                from: base_context.revision(),
                to: changed_context.revision(),
            })
        );
    }
    #[test]
    fn commit_rejects_pinned_but_uninstalled_semantic_module_even_when_empty() {
        let relation = SemanticId::new(1000);
        let text_eq = SemanticId::new(1001);
        let mut schema = Schema::new(SchemaRevisionId::new(2));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(4));
        environment.pin_module(text_eq, kernel_schema::ModuleDigest([2; 32]));
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut store = MemoryStore::default();
        assert_eq!(
            store.bootstrap(&context, DatabaseState::default()),
            Err(StoreError::InvalidSemantics(
                SemanticError::ModuleUnavailable(kernel_schema::ModuleDigest([2; 32]))
            ))
        );
    }
    #[test]
    fn identical_revision_numbers_cannot_alias_different_schema_contents() {
        let base_context = context();
        let mut changed_context = context();
        changed_context
            .schema
            .define_relation(RelationDef {
                id: SemanticId::new(2000),
                columns: vec![],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![],
                },
            })
            .unwrap();
        assert_eq!(base_context.revision(), changed_context.revision());

        let state = DatabaseState::default();
        let mut store = MemoryStore::default();
        let base = store.bootstrap(&base_context, state.clone()).unwrap();
        assert_eq!(
            store.commit_snapshot_from(BTreeSet::from([base]), &changed_context, state),
            Err(StoreError::SemanticTransportRequired {
                from: base_context.revision(),
                to: changed_context.revision(),
            })
        );
    }
    #[test]
    fn definitional_transport_is_an_identity_data_rewrite_inside_lca_merge() {
        let source = context();
        let target = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(3)),
            environment: SemanticEnvironment::new(SemanticEnvId::new(5)),
        };
        let registry = SemanticRegistry::default();
        let transport = DefinitionalTransport::verify(&source, &target, &registry).unwrap();
        let mut store = MemoryStore::with_semantic_registry(registry);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([id(1), id(2)]);
        state.lifecycle.roots.insert(id(1));
        state
            .lifecycle
            .keeps_alive
            .insert(id(1), BTreeSet::from([id(2)]));
        let base = store.bootstrap(&source, state).unwrap();

        let mut left_intent = LifecycleIntent::default();
        left_intent.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );
        let left = store.commit_lifecycle(base, &source, left_intent).unwrap();
        assert!(
            !store
                .revision(left)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(2))
        );

        let transported = store
            .commit_definitional_transport(base, &transport)
            .unwrap();
        let mut right_intent = LifecycleIntent::default();
        right_intent.set(LifecycleFact::Root(id(2)), true);
        let right = store
            .commit_lifecycle(transported, &target, right_intent)
            .unwrap();

        let merged = store
            .merge_lifecycle_branches(left, right, &target)
            .unwrap();
        assert!(
            store
                .revision(merged)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(2))
        );
        assert_eq!(store.revision(merged).unwrap().semantic_context(), &target);
    }
    #[test]
    fn typed_field_transport_commits_as_first_class_revision_edge() {
        let entity_type = SemanticId::new(700);
        let source_field = SemanticId::new(701);
        let target_field = SemanticId::new(702);
        let entity = id(70);
        let registry = SemanticRegistry::default();

        let mut source_schema = Schema::new(SchemaRevisionId::new(70));
        source_schema
            .define_field(kernel_schema::FieldDef {
                id: source_field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let source_context = SemanticContext {
            schema: source_schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(70)),
        };
        let mut target_schema = Schema::new(SchemaRevisionId::new(71));
        target_schema
            .define_field(kernel_schema::FieldDef {
                id: target_field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let target_context = SemanticContext {
            schema: target_schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(71)),
        };
        let transport = TypedFieldTransport::verify(
            &source_context,
            &target_context,
            &registry,
            vec![FieldRewrite {
                source_field,
                target_field,
                transform: ExactQuery::new(Expr::AddI64(
                    Box::new(Expr::Input),
                    Box::new(Expr::Const(Value::I64(1))),
                )),
            }],
        )
        .unwrap();
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([entity]));
        state
            .model
            .fields
            .insert((source_field, entity), Value::I64(9));

        let mut store = MemoryStore::with_semantic_registry(registry);
        let base = store.bootstrap(&source_context, state).unwrap();
        let migrated = store
            .commit_typed_field_transport(base, &transport)
            .unwrap();
        let revision = store.revision(migrated).unwrap();
        assert_eq!(revision.semantic_context(), &target_context);
        assert_eq!(
            revision.state().model.fields.get(&(target_field, entity)),
            Some(&Value::I64(10))
        );
        assert_eq!(store.parents(migrated), Some(&BTreeSet::from([base])));
    }

    #[test]
    fn equivalent_semantic_implementation_transport_is_transparent_to_lifecycle_merge() {
        let relation = SemanticId::new(800);
        let equality = SemanticId::new(801);
        let mut registry = SemanticRegistry::default();
        let old = registry
            .install_equivalence_revision(kernel_semantics::EquivalenceModule::TextExact, 1);
        let new = registry
            .install_equivalence_revision(kernel_semantics::EquivalenceModule::TextExact, 2);
        let make_context = |env_revision, digest| {
            let mut schema = Schema::new(SchemaRevisionId::new(80));
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                    semantics: RelationSemantics::Set {
                        column_equivalences: vec![equality],
                    },
                })
                .unwrap();
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
            environment.pin_module(equality, digest);
            SemanticContext {
                schema,
                environment,
            }
        };
        let source = make_context(80, old);
        let target = make_context(81, new);
        let transport =
            EquivalentSemanticEnvironmentTransport::verify(&source, &target, &registry).unwrap();

        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([id(1), id(2)]);
        state.lifecycle.roots.insert(id(1));
        state
            .lifecycle
            .keeps_alive
            .insert(id(1), BTreeSet::from([id(2)]));
        let mut store = MemoryStore::with_semantic_registry(registry);
        let base = store.bootstrap(&source, state).unwrap();

        let mut left_intent = LifecycleIntent::default();
        left_intent.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );
        let left = store.commit_lifecycle(base, &source, left_intent).unwrap();

        let upgraded = store
            .commit_equivalent_semantic_environment_transport(base, &transport)
            .unwrap();
        let mut right_intent = LifecycleIntent::default();
        right_intent.set(LifecycleFact::Root(id(2)), true);
        let right = store
            .commit_lifecycle(upgraded, &target, right_intent)
            .unwrap();

        let merged = store
            .merge_lifecycle_branches(left, right, &target)
            .unwrap();
        assert!(
            store
                .revision(merged)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(2))
        );
    }

    #[test]
    fn genuine_semantic_law_migration_is_not_transparent_to_lifecycle_history() {
        let relation = SemanticId::new(900);
        let equality = SemanticId::new(901);
        let mut registry = SemanticRegistry::default();
        let exact = registry.install_equivalence(kernel_semantics::EquivalenceModule::TextExact);
        let ci = registry
            .install_equivalence(kernel_semantics::EquivalenceModule::TextAsciiCaseInsensitive);
        let make_context = |env_revision, digest| {
            let mut schema = Schema::new(SchemaRevisionId::new(90));
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                    semantics: RelationSemantics::Set {
                        column_equivalences: vec![equality],
                    },
                })
                .unwrap();
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
            environment.pin_module(equality, digest);
            SemanticContext {
                schema,
                environment,
            }
        };
        let exact_context = make_context(90, exact);
        let ci_context = make_context(91, ci);
        let to_ci = SemanticLawMigration::verify(&exact_context, &ci_context, &registry).unwrap();
        let back_to_exact =
            SemanticLawMigration::verify(&ci_context, &exact_context, &registry).unwrap();
        let mut store = MemoryStore::with_semantic_registry(registry);
        let base = store
            .bootstrap(&exact_context, DatabaseState::default())
            .unwrap();
        let left = store
            .commit_lifecycle(base, &exact_context, LifecycleIntent::default())
            .unwrap();
        let changed = store.commit_semantic_law_migration(base, &to_ci).unwrap();
        let right = store
            .commit_semantic_law_migration(changed, &back_to_exact)
            .unwrap();

        assert_eq!(
            store.merge_lifecycle_branches(left, right, &exact_context),
            Err(StoreError::NonIdentityRewriteInLifecyclePath)
        );
    }

    #[test]
    fn conservative_environment_extension_is_transparent_only_toward_extended_context() {
        let extra = SemanticId::new(950);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::TextExact);
        let source = context();
        let mut target = source.clone();
        target.environment.revision = SemanticEnvId::new(950);
        target.environment.pin_module(extra, digest);
        let extension =
            ConservativeSemanticEnvironmentExtension::verify(&source, &target, &registry).unwrap();

        let mut state = DatabaseState::default();
        state.lifecycle.entities.extend([id(1), id(2)]);
        state.lifecycle.roots.insert(id(1));
        state
            .lifecycle
            .keeps_alive
            .insert(id(1), BTreeSet::from([id(2)]));
        let mut store = MemoryStore::with_semantic_registry(registry);
        let base = store.bootstrap(&source, state).unwrap();

        let mut left_intent = LifecycleIntent::default();
        left_intent.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );
        let left = store.commit_lifecycle(base, &source, left_intent).unwrap();

        let extended = store
            .commit_conservative_semantic_environment_extension(base, &extension)
            .unwrap();
        let mut right_intent = LifecycleIntent::default();
        right_intent.set(LifecycleFact::Root(id(2)), true);
        let right = store
            .commit_lifecycle(extended, &target, right_intent)
            .unwrap();

        assert!(store.merge_lifecycle_branches(left, right, &target).is_ok());
        assert_eq!(
            store.merge_lifecycle_branches(left, right, &source),
            Err(StoreError::SemanticTransportRequired {
                from: target.revision(),
                to: source.revision(),
            })
        );
    }

    #[test]
    fn bijective_identity_edge_becomes_mergeable_after_intent_conjugation() {
        let source = context();
        let mut target = source.clone();
        target.environment.revision = SemanticEnvId::new(960);
        let old = BTreeSet::from([id(1)]);
        let new = BTreeSet::from([id(11)]);
        let identity =
            kernel_identity::IdentityTransport::new(&old, &new, BTreeMap::from([(id(1), id(11))]))
                .unwrap();
        let registry = SemanticRegistry::default();
        let transport =
            BijectiveIdentityRevisionTransport::verify(&source, &target, &registry, identity)
                .unwrap();
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        let mut store = MemoryStore::with_semantic_registry(registry);
        let base = store.bootstrap(&source, state).unwrap();
        let left = store
            .commit_lifecycle(base, &source, LifecycleIntent::default())
            .unwrap();
        let right = store
            .commit_bijective_identity_transport(base, &transport)
            .unwrap();

        let merged = store
            .merge_lifecycle_branches(left, right, &target)
            .unwrap();
        assert_eq!(
            store.revision(merged).unwrap().state().lifecycle.roots,
            BTreeSet::from([id(11)])
        );
    }
    #[test]
    fn identity_retention_domain_preserves_gc_pruned_atom_for_cross_branch_rescue() {
        let (source, target, entity_type, field) = identity_rescue_contexts();
        let old = BTreeSet::from([id(1), id(2), id(3)]);
        let new = BTreeSet::from([id(11), id(12), id(13)]);
        let identity = kernel_identity::IdentityTransport::new(
            &old,
            &new,
            BTreeMap::from([(id(1), id(11)), (id(2), id(12)), (id(3), id(13))]),
        )
        .unwrap();
        let registry = SemanticRegistry::default();
        let transport =
            BijectiveIdentityRevisionTransport::verify(&source, &target, &registry, identity)
                .unwrap();

        let state = identity_rescue_state(entity_type, field);
        let mut store = MemoryStore::with_semantic_registry(registry);
        let base = store.bootstrap(&source, state).unwrap();

        let mut left_intent = LifecycleIntent::default();
        left_intent.set(
            LifecycleFact::KeepsAlive {
                parent: id(1),
                child: id(2),
            },
            false,
        );
        let left_pruned = store.commit_lifecycle(base, &source, left_intent).unwrap();
        assert!(
            !store
                .revision(left_pruned)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(2))
        );
        assert!(
            !store
                .revision(left_pruned)
                .unwrap()
                .state()
                .model
                .fields
                .contains_key(&(field, id(2)))
        );
        let left = store
            .commit_bijective_identity_transport(left_pruned, &transport)
            .unwrap();
        assert!(
            !store
                .revision(left)
                .unwrap()
                .state()
                .lifecycle
                .entities
                .contains(&id(12))
        );

        let mut right_intent = LifecycleIntent::default();
        right_intent.set(
            LifecycleFact::KeepsAlive {
                parent: id(3),
                child: id(2),
            },
            true,
        );
        let right = store.commit_lifecycle(base, &source, right_intent).unwrap();

        let merged = store
            .merge_lifecycle_branches(left, right, &target)
            .unwrap();
        let merged = store.revision(merged).unwrap();
        assert!(merged.state().lifecycle.entities.contains(&id(12)));
        assert!(merged.state().model.carriers[&entity_type].contains(&id(12)));
        assert_eq!(
            merged.state().model.fields.get(&(field, id(12))),
            Some(&Value::I64(42))
        );
        assert!(
            merged
                .state()
                .lifecycle
                .keeps_alive
                .get(&id(13))
                .is_some_and(|children| children.contains(&id(12)))
        );
    }
    #[test]
    fn same_semantic_context_requires_explicit_identity_coordinate_choice() {
        let context = context();
        let identity = kernel_identity::IdentityTransport::new(
            &BTreeSet::from([id(1)]),
            &BTreeSet::from([id(11)]),
            BTreeMap::from([(id(1), id(11))]),
        )
        .unwrap();
        let registry = SemanticRegistry::default();
        let transport =
            BijectiveIdentityRevisionTransport::verify(&context, &context, &registry, identity)
                .unwrap();
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        let mut store = MemoryStore::with_semantic_registry(registry);
        let base = store.bootstrap(&context, state).unwrap();
        let left = store
            .commit_lifecycle(base, &context, LifecycleIntent::default())
            .unwrap();
        let right = store
            .commit_bijective_identity_transport(base, &transport)
            .unwrap();

        assert_eq!(
            store.merge_lifecycle_branches(left, right, &context),
            Err(StoreError::AmbiguousIdentityCoordinate)
        );
        let merged = store
            .merge_lifecycle_branches_in_revision_space(left, right, &context, right)
            .unwrap();
        assert_eq!(
            store.revision(merged).unwrap().state().lifecycle.roots,
            BTreeSet::from([id(11)])
        );
    }
}
