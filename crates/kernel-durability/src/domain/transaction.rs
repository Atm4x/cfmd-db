use kernel_model::{DatabaseState, Value};
use kernel_semantics::BuiltinSemanticModuleSpec;
use kernel_types::{ClientTransactionId, EntityId, RevisionId, SemanticId, SemanticRevision};

use crate::checkpoint;
use crate::descriptor::DurableMaterializationSpec;
use crate::runtime::CodecError;

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
    LegacyTargetOnly,
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
pub struct DurableRelationMutation {
    pub relation: SemanticId,
    pub inserted: Vec<Vec<Value>>,
    pub removed: Vec<Vec<Value>>,
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
pub enum DurableRevisionChange {
    RelationData {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
    },
    MixedRevision {
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        model_delta: DurableModelDelta,
    },
    FullRevision {
        encoded_target_revision: Vec<u8>,
    },
    FullRevisionAndMaterializations {
        encoded_target_revision: Vec<u8>,
        materializations: Vec<DurableMaterializationSpec>,
    },
}

/// Exact client-visible intent retained across WAL, checkpoint rotation and
/// restart. Relation-data transactions retain their immutable source/target
/// lineage plus the exact typed delta instead of duplicating the complete target
/// Revision. Full-revision replacements still retain canonical target bytes.
/// Legacy stores can still be decoded, but their old target-id-only entries are
/// never treated as exact idempotency matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableTransactionIntent {
    /// Exact relation-data request identity.  The immutable source revision plus
    /// the canonical relation mutations determine the target logical state, so
    /// retaining a second full target checkpoint is redundant.
    RelationDataExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    RelationRewriteExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    /// Exact resolution Rewrite whose causal parent revisions are part of the
    /// durable transaction identity. The store derives the prerequisite effect
    /// cut from those already-authoritative revision frontiers; callers never
    /// supply raw effect ids.
    RelationResolutionExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        rewrite_intents: Vec<DurableRelationRewriteIntent>,
        causal_parents: Vec<RevisionId>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    /// Exact mixed revision identity.  The immutable source plus canonical
    /// relation and non-relation deltas reconstruct the complete target state.
    MixedRevisionExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        semantic_revision: SemanticRevision,
        relation_mutations: Vec<DurableRelationMutation>,
        model_delta: DurableModelDelta,
        model_complement: Option<Box<DurableModelDelta>>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    /// Exact full-revision replacement.  Full payload bytes remain necessary
    /// because this transition is not derivable from a smaller typed delta.
    Exact {
        target_revision: RevisionId,
        encoded_target_revision: Vec<u8>,
        materializations: Option<Vec<DurableMaterializationSpec>>,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    /// Exact schema migration whose inverse information is part of the same
    /// PREPARE/COMMIT identity as the target revision.  The complement must
    /// not be staged in a separate generation: recovery either observes both
    /// the committed schema transition and this authority or neither.
    SchemaMigrationExact {
        source_revision: RevisionId,
        target_revision: RevisionId,
        encoded_target_revision: Vec<u8>,
        migration_complement: DurableMigrationComplement,
        semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    },
    LegacyTargetOnly {
        target_revision: RevisionId,
    },
}

impl DurableTransactionIntent {
    #[must_use]
    pub const fn effect_kind(&self) -> DurableEffectKind {
        match self {
            Self::RelationDataExact { .. } => DurableEffectKind::RelationData,
            Self::RelationRewriteExact { .. } => DurableEffectKind::RelationRewrite,
            Self::RelationResolutionExact { .. } => DurableEffectKind::RelationResolution,
            Self::MixedRevisionExact { .. } => DurableEffectKind::MixedRevision,
            Self::Exact { .. } => DurableEffectKind::FullRevision,
            Self::SchemaMigrationExact { .. } => DurableEffectKind::SchemaMigration,
            Self::LegacyTargetOnly { .. } => DurableEffectKind::LegacyTargetOnly,
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
        Ok(Self::RelationDataExact {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations,
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
        Ok(Self::RelationRewriteExact {
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
        Ok(Self::RelationResolutionExact {
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
        model_complement: DurableModelDelta,
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
        Ok(Self::MixedRevisionExact {
            source_revision,
            target_revision: target.id(),
            semantic_revision,
            relation_mutations,
            model_delta,
            model_complement: Some(Box::new(model_complement)),
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
        Ok(Self::Exact {
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
        Ok(Self::Exact {
            target_revision: target.id(),
            encoded_target_revision: checkpoint::encode_revision(target)?,
            materializations: Some(materializations),
            semantic_modules,
        })
    }

    pub fn schema_migration(
        source_revision: RevisionId,
        target: &kernel_revision::Revision,
        migration_complement: DurableMigrationComplement,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, CodecError> {
        migration_complement
            .validate()
            .map_err(|_| CodecError::CollectionTooLarge)?;
        if migration_complement.target_schema != target.semantic_revision().schema {
            return Err(CodecError::CollectionTooLarge);
        }
        let semantic_modules = registry
            .builtin_modules_for_context(target.semantic_context())
            .map_err(|_| CodecError::SemanticModuleUnavailable)?;
        Ok(Self::SchemaMigrationExact {
            source_revision,
            target_revision: target.id(),
            encoded_target_revision: checkpoint::encode_revision(target)?,
            migration_complement,
            semantic_modules,
        })
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        match self {
            Self::RelationDataExact {
                target_revision, ..
            }
            | Self::RelationRewriteExact {
                target_revision, ..
            }
            | Self::RelationResolutionExact {
                target_revision, ..
            }
            | Self::MixedRevisionExact {
                target_revision, ..
            }
            | Self::Exact {
                target_revision, ..
            }
            | Self::SchemaMigrationExact {
                target_revision, ..
            }
            | Self::LegacyTargetOnly { target_revision } => *target_revision,
        }
    }

    #[must_use]
    pub const fn is_exact(&self) -> bool {
        matches!(
            self,
            Self::RelationDataExact { .. }
                | Self::RelationRewriteExact { .. }
                | Self::RelationResolutionExact { .. }
                | Self::MixedRevisionExact { .. }
                | Self::Exact { .. }
                | Self::SchemaMigrationExact { .. }
        )
    }

    #[must_use]
    pub const fn source_revision(&self) -> Option<RevisionId> {
        match self {
            Self::RelationDataExact {
                source_revision, ..
            }
            | Self::RelationRewriteExact {
                source_revision, ..
            }
            | Self::RelationResolutionExact {
                source_revision, ..
            }
            | Self::SchemaMigrationExact {
                source_revision, ..
            }
            | Self::MixedRevisionExact {
                source_revision, ..
            } => Some(*source_revision),
            Self::Exact { .. } | Self::LegacyTargetOnly { .. } => None,
        }
    }
}
