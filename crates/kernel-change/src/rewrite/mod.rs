mod coordination;
mod laws;
mod residual;

pub use coordination::*;
pub use laws::*;
pub use residual::*;

use kernel_types::SemanticId;
use std::sync::Arc;

use crate::change::{Change, FineChange, FineChangeKind, SeqChangeError, SeqSplice};
use crate::stable_seq::{
    PreparedStableSeqSnapshot, StableSeqOccurrence, StableSeqRewriteError, StableSeqRewriteIntent,
    StableSeqSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteSpecId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteLawSetId(pub SemanticId);

/// Extensional effect produced by a Rewrite intent. No-op is represented by
/// absence of a prepared rewrite rather than by erasing its intent identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteEffect<T> {
    Fine(FineChange<T>),
    Replace(T),
}

impl<T> RewriteEffect<T> {
    /// Borrows the exact extensional endpoint carried by this effect.
    ///
    /// `FineChange` is currently endpoint-backed, so callers that only need to
    /// validate the represented result must not materialize another `T` via
    /// `apply`. This accessor is representation-level only; it does not grant
    /// semantic authority beyond the prepared Rewrite that owns the effect.
    #[must_use]
    pub const fn endpoint(&self) -> &T {
        match self {
            Self::Fine(fine) => fine.endpoint(),
            Self::Replace(next) => next,
        }
    }
}

impl<T: Clone> RewriteEffect<T> {
    #[must_use]
    pub fn apply(&self, old: &T) -> T {
        match self {
            Self::Fine(fine) => fine.apply(old),
            Self::Replace(next) => next.clone(),
        }
    }

    #[must_use]
    pub fn into_change(self) -> Change<T> {
        match self {
            Self::Fine(fine) => Change::Fine(fine),
            Self::Replace(next) => Change::Replace(next),
        }
    }
}

/// Intent-bearing prepared rewrite. `I` is the explicit-input representation
/// chosen by the owning layer (often `Value`); it is deliberately generic so
/// kernel-change does not depend on model/storage representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRewrite<T, I = ()> {
    pub spec: RewriteSpecId,
    pub explicit_inputs: Vec<I>,
    pub effect: RewriteEffect<T>,
    pub law_set: RewriteLawSetId,
}

/// Immutable shared authority for an already prepared Rewrite.
///
/// Proof graphs frequently need to retain the same certified rewrite along
/// several certificate edges. Sharing the prepared authority keeps those
/// graph copies O(1) with respect to the endpoint `T`; the underlying
/// `PreparedRewrite` remains immutable and is still the sole semantic value.
#[derive(Debug, PartialEq, Eq)]
pub struct SharedPreparedRewrite<T, I = ()>(Arc<PreparedRewrite<T, I>>);

impl<T, I> Clone for SharedPreparedRewrite<T, I> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T, I> SharedPreparedRewrite<T, I> {
    #[must_use]
    pub fn new(rewrite: PreparedRewrite<T, I>) -> Self {
        Self(Arc::new(rewrite))
    }

    #[must_use]
    pub fn as_rewrite(&self) -> &PreparedRewrite<T, I> {
        self.0.as_ref()
    }

    #[must_use]
    pub fn endpoint(&self) -> &T {
        self.0.endpoint()
    }
}

impl<T, I> From<PreparedRewrite<T, I>> for SharedPreparedRewrite<T, I> {
    fn from(value: PreparedRewrite<T, I>) -> Self {
        Self::new(value)
    }
}

impl<T, I> std::ops::Deref for SharedPreparedRewrite<T, I> {
    type Target = PreparedRewrite<T, I>;

    fn deref(&self) -> &Self::Target {
        self.as_rewrite()
    }
}

impl<T, I> PreparedRewrite<T, I> {
    /// Borrows the authoritative extensional endpoint already certified by
    /// this prepared Rewrite. Proof layers should prefer this accessor over
    /// `apply()` when they only need endpoint equality: endpoint-backed
    /// Rewrite effects do not inspect the supplied base.
    #[must_use]
    pub const fn endpoint(&self) -> &T {
        self.effect.endpoint()
    }
}

pub trait PreparedRewriteIntent {
    fn rewrite_spec(&self) -> RewriteSpecId;
    fn rewrite_law_set(&self) -> RewriteLawSetId;
}

impl<T, I> PreparedRewriteIntent for PreparedRewrite<T, I> {
    fn rewrite_spec(&self) -> RewriteSpecId {
        self.spec
    }

    fn rewrite_law_set(&self) -> RewriteLawSetId {
        self.law_set
    }
}

impl<T, I> PreparedRewriteIntent for SharedPreparedRewrite<T, I> {
    fn rewrite_spec(&self) -> RewriteSpecId {
        self.spec
    }

    fn rewrite_law_set(&self) -> RewriteLawSetId {
        self.law_set
    }
}

pub type PreparedStableSeqRewrite<T> =
    PreparedRewrite<Vec<StableSeqOccurrence<T>>, StableSeqRewriteIntent<T>>;

/// Structural effect which can derive an exact extensional endpoint from an
/// authoritative base without storing that endpoint eagerly.
///
/// This is the representation-independent preparation boundary: sequence
/// splices are the first production implementation, but Rewrite semantics do
/// not depend on a sequence-specific lazy path. Consumers that require an
/// extensional `PreparedRewrite` materialize exactly once at their
/// certification boundary.
pub trait StructuralRewriteEffect<T, C: ?Sized = ()> {
    type Error;

    fn apply_structural_with(&self, old: &T, context: &C) -> Result<T, Self::Error>;
}

impl<T: Clone> StructuralRewriteEffect<Vec<T>> for SeqSplice<T> {
    type Error = SeqChangeError;

    fn apply_structural_with(&self, old: &Vec<T>, (): &()) -> Result<Vec<T>, Self::Error> {
        self.apply(old)
    }
}

/// Intent-bearing Rewrite whose structural effect is certified but whose full
/// endpoint has not yet been materialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedStructuralRewrite<T, I, E, C: ?Sized = ()> {
    spec: RewriteSpecId,
    explicit_inputs: Vec<I>,
    effect: E,
    law_set: RewriteLawSetId,
    fine_kind: FineChangeKind,
    target: std::marker::PhantomData<fn(&C) -> T>,
}

impl<T, I, E, C: ?Sized> PreparedStructuralRewrite<T, I, E, C> {
    #[must_use]
    pub const fn spec(&self) -> RewriteSpecId {
        self.spec
    }

    #[must_use]
    pub const fn law_set(&self) -> RewriteLawSetId {
        self.law_set
    }

    #[must_use]
    pub fn explicit_inputs(&self) -> &[I] {
        &self.explicit_inputs
    }

    #[must_use]
    pub const fn effect(&self) -> &E {
        &self.effect
    }
}

impl<T, I, E, C: ?Sized> PreparedRewriteIntent for PreparedStructuralRewrite<T, I, E, C> {
    fn rewrite_spec(&self) -> RewriteSpecId {
        self.spec
    }

    fn rewrite_law_set(&self) -> RewriteLawSetId {
        self.law_set
    }
}

impl<T, I, E, C: ?Sized> PreparedStructuralRewrite<T, I, E, C>
where
    E: StructuralRewriteEffect<T, C>,
{
    pub fn apply_structural_with(&self, old: &T, context: &C) -> Result<T, E::Error> {
        self.effect.apply_structural_with(old, context)
    }

    pub fn materialize_with(self, old: &T, context: &C) -> Result<PreparedRewrite<T, I>, E::Error> {
        let endpoint = self.effect.apply_structural_with(old, context)?;
        Ok(PreparedRewrite {
            spec: self.spec,
            explicit_inputs: self.explicit_inputs,
            effect: RewriteEffect::Fine(FineChange::new(self.fine_kind, endpoint)),
            law_set: self.law_set,
        })
    }
}

impl<T, I, E> PreparedStructuralRewrite<T, I, E>
where
    E: StructuralRewriteEffect<T>,
{
    pub fn apply_structural(&self, old: &T) -> Result<T, E::Error> {
        self.apply_structural_with(old, &())
    }

    pub fn materialize(self, old: &T) -> Result<PreparedRewrite<T, I>, E::Error> {
        self.materialize_with(old, &())
    }
}

pub type PreparedStableSeqStructuralRewrite<T> = PreparedStructuralRewrite<
    Vec<StableSeqOccurrence<T>>,
    StableSeqRewriteIntent<T>,
    SeqSplice<StableSeqOccurrence<T>>,
>;

impl<T: Clone, I> PreparedRewrite<T, I> {
    #[must_use]
    pub fn apply(&self, old: &T) -> T {
        self.effect.apply(old)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteSpec {
    pub id: RewriteSpecId,
    pub law_set: RewriteLawSetId,
    pub footprint: RewriteFootprint,
}

impl RewriteSpec {
    #[must_use]
    pub fn prepare<T, I>(
        &self,
        explicit_inputs: Vec<I>,
        effect: RewriteEffect<T>,
    ) -> PreparedRewrite<T, I> {
        PreparedRewrite {
            spec: self.id,
            explicit_inputs,
            effect,
            law_set: self.law_set,
        }
    }

    #[must_use]
    pub fn pair_law_with(&self, other: &Self) -> PairRewriteLaw {
        infer_pair_rewrite_law(&self.footprint, &other.footprint)
    }

    #[must_use]
    pub fn prepare_structural<T, I, E>(
        &self,
        explicit_inputs: Vec<I>,
        effect: E,
        fine_kind: FineChangeKind,
    ) -> PreparedStructuralRewrite<T, I, E>
    where
        E: StructuralRewriteEffect<T>,
    {
        PreparedStructuralRewrite {
            spec: self.id,
            explicit_inputs,
            effect,
            law_set: self.law_set,
            fine_kind,
            target: std::marker::PhantomData,
        }
    }

    #[must_use]
    pub fn prepare_structural_with_context<T, I, E, C: ?Sized>(
        &self,
        explicit_inputs: Vec<I>,
        effect: E,
        fine_kind: FineChangeKind,
    ) -> PreparedStructuralRewrite<T, I, E, C>
    where
        E: StructuralRewriteEffect<T, C>,
    {
        PreparedStructuralRewrite {
            spec: self.id,
            explicit_inputs,
            effect,
            law_set: self.law_set,
            fine_kind,
            target: std::marker::PhantomData,
        }
    }

    /// Resolves stable semantic sequence intent against one authoritative
    /// snapshot and preserves that intent as the explicit input of the
    /// prepared Rewrite. The resulting effect remains extensional while the
    /// durable/concurrent meaning is carried by stable occurrence/gap IDs.
    pub fn prepare_stable_seq<T: Clone>(
        &self,
        snapshot: &StableSeqSnapshot<T>,
        intent: StableSeqRewriteIntent<T>,
    ) -> Result<PreparedStableSeqRewrite<T>, StableSeqRewriteError> {
        let prepared_snapshot = snapshot.prepare()?;
        self.prepare_stable_seq_on(&prepared_snapshot, intent)
    }

    pub fn prepare_stable_seq_on<T: Clone>(
        &self,
        snapshot: &PreparedStableSeqSnapshot<'_, T>,
        intent: StableSeqRewriteIntent<T>,
    ) -> Result<PreparedStableSeqRewrite<T>, StableSeqRewriteError> {
        self.prepare_stable_seq_structural_on(snapshot, intent)?
            .materialize(&snapshot.snapshot().occurrences)
            .map_err(|_| StableSeqRewriteError::ResolvedSpliceInvalid)
    }

    /// Prepares stable sequence intent without eagerly cloning/materializing
    /// the full sequence endpoint. The returned value is a normal structural
    /// Rewrite and can be materialized once when an endpoint-owning consumer
    /// actually requires it.
    pub fn prepare_stable_seq_structural_on<T: Clone>(
        &self,
        snapshot: &PreparedStableSeqSnapshot<'_, T>,
        intent: StableSeqRewriteIntent<T>,
    ) -> Result<PreparedStableSeqStructuralRewrite<T>, StableSeqRewriteError> {
        if !self
            .footprint
            .conservatively_covers(&intent.rewrite_footprint())
        {
            return Err(StableSeqRewriteError::FootprintMismatch);
        }
        let splice = snapshot.resolve_intent(&intent)?;
        Ok(self.prepare_structural(vec![intent], splice, FineChangeKind::Seq))
    }
}
