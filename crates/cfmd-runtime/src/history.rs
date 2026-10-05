use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};

use crate::{Error, ErrorKind, Plan, RelationId, Result, RevisionId, Row, TransactionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryEffectKind {
    RelationData,
    RelationRewrite,
    RelationResolution,
    MixedRevision,
    FullRevision,
    SchemaMigration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryReversibility {
    ExactPlanInverse,
    ComplementRequired,
    NonPlanTransition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryBoundaryAuthority {
    LocalComplement,
    ExternalArchive { proof: u128 },
    ExplicitlyForgotten,
    LocalPayloadReleased,
}

impl HistoryBoundaryAuthority {
    #[must_use]
    pub const fn retains_history_authority(self) -> bool {
        matches!(self, Self::LocalComplement | Self::ExternalArchive { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistorySemanticChange {
    source_schema: u64,
    target_schema: u64,
    migration_spec: u128,
    semantic_manifest: u128,
    encoding_version: u32,
    historical_authority: HistoryBoundaryAuthority,
}

impl HistorySemanticChange {
    #[must_use]
    pub const fn source_schema(&self) -> u64 {
        self.source_schema
    }

    #[must_use]
    pub const fn target_schema(&self) -> u64 {
        self.target_schema
    }

    #[must_use]
    pub const fn migration_spec(&self) -> u128 {
        self.migration_spec
    }

    #[must_use]
    pub const fn semantic_manifest(&self) -> u128 {
        self.semantic_manifest
    }

    #[must_use]
    pub const fn encoding_version(&self) -> u32 {
        self.encoding_version
    }

    #[must_use]
    pub const fn historical_authority(&self) -> HistoryBoundaryAuthority {
        self.historical_authority
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryUndoReadiness {
    Ready,
    Rebased {
        current_revision: RevisionId,
        intervening_effects: Vec<u128>,
    },
    Conflict {
        current_revision: RevisionId,
        conflicting_effects: Vec<u128>,
        opaque_effects: Vec<u128>,
        conflicting_coordinates: usize,
    },
    RuntimeClosed,
    NonReversible,
    Unavailable {
        current_revision: RevisionId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRelationChange {
    relation: RelationId,
    inserted: Vec<Row>,
    removed: Vec<Row>,
    object_field_writes: Vec<kernel_durability::DurableObjectFieldWrite>,
    authorization: kernel_durability::DurableRelationAuthorization,
}

impl HistoryRelationChange {
    #[must_use]
    pub const fn relation(&self) -> RelationId {
        self.relation
    }

    #[must_use]
    pub fn inserted(&self) -> &[Row] {
        &self.inserted
    }

    #[must_use]
    pub fn removed(&self) -> &[Row] {
        &self.removed
    }
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    effect_id: u128,
    prerequisites: Vec<u128>,
    transaction: TransactionId,
    source_revision: RevisionId,
    target_revision: RevisionId,
    kind: HistoryEffectKind,
    reversibility: HistoryReversibility,
    semantic_change: Option<HistorySemanticChange>,
    changes: Vec<HistoryRelationChange>,
    model_complement: Option<kernel_plan::DurableModelDelta>,
    runtime: Weak<kernel_plan::DurableRuntime>,
    database_identity: u64,
    authority: crate::security::RuntimeAuthority,
}

impl HistoryEntry {
    #[must_use]
    pub const fn effect_id(&self) -> u128 {
        self.effect_id
    }

    #[must_use]
    pub fn prerequisites(&self) -> &[u128] {
        &self.prerequisites
    }

    #[must_use]
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }

    #[must_use]
    pub const fn source_revision(&self) -> RevisionId {
        self.source_revision
    }

    #[must_use]
    pub const fn target_revision(&self) -> RevisionId {
        self.target_revision
    }

    #[must_use]
    pub const fn kind(&self) -> HistoryEffectKind {
        self.kind
    }

    #[must_use]
    pub const fn reversibility(&self) -> HistoryReversibility {
        self.reversibility
    }

    #[must_use]
    pub const fn semantic_change(&self) -> Option<&HistorySemanticChange> {
        self.semantic_change.as_ref()
    }

    #[must_use]
    pub fn changes(&self) -> &[HistoryRelationChange] {
        &self.changes
    }

    /// Reports whether this historical inverse can be transported to the
    /// current head. Non-head transport is certified by the kernel from exact
    /// semantic write footprints; it is never an optimistic stale replay.
    #[must_use]
    pub fn undo_readiness(&self) -> HistoryUndoReadiness {
        if self.reversibility != HistoryReversibility::ExactPlanInverse {
            return HistoryUndoReadiness::NonReversible;
        }
        let Some(runtime) = self.runtime.upgrade() else {
            return HistoryUndoReadiness::RuntimeClosed;
        };
        let Ok(snapshot) = runtime.snapshot() else {
            return HistoryUndoReadiness::RuntimeClosed;
        };
        let current_revision = RevisionId::from(snapshot.revision().id());
        if current_revision == self.target_revision {
            return HistoryUndoReadiness::Ready;
        }
        match runtime.certify_history_inverse_rebase(self.effect_id) {
            Ok(kernel_plan::RuntimeHistoryRebaseOutcome::Certified(certificate)) => {
                HistoryUndoReadiness::Rebased {
                    current_revision: certificate.current_revision.into(),
                    intervening_effects: certificate.intervening_effects,
                }
            }
            Ok(kernel_plan::RuntimeHistoryRebaseOutcome::Conflict(conflict)) => {
                HistoryUndoReadiness::Conflict {
                    current_revision: conflict.current_revision.into(),
                    conflicting_effects: conflict.conflicting_effects,
                    opaque_effects: conflict.opaque_effects,
                    conflicting_coordinates: conflict.coordinates.len(),
                }
            }
            Err(_) => HistoryUndoReadiness::Unavailable { current_revision },
        }
    }

    /// Derives the compensating transition as an ordinary `Plan`.
    ///
    /// Non-head entries are transported only when the kernel proves that their
    /// inverse strongly commutes with every intervening exact effect.
    pub fn undo_plan(&self) -> Result<Plan> {
        self.authority.require_write_entry()?;
        if self.reversibility != HistoryReversibility::ExactPlanInverse {
            let reason = match self.reversibility {
                HistoryReversibility::ComplementRequired => {
                    "history transition needs a persisted non-relation complement before exact undo"
                }
                HistoryReversibility::NonPlanTransition => {
                    "history transition is not representable by the current product Plan calculus"
                }
                HistoryReversibility::ExactPlanInverse => unreachable!(),
            };
            return Err(Error::new(ErrorKind::NonReversibleHistory, reason));
        }

        let runtime = self.runtime.upgrade().ok_or_else(|| {
            Error::new(
                ErrorKind::Recovery,
                "history database runtime is no longer open",
            )
        })?;
        let snapshot = runtime.snapshot().map_err(|error| {
            Error::new(
                ErrorKind::Internal,
                format!("history snapshot failed: {error:?}"),
            )
        })?;
        let current_revision = RevisionId::from(snapshot.revision().id());
        if current_revision != self.target_revision {
            match runtime.certify_history_inverse_rebase(self.effect_id) {
                Ok(kernel_plan::RuntimeHistoryRebaseOutcome::Certified(_)) => {}
                Ok(kernel_plan::RuntimeHistoryRebaseOutcome::Conflict(conflict)) => {
                    return Err(Error::new(
                        ErrorKind::HistoryRebaseConflict,
                        format!(
                            "historical inverse conflicts at revision {} with effects {:?}; opaque effects {:?}; {} semantic coordinates overlap",
                            current_revision.raw(),
                            conflict.conflicting_effects,
                            conflict.opaque_effects,
                            conflict.coordinates.len()
                        ),
                    ));
                }
                Err(error) => {
                    return Err(Error::new(
                        ErrorKind::HistoryRebaseConflict,
                        format!("historical inverse rebase could not be certified: {error:?}"),
                    ));
                }
            }
        }

        let mut plan = Plan::new(
            &runtime,
            snapshot,
            self.database_identity,
            self.authority.clone(),
        );
        for change in &self.changes {
            for row in &change.inserted {
                plan.remove(change.relation, row.clone());
            }
            for row in &change.removed {
                plan.insert(change.relation, row.clone());
            }
            let authorization = invert_history_authorization(change.authorization);
            let fields = change
                .object_field_writes
                .iter()
                .map(|write| crate::RelationColumnId::new(write.field.raw()))
                .collect::<Vec<_>>();
            if authorization != kernel_durability::DurableRelationAuthorization::default()
                || !fields.is_empty()
            {
                plan.register_history_authorization(change.relation, authorization, fields);
            }
        }
        plan.model_delta.clone_from(&self.model_complement);
        if plan.is_empty() && plan.model_delta.is_none() {
            return Err(Error::new(
                ErrorKind::NonReversibleHistory,
                "history transition has no inverse representable by Plan",
            ));
        }
        Ok(plan)
    }
}

#[derive(Debug, Clone)]
pub struct History {
    revision: RevisionId,
    entries: Vec<HistoryEntry>,
}

impl History {
    #[allow(
        clippy::too_many_lines,
        reason = "Keep exact historical reconstruction case analysis together."
    )]
    pub(crate) fn from_runtime_at(
        runtime: &Arc<kernel_plan::DurableRuntime>,
        database_identity: u64,
        revision: RevisionId,
        authority: &crate::security::RuntimeAuthority,
    ) -> Result<Self> {
        let effects = runtime
            .revision_history(kernel_types::RevisionId::new(revision.raw()))
            .map_err(|error| {
                Error::new(
                    ErrorKind::Recovery,
                    format!("durable history read failed: {error:?}"),
                )
            })?
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::Recovery,
                    "revision is outside durable causal history coverage",
                )
            })?;

        let weak = Arc::downgrade(runtime);
        let mut entries = effects
            .into_iter()
            .map(|effect| {
                let semantic_change = effect.semantic_change.map(|change| HistorySemanticChange {
                    source_schema: change.source_schema.raw(),
                    target_schema: change.target_schema.raw(),
                    migration_spec: change.lens_spec.0.raw(),
                    semantic_manifest: change.semantic_pins.0.raw(),
                    encoding_version: change.encoding_version,
                    historical_authority: match change.historical_authority {
                        kernel_durability::HistoricalBoundaryAuthority::LocalComplement => {
                            HistoryBoundaryAuthority::LocalComplement
                        }
                        kernel_durability::HistoricalBoundaryAuthority::ExternalArchive(proof) => {
                            HistoryBoundaryAuthority::ExternalArchive {
                                proof: proof.0.raw(),
                            }
                        }
                        kernel_durability::HistoricalBoundaryAuthority::ExplicitlyForgotten => {
                            HistoryBoundaryAuthority::ExplicitlyForgotten
                        }
                        kernel_durability::HistoricalBoundaryAuthority::LocalPayloadReleased => {
                            HistoryBoundaryAuthority::LocalPayloadReleased
                        }
                    },
                });
                HistoryEntry {
                    effect_id: effect.effect_id,
                    prerequisites: effect.prerequisites,
                    transaction: TransactionId::new(effect.transaction_id.raw()),
                    source_revision: effect.source_revision.into(),
                    target_revision: effect.target_revision.into(),
                    kind: match effect.kind {
                        kernel_plan::RuntimeHistoryEffectKind::RelationData => {
                            HistoryEffectKind::RelationData
                        }
                        kernel_plan::RuntimeHistoryEffectKind::RelationRewrite => {
                            HistoryEffectKind::RelationRewrite
                        }
                        kernel_plan::RuntimeHistoryEffectKind::RelationResolution => {
                            HistoryEffectKind::RelationResolution
                        }
                        kernel_plan::RuntimeHistoryEffectKind::MixedRevision => {
                            HistoryEffectKind::MixedRevision
                        }
                        kernel_plan::RuntimeHistoryEffectKind::FullRevision => {
                            HistoryEffectKind::FullRevision
                        }
                        kernel_plan::RuntimeHistoryEffectKind::SchemaMigration => {
                            HistoryEffectKind::SchemaMigration
                        }
                    },
                    reversibility: match effect.reversibility {
                        kernel_plan::RuntimeHistoryReversibility::ExactPlanInverse => {
                            HistoryReversibility::ExactPlanInverse
                        }
                        kernel_plan::RuntimeHistoryReversibility::ComplementRequired => {
                            HistoryReversibility::ComplementRequired
                        }
                        kernel_plan::RuntimeHistoryReversibility::NonPlanTransition => {
                            HistoryReversibility::NonPlanTransition
                        }
                    },
                    semantic_change,
                    model_complement: effect.model_complement,
                    changes: effect
                        .relation_mutations
                        .into_iter()
                        .map(|mutation| HistoryRelationChange {
                            relation: RelationId::new(mutation.relation.raw()),
                            inserted: mutation
                                .inserted
                                .into_iter()
                                .map(|row| row.into_iter().map(Into::into).collect())
                                .collect(),
                            removed: mutation
                                .removed
                                .into_iter()
                                .map(|row| row.into_iter().map(Into::into).collect())
                                .collect(),
                            object_field_writes: mutation.object_field_writes,
                            authorization: mutation.authorization,
                        })
                        .collect(),
                    runtime: weak.clone(),
                    database_identity,
                    authority: authority.clone(),
                }
            })
            .collect::<Vec<_>>();
        topological_history_order(&mut entries)?;
        Ok(Self { revision, entries })
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionId {
        self.revision
    }

    #[must_use]
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    #[must_use]
    pub fn latest(&self) -> Option<&HistoryEntry> {
        self.entries
            .iter()
            .rev()
            .find(|entry| entry.target_revision == self.revision)
    }

    pub fn undo_latest(&self) -> Result<Plan> {
        self.latest()
            .ok_or_else(|| {
                Error::new(ErrorKind::NotFound, "history has no transition at its head")
            })?
            .undo_plan()
    }
}

fn invert_history_authorization(
    forward: kernel_durability::DurableRelationAuthorization,
) -> kernel_durability::DurableRelationAuthorization {
    kernel_durability::DurableRelationAuthorization {
        relation_write: forward.relation_write,
        object_create: forward.object_delete,
        object_delete: forward.object_create,
        relationship_attach: forward.relationship_detach,
        relationship_detach: forward.relationship_attach,
        relationship_move: forward.relationship_move,
    }
}

fn topological_history_order(entries: &mut Vec<HistoryEntry>) -> Result<()> {
    let by_id = entries
        .drain(..)
        .map(|entry| (entry.effect_id, entry))
        .collect::<BTreeMap<_, _>>();
    let ids = by_id.keys().copied().collect::<BTreeSet<_>>();
    let mut remaining = by_id
        .iter()
        .map(|(&id, entry)| {
            let count = entry
                .prerequisites
                .iter()
                .filter(|prerequisite| ids.contains(prerequisite))
                .count();
            (id, count)
        })
        .collect::<BTreeMap<_, _>>();
    let mut successors = BTreeMap::<u128, Vec<u128>>::new();
    for (&id, entry) in &by_id {
        for prerequisite in &entry.prerequisites {
            if ids.contains(prerequisite) {
                successors.entry(*prerequisite).or_default().push(id);
            }
        }
    }
    let mut ready = remaining
        .iter()
        .filter_map(|(&id, &count)| (count == 0).then_some(id))
        .collect::<BTreeSet<_>>();
    let mut ordered_ids = Vec::with_capacity(by_id.len());
    while let Some(id) = ready.pop_first() {
        remaining.remove(&id);
        ordered_ids.push(id);
        if let Some(children) = successors.get(&id) {
            for child in children {
                let count = remaining.get_mut(child).ok_or_else(|| {
                    Error::new(ErrorKind::Internal, "history causal graph is inconsistent")
                })?;
                *count -= 1;
                if *count == 0 {
                    ready.insert(*child);
                }
            }
        }
    }
    if !remaining.is_empty() {
        return Err(Error::new(
            ErrorKind::Internal,
            "history causal graph unexpectedly contains a cycle",
        ));
    }
    let mut by_id = by_id;
    entries.extend(
        ordered_ids
            .into_iter()
            .map(|id| by_id.remove(&id).expect("ordered history id must exist")),
    );
    Ok(())
}
