use kernel_model::{DatabaseState, Value};
use kernel_semantics::BuiltinSemanticModuleSpec;
use kernel_types::{ClientTransactionId, EntityId, RevisionId, SemanticId, SemanticRevision};

use crate::checkpoint;
use crate::descriptor::DurableMaterializationSpec;
use crate::runtime::CodecError;

/// Canonical digest of passive client-side publication guards (for example
/// `Transaction::require(...)`).  The digest is part of durable retry identity,
/// not of the realized database effect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientIntentGuardDigest(pub [u8; 32]);

impl ClientIntentGuardDigest {
    #[must_use]
    pub fn canonical(bytes: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        Self(Sha256::digest(bytes).into())
    }
}

use super::historical::DurableMigrationComplement;

/// Durable namespace for exact client retry identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdempotencyEpoch(u64);

impl IdempotencyEpoch {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Exact retry key. Raw client transaction ids may be reused only in a later
/// explicit idempotency epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DurableTransactionKey {
    pub epoch: IdempotencyEpoch,
    pub transaction_id: ClientTransactionId,
}

impl DurableTransactionKey {
    #[must_use]
    pub const fn new(epoch: IdempotencyEpoch, transaction_id: ClientTransactionId) -> Self {
        Self {
            epoch,
            transaction_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableEffectKind {
    RelationData,
    RelationRewrite,
    RelationResolution,
    MixedRevision,
    FullRevision,
    SchemaMigration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableCarrierPatch {
    pub carrier: SemanticId,
    pub target_present: bool,
    pub inserted: Vec<EntityId>,
    pub removed: Vec<EntityId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableFieldPatch {
    pub field: SemanticId,
    pub owner: EntityId,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableKeepsAlivePatch {
    pub parent: EntityId,
    pub target_present: bool,
    pub inserted: Vec<EntityId>,
    pub removed: Vec<EntityId>,
}

/// Canonical source-relative patch for the non-relation portion of one
/// `DatabaseState`.  Relation rows remain represented by
/// `DurableRelationMutation`; this patch covers exactly the orthogonal carrier,
/// field and lifecycle authorities.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DurableModelDelta {
    pub carriers: Vec<DurableCarrierPatch>,
    pub fields: Vec<DurableFieldPatch>,
    pub lifecycle_entities_inserted: Vec<EntityId>,
    pub lifecycle_entities_removed: Vec<EntityId>,
    pub lifecycle_roots_inserted: Vec<EntityId>,
    pub lifecycle_roots_removed: Vec<EntityId>,
    pub lifecycle_keeps_alive: Vec<DurableKeepsAlivePatch>,
}

impl DurableModelDelta {
    #[must_use]
    #[allow(clippy::too_many_lines)] // One extensional diff pass; splitting would duplicate state-key traversal.
    pub fn between(source: &DatabaseState, target: &DatabaseState) -> Self {
        use std::collections::BTreeSet;

        let carrier_keys = source
            .model
            .carriers
            .keys()
            .chain(target.model.carriers.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut carriers = Vec::new();
        for carrier in carrier_keys {
            let source_set = source.model.carriers.get(&carrier);
            let target_set = target.model.carriers.get(&carrier);
            if source_set == target_set {
                continue;
            }
            let empty = BTreeSet::new();
            let source_values = source_set.unwrap_or(&empty);
            let target_values = target_set.unwrap_or(&empty);
            carriers.push(DurableCarrierPatch {
                carrier,
                target_present: target_set.is_some(),
                inserted: target_values.difference(source_values).copied().collect(),
                removed: source_values.difference(target_values).copied().collect(),
            });
        }

        let field_keys = source
            .model
            .fields
            .keys()
            .chain(target.model.fields.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut fields = Vec::new();
        for (field, owner) in field_keys {
            if source.model.fields.get(&(field, owner)) == target.model.fields.get(&(field, owner))
            {
                continue;
            }
            fields.push(DurableFieldPatch {
                field,
                owner,
                value: target.model.fields.get(&(field, owner)).cloned(),
            });
        }

        let lifecycle_entities_inserted = target
            .lifecycle
            .entities
            .difference(&source.lifecycle.entities)
            .copied()
            .collect();
        let lifecycle_entities_removed = source
            .lifecycle
            .entities
            .difference(&target.lifecycle.entities)
            .copied()
            .collect();
        let lifecycle_roots_inserted = target
            .lifecycle
            .roots
            .difference(&source.lifecycle.roots)
            .copied()
            .collect();
        let lifecycle_roots_removed = source
            .lifecycle
            .roots
            .difference(&target.lifecycle.roots)
            .copied()
            .collect();

        let keeps_keys = source
            .lifecycle
            .keeps_alive
            .keys()
            .chain(target.lifecycle.keeps_alive.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        let mut lifecycle_keeps_alive = Vec::new();
        for parent in keeps_keys {
            let source_set = source.lifecycle.keeps_alive.get(&parent);
            let target_set = target.lifecycle.keeps_alive.get(&parent);
            if source_set == target_set {
                continue;
            }
            let empty = BTreeSet::new();
            let source_values = source_set.unwrap_or(&empty);
            let target_values = target_set.unwrap_or(&empty);
            lifecycle_keeps_alive.push(DurableKeepsAlivePatch {
                parent,
                target_present: target_set.is_some(),
                inserted: target_values.difference(source_values).copied().collect(),
                removed: source_values.difference(target_values).copied().collect(),
            });
        }

        Self {
            carriers,
            fields,
            lifecycle_entities_inserted,
            lifecycle_entities_removed,
            lifecycle_roots_inserted,
            lifecycle_roots_removed,
            lifecycle_keeps_alive,
        }
    }

    pub fn apply_to(&self, state: &mut DatabaseState) {
        for patch in &self.carriers {
            let carrier = state.model.carriers.entry(patch.carrier).or_default();
            for entity in &patch.removed {
                carrier.remove(entity);
            }
            carrier.extend(patch.inserted.iter().copied());
            if !patch.target_present {
                state.model.carriers.remove(&patch.carrier);
            }
        }
        for patch in &self.fields {
            match &patch.value {
                Some(value) => {
                    state
                        .model
                        .fields
                        .insert((patch.field, patch.owner), value.clone());
                }
                None => {
                    state.model.fields.remove(&(patch.field, patch.owner));
                }
            }
        }
        for entity in &self.lifecycle_entities_removed {
            state.lifecycle.entities.remove(entity);
        }
        state
            .lifecycle
            .entities
            .extend(self.lifecycle_entities_inserted.iter().copied());
        for entity in &self.lifecycle_roots_removed {
            state.lifecycle.roots.remove(entity);
        }
        state
            .lifecycle
            .roots
            .extend(self.lifecycle_roots_inserted.iter().copied());
        for patch in &self.lifecycle_keeps_alive {
            let children = state.lifecycle.keeps_alive.entry(patch.parent).or_default();
            for child in &patch.removed {
                children.remove(child);
            }
            children.extend(patch.inserted.iter().copied());
            if !patch.target_present {
                state.lifecycle.keeps_alive.remove(&patch.parent);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableObjectFieldWrite {
    pub owner: EntityId,
    pub field: SemanticId,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "These flags record independent transaction validation facts."
)]
pub struct DurableRelationAuthorization {
    pub relation_write: bool,
    pub object_create: bool,
    pub object_delete: bool,
    pub relationship_attach: bool,
    pub relationship_detach: bool,
    pub relationship_move: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRelationMutation {
    pub relation: SemanticId,
    pub inserted: Vec<Vec<Value>>,
    pub removed: Vec<Vec<Value>>,
    pub object_field_writes: Vec<DurableObjectFieldWrite>,
    pub authorization: DurableRelationAuthorization,
}

/// Durable identity of one semantic Rewrite family attached to one relation
/// mutation. Explicit user inputs/effects are not duplicated here: the exact
/// typed relation delta is persisted once in the same intent, while these IDs
/// preserve future merge/rebase meaning that endpoint equality cannot recover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DurableRelationRewriteIntent {
    pub relation: SemanticId,
    pub rewrite_spec: SemanticId,
    pub law_set: SemanticId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableRelationResolution {
    pub relation_mutations: Vec<DurableRelationMutation>,
    pub rewrite_intents: Vec<DurableRelationRewriteIntent>,
    pub causal_parents: Vec<RevisionId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations."
)]
pub enum DurableRevisionChange {
    RelationData {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
    },
    MixedRevision {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        model_delta: DurableModelDelta,
        /// Exact inverse/recovery material for the realized model-side effect.
        /// This belongs to publication/recovery authority, never client identity.
        model_complement: Option<Box<DurableModelDelta>>,
    },
    FullRevision {
        encoded_target_revision: Vec<u8>,
    },
    FullRevisionAndMaterializations {
        encoded_target_revision: Vec<u8>,
        materializations: Vec<DurableMaterializationSpec>,
    },
    SchemaMigration {
        program: kernel_transport::SchemaMigrationProgram,
    },
}

/// Exact client-visible intent retained across WAL, checkpoint rotation and
/// restart. Relation-data transactions retain their immutable source/target
/// lineage plus the exact typed delta instead of duplicating the complete target
/// Revision. Full-revision replacements still retain canonical target bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations."
)]
pub enum DurableTransactionIntent {
    RelationData {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        client_guard_digest: Option<ClientIntentGuardDigest>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    RelationRewrite {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    RelationResolution {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        causal_parents: Vec<RevisionId>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    MixedRevision {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        model_delta: DurableModelDelta,
        client_guard_digest: Option<ClientIntentGuardDigest>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    FullRevision {
        target_revision: RevisionId,
        encoded_target_revision: Vec<u8>,
        materializations: Option<Vec<DurableMaterializationSpec>>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    SchemaMigration {
        source_revision: RevisionId,
        target_revision: RevisionId,
        program: kernel_transport::SchemaMigrationProgram,
        migration_complement: DurableMigrationComplement,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
}

/// Canonical retry identity projected from a durable transaction record.
///
/// Realized publication state, recovery complements, formation/target revision
/// ids and executable deployment artifacts are intentionally absent.  They are
/// proof/recovery authorities, not part of the client request identity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations."
)]
pub enum DurableClientIntent {
    RelationData {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        guard_digest: Option<ClientIntentGuardDigest>,
    },
    MixedRevision {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        model_delta: DurableModelDelta,
        guard_digest: Option<ClientIntentGuardDigest>,
    },
    RelationRewrite {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
    },
    RelationResolution {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        causal_parents: Vec<RevisionId>,
    },
    FullRevision {
        encoded_target_revision: Vec<u8>,
        materializations: Option<Vec<DurableMaterializationSpec>>,
    },
    SchemaMigration {
        program: kernel_transport::SchemaMigrationProgram,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableCommittedTransaction {
    pub target_revision: RevisionId,
    pub intent: DurableClientIntent,
}

impl DurableCommittedTransaction {
    #[must_use]
    pub fn from_descriptor_intent(
        target_revision: RevisionId,
        intent: &DurableTransactionIntent,
    ) -> Self {
        Self {
            target_revision,
            intent: intent.client_intent_owned(),
        }
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub fn same_client_intent(&self, requested: &DurableTransactionIntent) -> bool {
        self.intent == requested.client_intent_owned()
    }

    #[must_use]
    pub fn matches_mixed_client_intent(
        &self,
        semantic_revision: SemanticRevision,
        relation_mutations: &[DurableRelationMutation],
        model_delta: &DurableModelDelta,
        guard_digest: Option<ClientIntentGuardDigest>,
    ) -> bool {
        self.intent
            == DurableClientIntent::MixedRevision {
                semantic_revision,
                relation_mutations: relation_mutations.to_vec(),
                model_delta: model_delta.clone(),
                guard_digest,
            }
    }

    #[must_use]
    pub const fn client_guard_digest(&self) -> Option<ClientIntentGuardDigest> {
        match &self.intent {
            DurableClientIntent::RelationData { guard_digest, .. }
            | DurableClientIntent::MixedRevision { guard_digest, .. } => *guard_digest,
            _ => None,
        }
    }
}

impl DurableTransactionIntent {
    /// Returns whether two durable realizations carry the same client-visible
    /// semantic intent even when they were certified against different live
    /// revisions.
    ///
    /// Source/target revision ids and source-relative recovery complements are
    /// realization metadata, not the logical request identity.  Relation and
    /// mixed-data intents therefore compare their canonical forward effect and
    /// pinned semantic authority.  Full replacements, schema migrations and
    /// causal resolutions remain exact because their endpoint/parent identity
    /// is part of the requested operation itself.
    #[must_use]
    pub fn client_intent_owned(&self) -> DurableClientIntent {
        match self {
            Self::RelationData {
                semantic_revision,
                relation_mutations,
                client_guard_digest,
                ..
            } => DurableClientIntent::RelationData {
                semantic_revision: *semantic_revision,
                relation_mutations: relation_mutations.clone(),
                guard_digest: *client_guard_digest,
            },
            Self::MixedRevision {
                semantic_revision,
                relation_mutations,
                model_delta,
                client_guard_digest,
                ..
            } => DurableClientIntent::MixedRevision {
                semantic_revision: *semantic_revision,
                relation_mutations: relation_mutations.clone(),
                model_delta: model_delta.clone(),
                guard_digest: *client_guard_digest,
            },
            Self::RelationRewrite {
                semantic_revision,
                relation_mutations,
                rewrite_intents,
                ..
            } => DurableClientIntent::RelationRewrite {
                semantic_revision: *semantic_revision,
                relation_mutations: relation_mutations.clone(),
                rewrite_intents: rewrite_intents.clone(),
            },
            Self::RelationResolution {
                semantic_revision,
                relation_mutations,
                rewrite_intents,
                causal_parents,
                ..
            } => DurableClientIntent::RelationResolution {
                semantic_revision: *semantic_revision,
                relation_mutations: relation_mutations.clone(),
                rewrite_intents: rewrite_intents.clone(),
                causal_parents: causal_parents.clone(),
            },
            Self::FullRevision {
                encoded_target_revision,
                materializations,
                ..
            } => DurableClientIntent::FullRevision {
                encoded_target_revision: encoded_target_revision.clone(),
                materializations: materializations.clone(),
            },
            Self::SchemaMigration { program, .. } => DurableClientIntent::SchemaMigration {
                program: program.clone(),
            },
        }
    }

    #[must_use]
    pub fn same_client_intent(&self, other: &Self) -> bool {
        self.client_intent_owned() == other.client_intent_owned()
    }

    #[must_use]
    pub fn matches_mixed_client_intent(
        &self,
        semantic_revision: SemanticRevision,
        relation_mutations: &[DurableRelationMutation],
        model_delta: &DurableModelDelta,
        guard_digest: Option<ClientIntentGuardDigest>,
    ) -> bool {
        self.client_intent_owned()
            == DurableClientIntent::MixedRevision {
                semantic_revision,
                relation_mutations: relation_mutations.to_vec(),
                model_delta: model_delta.clone(),
                guard_digest,
            }
    }

    #[must_use]
    pub const fn client_guard_digest(&self) -> Option<ClientIntentGuardDigest> {
        match self {
            Self::RelationData {
                client_guard_digest,
                ..
            }
            | Self::MixedRevision {
                client_guard_digest,
                ..
            } => *client_guard_digest,
            _ => None,
        }
    }

    #[must_use]
    pub fn with_client_guard_digest(mut self, digest: Option<ClientIntentGuardDigest>) -> Self {
        match &mut self {
            Self::RelationData {
                client_guard_digest,
                ..
            }
            | Self::MixedRevision {
                client_guard_digest,
                ..
            } => {
                *client_guard_digest = digest;
            }
            _ => debug_assert!(digest.is_none()),
        }
        self
    }

    #[must_use]
    pub const fn effect_kind(&self) -> DurableEffectKind {
        match self {
            Self::RelationData { .. } => DurableEffectKind::RelationData,
            Self::RelationRewrite { .. } => DurableEffectKind::RelationRewrite,
            Self::RelationResolution { .. } => DurableEffectKind::RelationResolution,
            Self::MixedRevision { .. } => DurableEffectKind::MixedRevision,
            Self::FullRevision { .. } => DurableEffectKind::FullRevision,
            Self::SchemaMigration { .. } => DurableEffectKind::SchemaMigration,
        }
    }

    pub fn relation_data(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        mut relation_mutations: Vec<DurableRelationMutation>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        relation_mutations.sort_by_key(|mutation| mutation.relation);
        if relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::RelationData {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations,
            client_guard_digest: None,
            semantic_modules,
        })
    }

    pub fn relation_data_residual(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        client_semantic_revision: SemanticRevision,
        mut client_mutations: Vec<DurableRelationMutation>,
        mut realized_mutations: Vec<DurableRelationMutation>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        client_mutations.sort_by_key(|mutation| mutation.relation);
        realized_mutations.sort_by_key(|mutation| mutation.relation);
        if client_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
            || realized_mutations
                .windows(2)
                .any(|pair| pair[0].relation == pair[1].relation)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::RelationData {
            source_revision,
            target_revision: target.id(),
            semantic_revision: client_semantic_revision,
            relation_mutations: client_mutations,
            client_guard_digest: None,
            semantic_modules,
        })
    }

    pub fn relation_rewrites(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        mut relation_mutations: Vec<DurableRelationMutation>,
        mut rewrite_intents: Vec<DurableRelationRewriteIntent>,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        relation_mutations.sort_by_key(|mutation| mutation.relation);
        rewrite_intents.sort_by_key(|intent| intent.relation);
        if relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
            || rewrite_intents
                .windows(2)
                .any(|pair| pair[0].relation == pair[1].relation)
            || relation_mutations.len() != rewrite_intents.len()
            || relation_mutations
                .iter()
                .zip(&rewrite_intents)
                .any(|(mutation, intent)| mutation.relation != intent.relation)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::RelationRewrite {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            semantic_modules,
        })
    }

    pub fn relation_resolution(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        mut resolution: DurableRelationResolution,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        resolution
            .relation_mutations
            .sort_by_key(|mutation| mutation.relation);
        resolution
            .rewrite_intents
            .sort_by_key(|intent| intent.relation);
        resolution.causal_parents.sort();
        if resolution
            .relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
            || resolution
                .rewrite_intents
                .windows(2)
                .any(|pair| pair[0].relation == pair[1].relation)
            || resolution.relation_mutations.len() != resolution.rewrite_intents.len()
            || resolution
                .relation_mutations
                .iter()
                .zip(&resolution.rewrite_intents)
                .any(|(mutation, intent)| mutation.relation != intent.relation)
            || resolution.causal_parents.len() < 2
            || resolution
                .causal_parents
                .windows(2)
                .any(|pair| pair[0] == pair[1])
            || resolution
                .causal_parents
                .binary_search(&source_revision)
                .is_err()
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::RelationResolution {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations: resolution.relation_mutations,
            rewrite_intents: resolution.rewrite_intents,
            causal_parents: resolution.causal_parents,
            semantic_modules,
        })
    }

    pub fn mixed_revision(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        semantic_revision: SemanticRevision,
        mut relation_mutations: Vec<DurableRelationMutation>,
        model_delta: DurableModelDelta,
        _model_complement: DurableModelDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        relation_mutations.sort_by_key(|mutation| mutation.relation);
        if relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::MixedRevision {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations,
            model_delta,
            client_guard_digest: None,
            semantic_modules,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn mixed_revision_residual(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        client_semantic_revision: SemanticRevision,
        mut client_relation_mutations: Vec<DurableRelationMutation>,
        client_model_delta: DurableModelDelta,
        mut realized_relation_mutations: Vec<DurableRelationMutation>,
        _realized_model_delta: DurableModelDelta,
        _realized_model_complement: DurableModelDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        client_relation_mutations.sort_by_key(|mutation| mutation.relation);
        realized_relation_mutations.sort_by_key(|mutation| mutation.relation);
        if client_relation_mutations
            .windows(2)
            .any(|pair| pair[0].relation == pair[1].relation)
            || realized_relation_mutations
                .windows(2)
                .any(|pair| pair[0].relation == pair[1].relation)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::MixedRevision {
            source_revision,
            target_revision: target.id(),
            semantic_revision: client_semantic_revision,
            relation_mutations: client_relation_mutations,
            model_delta: client_model_delta,
            client_guard_digest: None,
            semantic_modules,
        })
    }

    pub fn revision(
        target: &kernel_revision::Revision,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::FullRevision {
            target_revision: target.id(),
            encoded_target_revision: checkpoint::encode_revision(target)?,
            materializations: None,
            semantic_modules,
        })
    }

    pub fn revision_and_materializations(
        target: &kernel_revision::Revision,
        materializations: &[DurableMaterializationSpec],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        let mut materializations = materializations.to_vec();
        materializations.sort_by_key(|spec| spec.id);
        if materializations
            .windows(2)
            .any(|pair| pair[0].id == pair[1].id)
        {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::FullRevision {
            target_revision: target.id(),
            encoded_target_revision: checkpoint::encode_revision(target)?,
            materializations: Some(materializations),
            semantic_modules,
        })
    }

    pub fn schema_migration(
        source_revision: RevisionId,
        target_revision: RevisionId,
        program: kernel_transport::SchemaMigrationProgram,
        migration_complement: DurableMigrationComplement,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        migration_complement
            .validate()
            .map_err(|_| CodecError::CollectionTooLarge)?;
        if migration_complement.target_schema != program.target().schema.revision {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(program.target())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::SchemaMigration {
            source_revision,
            target_revision,
            program,
            migration_complement,
            semantic_modules,
        })
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        match self {
            Self::RelationData {
                target_revision, ..
            }
            | Self::RelationRewrite {
                target_revision, ..
            }
            | Self::RelationResolution {
                target_revision, ..
            }
            | Self::MixedRevision {
                target_revision, ..
            }
            | Self::FullRevision {
                target_revision, ..
            }
            | Self::SchemaMigration {
                target_revision, ..
            } => *target_revision,
        }
    }

    #[must_use]
    pub const fn is_exact(&self) -> bool {
        matches!(
            self,
            Self::RelationData { .. }
                | Self::RelationRewrite { .. }
                | Self::RelationResolution { .. }
                | Self::MixedRevision { .. }
                | Self::FullRevision { .. }
                | Self::SchemaMigration { .. }
        )
    }

    #[must_use]
    pub const fn source_revision(&self) -> Option<RevisionId> {
        match self {
            Self::RelationData {
                source_revision, ..
            }
            | Self::RelationRewrite {
                source_revision, ..
            }
            | Self::RelationResolution {
                source_revision, ..
            }
            | Self::SchemaMigration {
                source_revision, ..
            }
            | Self::MixedRevision {
                source_revision, ..
            } => Some(*source_revision),
            Self::FullRevision { .. } => None,
        }
    }
}
