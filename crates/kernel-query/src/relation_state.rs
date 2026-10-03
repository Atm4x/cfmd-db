use super::{
    CanonicalRowKey, Change, ExactDelta, ExactDeltaSink, ExactDeltaView, FineChange,
    FineChangeKind, PlannedDeltaEffect, PreparedRewrite, PreparedStructuralRewrite, RelExpr,
    RelQueryError, RelType, RelationDeltaView, RelationValue, RewriteActionLaw, RewriteEffect,
    RewriteFootprint, RewriteSpec, Row, SemanticWriteCoordinate, StructuralRewriteEffect, Value,
    canonical_row_key, project_rows, rel_delta_optimized, relation_column_equivalences,
    relation_value_from_rows, unmatched_semantic_rows, validate_query_equivalence,
    value_shape_matches_type,
};
use kernel_persistent::{
    PersistentOrdMap, PersistentOrdMapStorageProbe, PersistentVec, PersistentVecStorageProbe,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Persistent FIFO of stable occurrences for one Γ-class.  The head offset
/// makes Bag removals follow logical survivor-order without O(class-size)
/// front shifts.  Periodic tail compaction keeps dead prefixes bounded and is
/// amortized linear across a deletion run.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OccurrenceBucket {
    handles: PersistentVec<kernel_types::StableRowHandle>,
    head: usize,
}

impl OccurrenceBucket {
    fn from_vec(handles: Vec<kernel_types::StableRowHandle>) -> Self {
        Self {
            handles: PersistentVec::from_vec(handles),
            head: 0,
        }
    }

    fn len(&self) -> usize {
        self.handles.len().saturating_sub(self.head)
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn iter(&self) -> impl Iterator<Item = &kernel_types::StableRowHandle> {
        self.handles.iter().skip(self.head)
    }

    fn push(&mut self, handle: kernel_types::StableRowHandle) {
        self.handles.push(handle);
    }

    fn pop_front(&mut self) -> Option<kernel_types::StableRowHandle> {
        let handle = self.handles.get(self.head).copied()?;
        self.head += 1;
        // Geometric compaction bounds retained dead prefixes while preserving
        // persistent sharing for short mutation runs.
        if self.head >= 64 && self.head.saturating_mul(2) >= self.handles.len() {
            self.handles = PersistentVec::from_vec(self.iter().copied().collect());
            self.head = 0;
        }
        Some(handle)
    }
}

impl<'a> IntoIterator for &'a OccurrenceBucket {
    type Item = &'a kernel_types::StableRowHandle;
    type IntoIter =
        std::iter::Skip<kernel_persistent::PersistentVecIter<'a, kernel_types::StableRowHandle>>;

    fn into_iter(self) -> Self::IntoIter {
        self.handles.iter().skip(self.head)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationDelta {
    pub inserted: Vec<Row>,
    pub removed: Vec<Row>,
    pub result_type: RelType,
}

/// Exact Γ-occurrence certificate emitted by relational execution together
/// with its result rows. Fresh output positions are already bound to stable
/// generation-zero handles, so a physical owner can adopt the persistent
/// occurrence root without canonicalizing the result again.
#[derive(Debug, Clone)]
pub struct RelationOccurrenceCertificate {
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    occurrences: PersistentOrdMap<CanonicalRowKey, OccurrenceBucket>,
    row_count: usize,
}

/// Opaque Γ evidence for one exact semantic row. The canonical key is public
/// only by reference; construction is sealed behind `RelationRowCanonicalizer`
/// so downstream storage code cannot forge a key and present it as certified
/// query evidence.
#[derive(Debug, Clone)]
pub struct CertifiedCanonicalRowKey {
    authority: Arc<RelationCanonicalAuthority>,
    key: CanonicalRowKey,
}

#[derive(Debug)]
struct RelationCanonicalAuthority {
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    canonicalizers: Vec<kernel_semantics::CompiledEquivalence>,
}

/// Compiled Γ authority for a single relation result type/context. Reusing one
/// instance lets an operator both make its semantic decision and emit sealed
/// row-aligned evidence without canonicalizing the same row again downstream.
#[derive(Debug, Clone)]
pub struct RelationRowCanonicalizer {
    authority: Arc<RelationCanonicalAuthority>,
}

impl CertifiedCanonicalRowKey {
    #[must_use]
    pub fn canonical_key(&self) -> &CanonicalRowKey {
        &self.key
    }
}

impl RelationRowCanonicalizer {
    pub fn compile(
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let canonicalizers =
            RelationBaseWitness::compile_canonicalizers(&result_type, context, registry)?;
        Ok(Self {
            authority: Arc::new(RelationCanonicalAuthority {
                result_type,
                semantic_context: context.clone(),
                canonicalizers,
            }),
        })
    }

    pub fn certify_row(&self, row: &Row) -> Result<CertifiedCanonicalRowKey, RelQueryError> {
        if row.len() != self.authority.result_type.columns.len()
            || !row
                .iter()
                .zip(&self.authority.result_type.columns)
                .all(|(value, ty)| value_shape_matches_type(value, ty))
        {
            return Err(RelQueryError::TypeMismatch);
        }
        let key = row
            .iter()
            .zip(&self.authority.canonicalizers)
            .map(|(value, equivalence)| equivalence.canonical_key(value).map_err(Into::into))
            .collect::<Result<Vec<_>, RelQueryError>>()?;
        Ok(CertifiedCanonicalRowKey {
            authority: Arc::clone(&self.authority),
            key,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationScanOccurrenceSeed {
    relation: kernel_types::SemanticId,
    result_type: RelType,
    semantic_context: kernel_schema::SemanticContext,
    canonical_keys_by_row: CanonicalRowEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CanonicalRowEvidence {
    Dense(PersistentVec<CanonicalRowKey>),
    StableOrder(PersistentOrdMap<kernel_types::StableRowHandle, CanonicalRowKey>),
}

impl CanonicalRowEvidence {
    pub(crate) fn from_dense(values: Vec<CanonicalRowKey>) -> Self {
        Self::Dense(PersistentVec::from_vec(values))
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Dense(values) => values.len(),
            Self::StableOrder(values) => values.len(),
        }
    }

    pub(crate) fn iter(&self) -> Box<dyn Iterator<Item = &CanonicalRowKey> + '_> {
        match self {
            Self::Dense(values) => Box::new(values.iter()),
            Self::StableOrder(values) => Box::new(values.values()),
        }
    }

    fn dense_mut(&mut self) -> Result<&mut PersistentVec<CanonicalRowKey>, RelQueryError> {
        match self {
            Self::Dense(values) => Ok(values),
            Self::StableOrder(_) => Err(RelQueryError::StructuralRewriteBaseMismatch),
        }
    }
}

impl RelationScanOccurrenceSeed {
    #[must_use]
    pub const fn relation(&self) -> kernel_types::SemanticId {
        self.relation
    }

    #[must_use]
    pub const fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub const fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        &self.semantic_context
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.canonical_keys_by_row.len()
    }

    /// Returns true only when this execution seed is a persistent view of the
    /// exact logical-order evidence owned by `witness`, rather than an
    /// independently rebuilt row-key vector.
    #[must_use]
    pub fn shares_logical_order_root_with(&self, witness: &RelationBaseWitness) -> bool {
        match &self.canonical_keys_by_row {
            CanonicalRowEvidence::StableOrder(values) => {
                values.shares_root_with(&witness.ordered_occurrences)
            }
            CanonicalRowEvidence::Dense(_) => false,
        }
    }

    pub(crate) fn canonical_keys_by_row(&self) -> CanonicalRowEvidence {
        self.canonical_keys_by_row.clone()
    }

    /// Advances row-aligned Γ evidence with the same survivor-order + append
    /// law used by logical relation delta application. Unchanged rows are never
    /// re-canonicalized; only delta rows cross Γ again.
    #[cfg(test)]
    pub(crate) fn advance_logical_delta(
        &self,
        delta: &RelationDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if delta.result_type != self.result_type || context != &self.semantic_context {
            return Err(RelQueryError::StructuralRewriteBaseMismatch);
        }
        let equivalences = relation_column_equivalences(&self.result_type);
        let mut removals = BTreeMap::<CanonicalRowKey, usize>::new();
        for row in &delta.removed {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            *removals.entry(key).or_default() += 1;
        }

        let CanonicalRowEvidence::Dense(current) = &self.canonical_keys_by_row else {
            return Err(RelQueryError::StructuralRewriteBaseMismatch);
        };
        let mut next = Vec::with_capacity(
            current
                .len()
                .saturating_sub(delta.removed.len())
                .saturating_add(delta.inserted.len()),
        );
        for key in current {
            let remove = removals.get_mut(key).is_some_and(|count| {
                if *count == 0 {
                    false
                } else {
                    *count -= 1;
                    true
                }
            });
            if !remove {
                next.push(key.clone());
            }
        }
        if removals.values().any(|count| *count != 0) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        for row in &delta.inserted {
            next.push(canonical_row_key(row, equivalences, context, registry)?);
        }
        Ok(Self {
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            canonical_keys_by_row: CanonicalRowEvidence::from_dense(next),
        })
    }
    pub fn apply_prepared_storage_transition<I>(
        &mut self,
        prepared: &PreparedRelationRewrite<I>,
        removed_positions: &[usize],
    ) -> Result<(), RelQueryError> {
        if prepared.relation() != self.relation
            || removed_positions.len() != prepared.removed_occurrence_keys().len()
            || prepared.inserted_occurrence_keys().len()
                != prepared.inserted_occurrence_handles().len()
        {
            return Err(RelQueryError::StructuralRewriteBaseMismatch);
        }
        for (&position, expected_key) in removed_positions
            .iter()
            .zip(prepared.removed_occurrence_keys())
        {
            let dense = self.canonical_keys_by_row.dense_mut()?;
            let actual = dense
                .get(position)
                .ok_or(RelQueryError::StructuralRewriteBaseMismatch)?;
            if actual != expected_key {
                return Err(RelQueryError::StructuralRewriteBaseMismatch);
            }
            dense.swap_remove(position);
        }
        for key in prepared.inserted_occurrence_keys() {
            self.canonical_keys_by_row.dense_mut()?.push(key.clone());
        }
        Ok(())
    }
}

impl RelationOccurrenceCertificate {
    pub fn from_dense_certified_keys(
        certified: Vec<CertifiedCanonicalRowKey>,
    ) -> Result<Self, RelQueryError> {
        let Some(first) = certified.first() else {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        };
        let authority = Arc::clone(&first.authority);
        if certified
            .iter()
            .any(|item| !Arc::ptr_eq(&item.authority, &authority))
        {
            return Err(RelQueryError::StructuralRewriteBaseMismatch);
        }
        let row_count = certified.len();
        let mut grouped = BTreeMap::<CanonicalRowKey, Vec<kernel_types::StableRowHandle>>::new();
        for (slot, item) in certified.into_iter().enumerate() {
            grouped
                .entry(item.key)
                .or_default()
                .push(kernel_types::StableRowHandle {
                    slot,
                    generation: 0,
                });
        }
        if matches!(
            authority.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) && grouped.values().any(|handles| handles.len() != 1)
        {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        Self::from_sorted_occurrences(
            authority.result_type.clone(),
            &authority.semantic_context,
            grouped.into_iter().collect(),
            row_count,
        )
    }

    #[allow(
        clippy::needless_pass_by_value,
        reason = "Preserve the existing value-taking boundary contract."
    )]
    pub(crate) fn from_dense_set_keys(
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        canonical_keys_by_row: CanonicalRowEvidence,
    ) -> Result<Self, RelQueryError> {
        let row_count = canonical_keys_by_row.len();
        let strictly_sorted = canonical_keys_by_row
            .iter()
            .zip(canonical_keys_by_row.iter().skip(1))
            .all(|(left, right)| left < right);
        let mut grouped = canonical_keys_by_row
            .iter()
            .cloned()
            .enumerate()
            .map(|(slot, key)| {
                (
                    key,
                    vec![kernel_types::StableRowHandle {
                        slot,
                        generation: 0,
                    }],
                )
            })
            .collect::<Vec<_>>();
        if !strictly_sorted {
            grouped.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            if grouped.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        Self::from_sorted_occurrences(result_type, context, grouped, row_count)
    }

    pub(crate) fn from_sorted_occurrences(
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        grouped: Vec<(CanonicalRowKey, Vec<kernel_types::StableRowHandle>)>,
        row_count: usize,
    ) -> Result<Self, RelQueryError> {
        let occurrences = PersistentOrdMap::from_sorted_unique_owned(
            grouped
                .into_iter()
                .map(|(key, handles)| (key, OccurrenceBucket::from_vec(handles)))
                .collect(),
        )
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        if occurrences
            .values()
            .map(OccurrenceBucket::len)
            .sum::<usize>()
            != row_count
        {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        Ok(Self {
            result_type,
            semantic_context: context.clone(),
            occurrences,
            row_count,
        })
    }

    #[must_use]
    pub const fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    #[must_use]
    pub fn shares_occurrence_root_with(&self, witness: &RelationBaseWitness) -> bool {
        self.occurrences.shares_root_with(&witness.occurrences)
    }
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
    canonicalizers: Vec<kernel_semantics::CompiledEquivalence>,
    occurrences: PersistentOrdMap<CanonicalRowKey, OccurrenceBucket>,
    ordered_occurrences: PersistentOrdMap<kernel_types::StableRowHandle, CanonicalRowKey>,
    next_occurrence_slot: usize,
    authority: Arc<()>,
}

/// Restricted Γ-support authority projected from a [`RelationBaseWitness`].
///
/// This carrier deliberately does not expose scan-order/physical-position
/// evidence.  That distinction matters when historical support is recovered
/// by reversing exact deltas: multiplicity/equivalence support is exactly
/// reversible, while the old stable-handle order is not reconstructible from
/// a value-only durable delta.  Formation-world delta validation needs only
/// the former.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationSupportWitness {
    witness: RelationBaseWitness,
}

impl RelationSupportWitness {
    #[must_use]
    pub const fn relation(&self) -> kernel_types::SemanticId {
        self.witness.relation
    }

    #[must_use]
    pub const fn result_type(&self) -> &RelType {
        &self.witness.result_type
    }

    #[must_use]
    pub const fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        &self.witness.semantic_context
    }

    /// Applies an exact value delta to support only.  The successor remains a
    /// restricted support witness; synthesized handles never escape as scan
    /// evidence.
    pub fn advance_exact(
        &self,
        target_revision: kernel_types::RevisionId,
        delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        Ok(Self {
            witness: self.witness.advance(target_revision, delta, registry)?,
        })
    }

    /// Reverses one exact durable relation delta at Γ-support level.
    pub fn rewind_exact(
        &self,
        source_revision: kernel_types::RevisionId,
        forward_delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let inverse = RelationDelta {
            inserted: forward_delta.removed.clone(),
            removed: forward_delta.inserted.clone(),
            result_type: forward_delta.result_type.clone(),
        };
        self.advance_exact(source_revision, &inverse, registry)
    }

    /// Checks that an exact delta is defined in this Γ-support world without
    /// constructing or materializing a relation value.
    pub fn validate_exact_delta(
        &self,
        delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(), RelQueryError> {
        self.witness
            .apply_delta_supports(delta, registry)
            .map(|_| ())
    }
}

/// Structural persistent-node accounting for one relation witness. This is a
/// kernel diagnostic used by retention/GC hostile tests; it deliberately
/// counts persistent data-structure nodes rather than attempting allocator
/// byte accounting for semantic values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelationWitnessStorageStats {
    pub occurrence_map_nodes: usize,
    pub ordered_map_nodes: usize,
    pub bucket_vector_nodes: usize,
}

/// Exact persistent-source patch derived from a relation base witness and one
/// semantic delta. Removal ordinals are expressed in the source logical row
/// order; the successor witness is the same Γ-canonical support authority
/// advanced by that delta.
#[derive(Debug, Clone)]
pub struct RelationBaseDeltaAdvance {
    removed_positions: Vec<usize>,
    successor: RelationBaseWitness,
}

impl RelationBaseDeltaAdvance {
    #[must_use]
    pub fn removed_positions(&self) -> &[usize] {
        &self.removed_positions
    }

    #[must_use]
    pub const fn successor(&self) -> &RelationBaseWitness {
        &self.successor
    }

    #[must_use]
    pub fn into_successor(self) -> RelationBaseWitness {
        self.successor
    }
}

impl RelationWitnessStorageStats {
    #[must_use]
    pub const fn total_nodes(self) -> usize {
        self.occurrence_map_nodes
            .saturating_add(self.ordered_map_nodes)
            .saturating_add(self.bucket_vector_nodes)
    }
}

/// Weak probe for persistent nodes unique to one witness relative to a direct
/// successor. Holding the probe never prolongs the lifetime of those nodes.
#[derive(Debug)]
pub struct RelationWitnessStorageProbe {
    occurrence_map: PersistentOrdMapStorageProbe<CanonicalRowKey, OccurrenceBucket>,
    ordered_map: PersistentOrdMapStorageProbe<kernel_types::StableRowHandle, CanonicalRowKey>,
    bucket_vectors: Vec<PersistentVecStorageProbe<kernel_types::StableRowHandle>>,
}

impl RelationWitnessStorageProbe {
    #[must_use]
    pub fn total_nodes(&self) -> usize {
        self.occurrence_map
            .total_nodes()
            .saturating_add(self.ordered_map.total_nodes())
            .saturating_add(
                self.bucket_vectors
                    .iter()
                    .map(PersistentVecStorageProbe::total_nodes)
                    .sum::<usize>(),
            )
    }

    #[must_use]
    pub fn live_nodes(&self) -> usize {
        self.occurrence_map
            .live_nodes()
            .saturating_add(self.ordered_map.live_nodes())
            .saturating_add(
                self.bucket_vectors
                    .iter()
                    .map(PersistentVecStorageProbe::live_nodes)
                    .sum::<usize>(),
            )
    }

    #[must_use]
    pub fn is_fully_reclaimed(&self) -> bool {
        self.live_nodes() == 0
    }
}

struct RelationSupportTransition {
    occurrences: PersistentOrdMap<CanonicalRowKey, OccurrenceBucket>,
    ordered_occurrences: PersistentOrdMap<kernel_types::StableRowHandle, CanonicalRowKey>,
    next_occurrence_slot: usize,
    removed_keys: Vec<CanonicalRowKey>,
    inserted_keys: Vec<CanonicalRowKey>,
    removed_handles: Vec<kernel_types::StableRowHandle>,
    inserted_handles: Vec<kernel_types::StableRowHandle>,
}

impl PartialEq for RelationBaseWitness {
    fn eq(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.relation == other.relation
            && self.result_type == other.result_type
            && self.semantic_context == other.semantic_context
            && self.occurrences == other.occurrences
            && self.ordered_occurrences == other.ordered_occurrences
            && self.next_occurrence_slot == other.next_occurrence_slot
    }
}

impl Eq for RelationBaseWitness {}

impl RelationBaseWitness {
    /// O(1) restricted projection of this persistent Γ occurrence root for
    /// support-only validation/history transport.
    #[must_use]
    pub fn support_witness(&self) -> RelationSupportWitness {
        RelationSupportWitness {
            witness: self.clone(),
        }
    }

    /// Residualizes a certified stale Set intent against this current Γ root
    /// without scanning/materializing the relation. Exact current row values
    /// needed by removals are fetched only for the touched Γ classes through
    /// `row_at_logical_position`.
    pub fn residualize_delta_against_support<F>(
        &self,
        delta: &RelationDelta,
        mut row_at_logical_position: F,
    ) -> Result<RelationDelta, RelQueryError>
    where
        F: FnMut(usize) -> Option<Row>,
    {
        if delta.result_type != self.result_type {
            return Err(RelQueryError::TypeMismatch);
        }
        if !matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) {
            return Ok(delta.clone());
        }

        let removed_keys = delta
            .removed
            .iter()
            .map(|row| self.canonical_key(row))
            .collect::<Result<Vec<_>, _>>()?;
        let inserted_keys = delta
            .inserted
            .iter()
            .map(|row| self.canonical_key(row))
            .collect::<Result<Vec<_>, _>>()?;
        let removed_set = removed_keys.iter().cloned().collect::<BTreeSet<_>>();
        let inserted_set = inserted_keys.iter().cloned().collect::<BTreeSet<_>>();
        if !removed_set.is_disjoint(&inserted_set) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }

        let mut class_state = BTreeMap::<CanonicalRowKey, bool>::new();
        let mut removed = Vec::new();
        for key in removed_keys {
            let present = *class_state
                .entry(key.clone())
                .or_insert_with(|| self.occurrences.contains_key(&key));
            if !present {
                continue;
            }
            let handle = self
                .occurrences
                .get(&key)
                .and_then(|bucket| bucket.iter().next().copied())
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let position = self
                .ordered_occurrences
                .rank_of(&handle)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            removed.push(
                row_at_logical_position(position)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
            );
            class_state.insert(key, false);
        }

        let mut inserted = Vec::new();
        for (row, key) in delta.inserted.iter().zip(inserted_keys) {
            let present = *class_state
                .entry(key.clone())
                .or_insert_with(|| self.occurrences.contains_key(&key));
            if present {
                continue;
            }
            inserted.push(row.clone());
            class_state.insert(key, true);
        }

        Ok(RelationDelta {
            inserted,
            removed,
            result_type: self.result_type.clone(),
        })
    }

    /// Rebinds this exact semantic relation to a fresh dense generation-zero
    /// storage-handle domain in the exact physical row order certified by
    /// `seed`, without canonicalizing any row again.
    ///
    /// This is intentionally seed-driven rather than witness-logical-order
    /// driven: a physical overlay may use swap-remove order while the semantic
    /// witness preserves survivor-order + append. Compaction must bind the
    /// semantic classes to the rows it actually materialized.
    pub fn rebind_dense_storage_identity_from_seed(
        &self,
        revision: kernel_types::RevisionId,
        seed: &RelationScanOccurrenceSeed,
    ) -> Result<Self, RelQueryError> {
        if seed.relation != self.relation
            || seed.result_type != self.result_type
            || seed.semantic_context != self.semantic_context
        {
            return Err(RelQueryError::StructuralRewriteBaseMismatch);
        }
        let row_count = seed.row_count();
        let mut ordered = Vec::with_capacity(row_count);
        let mut grouped = BTreeMap::<CanonicalRowKey, Vec<kernel_types::StableRowHandle>>::new();
        for (slot, key) in seed.canonical_keys_by_row.iter().enumerate() {
            let handle = kernel_types::StableRowHandle {
                slot,
                generation: 0,
            };
            ordered.push((handle, key.clone()));
            grouped.entry(key.clone()).or_default().push(handle);
        }
        if matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) && grouped.values().any(|handles| handles.len() != 1)
        {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let ordered_occurrences = PersistentOrdMap::from_sorted_unique_owned(ordered)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let occurrences = PersistentOrdMap::from_sorted_unique_owned(
            grouped
                .into_iter()
                .map(|(key, handles)| (key, OccurrenceBucket::from_vec(handles)))
                .collect(),
        )
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        Ok(Self {
            revision,
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            canonicalizers: self.canonicalizers.clone(),
            occurrences,
            ordered_occurrences,
            next_occurrence_slot: row_count,
            authority: Arc::new(()),
        })
    }

    /// Structural persistent-node footprint reachable from this witness.
    #[must_use]
    pub fn storage_stats(&self) -> RelationWitnessStorageStats {
        RelationWitnessStorageStats {
            occurrence_map_nodes: self.occurrences.structural_node_count(),
            ordered_map_nodes: self.ordered_occurrences.structural_node_count(),
            bucket_vector_nodes: self
                .occurrences
                .values()
                .map(|bucket| bucket.handles.structural_node_count())
                .sum(),
        }
    }

    /// Exact number of persistent nodes newly introduced by this witness
    /// relative to a direct predecessor. For a lineage produced only by
    /// persistent path-copy transitions, summing this value across successors
    /// gives the exact structural-node growth retained by pinning the lineage.
    #[must_use]
    pub fn structural_nodes_new_since(&self, predecessor: &Self) -> usize {
        let occurrence_map_nodes = self.occurrences.structural_node_count().saturating_sub(
            self.occurrences
                .shared_structural_node_count_with(&predecessor.occurrences),
        );
        let ordered_map_nodes = self
            .ordered_occurrences
            .structural_node_count()
            .saturating_sub(
                self.ordered_occurrences
                    .shared_structural_node_count_with(&predecessor.ordered_occurrences),
            );
        let mut bucket_vector_nodes = 0usize;
        for (key, bucket) in &self.occurrences {
            let nodes = bucket.handles.structural_node_count();
            bucket_vector_nodes = bucket_vector_nodes.saturating_add(
                predecessor.occurrences.get(key).map_or(nodes, |previous| {
                    nodes.saturating_sub(
                        bucket
                            .handles
                            .shared_structural_node_count_with(&previous.handles),
                    )
                }),
            );
        }
        occurrence_map_nodes
            .saturating_add(ordered_map_nodes)
            .saturating_add(bucket_vector_nodes)
    }

    /// Weak probe for nodes reachable only from this witness when compared to
    /// a direct successor. Once all older roots are dropped these nodes must be
    /// reclaimed; the probe itself cannot keep them alive.
    #[must_use]
    pub fn unique_storage_probe_against(&self, successor: &Self) -> RelationWitnessStorageProbe {
        let occurrence_map = self
            .occurrences
            .unique_storage_probe_against(&successor.occurrences);
        let ordered_map = self
            .ordered_occurrences
            .unique_storage_probe_against(&successor.ordered_occurrences);
        let mut bucket_vectors = Vec::new();
        for (key, bucket) in &self.occurrences {
            if let Some(next) = successor.occurrences.get(key) {
                let probe = bucket.handles.unique_storage_probe_against(&next.handles);
                if probe.total_nodes() != 0 {
                    bucket_vectors.push(probe);
                }
            } else {
                let empty = PersistentVec::<kernel_types::StableRowHandle>::default();
                let probe = bucket.handles.unique_storage_probe_against(&empty);
                if probe.total_nodes() != 0 {
                    bucket_vectors.push(probe);
                }
            }
        }
        RelationWitnessStorageProbe {
            occurrence_map,
            ordered_map,
            bucket_vectors,
        }
    }

    pub fn from_occurrence_certificate(
        revision: kernel_types::RevisionId,
        relation: kernel_types::SemanticId,
        certificate: RelationOccurrenceCertificate,
        target_type: RelType,
        target_context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if certificate.result_type != target_type {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let source_canonicalizers = Self::compile_canonicalizers(
            &certificate.result_type,
            &certificate.semantic_context,
            registry,
        )?;
        let canonicalizers = Self::compile_canonicalizers(&target_type, target_context, registry)?;
        if source_canonicalizers != canonicalizers {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let ordered_occurrences = Self::ordered_occurrences_from_support(
            &certificate.occurrences,
            certificate.row_count,
        )?;
        Ok(Self {
            revision,
            relation,
            result_type: target_type,
            semantic_context: target_context.clone(),
            canonicalizers,
            occurrences: certificate.occurrences,
            ordered_occurrences,
            next_occurrence_slot: certificate.row_count,
            authority: Arc::new(()),
        })
    }

    pub fn build(
        revision: kernel_types::RevisionId,
        relation: kernel_types::SemanticId,
        rows: &[Row],
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let canonicalizers = Self::compile_canonicalizers(&result_type, context, registry)?;
        let mut keyed_occurrences = Vec::with_capacity(rows.len());
        for (slot, row) in rows.iter().enumerate() {
            if row.len() != result_type.columns.len()
                || !row
                    .iter()
                    .zip(&result_type.columns)
                    .all(|(value, ty)| value_shape_matches_type(value, ty))
            {
                return Err(RelQueryError::TypeMismatch);
            }
            let key = row
                .iter()
                .zip(&canonicalizers)
                .map(|(value, equivalence)| equivalence.canonical_key(value).map_err(Into::into))
                .collect::<Result<Vec<_>, RelQueryError>>()?;
            keyed_occurrences.push((
                key,
                kernel_types::StableRowHandle {
                    slot,
                    generation: 0,
                },
            ));
        }
        Self::from_keyed_occurrences(
            revision,
            relation,
            result_type,
            context,
            canonicalizers,
            keyed_occurrences,
            rows.len(),
        )
    }

    pub fn build_columnar(
        revision: kernel_types::RevisionId,
        relation: kernel_types::SemanticId,
        row_count: usize,
        columns: &[&[Value]],
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if columns.len() != result_type.columns.len()
            || columns.iter().any(|column| column.len() != row_count)
        {
            return Err(RelQueryError::TypeMismatch);
        }
        let canonicalizers = Self::compile_canonicalizers(&result_type, context, registry)?;
        let mut keyed_occurrences = Vec::with_capacity(row_count);
        for slot in 0..row_count {
            let mut key = Vec::with_capacity(columns.len());
            for (ordinal, ((column, ty), equivalence)) in columns
                .iter()
                .zip(&result_type.columns)
                .zip(&canonicalizers)
                .enumerate()
            {
                let value = column.get(slot).ok_or(RelQueryError::TypeMismatch)?;
                let _ = ordinal;
                if !value_shape_matches_type(value, ty) {
                    return Err(RelQueryError::TypeMismatch);
                }
                key.push(
                    equivalence
                        .canonical_key(value)
                        .map_err(RelQueryError::from)?,
                );
            }
            keyed_occurrences.push((
                key,
                kernel_types::StableRowHandle {
                    slot,
                    generation: 0,
                },
            ));
        }
        Self::from_keyed_occurrences(
            revision,
            relation,
            result_type,
            context,
            canonicalizers,
            keyed_occurrences,
            row_count,
        )
    }

    fn compile_canonicalizers(
        result_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Vec<kernel_semantics::CompiledEquivalence>, RelQueryError> {
        let equivalences = relation_column_equivalences(result_type);
        if equivalences.len() != result_type.columns.len() {
            return Err(RelQueryError::TypeMismatch);
        }
        for (column_type, equivalence) in result_type.columns.iter().zip(equivalences) {
            validate_query_equivalence(*equivalence, column_type, context, registry)?;
        }
        equivalences
            .iter()
            .map(|equivalence| {
                registry
                    .compile_equivalence(context, *equivalence)
                    .map_err(Into::into)
            })
            .collect()
    }

    fn from_keyed_occurrences(
        revision: kernel_types::RevisionId,
        relation: kernel_types::SemanticId,
        result_type: RelType,
        context: &kernel_schema::SemanticContext,
        canonicalizers: Vec<kernel_semantics::CompiledEquivalence>,
        mut keyed_occurrences: Vec<(CanonicalRowKey, kernel_types::StableRowHandle)>,
        row_count: usize,
    ) -> Result<Self, RelQueryError> {
        let set_semantics = matches!(
            result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        );
        let mut ordered = keyed_occurrences
            .iter()
            .map(|(key, handle)| (*handle, key.clone()))
            .collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|(handle, _)| *handle);
        let ordered_occurrences = PersistentOrdMap::from_sorted_unique_owned(ordered)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;

        keyed_occurrences.sort_unstable_by(|left, right| {
            left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1))
        });
        let mut grouped = Vec::<(CanonicalRowKey, Vec<kernel_types::StableRowHandle>)>::new();
        for (key, handle) in keyed_occurrences {
            if let Some((last_key, handles)) = grouped.last_mut()
                && *last_key == key
            {
                if set_semantics {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                handles.push(handle);
            } else {
                grouped.push((key, vec![handle]));
            }
        }
        let occurrences = PersistentOrdMap::from_sorted_unique_owned(
            grouped
                .into_iter()
                .map(|(key, handles)| (key, OccurrenceBucket::from_vec(handles)))
                .collect(),
        )
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        Ok(Self {
            revision,
            relation,
            result_type,
            semantic_context: context.clone(),
            canonicalizers,
            occurrences,
            ordered_occurrences,
            next_occurrence_slot: row_count,
            authority: Arc::new(()),
        })
    }

    fn ordered_occurrences_from_support(
        occurrences: &PersistentOrdMap<CanonicalRowKey, OccurrenceBucket>,
        row_count: usize,
    ) -> Result<PersistentOrdMap<kernel_types::StableRowHandle, CanonicalRowKey>, RelQueryError>
    {
        let mut ordered = Vec::with_capacity(row_count);
        for (key, handles) in occurrences {
            for handle in handles {
                ordered.push((*handle, key.clone()));
            }
        }
        if ordered.len() != row_count {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        ordered.sort_unstable_by_key(|(handle, _)| *handle);
        PersistentOrdMap::from_sorted_unique_owned(ordered)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    }

    /// Returns execution-facing row evidence directly from this witness
    /// authority. Occurrence buckets remove the oldest live handle, matching
    /// logical survivor-order for both Set and Bag; stable slots are monotone
    /// creation ordinals, so ordered live occurrences are survivor-order + append.
    pub fn logical_scan_occurrence_seed(
        &self,
    ) -> Result<RelationScanOccurrenceSeed, RelQueryError> {
        Ok(RelationScanOccurrenceSeed {
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            canonical_keys_by_row: CanonicalRowEvidence::StableOrder(
                self.ordered_occurrences.clone(),
            ),
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
    pub const fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        &self.semantic_context
    }

    /// Returns true when two witnesses share the exact persistent Γ-occurrence
    /// root rather than merely describing extensionally equal support.
    #[must_use]
    pub fn shares_occurrence_root_with(&self, other: &Self) -> bool {
        self.occurrences.shares_root_with(&other.occurrences)
    }

    pub fn scan_occurrence_seed(
        &self,
        handles_by_row: &[kernel_types::StableRowHandle],
    ) -> Result<RelationScanOccurrenceSeed, RelQueryError> {
        let live_count = self
            .occurrences
            .values()
            .map(OccurrenceBucket::len)
            .sum::<usize>();
        if handles_by_row.len() != live_count {
            return Err(RelQueryError::StructuralRewriteBaseMismatch);
        }

        let dense_limit = live_count.saturating_mul(4).saturating_add(1024);
        let canonical_keys_by_row = if self.next_occurrence_slot <= dense_limit {
            let mut by_slot = vec![None; self.next_occurrence_slot];
            for (key, handles) in &self.occurrences {
                for handle in handles {
                    let entry = by_slot
                        .get_mut(handle.slot)
                        .ok_or(RelQueryError::StructuralRewriteBaseMismatch)?;
                    if entry.is_some() {
                        return Err(RelQueryError::StructuralRewriteBaseMismatch);
                    }
                    *entry = Some((handle.generation, key.clone()));
                }
            }
            let mut keys = Vec::with_capacity(handles_by_row.len());
            for handle in handles_by_row {
                let (generation, key) = by_slot
                    .get_mut(handle.slot)
                    .and_then(Option::take)
                    .ok_or(RelQueryError::StructuralRewriteBaseMismatch)?;
                if generation != handle.generation {
                    return Err(RelQueryError::StructuralRewriteBaseMismatch);
                }
                keys.push(key);
            }
            if by_slot.into_iter().any(|entry| entry.is_some()) {
                return Err(RelQueryError::StructuralRewriteBaseMismatch);
            }
            keys
        } else {
            let mut by_handle = BTreeMap::new();
            for (key, handles) in &self.occurrences {
                for handle in handles {
                    if by_handle.insert(*handle, key.clone()).is_some() {
                        return Err(RelQueryError::StructuralRewriteBaseMismatch);
                    }
                }
            }
            let mut keys = Vec::with_capacity(handles_by_row.len());
            for handle in handles_by_row {
                keys.push(
                    by_handle
                        .remove(handle)
                        .ok_or(RelQueryError::StructuralRewriteBaseMismatch)?,
                );
            }
            if !by_handle.is_empty() {
                return Err(RelQueryError::StructuralRewriteBaseMismatch);
            }
            keys
        };

        Ok(RelationScanOccurrenceSeed {
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            canonical_keys_by_row: CanonicalRowEvidence::from_dense(canonical_keys_by_row),
        })
    }

    /// Certifies the identity storage-handle domain used by a freshly
    /// factorized current relation. This check deliberately does not
    /// canonicalize any row: the Γ root was already built by this witness.
    #[must_use]
    pub fn certifies_identity_storage_handles(
        &self,
        handles: &[kernel_types::StableRowHandle],
    ) -> bool {
        self.next_occurrence_slot == handles.len()
            && handles
                .iter()
                .copied()
                .enumerate()
                .all(|(slot, handle)| handle.slot == slot && handle.generation == 0)
            && self
                .occurrences
                .values()
                .map(OccurrenceBucket::len)
                .sum::<usize>()
                == handles.len()
    }

    fn canonical_key(&self, row: &Row) -> Result<CanonicalRowKey, RelQueryError> {
        if row.len() != self.canonicalizers.len() {
            return Err(RelQueryError::EquivalenceArityMismatch);
        }
        row.iter()
            .zip(&self.canonicalizers)
            .map(|(value, equivalence)| equivalence.canonical_key(value).map_err(Into::into))
            .collect()
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
        self.occurrences == other.occurrences
            && self.next_occurrence_slot == other.next_occurrence_slot
    }

    fn apply_delta_supports(
        &self,
        delta: &RelationDelta,
        _registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationSupportTransition, RelQueryError> {
        if delta.result_type != self.result_type {
            return Err(RelQueryError::TypeMismatch);
        }
        let removed_keys = delta
            .removed
            .iter()
            .map(|row| self.canonical_key(row))
            .collect::<Result<Vec<_>, _>>()?;
        let inserted_keys = delta
            .inserted
            .iter()
            .map(|row| self.canonical_key(row))
            .collect::<Result<Vec<_>, _>>()?;
        self.apply_delta_support_keys(delta, &removed_keys, &inserted_keys)
    }

    fn apply_delta_support_keys(
        &self,
        delta: &RelationDelta,
        removed_keys: &[CanonicalRowKey],
        inserted_keys: &[CanonicalRowKey],
    ) -> Result<RelationSupportTransition, RelQueryError> {
        if delta.result_type != self.result_type
            || removed_keys.len() != delta.removed.len()
            || inserted_keys.len() != delta.inserted.len()
        {
            return Err(RelQueryError::TypeMismatch);
        }
        let set_semantics = matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        );
        let mut occurrences = self.occurrences.clone();
        let mut ordered_occurrences = self.ordered_occurrences.clone();
        let mut next_occurrence_slot = self.next_occurrence_slot;
        let mut removed_handles = Vec::with_capacity(delta.removed.len());
        for key in removed_keys {
            let mut bucket = occurrences
                .get(key)
                .cloned()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let handle = bucket
                .pop_front()
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if bucket.is_empty() {
                occurrences.remove(key);
            } else {
                occurrences.insert(key.clone(), bucket);
            }
            let removed_key = ordered_occurrences
                .remove(&handle)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            if removed_key != *key {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            removed_handles.push(handle);
        }
        let mut inserted_handles = Vec::with_capacity(delta.inserted.len());
        for key in inserted_keys {
            let mut bucket = occurrences
                .get(key)
                .cloned()
                .unwrap_or_else(|| OccurrenceBucket::from_vec(Vec::new()));
            if set_semantics && !bucket.is_empty() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let handle = kernel_types::StableRowHandle {
                slot: next_occurrence_slot,
                generation: 0,
            };
            next_occurrence_slot = next_occurrence_slot
                .checked_add(1)
                .ok_or(RelQueryError::DerivedIdentityExhausted)?;
            bucket.push(handle);
            occurrences.insert(key.clone(), bucket);
            if ordered_occurrences.insert(handle, key.clone()).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            inserted_handles.push(handle);
        }
        Ok(RelationSupportTransition {
            occurrences,
            ordered_occurrences,
            next_occurrence_slot,
            removed_keys: removed_keys.to_vec(),
            inserted_keys: inserted_keys.to_vec(),
            removed_handles,
            inserted_handles,
        })
    }

    pub fn advance(
        &self,
        target_revision: kernel_types::RevisionId,
        delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        Ok(self
            .advance_with_source_positions(target_revision, delta, registry)?
            .into_successor())
    }

    /// Advances this exact support witness and simultaneously resolves every
    /// removed occurrence to its source logical ordinal in O(delta log N).
    /// No relation row buffer is materialized or scanned.
    pub fn advance_with_source_positions(
        &self,
        target_revision: kernel_types::RevisionId,
        delta: &RelationDelta,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationBaseDeltaAdvance, RelQueryError> {
        let transition = self.apply_delta_supports(delta, registry)?;
        let mut removed_positions = transition
            .removed_handles
            .iter()
            .map(|handle| {
                self.ordered_occurrences
                    .rank_of(handle)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)
            })
            .collect::<Result<Vec<_>, _>>()?;
        removed_positions.sort_unstable();
        if removed_positions.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let successor = Self {
            revision: target_revision,
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            canonicalizers: self.canonicalizers.clone(),
            occurrences: transition.occurrences,
            ordered_occurrences: transition.ordered_occurrences,
            next_occurrence_slot: transition.next_occurrence_slot,
            authority: Arc::new(()),
        };
        Ok(RelationBaseDeltaAdvance {
            removed_positions,
            successor,
        })
    }

    pub fn advance_storage_resolved(
        &self,
        target_revision: kernel_types::RevisionId,
        resolved: &StorageResolvedRelationDelta,
    ) -> Result<Self, RelQueryError> {
        if resolved.relation != self.relation || resolved.semantic_context != self.semantic_context
        {
            return Err(RelQueryError::StructuralRewriteBaseMismatch);
        }
        let transition = self.apply_delta_support_keys(
            &resolved.delta,
            &resolved.removed_keys,
            &resolved.inserted_keys,
        )?;
        Ok(Self {
            revision: target_revision,
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            canonicalizers: self.canonicalizers.clone(),
            occurrences: transition.occurrences,
            ordered_occurrences: transition.ordered_occurrences,
            next_occurrence_slot: transition.next_occurrence_slot,
            authority: Arc::new(()),
        })
    }

    pub(crate) fn advance_with_expected_handles(
        &self,
        target_revision: kernel_types::RevisionId,
        delta: &RelationDelta,
        removed_handles: &[kernel_types::StableRowHandle],
        inserted_handles: &[kernel_types::StableRowHandle],
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Self>, RelQueryError> {
        let transition = self.apply_delta_supports(delta, registry)?;
        if transition.removed_handles != removed_handles
            || transition.inserted_handles != inserted_handles
        {
            return Ok(None);
        }
        Ok(Some(Self {
            revision: target_revision,
            relation: self.relation,
            result_type: self.result_type.clone(),
            semantic_context: self.semantic_context.clone(),
            canonicalizers: self.canonicalizers.clone(),
            occurrences: transition.occurrences,
            ordered_occurrences: transition.ordered_occurrences,
            next_occurrence_slot: transition.next_occurrence_slot,
            authority: Arc::new(()),
        }))
    }
}

impl<I> PreparedRelationRewrite<I> {
    #[must_use]
    pub fn relation(&self) -> kernel_types::SemanticId {
        self.rewrite.effect().base.relation()
    }

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

    #[must_use]
    pub fn base_witness(&self) -> &RelationBaseWitness {
        &self.rewrite.effect().base
    }

    #[must_use]
    pub fn removed_occurrence_handles(&self) -> &[kernel_types::StableRowHandle] {
        &self.rewrite.effect().removed_handles
    }

    #[must_use]
    pub fn inserted_occurrence_handles(&self) -> &[kernel_types::StableRowHandle] {
        &self.rewrite.effect().inserted_handles
    }

    #[must_use]
    pub fn removed_occurrence_keys(&self) -> &[CanonicalRowKey] {
        &self.rewrite.effect().removed_keys
    }

    #[must_use]
    pub fn inserted_occurrence_keys(&self) -> &[CanonicalRowKey] {
        &self.rewrite.effect().inserted_keys
    }

    #[must_use]
    pub fn advanced_base_witness(
        &self,
        target_revision: kernel_types::RevisionId,
    ) -> RelationBaseWitness {
        self.rewrite.effect().advanced_base_witness(target_revision)
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
    next_occurrences: PersistentOrdMap<CanonicalRowKey, OccurrenceBucket>,
    next_ordered_occurrences: PersistentOrdMap<kernel_types::StableRowHandle, CanonicalRowKey>,
    next_occurrence_slot: usize,
    removed_keys: Vec<CanonicalRowKey>,
    inserted_keys: Vec<CanonicalRowKey>,
    removed_handles: Vec<kernel_types::StableRowHandle>,
    inserted_handles: Vec<kernel_types::StableRowHandle>,
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
            next_occurrences: transition.occurrences,
            next_ordered_occurrences: transition.ordered_occurrences,
            next_occurrence_slot: transition.next_occurrence_slot,
            removed_keys: transition.removed_keys,
            inserted_keys: transition.inserted_keys,
            removed_handles: transition.removed_handles,
            inserted_handles: transition.inserted_handles,
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

    #[must_use]
    fn advanced_base_witness(
        &self,
        target_revision: kernel_types::RevisionId,
    ) -> RelationBaseWitness {
        RelationBaseWitness {
            revision: target_revision,
            relation: self.base.relation,
            result_type: self.base.result_type.clone(),
            semantic_context: self.base.semantic_context.clone(),
            canonicalizers: self.base.canonicalizers.clone(),
            occurrences: self.next_occurrences.clone(),
            ordered_occurrences: self.next_ordered_occurrences.clone(),
            next_occurrence_slot: self.next_occurrence_slot,
            authority: Arc::new(()),
        }
    }

    fn rewrite_footprint(&self, relation: kernel_types::SemanticId) -> RewriteFootprint {
        let mut footprint = RewriteFootprint::default();
        for key in &self.removed_keys {
            record_relation_class_action(
                &mut footprint,
                relation,
                key,
                relation_class_action(&self.base.result_type.semantics, false),
            );
        }
        for key in &self.inserted_keys {
            record_relation_class_action(
                &mut footprint,
                relation,
                key,
                relation_class_action(&self.base.result_type.semantics, true),
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
    semantic_context: kernel_schema::SemanticContext,
    removed_keys: Vec<CanonicalRowKey>,
    inserted_keys: Vec<CanonicalRowKey>,
}

impl StorageResolvedRelationDelta {
    pub fn from_parts(
        relation: kernel_types::SemanticId,
        delta: RelationDelta,
        removed_handles: Vec<kernel_types::StableRowHandle>,
        inserted_handles: Vec<kernel_types::StableRowHandle>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if removed_handles.len() != delta.removed.len()
            || inserted_handles.len() != delta.inserted.len()
        {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let equivalences = relation_column_equivalences(&delta.result_type);
        let removed_keys = delta
            .removed
            .iter()
            .map(|row| canonical_row_key(row, equivalences, context, registry))
            .collect::<Result<Vec<_>, _>>()?;
        let inserted_keys = delta
            .inserted
            .iter()
            .map(|row| canonical_row_key(row, equivalences, context, registry))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            relation,
            delta,
            removed_handles,
            inserted_handles,
            semantic_context: context.clone(),
            removed_keys,
            inserted_keys,
        })
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

    pub(super) fn inserted_keys(&self) -> &[CanonicalRowKey] {
        &self.inserted_keys
    }

    #[must_use]
    pub fn semantic_context(&self) -> &kernel_schema::SemanticContext {
        &self.semantic_context
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
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations."
)]
pub(super) enum MaintainedScanCommitPatch {
    Semantic(RelationMutationPlan),
    StorageResolved(StorageResolvedScanPatch),
}

#[derive(Debug)]
pub(super) struct StorageResolvedScanPatch {
    pub(super) removed_handles: Vec<kernel_types::StableRowHandle>,
    pub(super) inserted: Vec<(kernel_types::StableRowHandle, CanonicalRowKey, Row)>,
    pub(super) next_base_witness: Option<RelationBaseWitness>,
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

fn relation_class_action(
    semantics: &kernel_schema::RelationSemantics,
    present: bool,
) -> RewriteActionLaw {
    match semantics {
        kernel_schema::RelationSemantics::Set { .. } => {
            if present {
                RewriteActionLaw::EnsurePresent
            } else {
                RewriteActionLaw::EnsureAbsent
            }
        }
        // Bag classes carry exact multiplicity, not boolean presence. Until the rewrite law
        // records the signed multiplicity delta/algebra, overlapping bag-class writes require
        // generic coordination rather than being weakened into set semantics.
        kernel_schema::RelationSemantics::Bag { .. } => RewriteActionLaw::Opaque,
    }
}

fn record_relation_class_action(
    footprint: &mut RewriteFootprint,
    relation: kernel_types::SemanticId,
    key: &CanonicalRowKey,
    action: RewriteActionLaw,
) {
    use std::collections::btree_map::Entry;

    let coordinate = SemanticWriteCoordinate::RelationClass {
        relation,
        canonical_key: kernel_semantics::encode_canonical_eq_key_tuple(key).into_boxed_slice(),
    };
    match footprint.writes.entry(coordinate) {
        Entry::Vacant(entry) => {
            entry.insert(action);
        }
        Entry::Occupied(mut entry) if entry.get() != &action => {
            entry.insert(RewriteActionLaw::Opaque);
        }
        Entry::Occupied(_) => {}
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
        for row in &self.removed {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            record_relation_class_action(
                &mut footprint,
                relation,
                &key,
                relation_class_action(&self.result_type.semantics, false),
            );
        }
        for row in &self.inserted {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            record_relation_class_action(
                &mut footprint,
                relation,
                &key,
                relation_class_action(&self.result_type.semantics, true),
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

    /// Residualizes this exact relation effect against a newer relation value.
    ///
    /// Set deltas denote presence intents (`EnsurePresent` / `EnsureAbsent`) in
    /// the rewrite-law layer.  When a stale transition has already been
    /// certified coordination-free, an intervening equal presence intent may
    /// therefore have satisfied part of the original effect.  This method
    /// removes exactly those already-satisfied set actions using the pinned Γ
    /// canonical classes.  Bag deltas retain their exact multiplicity delta;
    /// overlapping bag writes are not classified as coordination-free by the
    /// rewrite-law engine.
    pub fn residualize_against(
        &self,
        current: &RelationValue,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        if !matches!(
            self.result_type.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) {
            return Ok(self.clone());
        }

        let equivalences = relation_column_equivalences(&self.result_type);
        let mut current_by_key = BTreeMap::<CanonicalRowKey, Row>::new();
        for row in current.rows() {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            if current_by_key.insert(key, row.clone()).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }

        let removed_keys = self
            .removed
            .iter()
            .map(|row| canonical_row_key(row, equivalences, context, registry))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let inserted_keys = self
            .inserted
            .iter()
            .map(|row| canonical_row_key(row, equivalences, context, registry))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if !removed_keys.is_disjoint(&inserted_keys) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }

        let mut removed = Vec::new();
        for row in &self.removed {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            if let Some(existing) = current_by_key.remove(&key) {
                removed.push(existing);
            }
        }

        let mut inserted = Vec::new();
        for row in &self.inserted {
            let key = canonical_row_key(row, equivalences, context, registry)?;
            if let std::collections::btree_map::Entry::Vacant(e) = current_by_key.entry(key) {
                e.insert(row.clone());
                inserted.push(row.clone());
            }
        }

        Ok(Self {
            inserted,
            removed,
            result_type: self.result_type.clone(),
        })
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
    _registry: &kernel_semantics::SemanticRegistry,
) -> Result<RelationValue, RelQueryError> {
    let expected_set = matches!(
        effect.delta.result_type.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    );
    if expected_set != matches!(old, RelationValue::Set { .. }) {
        return Err(RelQueryError::StructuralRewriteBaseMismatch);
    }

    let mut support_counts = BTreeMap::<CanonicalRowKey, usize>::new();
    let mut row_keys = Vec::with_capacity(old.rows().len());
    for row in old.rows() {
        let key = effect.base.canonical_key(row)?;
        *support_counts.entry(key.clone()).or_default() += 1;
        row_keys.push(key);
    }
    if support_counts.len() != effect.base.occurrences.len()
        || !support_counts.iter().all(|(key, count)| {
            effect.base.occurrences.get(key).map(OccurrenceBucket::len) == Some(*count)
        })
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
