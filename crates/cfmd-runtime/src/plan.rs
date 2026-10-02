use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

use crate::{FieldId, RelationId, RevisionId, Row, TypeId};

#[derive(Debug, Clone)]
pub struct Plan {
    pub(crate) database_identity: u64,
    pub(crate) runtime: Weak<kernel_plan::DurableRuntime>,
    pub(crate) registry: kernel_semantics::SemanticRegistry,
    pub(crate) source: kernel_plan::RuntimeRevisionSnapshot,
    pub(crate) base_revision: RevisionId,
    pub(crate) mutations: BTreeMap<RelationId, PendingRelationMutation>,
    pub(crate) object_field_patches: BTreeMap<(RelationId, u128), PendingObjectFieldPatch>,
    pub(crate) object_contracts: BTreeMap<RelationId, ObjectContract>,
    pub(crate) owned_relations: BTreeMap<RelationId, OwnedRelationContract>,
    pub(crate) model_delta: Option<kernel_plan::DurableModelDelta>,
    pub(crate) mutation_actions: BTreeMap<(RelationId, MutationDirection, usize), MutationAction>,
    pub(crate) history_authorization: BTreeMap<RelationId, HistoryAuthorizationCoverage>,
    pub(crate) authority: crate::security::RuntimeAuthority,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReferenceContract {
    pub(crate) column: usize,
    pub(crate) field: FieldId,
    pub(crate) target_type: TypeId,
    pub(crate) target_relation: RelationId,
    pub(crate) target_identity_column: usize,
    pub(crate) optional: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanPolicy {
    Keep,
    DeleteIfUnowned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedRelationContract {
    pub(crate) relation: RelationId,
    pub(crate) target_relation: RelationId,
    pub(crate) target_identity_column: usize,
    pub(crate) orphan_policy: OrphanPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectContract {
    pub(crate) relation: RelationId,
    pub(crate) entity_type: TypeId,
    pub(crate) identity_column: usize,
    pub(crate) identity_type: TypeId,
    pub(crate) references: Vec<ReferenceContract>,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MutationDirection {
    Insert,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum MutationAction {
    ObjectCreate,
    ObjectDelete,
    RelationshipAttach,
    RelationshipDetach,
    RelationshipMove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HistoryAuthorizationCoverage {
    pub(crate) inserted_prefix: usize,
    pub(crate) removed_prefix: usize,
    pub(crate) authorization: kernel_durability::DurableRelationAuthorization,
    pub(crate) fields: Vec<crate::RelationColumnId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PendingRelationMutation {
    pub(crate) inserted: Vec<Row>,
    pub(crate) removed: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PendingObjectFieldPatch {
    pub(crate) identity_column: usize,
    pub(crate) identity_value: crate::Value,
    pub(crate) owner: kernel_types::EntityId,
    pub(crate) fields: BTreeMap<usize, (crate::Value, kernel_types::SemanticId)>,
}

impl Plan {
    pub(crate) fn new(
        runtime: &Arc<kernel_plan::DurableRuntime>,
        source: kernel_plan::RuntimeRevisionSnapshot,
        database_identity: u64,
        authority: crate::security::RuntimeAuthority,
    ) -> Self {
        let base_revision = source.revision().id().into();
        Self {
            database_identity,
            registry: runtime.semantic_registry().clone(),
            runtime: Arc::downgrade(runtime),
            source,
            base_revision,
            mutations: BTreeMap::new(),
            object_field_patches: BTreeMap::new(),
            object_contracts: BTreeMap::new(),
            owned_relations: BTreeMap::new(),
            model_delta: None,
            mutation_actions: BTreeMap::new(),
            history_authorization: BTreeMap::new(),
            authority,
        }
    }

    #[must_use]
    pub const fn base_revision(&self) -> RevisionId {
        self.base_revision
    }

    pub fn insert(&mut self, relation: RelationId, row: Row) -> &mut Self {
        self.mutations.entry(relation).or_default().inserted.push(row);
        self
    }

    pub fn remove(&mut self, relation: RelationId, row: Row) -> &mut Self {
        self.mutations.entry(relation).or_default().removed.push(row);
        self
    }

    pub(crate) fn insert_semantic(
        &mut self,
        relation: RelationId,
        row: Row,
        action: MutationAction,
    ) -> &mut Self {
        let mutation = self.mutations.entry(relation).or_default();
        let index = mutation.inserted.len();
        mutation.inserted.push(row);
        self.mutation_actions
            .insert((relation, MutationDirection::Insert, index), action);
        self
    }

    pub(crate) fn remove_semantic(
        &mut self,
        relation: RelationId,
        row: Row,
        action: MutationAction,
    ) -> &mut Self {
        let mutation = self.mutations.entry(relation).or_default();
        let index = mutation.removed.len();
        mutation.removed.push(row);
        self.mutation_actions
            .insert((relation, MutationDirection::Remove, index), action);
        self
    }

    pub(crate) fn register_history_authorization(
        &mut self,
        relation: RelationId,
        authorization: kernel_durability::DurableRelationAuthorization,
        mut fields: Vec<crate::RelationColumnId>,
    ) {
        fields.sort_unstable();
        fields.dedup();
        let mutation = self.mutations.get(&relation).cloned().unwrap_or_default();
        self.history_authorization.insert(
            relation,
            HistoryAuthorizationCoverage {
                inserted_prefix: mutation.inserted.len(),
                removed_prefix: mutation.removed.len(),
                authorization,
                fields,
            },
        );
    }

    pub fn insert_typed<R, T: crate::RowCodec>(
        &mut self,
        relation: &crate::Relation<R>,
        row: T,
    ) -> crate::Result<&mut Self> {
        let row = relation.encode_row(row)?;
        Ok(self.insert(relation.id(), row))
    }

    pub fn remove_typed<R, T: crate::RowCodec>(
        &mut self,
        relation: &crate::Relation<R>,
        row: T,
    ) -> crate::Result<&mut Self> {
        let row = relation.encode_row(row)?;
        Ok(self.remove(relation.id(), row))
    }

    pub(crate) fn patch_object_field(
        &mut self,
        relation: RelationId,
        identity_raw: u128,
        identity_column: usize,
        identity_value: crate::Value,
        target_column: usize,
        value: crate::Value,
        owner: kernel_types::EntityId,
        field: kernel_types::SemanticId,
    ) -> crate::Result<()> {
        let patch = self
            .object_field_patches
            .entry((relation, identity_raw))
            .or_insert_with(|| PendingObjectFieldPatch {
                identity_column,
                identity_value: identity_value.clone(),
                owner,
                fields: BTreeMap::new(),
            });
        if patch.identity_column != identity_column || patch.identity_value != identity_value || patch.owner != owner {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "object field patches disagree on semantic identity",
            ));
        }
        patch.fields.insert(target_column, (value, field));
        Ok(())
    }

    pub(crate) fn register_object_contract(&mut self, contract: ObjectContract) {
        self.object_contracts.insert(contract.relation, contract);
    }

    pub(crate) fn register_owned_relation(&mut self, contract: OwnedRelationContract) {
        self.owned_relations.insert(contract.relation, contract);
    }

    pub(crate) fn patch_model_field(
        &mut self,
        field: kernel_types::SemanticId,
        owner: kernel_types::EntityId,
        value: Option<kernel_model::Value>,
    ) -> crate::Result<()> {
        let delta = self.model_delta.get_or_insert_with(Default::default);
        if !delta.carriers.is_empty()
            || !delta.lifecycle_entities_inserted.is_empty()
            || !delta.lifecycle_entities_removed.is_empty()
            || !delta.lifecycle_roots_inserted.is_empty()
            || !delta.lifecycle_roots_removed.is_empty()
            || !delta.lifecycle_keeps_alive.is_empty()
        {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "cannot compose a Context reference patch with a non-field explicit model transition",
            ));
        }
        if let Some(existing) = delta
            .fields
            .iter_mut()
            .find(|patch| patch.field == field && patch.owner == owner)
        {
            if existing.value != value {
                existing.value = value;
            }
            return Ok(());
        }
        delta.fields.push(kernel_durability::DurableFieldPatch { field, owner, value });
        delta.fields.sort_by_key(|patch| (patch.field, patch.owner));
        Ok(())
    }

    /// Adds another proposed transition built from the exact same database snapshot.
    ///
    /// Composition is structural: no hidden retry, rebase, or read of a newer HEAD occurs.
    /// A plan from any other snapshot fails closed.
    pub fn extend(&mut self, other: Self) -> crate::Result<&mut Self> {
        if self.database_identity != other.database_identity
            || self.source != other.source
            || self.authority != other.authority
        {
            return Err(crate::Error::new(
                crate::ErrorKind::InvalidPlan,
                "cannot compose plans from different database snapshots",
            ));
        }
        match (self.model_delta.as_mut(), other.model_delta) {
            (None, model_delta) => self.model_delta = model_delta,
            (Some(existing), Some(other)) if existing != &other => {
                let fields_only = |delta: &kernel_plan::DurableModelDelta| {
                    delta.carriers.is_empty()
                        && delta.lifecycle_entities_inserted.is_empty()
                        && delta.lifecycle_entities_removed.is_empty()
                        && delta.lifecycle_roots_inserted.is_empty()
                        && delta.lifecycle_roots_removed.is_empty()
                        && delta.lifecycle_keeps_alive.is_empty()
                };
                if !fields_only(existing) || !fields_only(&other) {
                    return Err(crate::Error::new(
                        crate::ErrorKind::InvalidPlan,
                        "cannot compose plans with distinct explicit model transitions",
                    ));
                }
                for patch in other.fields {
                    if let Some(current) = existing
                        .fields
                        .iter_mut()
                        .find(|current| current.field == patch.field && current.owner == patch.owner)
                    {
                        current.value = patch.value;
                    } else {
                        existing.fields.push(patch);
                    }
                }
                existing.fields.sort_by_key(|patch| (patch.field, patch.owner));
            }
            _ => {}
        }
        for (relation, contract) in other.object_contracts {
            self.object_contracts.insert(relation, contract);
        }
        for (relation, contract) in other.owned_relations {
            self.owned_relations.insert(relation, contract);
        }
        for (key, patch) in other.object_field_patches {
            let target = self.object_field_patches.entry(key).or_insert_with(|| PendingObjectFieldPatch {
                identity_column: patch.identity_column,
                identity_value: patch.identity_value.clone(),
                owner: patch.owner,
                fields: BTreeMap::new(),
            });
            if target.identity_column != patch.identity_column
                || target.identity_value != patch.identity_value
                || target.owner != patch.owner
            {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidPlan,
                    "cannot compose object field patches with inconsistent identity coordinates",
                ));
            }
            target.fields.extend(patch.fields);
        }
        let other_actions = other.mutation_actions;
        let other_history_authorization = other.history_authorization;
        for (relation, mutation) in other.mutations {
            if let Some(coverage) = other_history_authorization.get(&relation) {
                let existing = self.mutations.get(&relation);
                if existing.is_some_and(|mutation| !mutation.inserted.is_empty() || !mutation.removed.is_empty()) {
                    return Err(crate::Error::new(
                        crate::ErrorKind::InvalidPlan,
                        "cannot compose history-authorized inverse after existing mutations of the same relation",
                    ));
                }
                self.history_authorization.insert(relation, coverage.clone());
            }
            let target = self.mutations.entry(relation).or_default();
            let inserted_offset = target.inserted.len();
            let removed_offset = target.removed.len();
            target.inserted.extend(mutation.inserted);
            target.removed.extend(mutation.removed);
            for ((candidate, direction, index), action) in other_actions
                .iter()
                .filter(|((candidate, _, _), _)| *candidate == relation)
            {
                let adjusted = match direction {
                    MutationDirection::Insert => inserted_offset + *index,
                    MutationDirection::Remove => removed_offset + *index,
                };
                self.mutation_actions
                    .insert((*candidate, *direction, adjusted), *action);
            }
        }
        Ok(self)
    }

    /// Composes two proposed transitions that were built from the exact same database snapshot.
    pub fn and(mut self, other: Self) -> crate::Result<Self> {
        self.extend(other)?;
        Ok(self)
    }

    pub fn candidate(&self) -> crate::Result<crate::Candidate> {
        crate::Candidate::from_plan(self)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.model_delta.is_none()
            && self.object_field_patches.is_empty()
            && self
                .mutations
                .values()
                .all(|mutation| mutation.inserted.is_empty() && mutation.removed.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    Committed { revision: RevisionId },
    AlreadyCommitted { revision: RevisionId },
}
