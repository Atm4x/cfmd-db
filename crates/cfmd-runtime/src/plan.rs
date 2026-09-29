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
    pub(crate) object_contracts: BTreeMap<RelationId, ObjectContract>,
    pub(crate) owned_relations: BTreeMap<RelationId, OwnedRelationContract>,
    pub(crate) model_delta: Option<kernel_plan::DurableModelDelta>,
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct PendingRelationMutation {
    pub(crate) inserted: Vec<Row>,
    pub(crate) removed: Vec<Row>,
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
            object_contracts: BTreeMap::new(),
            owned_relations: BTreeMap::new(),
            model_delta: None,
            authority,
        }
    }

    #[must_use]
    pub const fn base_revision(&self) -> RevisionId {
        self.base_revision
    }

    pub fn insert(&mut self, relation: RelationId, row: Row) -> &mut Self {
        self.mutations
            .entry(relation)
            .or_default()
            .inserted
            .push(row);
        self
    }

    pub fn remove(&mut self, relation: RelationId, row: Row) -> &mut Self {
        self.mutations
            .entry(relation)
            .or_default()
            .removed
            .push(row);
        self
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

    pub(crate) fn register_object_contract(&mut self, contract: ObjectContract) {
        self.object_contracts.insert(contract.relation, contract);
    }

    pub(crate) fn register_owned_relation(&mut self, contract: OwnedRelationContract) {
        self.owned_relations.insert(contract.relation, contract);
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
        match (&self.model_delta, other.model_delta) {
            (None, model_delta) => self.model_delta = model_delta,
            (Some(existing), Some(other)) if existing != &other => {
                return Err(crate::Error::new(
                    crate::ErrorKind::InvalidPlan,
                    "cannot compose plans with distinct explicit model transitions",
                ));
            }
            _ => {}
        }
        for (relation, contract) in other.object_contracts {
            self.object_contracts.insert(relation, contract);
        }
        for (relation, contract) in other.owned_relations {
            self.owned_relations.insert(relation, contract);
        }
        for (relation, mutation) in other.mutations {
            let target = self.mutations.entry(relation).or_default();
            target.inserted.extend(mutation.inserted);
            target.removed.extend(mutation.removed);
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
