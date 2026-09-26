//! Certified internal signed-delta carrier boundary.
//!
//! This module is intentionally execution-neutral in its first production
//! stage: existing maintained operators still consume `RelationDelta`.  The
//! types here establish one representation-independent ABI so later passes can
//! migrate internal edges without changing query semantics.

/// One weighted row in a finite signed delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Weighted<R> {
    pub weight: i64,
    pub row: R,
}

/// One row in the exact signed Γ-measure carrier.
///
/// Unlike [`Weighted`], the coefficient is not bounded by the machine delta
/// word. This is the coefficient domain required by bilinear maintained
/// operators such as Join, where multiplicities multiply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactWeighted<R> {
    pub weight: kernel_exact::ExactInteger,
    pub row: R,
}

/// Read-only interface for a finite signed Γ-measure over rows.
pub trait ExactDeltaView<R: ?Sized> {
    fn support_len(&self) -> usize;
    fn visit_exact(&self, visitor: impl FnMut(&kernel_exact::ExactInteger, &R));
}

/// Mutable construction boundary for exact signed Γ-measures.
pub trait ExactDeltaSink<R> {
    fn clear(&mut self);
    fn push_exact(&mut self, weight: kernel_exact::ExactInteger, row: R);
}

/// Exact-coefficient carrier used while migrating maintained operators away
/// from the legacy `i64` coefficient boundary.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExactDelta<R> {
    entries: Vec<ExactWeighted<R>>,
}

impl<R> ExactDelta<R> {
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
        }
    }

    pub fn push_scaled_i64(
        &mut self,
        weight: i64,
        multiplicity: &kernel_exact::ExactNatural,
        row: R,
    ) {
        let coefficient =
            kernel_exact::ExactInteger::from_i64(weight).scale_by_natural(multiplicity);
        if !coefficient.is_zero() {
            self.entries.push(ExactWeighted {
                weight: coefficient,
                row,
            });
        }
    }
}

impl<R> ExactDeltaSink<R> for ExactDelta<R> {
    fn clear(&mut self) {
        self.entries.clear();
    }

    fn push_exact(&mut self, weight: kernel_exact::ExactInteger, row: R) {
        if !weight.is_zero() {
            self.entries.push(ExactWeighted { weight, row });
        }
    }
}

impl<R> ExactDeltaView<R> for ExactDelta<R> {
    fn support_len(&self) -> usize {
        self.entries.len()
    }

    fn visit_exact(&self, mut visitor: impl FnMut(&kernel_exact::ExactInteger, &R)) {
        for entry in &self.entries {
            visitor(&entry.weight, &entry.row);
        }
    }
}

/// Stable identity of one compiled maintained-plan edge within a transition.
///
/// The ordinal is assigned by deterministic depth-first traversal of the
/// compiled maintained plan; the relation component prevents a prepared leaf
/// proof from being consumed by a different scan at the same ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompiledDeltaEdgeIdentity {
    ordinal: u32,
    relation: kernel_types::SemanticId,
}

impl CompiledDeltaEdgeIdentity {
    #[must_use]
    pub const fn new(ordinal: u32, relation: kernel_types::SemanticId) -> Self {
        Self { ordinal, relation }
    }

    #[must_use]
    pub const fn ordinal(self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub const fn relation(self) -> kernel_types::SemanticId {
        self.relation
    }
}

/// Validation evidence for one exact compiled transition edge.
///
/// Validation owns the already-computed source patch and the certified signed
/// effect.  Commit code consumes the frame instead of repeating source
/// membership/type work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedTransitionFrame<P, D> {
    edge: CompiledDeltaEdgeIdentity,
    prepared_source_patch: P,
    certified_effect: D,
}

impl<P, D> ValidatedTransitionFrame<P, D> {
    #[must_use]
    pub const fn new(
        edge: CompiledDeltaEdgeIdentity,
        prepared_source_patch: P,
        certified_effect: D,
    ) -> Self {
        Self {
            edge,
            prepared_source_patch,
            certified_effect,
        }
    }

    #[must_use]
    pub const fn edge(&self) -> CompiledDeltaEdgeIdentity {
        self.edge
    }

    pub fn into_parts(self) -> (P, D) {
        (self.prepared_source_patch, self.certified_effect)
    }
}

/// Read-only semantic interface for a finite signed delta.
///
/// Implementations own storage; consumers observe only `(weight, row)` pairs.
pub trait DeltaView<R: ?Sized> {
    fn support_len(&self) -> usize;
    fn visit(&self, visitor: impl FnMut(i64, &R));
}

/// Mutable construction boundary for a signed delta carrier.
pub trait DeltaSink<R> {
    fn clear(&mut self);
    fn push_weighted(&mut self, weight: i64, row: R);
}

/// Two-phase result produced by a stateful differential barrier.
///
/// `plan` is read-only over authoritative maintained state. The returned patch
/// may be committed only after the enclosing transition has succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDeltaEffect<P, D> {
    pub patch: P,
    pub effect: D,
}

/// Representation-independent unary state-kernel boundary.
pub trait UnaryDeltaKernel<I: ?Sized, O> {
    type Patch;
    type Output: DeltaView<O>;
    type Error;

    fn plan<D: DeltaView<I>>(
        &self,
        input: &D,
    ) -> Result<PlannedDeltaEffect<Self::Patch, Self::Output>, Self::Error>;

    fn commit(&mut self, patch: Self::Patch);
}

/// Representation-independent binary state-kernel boundary.
pub trait BinaryDeltaKernel<L: ?Sized, R: ?Sized, O> {
    type Patch;
    type Output: DeltaView<O>;
    type Error;

    fn plan<LD: DeltaView<L>, RD: DeltaView<R>>(
        &self,
        left: &LD,
        right: &RD,
    ) -> Result<PlannedDeltaEffect<Self::Patch, Self::Output>, Self::Error>;

    fn commit(&mut self, patch: Self::Patch);
}

/// Allocation-free common shapes used by maintained-query propagation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactDelta<R> {
    Empty,
    One(Weighted<R>),
    Replace { removed: R, inserted: R },
    Two(Weighted<R>, Weighted<R>),
}

impl<R> CompactDelta<R> {
    #[must_use]
    pub fn one(weight: i64, row: R) -> Self {
        Self::One(Weighted { weight, row })
    }

    #[must_use]
    pub fn replace(removed: R, inserted: R) -> Self {
        Self::Replace { removed, inserted }
    }
}

impl<R> DeltaView<R> for CompactDelta<R> {
    fn support_len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::One(_) => 1,
            Self::Replace { .. } | Self::Two(_, _) => 2,
        }
    }

    fn visit(&self, mut visitor: impl FnMut(i64, &R)) {
        match self {
            Self::Empty => {}
            Self::One(entry) => visitor(entry.weight, &entry.row),
            Self::Replace { removed, inserted } => {
                visitor(-1, removed);
                visitor(1, inserted);
            }
            Self::Two(first, second) => {
                visitor(first.weight, &first.row);
                visitor(second.weight, &second.row);
            }
        }
    }
}

/// Fixed-capacity inline storage used before an adaptive carrier spills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineDelta<R, const INLINE: usize> {
    len: usize,
    entries: [Option<Weighted<R>>; INLINE],
}

impl<R, const INLINE: usize> Default for InlineDelta<R, INLINE> {
    fn default() -> Self {
        Self {
            len: 0,
            entries: std::array::from_fn(|_| None),
        }
    }
}

impl<R, const INLINE: usize> InlineDelta<R, INLINE> {
    fn try_push(&mut self, entry: Weighted<R>) -> Result<(), Weighted<R>> {
        if self.len == INLINE {
            return Err(entry);
        }
        self.entries[self.len] = Some(entry);
        self.len += 1;
        Ok(())
    }

    fn drain_into(&mut self, destination: &mut Vec<Weighted<R>>) {
        for slot in &mut self.entries[..self.len] {
            destination.push(slot.take().expect("occupied inline delta slot"));
        }
        self.len = 0;
    }
}

impl<R, const INLINE: usize> DeltaView<R> for InlineDelta<R, INLINE> {
    fn support_len(&self) -> usize {
        self.len
    }

    fn visit(&self, mut visitor: impl FnMut(i64, &R)) {
        for entry in self.entries[..self.len].iter().flatten() {
            visitor(entry.weight, &entry.row);
        }
    }
}

/// Small-inline carrier with reusable spill storage for larger support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdaptiveDelta<R, const INLINE: usize = 2> {
    Inline(InlineDelta<R, INLINE>),
    Spill(Vec<Weighted<R>>),
}

impl<R, const INLINE: usize> Default for AdaptiveDelta<R, INLINE> {
    fn default() -> Self {
        Self::Inline(InlineDelta::default())
    }
}

impl<R, const INLINE: usize> AdaptiveDelta<R, INLINE> {
    #[must_use]
    pub fn with_spill_capacity(spill_capacity: usize) -> Self {
        Self::Spill(Vec::with_capacity(spill_capacity))
    }

    #[must_use]
    pub const fn is_spilled(&self) -> bool {
        matches!(self, Self::Spill(_))
    }

    #[must_use]
    pub fn spill_capacity(&self) -> Option<usize> {
        match self {
            Self::Inline(_) => None,
            Self::Spill(spill) => Some(spill.capacity()),
        }
    }
}

impl<R, const INLINE: usize> DeltaSink<R> for AdaptiveDelta<R, INLINE> {
    fn clear(&mut self) {
        match self {
            Self::Inline(inline) => {
                for slot in &mut inline.entries[..inline.len] {
                    *slot = None;
                }
                inline.len = 0;
            }
            Self::Spill(spill) => spill.clear(),
        }
    }

    fn push_weighted(&mut self, weight: i64, row: R) {
        let entry = Weighted { weight, row };
        match self {
            Self::Inline(inline) => {
                if let Err(extra) = inline.try_push(entry) {
                    let mut spill = Vec::with_capacity(INLINE.saturating_mul(2).max(4));
                    inline.drain_into(&mut spill);
                    spill.push(extra);
                    *self = Self::Spill(spill);
                }
            }
            Self::Spill(spill) => spill.push(entry),
        }
    }
}

impl<R, const INLINE: usize> DeltaView<R> for AdaptiveDelta<R, INLINE> {
    fn support_len(&self) -> usize {
        match self {
            Self::Inline(inline) => DeltaView::support_len(inline),
            Self::Spill(spill) => spill.len(),
        }
    }

    fn visit(&self, mut visitor: impl FnMut(i64, &R)) {
        match self {
            Self::Inline(inline) => inline.visit(visitor),
            Self::Spill(spill) => {
                for entry in spill {
                    visitor(entry.weight, &entry.row);
                }
            }
        }
    }
}

/// Zero-copy compatibility adapter over the current public `RelationDelta`.
pub struct RelationDeltaView<'a> {
    pub(crate) removed: &'a [crate::Row],
    pub(crate) inserted: &'a [crate::Row],
}

impl DeltaView<crate::Row> for RelationDeltaView<'_> {
    fn support_len(&self) -> usize {
        self.removed.len() + self.inserted.len()
    }

    fn visit(&self, mut visitor: impl FnMut(i64, &crate::Row)) {
        for row in self.removed {
            visitor(-1, row);
        }
        for row in self.inserted {
            visitor(1, row);
        }
    }
}

impl<R> ExactDeltaView<R> for CompactDelta<R> {
    fn support_len(&self) -> usize {
        DeltaView::support_len(self)
    }

    fn visit_exact(&self, mut visitor: impl FnMut(&kernel_exact::ExactInteger, &R)) {
        self.visit(|weight, row| {
            let exact = kernel_exact::ExactInteger::from_i64(weight);
            visitor(&exact, row);
        });
    }
}

impl<R, const INLINE: usize> ExactDeltaView<R> for InlineDelta<R, INLINE> {
    fn support_len(&self) -> usize {
        DeltaView::support_len(self)
    }

    fn visit_exact(&self, mut visitor: impl FnMut(&kernel_exact::ExactInteger, &R)) {
        self.visit(|weight, row| {
            let exact = kernel_exact::ExactInteger::from_i64(weight);
            visitor(&exact, row);
        });
    }
}

impl<R, const INLINE: usize> ExactDeltaView<R> for AdaptiveDelta<R, INLINE> {
    fn support_len(&self) -> usize {
        DeltaView::support_len(self)
    }

    fn visit_exact(&self, mut visitor: impl FnMut(&kernel_exact::ExactInteger, &R)) {
        self.visit(|weight, row| {
            let exact = kernel_exact::ExactInteger::from_i64(weight);
            visitor(&exact, row);
        });
    }
}

impl ExactDeltaView<crate::Row> for RelationDeltaView<'_> {
    fn support_len(&self) -> usize {
        DeltaView::support_len(self)
    }

    fn visit_exact(&self, mut visitor: impl FnMut(&kernel_exact::ExactInteger, &crate::Row)) {
        self.visit(|weight, row| {
            let exact = kernel_exact::ExactInteger::from_i64(weight);
            visitor(&exact, row);
        });
    }
}

#[cfg(test)]
mod tests {
    use kernel_model::Value;

    use super::*;
    use crate::{RelType, RelationDelta, Row};

    fn collect<R: Clone>(view: &impl DeltaView<R>) -> Vec<(i64, R)> {
        let mut out = Vec::new();
        view.visit(|weight, row| out.push((weight, row.clone())));
        out
    }

    #[test]
    fn compact_replace_is_the_same_signed_algebra_as_two_rows() {
        let compact = CompactDelta::replace(11_i64, 13_i64);
        assert_eq!(collect(&compact), vec![(-1, 11), (1, 13)]);
    }

    #[test]
    fn adaptive_delta_reuses_spill_after_clear() {
        let mut delta = AdaptiveDelta::<i64, 2>::with_spill_capacity(8);
        for row in 0..4 {
            delta.push_weighted(1, row);
        }
        assert!(delta.is_spilled());
        let capacity = delta.spill_capacity().unwrap();
        delta.clear();
        for row in 0..4 {
            delta.push_weighted(-1, row);
        }
        assert!(delta.is_spilled());
        assert_eq!(delta.spill_capacity(), Some(capacity));
    }

    #[test]
    fn exact_delta_scales_multiplicity_without_expanding_support() {
        let mut delta = ExactDelta::with_capacity(1);
        let multiplicity = kernel_exact::ExactNatural::from_u128(u128::MAX);
        delta.push_scaled_i64(i64::MAX, &multiplicity, 7_i64);
        assert_eq!(ExactDeltaView::support_len(&delta), 1);
        let mut seen = 0;
        delta.visit_exact(|weight, row| {
            assert!(!weight.is_zero());
            assert!(!weight.is_negative());
            assert_eq!(*row, 7);
            seen += 1;
        });
        assert_eq!(seen, 1);
    }

    #[test]
    fn relation_delta_view_is_zero_copy_and_representation_equivalent() {
        let removed: Row = vec![Value::I64(1)];
        let inserted: Row = vec![Value::I64(2)];
        let delta = RelationDelta {
            removed: vec![removed],
            inserted: vec![inserted],
            result_type: RelType {
                columns: Vec::new(),
                semantics: kernel_schema::RelationSemantics::Bag {
                    column_equivalences: Vec::new(),
                },
            },
        };
        let removed_ptr = delta.removed[0].as_ptr();
        let inserted_ptr = delta.inserted[0].as_ptr();
        let view = delta.as_delta_view();
        let mut seen = Vec::new();
        view.visit(|weight, row| seen.push((weight, row.as_ptr())));
        assert_eq!(seen, vec![(-1, removed_ptr), (1, inserted_ptr)]);
    }
}
