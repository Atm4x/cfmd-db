use super::{
    CanonicalRowKey, Change, ExactDelta, ExactDeltaSink, ExactDeltaView, FineChange,
    FineChangeKind, PlannedDeltaEffect, PreparedRewrite, PreparedStructuralRewrite, RelExpr,
    RelQueryError, RelType, RelationDeltaView, RelationValue, RewriteActionLaw, RewriteEffect,
    RewriteFootprint, RewriteSpec, Row, SemanticWriteCoordinate, StructuralRewriteEffect, Value,
    canonical_row_key, project_rows, rel_delta_optimized, relation_column_equivalences,
    relation_value_from_rows, unmatched_semantic_rows, validate_query_equivalence,
    value_shape_matches_type,
};
use kernel_persistent::{PersistentOrdMap, PersistentVec};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDelta {
    pub inserted: Vec<Row>,
    pub removed: Vec<Row>,
    pub result_type: RelType,
}

/// One relation Rewrite intent whose derived effect is pinned to an exact
/// Γ-validated `RelationDelta`. The structural Rewrite is the sole authority:
/// the delta is projected from its sealed effect rather than mirrored in a
/// second independently mutable field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRelationRewrite<I = Value> {
    rewrite: PreparedStructuralRewrite<
        RelationValue,
        I,
        RelationStructuralEffect,
        kernel_semantics::SemanticRegistry,
    >,
}

/// Exact Γ-canonical support witness for one authoritative relation base.
///
/// Clones share the persistent support root and an unforgeable in-process
/// authority token. Runtime owners can therefore bind a prepared Rewrite to
/// the exact source base in O(1), while advancing the witness only along the
/// touched delta support.
#[derive(Debug, Clone)]
pub struct RelationBaseWitness {
    revision: kernel_types::RevisionId,
    relation: kernel_types::SemanticId,
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    supports: PersistentOrdMap<CanonicalRowKey, usize>,
    authority: Arc<()>,
}

struct RelationSupportTransition {
    supports: PersistentOrdMap<CanonicalRowKey, usize>,
    removed_keys: Vec<CanonicalRowKey>,
    inserted_keys: Vec<CanonicalRowKey>,
}

impl PartialEq for RelationBaseWitness {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.relation == other.relation
            && self.result_type == other.result_type
            && self.semantic_context == other.semantic_context
            && self.supports == other.supports
    }
}

impl Eq for RelationBaseWitness {}

impl RelationBaseWitness {
    pub fn build(
        revision: kernel_types::RevisionId,
        relation: kernel_types::SemanticId,
        rows: &[Row],
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        MaterializedSetSupportState::validate_rows(rows, &result_type, context, registry)?;
        let mut supports: PersistentOrdMap<CanonicalRowKey, usize> = PersistentOrdMap::default();
        let set_semantics = matches!(
            result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        );
        let equivalences = relation_column_equivalences(&result_type);
        for row in rows {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            let before = supports.get(&key).copied().unwrap_or_default();
            if set_semantics && before != 0 {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            supports.insert(
                key,
                before
                    .checked_add(1)
                    .ok_or(RelQueryError::DerivedIdentityExhausted)?,
            );
        }
        Ok(Self {
            revision,
            relation,
            result_type,
            semantic_context: context.clone(),
            supports,
            authority: Arc::new(()),
        })
    }

    #[must_use]
    pub const fn revision(&self) -> kernel_types::RevisionId {
        self.revision
    }

    #[must_use]
    pub const fn relation(&self) -> kernel_types::SemanticId {
        self.relation
    }

    #[must_use]
    pub const fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub fn certifies_same_base(&self, other: &Self) -> bool {
        if self.relation != other.relation
            || self.result_type != other.result_type
            || self.semantic_context != other.semantic_context
        {
            return false;
        }
        if self.revision == other.revision && Arc::ptr_eq(&self.authority, &other.authority) {
            return true;
        }
        if self.revision != kernel_types::RevisionId::new(0) {
            return false;
        }
        self.supports == other.supports
    }

    fn apply_delta_supports(
        &self,
        delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationSupportTransition, RelQueryError> {
        if delta.result_type != self.result_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let equivalences = relation_column_equivalences(&self.result_type);
        let set_semantics = matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        );
        let mut supports = self.supports.clone();
        let mut removed_keys = Vec::with_capacity(delta.removed.len());
        for row in &delta.removed {
            let key = canonical_row_key(row, equivalences, &self.semantic_context, registry)?;
            let before = supports.get(&key).copied().unwrap_or_default();
            if before == 0 {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            if before == 1 {
                supports.remove(&key);
            } else {
                supports.insert(key.clone(), before - 1);
            }
            removed_keys.push(key);
        }
        let mut inserted_keys = Vec::with_capacity(delta.inserted.len());
        for row in &delta.inserted {
            let key = canonical_row_key(row, equivalences, &self.semantic_context, registry)?;
            let before = supports.get(&key).copied().unwrap_or_default();
            if set_semantics && before != 0 {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            supports.insert(
                key.clone(),
                before
                    .checked_add(1)
                    .ok_or(RelQueryError::DerivedIdentityExhausted)?,
            );
            inserted_keys.push(key);
        }
        Ok(RelationSupportTransition {
            supports,
            removed_keys,
            inserted_keys,
        })
    }

    pub fn advance(
        &self,
        target_revision: kernel_types::RevisionId,
        delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let transition = self.apply_delta_supports(delta, registry)?;
        Ok(Self {
            revision: target_revision,
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            supports: transition.supports,
            authority: Arc::new(()),
        })
    }
}

impl<I> PreparedRelationRewrite<I> {
    #[must_use]
    pub const fn rewrite(
        &self,
    ) -> &PreparedStructuralRewrite<
        RelationValue,
        I,
        RelationStructuralEffect,
        kernel_semantics::SemanticRegistry,
    > {
        &self.rewrite
    }

    #[must_use]
    pub fn delta(&self) -> &RelationDelta {
        self.rewrite.effect().delta()
    }

    pub fn apply_structural(
        &self,
        old: &RelationValue,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        self.rewrite.apply_structural_with(old, registry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationStructuralEffect {
    delta: RelationDelta,
    base: RelationBaseWitness,
    removed_keys: Vec<CanonicalRowKey>,
    inserted_keys: Vec<CanonicalRowKey>,
}

impl RelationStructuralEffect {
    fn prepare_detached(
        relation: kernel_types::SemanticId,
        old: &RelationValue,
        delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let expected_set = matches!(
            delta.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        );
        if expected_set != matches!(old, RelationValue::Set { .. }) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let base = RelationBaseWitness::build(
            kernel_types::RevisionId::new(0),
            relation,
            old.rows(),
            delta.result_type.clone(),
            context,
            registry,
        )?;
        Self::prepare_on_base(&base, delta, registry)
    }

    fn prepare_on_base(
        base: &RelationBaseWitness,
        delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let transition = base.apply_delta_supports(delta, registry)?;
        Ok(Self {
            delta: delta.clone(),
            base: base.clone(),
            removed_keys: transition.removed_keys,
            inserted_keys: transition.inserted_keys,
        })
    }

    #[must_use]
    pub fn delta(&self) -> &RelationDelta {
        &self.delta
    }

    #[must_use]
    pub fn certifies_base_witness(&self, base: &RelationBaseWitness) -> bool {
        self.base.certifies_same_base(base)
    }

    fn rewrite_footprint(&self, relation: kernel_types::SemanticId) -> RewriteFootprint {
        let mut footprint = RewriteFootprint::default();
        for key in self.removed_keys.iter().chain(&self.inserted_keys) {
            footprint.writes.insert(
                SemanticWriteCoordinate::RelationClass {
                    relation,
                    canonical_key: kernel_semantics::encode_canonical_eq_key_tuple(key)
                        .into_boxed_slice(),
                },
                RewriteActionLaw::Opaque,
            );
        }
        footprint
    }
}

impl StructuralRewriteEffect<RelationValue, kernel_semantics::SemanticRegistry>
    for RelationStructuralEffect
{
    type Error = RelQueryError;

    fn apply_structural_with(
        &self,
        old: &RelationValue,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, Self::Error> {
        apply_prepared_relation_effect(old, self, registry)
    }
}

/// Cross-crate row-identity evidence for a relation delta.
///
/// This value is deliberately **not** a semantic certificate or publication
/// capability: callers may construct it, and every maintained leaf validates
/// the supplied handles against its current storage-handle snapshot before it
/// can produce a detached candidate. Authoritative publication remains owned
/// by `kernel-plan::RuntimeRevisionCell`.
///
/// The row payload is carried for operator semantics, while handles let `Scan`
/// update its local snapshot without repeating semantic membership search over
/// the whole base relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageResolvedRelationDelta {
    relation: kernel_types::SemanticId,
    delta: RelationDelta,
    removed_handles: Vec<kernel_types::StableRowHandle>,
    inserted_handles: Vec<kernel_types::StableRowHandle>,
}

impl StorageResolvedRelationDelta {
    #[must_use]
    pub fn from_parts(
        relation: kernel_types::SemanticId,
        delta: RelationDelta,
        removed_handles: Vec<kernel_types::StableRowHandle>,
        inserted_handles: Vec<kernel_types::StableRowHandle>,
    ) -> Self {
        Self {
            relation,
            delta,
            removed_handles,
            inserted_handles,
        }
    }

    #[must_use]
    pub fn relation(&self) -> kernel_types::SemanticId {
        self.relation
    }

    #[must_use]
    pub fn delta(&self) -> &RelationDelta {
        &self.delta
    }

    pub(super) fn removed_handles(&self) -> &[kernel_types::StableRowHandle] {
        &self.removed_handles
    }

    pub(super) fn inserted_handles(&self) -> &[kernel_types::StableRowHandle] {
        &self.inserted_handles
    }
}

pub(super) fn validate_exact_delta_view_rows<D: ExactDeltaView<Row>>(
    delta: &D,
    ty: &RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(), RelQueryError> {
    let mut error = None;
    delta.visit_exact(|weight, row| {
        if weight.is_zero() || error.is_some() {
            return;
        }
        if let Err(next) = MaterializedSetSupportState::validate_rows(
            std::slice::from_ref(row),
            ty,
            context,
            registry,
        ) {
            error = Some(next);
        }
    });
    error.map_or(Ok(()), Err)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RelationMutationPlan {
    remove_indices: Vec<usize>,
    inserted: Vec<Row>,
    canonical_keys: CanonicalRelationMutationKeys,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CanonicalRelationMutationKeys {
    removed: Vec<CanonicalRowKey>,
    inserted: Vec<CanonicalRowKey>,
}

#[derive(Debug)]
pub(super) enum MaintainedScanCommitPatch {
    Semantic(RelationMutationPlan),
    StorageResolved(StorageResolvedScanPatch),
}

#[derive(Debug)]
pub(super) struct StorageResolvedScanPatch {
    pub(super) removed_handles: Vec<kernel_types::StableRowHandle>,
    pub(super) inserted: Vec<(kernel_types::StableRowHandle, CanonicalRowKey, Row)>,
}

pub(super) fn plan_relation_mutation(
    value: &PersistentVec<Row>,
    delta: &RelationDelta,
    ty: &RelType,
    canonical_lookup: &super::CanonicalRowPositionIndex,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationMutationPlan, RelQueryError> {
    let equivalences = relation_column_equivalences(ty);
    let mut removed_per_key = BTreeMap::<CanonicalRowKey, usize>::new();
    let mut remove_indices = Vec::with_capacity(delta.removed.len());
    let mut removed_keys = Vec::with_capacity(delta.removed.len());
    for removed in &delta.removed {
        let key = canonical_row_key(removed, equivalences, context, registry)?;
        let used = removed_per_key.entry(key.clone()).or_default();
        let bucket = canonical_lookup
            .positions(&key)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let index = bucket
            .len()
            .checked_sub(*used + 1)
            .and_then(|position| bucket.get(position).copied())
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        *used += 1;
        remove_indices.push(index);
        removed_keys.push(key);
    }

    let is_set = matches!(ty.semantics, kernel_schema::RelationSemantics::Set { .. });
    let mut inserted_keys = Vec::with_capacity(delta.inserted.len());
    let mut inserted_set = std::collections::BTreeSet::new();
    for inserted in &delta.inserted {
        let key = canonical_row_key(inserted, equivalences, context, registry)?;
        if is_set {
            let existing = canonical_lookup
                .positions(&key)
                .map_or(0, PersistentVec::len);
            let removing = removed_per_key.get(&key).copied().unwrap_or(0);
            if existing > removing || !inserted_set.insert(key.clone()) {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        inserted_keys.push(key);
    }
    let _ = value;
    Ok(RelationMutationPlan {
        remove_indices,
        inserted: delta.inserted.clone(),
        canonical_keys: CanonicalRelationMutationKeys {
            removed: removed_keys,
            inserted: inserted_keys,
        },
    })
}

pub(super) fn commit_relation_mutation(
    value: &mut PersistentVec<Row>,
    canonical_lookup: &mut super::CanonicalRowPositionIndex,
    plan: RelationMutationPlan,
) {
    let keys = plan.canonical_keys;
    let mut removals = plan
        .remove_indices
        .into_iter()
        .zip(keys.removed)
        .collect::<Vec<_>>();
    removals.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
    for (index, _) in removals {
        canonical_lookup.remove_position(index);
        value.swap_remove(index);
    }
    for (row, key) in plan.inserted.into_iter().zip(keys.inserted) {
        value.push(row);
        canonical_lookup.push_key(key);
    }
}

impl RelationDelta {
    #[must_use]
    pub fn as_delta_view(&self) -> RelationDeltaView<'_> {
        RelationDeltaView {
            removed: &self.removed,
            inserted: &self.inserted,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty()
    }

    /// Canonical dynamic footprint of this concrete relation effect.
    ///
    /// Coordinates use the pinned Γ canonical tuple encoding directly rather
    /// than a host hash, so coordination authority is not weakened by hash
    /// collision or Rust equality.
    pub fn rewrite_footprint(
        &self,
        relation: kernel_types::SemanticId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RewriteFootprint, RelQueryError> {
        let equivalences = relation_column_equivalences(&self.result_type);
        let mut footprint = RewriteFootprint::default();
        for row in self.removed.iter().chain(&self.inserted) {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            footprint.writes.insert(
                SemanticWriteCoordinate::RelationClass {
                    relation,
                    canonical_key: kernel_semantics::encode_canonical_eq_key_tuple(&key)
                        .into_boxed_slice(),
                },
                RewriteActionLaw::Opaque,
            );
        }
        Ok(footprint)
    }

    pub fn apply_to_value(
        &self,
        old: RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        apply_relation_delta_to_value(old, self, context, registry)
    }

    pub fn between_values(
        old: &RelationValue,
        next: &RelationValue,
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        relation_delta_between_values(old, next, result_type, context, registry)
    }

    /// Compiles this Γ-validated relation delta into one intent-bearing
    /// prepared Rewrite. The exact endpoint is derived through the same pinned
    /// semantic relation-delta application used by maintained queries; callers
    /// cannot substitute a different endpoint while retaining the delta intent.
    pub fn prepare_rewrite<I>(
        &self,
        relation: kernel_types::SemanticId,
        old: &RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        spec: &RewriteSpec,
        explicit_inputs: Vec<I>,
    ) -> Result<PreparedRewrite<RelationValue, I>, RelQueryError> {
        let structural =
            RelationStructuralEffect::prepare_detached(relation, old, self, context, registry)?;
        let required = structural.rewrite_footprint(relation);
        if !spec.footprint.conservatively_covers(&required) {
            return Err(RelQueryError::RewriteFootprintMismatch);
        }
        let endpoint = structural.apply_structural_with(old, registry)?;
        Ok(spec.prepare(
            explicit_inputs,
            RewriteEffect::Fine(FineChange::new(FineChangeKind::Relation, endpoint)),
        ))
    }

    pub fn prepare_relation_rewrite<I>(
        &self,
        relation: kernel_types::SemanticId,
        old: &RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        spec: &RewriteSpec,
        explicit_inputs: Vec<I>,
    ) -> Result<PreparedRelationRewrite<I>, RelQueryError> {
        let structural =
            RelationStructuralEffect::prepare_detached(relation, old, self, context, registry)?;
        let required = structural.rewrite_footprint(relation);
        if !spec.footprint.conservatively_covers(&required) {
            return Err(RelQueryError::RewriteFootprintMismatch);
        }
        Ok(PreparedRelationRewrite {
            rewrite: spec.prepare_structural_with_context(
                explicit_inputs,
                structural,
                FineChangeKind::Relation,
            ),
        })
    }

    pub fn prepare_relation_rewrite_on_base<I>(
        &self,
        base: &RelationBaseWitness,
        registry: &kernel_semantics::SemanticRegistry,
        spec: &RewriteSpec,
        explicit_inputs: Vec<I>,
    ) -> Result<PreparedRelationRewrite<I>, RelQueryError> {
        let structural = RelationStructuralEffect::prepare_on_base(base, self, registry)?;
        let required = structural.rewrite_footprint(base.relation());
        if !spec.footprint.conservatively_covers(&required) {
            return Err(RelQueryError::RewriteFootprintMismatch);
        }
        Ok(PreparedRelationRewrite {
            rewrite: spec.prepare_structural_with_context(
                explicit_inputs,
                structural,
                FineChangeKind::Relation,
            ),
        })
    }
}

type SupportLookup = PersistentOrdMap<CanonicalRowKey, usize>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SetSupportPatchEntry {
    key: CanonicalRowKey,
    representative: Row,
    after: kernel_exact::ExactNatural,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SetSupportPatch {
    entries: Vec<SetSupportPatchEntry>,
}

#[derive(Debug)]
struct SupportDeltaPlan {
    key: CanonicalRowKey,
    representative: Row,
    removals: kernel_exact::ExactNatural,
    insertions: kernel_exact::ExactNatural,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedSetSupportState {
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    supports: PersistentVec<(Row, kernel_exact::ExactNatural)>,
    support_lookup: SupportLookup,
}

impl MaterializedSetSupportState {
    pub fn build(
        rows: &[Row],
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if !matches!(
            result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) {
            return Err(RelQueryError::TypeMismatch);
        }
        Self::validate_rows(rows, &result_type, context, registry)?;
        let (supports, support_lookup) = canonical_row_supports(
            rows,
            relation_column_equivalences(&result_type),
            context,
            registry,
        )?;
        Ok(Self {
            result_type,
            semantic_context: context.clone(),
            supports: supports.into(),
            support_lookup,
        })
    }

    #[must_use]
    pub fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        &self.semantic_context
    }

    pub fn support_count(
        &self,
        row: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<i64, RelQueryError> {
        self.check_context(context)?;
        Self::validate_rows(
            std::slice::from_ref(row),
            &self.result_type,
            context,
            registry,
        )?;
        let key = canonical_row_key(
            row,
            relation_column_equivalences(&self.result_type),
            context,
            registry,
        )?;
        let count = self
            .support_lookup
            .get(&key)
            .map_or_else(kernel_exact::ExactNatural::zero, |index| {
                self.supports[*index].1.clone()
            });
        count
            .to_u64()
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(RelQueryError::DerivedIdentityExhausted)
    }

    pub(super) fn output_value(&self) -> RelationValue {
        let rows = self
            .supports
            .iter()
            .filter(|(_, count)| !count.is_zero())
            .map(|(row, _)| row.clone())
            .collect();
        relation_value_from_rows(rows, &self.result_type)
    }

    pub fn apply_rows_delta(
        &mut self,
        inserted_rows: Vec<Row>,
        removed_rows: Vec<Row>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let delta = RelationDelta {
            inserted: inserted_rows,
            removed: removed_rows,
            result_type: self.result_type.clone(),
        };
        let planned = self.plan_delta_view(&delta.as_delta_view(), context, registry)?;
        let effect = materialize_set_support_effect(&planned.effect, self.result_type.clone())?;
        self.commit_support_patch(planned.patch);
        Ok(effect)
    }

    pub(super) fn plan_delta_view<D: ExactDeltaView<Row>>(
        &self,
        delta: &D,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedDeltaEffect<SetSupportPatch, ExactDelta<Row>>, RelQueryError> {
        self.check_context(context)?;
        let column_equivalences = relation_column_equivalences(&self.result_type);
        let mut changes = Vec::<SupportDeltaPlan>::new();
        let mut change_lookup = BTreeMap::<Vec<kernel_semantics::CanonicalEqKey>, usize>::new();
        let mut visit_error = None;
        delta.visit_exact(|weight, row| {
            if weight.is_zero() || visit_error.is_some() {
                return;
            }
            if let Err(error) = Self::validate_rows(
                std::slice::from_ref(row),
                &self.result_type,
                context,
                registry,
            ) {
                visit_error = Some(error);
                return;
            }
            let key = match canonical_row_key(row, column_equivalences, context, registry) {
                Ok(key) => key,
                Err(error) => {
                    visit_error = Some(error);
                    return;
                }
            };
            let change_index = if let Some(index) = change_lookup.get(&key).copied() {
                index
            } else {
                let index = changes.len();
                change_lookup.insert(key.clone(), index);
                changes.push(SupportDeltaPlan {
                    key,
                    representative: row.clone(),
                    removals: kernel_exact::ExactNatural::zero(),
                    insertions: kernel_exact::ExactNatural::zero(),
                });
                index
            };
            if weight.is_negative() {
                changes[change_index]
                    .removals
                    .add_assign(weight.magnitude());
            } else {
                changes[change_index]
                    .insertions
                    .add_assign(weight.magnitude());
            }
        });
        if let Some(error) = visit_error {
            return Err(error);
        }

        let mut patch_entries = Vec::with_capacity(changes.len());
        let mut effect = ExactDelta::<Row>::default();
        for change in changes {
            let before = self
                .support_lookup
                .get(&change.key)
                .map_or_else(kernel_exact::ExactNatural::zero, |index| {
                    self.supports[*index].1.clone()
                });
            let mut after = before.clone();
            if !after.checked_sub_assign(&change.removals) {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            after.add_assign(&change.insertions);
            if before.is_zero() && !after.is_zero() {
                effect.push_exact(
                    kernel_exact::ExactInteger::from_i64(1),
                    change.representative.clone(),
                );
            } else if !before.is_zero() && after.is_zero() {
                effect.push_exact(
                    kernel_exact::ExactInteger::from_i64(-1),
                    change.representative.clone(),
                );
            }
            patch_entries.push(SetSupportPatchEntry {
                key: change.key,
                representative: change.representative,
                after,
            });
        }
        Ok(PlannedDeltaEffect {
            patch: SetSupportPatch {
                entries: patch_entries,
            },
            effect,
        })
    }

    pub(super) fn commit_support_patch(&mut self, patch: SetSupportPatch) {
        for entry in patch.entries {
            if let Some(index) = self.support_lookup.get(&entry.key).copied() {
                self.supports[index].1 = entry.after;
            } else if !entry.after.is_zero() {
                let index = self.supports.len();
                self.supports
                    .push((entry.representative.clone(), entry.after));
                self.support_lookup.insert(entry.key, index);
            }
        }
    }

    pub(super) fn validate_rows(
        rows: &[Row],
        result_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        let equivalences = relation_column_equivalences(result_type);
        if equivalences.len() != result_type.columns.len() {
            return Err(RelQueryError::TypeMismatch);
        }
        for (column_type, equivalence) in result_type.columns.iter().zip(equivalences) {
            validate_query_equivalence(*equivalence, column_type, context, registry)?;
        }
        Self::validate_row_shapes(rows, result_type)
    }

    pub(super) fn validate_row_shapes(
        rows: &[Row],
        result_type: &RelType,
    ) -> Result<(), RelQueryError> {
        for row in rows {
            if row.len() != result_type.columns.len()
                || !row
                    .iter()
                    .zip(&result_type.columns)
                    .all(|(value, ty)| value_shape_matches_type(value, ty))
            {
                return Err(RelQueryError::TypeMismatch);
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn test_shares_storage_with(&self, other: &Self) -> (bool, bool) {
        (
            self.supports.shares_storage_with(&other.supports),
            self.support_lookup.shares_root_with(&other.support_lookup),
        )
    }

    #[cfg(test)]
    pub(super) fn test_support_count_exact_at(
        &self,
        index: usize,
    ) -> Option<&kernel_exact::ExactNatural> {
        self.supports.get(index).map(|(_, count)| count)
    }

    #[cfg(test)]
    pub(super) fn test_support_class_count(&self) -> usize {
        self.support_lookup.len()
    }

    fn check_context(&self, context: &kernel_schema::SemanticContext) -> Result<(), RelQueryError> {
        if context == &self.semantic_context {
            Ok(())
        } else {
            Err(RelQueryError::SemanticRevisionMismatch)
        }
    }
}

fn materialize_set_support_effect<D: ExactDeltaView<Row>>(
    effect: &D,
    result_type: RelType,
) -> Result<RelationDelta, RelQueryError> {
    let mut inserted = Vec::new();
    let mut removed = Vec::new();
    let mut error = None;
    effect.visit_exact(|weight, row| {
        if error.is_some() || weight.is_zero() {
            return;
        }
        if weight.magnitude() != &kernel_exact::ExactNatural::one() {
            error = Some(RelQueryError::InconsistentIncrementalDelta);
            return;
        }
        if weight.is_negative() {
            removed.push(row.clone());
        } else {
            inserted.push(row.clone());
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    Ok(RelationDelta {
        inserted,
        removed,
        result_type,
    })
}

fn canonical_row_supports(
    rows: &[Row],
    column_equivalences: &[kernel_types::SemanticId],
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<(Vec<(Row, kernel_exact::ExactNatural)>, SupportLookup), RelQueryError> {
    let mut supports: Vec<(Row, kernel_exact::ExactNatural)> = Vec::new();
    let mut lookup = SupportLookup::default();
    for row in rows {
        let key = canonical_row_key(row, column_equivalences, context, registry)?;
        if let Some(index) = lookup.get(&key).copied() {
            supports[index].1.add_u128(1);
        } else {
            let index = supports.len();
            supports.push((row.clone(), kernel_exact::ExactNatural::one()));
            lookup.insert(key, index);
        }
    }
    Ok((supports, lookup))
}

pub(super) fn apply_relation_delta_to_value(
    old: RelationValue,
    delta: &RelationDelta,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let column_equivalences = relation_column_equivalences(&delta.result_type);
    let expected_set = matches!(
        delta.result_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    if expected_set != matches!(old, RelationValue::Set { .. }) {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }
    if delta.removed.is_empty() && (delta.inserted.is_empty() || !expected_set) {
        let mut rows = old.into_rows();
        rows.extend(delta.inserted.iter().cloned());
        return Ok(match &delta.result_type.semantics {
            kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(rows),
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } => RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
        });
    }
    let rows = old.into_rows();
    let mut support_counts = BTreeMap::<CanonicalRowKey, usize>::new();
    let mut row_keys = Vec::with_capacity(rows.len());
    for row in &rows {
        let key = canonical_row_key(row, column_equivalences, context, registry)?;
        *support_counts.entry(key.clone()).or_default() += 1;
        row_keys.push(key);
    }
    let mut removals = BTreeMap::<CanonicalRowKey, usize>::new();
    for removed in &delta.removed {
        let key = canonical_row_key(removed, column_equivalences, context, registry)?;
        let Some(count) = support_counts.get_mut(&key) else {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        };
        if *count == 0 {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        *count -= 1;
        *removals.entry(key).or_default() += 1;
    }
    for inserted in &delta.inserted {
        let key = canonical_row_key(inserted, column_equivalences, context, registry)?;
        if expected_set && support_counts.get(&key).copied().unwrap_or_default() != 0 {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        *support_counts.entry(key).or_default() += 1;
    }

    let mut next_rows = Vec::with_capacity(rows.len() + delta.inserted.len());
    for (row, key) in rows.into_iter().zip(row_keys) {
        let remove = removals.get_mut(&key).is_some_and(|count| {
            if *count == 0 {
                false
            } else {
                *count -= 1;
                true
            }
        });
        if !remove {
            next_rows.push(row);
        }
    }
    next_rows.extend(delta.inserted.iter().cloned());
    Ok(match &delta.result_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(next_rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows: next_rows,
            column_equivalences: column_equivalences.clone(),
        },
    })
}

fn apply_prepared_relation_effect(
    old: &RelationValue,
    effect: &RelationStructuralEffect,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let expected_set = matches!(
        effect.delta.result_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    if expected_set != matches!(old, RelationValue::Set { .. }) {
        return Err(RelQueryError::StructuralRewriteBaseMismatch);
    }

    let equivalences = relation_column_equivalences(&effect.delta.result_type);
    let mut support_counts = BTreeMap::<CanonicalRowKey, usize>::new();
    let mut row_keys = Vec::with_capacity(old.rows().len());
    for row in old.rows() {
        let key = canonical_row_key(row, equivalences, &effect.base.semantic_context, registry)?;
        *support_counts.entry(key.clone()).or_default() += 1;
        row_keys.push(key);
    }
    if support_counts.len() != effect.base.supports.len()
        || !support_counts
            .iter()
            .all(|(key, count)| effect.base.supports.get(key) == Some(count))
    {
        return Err(RelQueryError::StructuralRewriteBaseMismatch);
    }

    let mut removals = BTreeMap::<CanonicalRowKey, usize>::new();
    for key in &effect.removed_keys {
        *removals.entry(key.clone()).or_default() += 1;
    }
    let mut next_rows = Vec::with_capacity(
        old.rows().len().saturating_sub(effect.delta.removed.len()) + effect.delta.inserted.len(),
    );
    for (row, key) in old.rows().iter().cloned().zip(row_keys) {
        let remove = removals.get_mut(&key).is_some_and(|count| {
            if *count == 0 {
                false
            } else {
                *count -= 1;
                true
            }
        });
        if !remove {
            next_rows.push(row);
        }
    }
    next_rows.extend(effect.delta.inserted.iter().cloned());
    Ok(match &effect.delta.result_type.semantics {
        kernel_schema::RelationSemantics::Bag { .. } => RelationValue::Bag(next_rows),
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => RelationValue::Set {
            rows: next_rows,
            column_equivalences: column_equivalences.clone(),
        },
    })
}

fn relation_delta_between_values(
    old: &RelationValue,
    next: &RelationValue,
    result_type: RelType,
    context: &kernel_schema::SemanticContext,
    registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationDelta, RelQueryError> {
    let column_equivalences = relation_column_equivalences(&result_type);
    Ok(RelationDelta {
        inserted: unmatched_semantic_rows(
            next.rows(),
            old.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        removed: unmatched_semantic_rows(
            old.rows(),
            next.rows(),
            column_equivalences,
            context,
            registry,
        )?,
        result_type,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MaterializedRelDeltaOperator {
    ProjectSet { input: RelExpr, columns: Vec<usize> },
    Distinct { input: RelExpr },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedRelDeltaState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    operator: MaterializedRelDeltaOperator,
    supports: MaterializedSetSupportState,
}

impl MaterializedRelDeltaState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        query.prepare(context, registry)?;
        match query {
            RelExpr::Project { input, columns } => {
                let input_type = input.typecheck(context, registry)?;
                if !matches!(
                    input_type.semantics,
                    kernel_schema::RelationSemantics::Set { .. }
                ) {
                    return Ok(None);
                }
                let result_type = query.typecheck(context, registry)?;
                let old_rows =
                    project_rows(input.evaluate(old, context, registry)?.into_rows(), columns)?;
                let supports =
                    MaterializedSetSupportState::build(&old_rows, result_type, context, registry)?;
                Ok(Some(Self {
                    query: query.clone(),
                    semantic_context: context.clone(),
                    operator: MaterializedRelDeltaOperator::ProjectSet {
                        input: input.as_ref().clone(),
                        columns: columns.clone(),
                    },
                    supports,
                }))
            }
            RelExpr::Distinct { input, .. } => {
                let result_type = query.typecheck(context, registry)?;
                let old_rows = input.evaluate(old, context, registry)?.into_rows();
                let supports =
                    MaterializedSetSupportState::build(&old_rows, result_type, context, registry)?;
                Ok(Some(Self {
                    query: query.clone(),
                    semantic_context: context.clone(),
                    operator: MaterializedRelDeltaOperator::Distinct {
                        input: input.as_ref().clone(),
                    },
                    supports,
                }))
            }
            _ => Ok(None),
        }
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub fn support_state(&self) -> &MaterializedSetSupportState {
        &self.supports
    }

    pub fn apply_model_change(
        &mut self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let (input, projected_columns) = match &self.operator {
            MaterializedRelDeltaOperator::ProjectSet { input, columns } => {
                (input, Some(columns.as_slice()))
            }
            MaterializedRelDeltaOperator::Distinct { input } => (input, None),
        };
        let input_delta = rel_delta_optimized(input, old, change, context, registry)?
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let (inserted, removed) = if let Some(columns) = projected_columns {
            (
                project_rows(input_delta.inserted, columns)?,
                project_rows(input_delta.removed, columns)?,
            )
        } else {
            (input_delta.inserted, input_delta.removed)
        };
        self.supports
            .apply_rows_delta(inserted, removed, context, registry)
    }
}
