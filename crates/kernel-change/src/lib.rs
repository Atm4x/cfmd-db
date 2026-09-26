use std::collections::{BTreeMap, BTreeSet};

use kernel_types::{EqClassId, RevisionObservableId, SemanticId};

/// Universal logical change. `Fine` is an extensional refinement of the same
/// endpoint semantics; it does not become authority for semantic equality by
/// virtue of its structural tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change<T> {
    NoChange,
    Replace(T),
    Fine(FineChange<T>),
}

impl<T: Clone> Change<T> {
    #[must_use]
    pub fn apply(&self, old: &T) -> T {
        match self {
            Self::NoChange => old.clone(),
            Self::Replace(new) => new.clone(),
            Self::Fine(fine) => fine.apply(old),
        }
    }

    /// Extensional composition. Fine changes carry an absolute endpoint, so a
    /// later non-empty change supersedes the earlier endpoint without needing
    /// representation-specific delta algebra.
    #[must_use]
    pub fn compose(self, later: Self) -> Self {
        match later {
            Self::NoChange => self,
            other => other,
        }
    }
}

impl<T: Clone + PartialEq> Change<T> {
    /// Removes a semantically *representation-level* no-op against an exact
    /// endpoint. Callers using coarser Γ equality must canonicalize/prove that
    /// equivalence before invoking this helper.
    #[must_use]
    pub fn normalize_exact(self, old: &T) -> Self {
        match &self {
            Self::NoChange => Self::NoChange,
            Self::Replace(next) if next == old => Self::NoChange,
            Self::Fine(fine) if fine.endpoint() == old => Self::NoChange,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FineChangeKind {
    Scalar,
    Product,
    Sum,
    Option,
    Set,
    Bag,
    Seq,
    Map,
    Relation,
    Recursive,
    Other(SemanticId),
}

/// Revision-local Γ class coordinate used by structural collection changes.
///
/// The observable pins the semantic quotient used to classify values. The
/// class id alone is intentionally insufficient because `EqClassId` is local
/// to one revision observable catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticClassCoordinate {
    pub observable: RevisionObservableId,
    pub class: EqClassId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticCollectionChangeError {
    ObservableMismatch,
    DuplicateSourceClass(SemanticClassCoordinate),
    MissingRemovedClass(SemanticClassCoordinate),
    ConflictingSetClass(SemanticClassCoordinate),
    BagRemovalExceedsMultiplicity {
        coordinate: SemanticClassCoordinate,
        available: u64,
        removed: u64,
    },
    BagMultiplicityOverflow(SemanticClassCoordinate),
    DuplicateMapKeyClass(SemanticClassCoordinate),
}

/// Γ-aware Set patch over revision-local semantic classes.
///
/// Values are carried only for newly inserted classes. Existing values are
/// addressed by semantic class, never by Rust equality/hash identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticSetChange<T> {
    pub observable: RevisionObservableId,
    pub inserted: BTreeMap<EqClassId, T>,
    pub removed: BTreeSet<EqClassId>,
}

impl<T: Clone> SemanticSetChange<T> {
    pub fn apply_classified(
        &self,
        old: impl IntoIterator<Item = (SemanticClassCoordinate, T)>,
    ) -> Result<BTreeMap<EqClassId, T>, SemanticCollectionChangeError> {
        let mut next = BTreeMap::new();
        for (coordinate, value) in old {
            if coordinate.observable != self.observable {
                return Err(SemanticCollectionChangeError::ObservableMismatch);
            }
            if next.insert(coordinate.class, value).is_some() {
                return Err(SemanticCollectionChangeError::DuplicateSourceClass(
                    coordinate,
                ));
            }
        }
        for class in &self.removed {
            let coordinate = SemanticClassCoordinate {
                observable: self.observable,
                class: *class,
            };
            if self.inserted.contains_key(class) {
                return Err(SemanticCollectionChangeError::ConflictingSetClass(
                    coordinate,
                ));
            }
            if next.remove(class).is_none() {
                return Err(SemanticCollectionChangeError::MissingRemovedClass(
                    coordinate,
                ));
            }
        }
        for (&class, value) in &self.inserted {
            next.entry(class).or_insert_with(|| value.clone());
        }
        Ok(next)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticBagClassChange<T> {
    pub representative: Option<T>,
    pub inserted: u64,
    pub removed: u64,
}

/// Γ-aware Bag patch. Multiplicity is changed per semantic class; a concrete
/// representative is required only when an insertion can create a new class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticBagChange<T> {
    pub observable: RevisionObservableId,
    pub classes: BTreeMap<EqClassId, SemanticBagClassChange<T>>,
}

impl<T: Clone> SemanticBagChange<T> {
    pub fn apply_classified(
        &self,
        old: impl IntoIterator<Item = (SemanticClassCoordinate, T, u64)>,
    ) -> Result<BTreeMap<EqClassId, (T, u64)>, SemanticCollectionChangeError> {
        let mut next = BTreeMap::<EqClassId, (T, u64)>::new();
        for (coordinate, value, count) in old {
            if coordinate.observable != self.observable {
                return Err(SemanticCollectionChangeError::ObservableMismatch);
            }
            if let Some((_, existing_count)) = next.get_mut(&coordinate.class) {
                *existing_count = existing_count.checked_add(count).ok_or(
                    SemanticCollectionChangeError::BagMultiplicityOverflow(coordinate),
                )?;
            } else {
                next.insert(coordinate.class, (value, count));
            }
        }

        for (&class, change) in &self.classes {
            let coordinate = SemanticClassCoordinate {
                observable: self.observable,
                class,
            };
            let available = next.get(&class).map_or(0, |(_, count)| *count);
            if change.removed > available {
                return Err(
                    SemanticCollectionChangeError::BagRemovalExceedsMultiplicity {
                        coordinate,
                        available,
                        removed: change.removed,
                    },
                );
            }
            let after_remove = available - change.removed;
            let after_insert = after_remove.checked_add(change.inserted).ok_or(
                SemanticCollectionChangeError::BagMultiplicityOverflow(coordinate),
            )?;
            if after_insert == 0 {
                next.remove(&class);
                continue;
            }
            let representative = match (next.get(&class), &change.representative) {
                (Some((existing, _)), _) => existing.clone(),
                (None, Some(representative)) => representative.clone(),
                (None, None) => {
                    return Err(SemanticCollectionChangeError::MissingRemovedClass(
                        coordinate,
                    ));
                }
            };
            next.insert(class, (representative, after_insert));
        }
        Ok(next)
    }
}

/// Γ-aware Map patch keyed by semantic key classes. Upsert is the only way to
/// replace a value for an existing class; remove+upsert of the same class is
/// rejected so intent does not depend on operation ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticMapChange<K, V> {
    pub key_observable: RevisionObservableId,
    pub upserted: BTreeMap<EqClassId, (K, V)>,
    pub removed: BTreeSet<EqClassId>,
}

impl<K: Clone, V: Clone> SemanticMapChange<K, V> {
    pub fn apply_classified(
        &self,
        old: impl IntoIterator<Item = (SemanticClassCoordinate, K, V)>,
    ) -> Result<BTreeMap<EqClassId, (K, V)>, SemanticCollectionChangeError> {
        let mut next = BTreeMap::new();
        for (coordinate, key, value) in old {
            if coordinate.observable != self.key_observable {
                return Err(SemanticCollectionChangeError::ObservableMismatch);
            }
            if next.insert(coordinate.class, (key, value)).is_some() {
                return Err(SemanticCollectionChangeError::DuplicateMapKeyClass(
                    coordinate,
                ));
            }
        }
        for class in &self.removed {
            let coordinate = SemanticClassCoordinate {
                observable: self.key_observable,
                class: *class,
            };
            if self.upserted.contains_key(class) {
                return Err(SemanticCollectionChangeError::DuplicateMapKeyClass(
                    coordinate,
                ));
            }
            if next.remove(class).is_none() {
                return Err(SemanticCollectionChangeError::MissingRemovedClass(
                    coordinate,
                ));
            }
        }
        for (&class, pair) in &self.upserted {
            next.insert(class, pair.clone());
        }
        Ok(next)
    }
}

/// First-class extensional fine effect.
///
/// `endpoint` is authoritative for universal `apply`; `kind` records which
/// structural calculus produced the refinement. This makes `FineChange` total
/// today while allowing Γ-aware Set/Bag/Map/Relation payloads to migrate in
/// without changing `Change<T>` again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FineChange<T> {
    kind: FineChangeKind,
    endpoint: T,
}

impl<T> FineChange<T> {
    #[must_use]
    pub const fn new(kind: FineChangeKind, endpoint: T) -> Self {
        Self { kind, endpoint }
    }

    #[must_use]
    pub const fn kind(&self) -> FineChangeKind {
        self.kind
    }

    #[must_use]
    pub const fn endpoint(&self) -> &T {
        &self.endpoint
    }

    #[must_use]
    pub fn into_endpoint(self) -> T {
        self.endpoint
    }
}

impl<T: Clone> FineChange<T> {
    #[must_use]
    pub fn apply(&self, _old: &T) -> T {
        self.endpoint.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetChange<T> {
    pub inserted: BTreeSet<T>,
    pub removed: BTreeSet<T>,
}

impl<T: Clone + Ord> SetChange<T> {
    #[must_use]
    pub fn apply(&self, old: &BTreeSet<T>) -> BTreeSet<T> {
        let mut next = old.clone();
        for value in &self.removed {
            next.remove(value);
        }
        next.extend(self.inserted.iter().cloned());
        next
    }

    #[must_use]
    pub fn between(old: &BTreeSet<T>, new: &BTreeSet<T>) -> Self {
        Self {
            inserted: new.difference(old).cloned().collect(),
            removed: old.difference(new).cloned().collect(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inserted.is_empty() && self.removed.is_empty()
    }

    /// Compatibility adapter. This adapter is only for exact Rust-ordered
    /// sets; Γ-class-aware collections must construct their endpoint through
    /// the pinned semantic module before creating `FineChange`.
    #[must_use]
    pub fn into_fine(&self, old: &BTreeSet<T>) -> FineChange<BTreeSet<T>> {
        FineChange::new(FineChangeKind::Set, self.apply(old))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeqSplice<T> {
    pub start: usize,
    pub delete_count: usize,
    pub insert: Vec<T>,
}

/// Stable semantic identity of one logical sequence occurrence. Unlike a
/// `SeqSplice` index this identity survives unrelated insertions/deletions
/// elsewhere in the sequence and may therefore be retained as Rewrite intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeqOccurrenceId(pub SemanticId);

/// Identity of the retained anchor-history epoch used to interpret a gap
/// anchor. An anchor from an epoch which is no longer retained must fail
/// closed rather than being reinterpreted against the current snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeqAnchorHistoryId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableSeqGapAnchor {
    pub id: SemanticId,
    pub history: SeqAnchorHistoryId,
    pub left: Option<SeqOccurrenceId>,
    pub right: Option<SeqOccurrenceId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableSeqOccurrence<T> {
    pub id: SeqOccurrenceId,
    pub value: T,
}

/// Snapshot plus the exact anchor-history epochs which remain authoritative
/// for resolving durable/concurrent sequence intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableSeqSnapshot<T> {
    pub sequence: SemanticId,
    pub retained_anchor_histories: BTreeSet<SeqAnchorHistoryId>,
    pub occurrences: Vec<StableSeqOccurrence<T>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StableSeqRewriteIntent<T> {
    Insert {
        sequence: SemanticId,
        anchor: StableSeqGapAnchor,
        occurrence: SeqOccurrenceId,
        value: T,
    },
    Replace {
        sequence: SemanticId,
        occurrence: SeqOccurrenceId,
        value: T,
    },
    Delete {
        sequence: SemanticId,
        occurrence: SeqOccurrenceId,
    },
}

impl<T> StableSeqRewriteIntent<T> {
    #[must_use]
    pub const fn sequence(&self) -> SemanticId {
        match self {
            Self::Insert { sequence, .. }
            | Self::Replace { sequence, .. }
            | Self::Delete { sequence, .. } => *sequence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableSeqRewriteError {
    WrongSequence,
    MissingOccurrence(SeqOccurrenceId),
    DuplicateOccurrence(SeqOccurrenceId),
    ExpiredAnchorHistory(SeqAnchorHistoryId),
    AnchorNoLongerGap(SemanticId),
    ResolvedSpliceInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableSeqPairDecision {
    CoordinationFree,
    SameGapOrderingRequired(StableSeqGapAnchor),
    ConflictingOccurrenceRewrite(SeqOccurrenceId),
}

impl<T: Clone> StableSeqSnapshot<T> {
    fn occurrence_index(
        &self,
        occurrence: SeqOccurrenceId,
    ) -> Result<usize, StableSeqRewriteError> {
        self.occurrences
            .iter()
            .position(|entry| entry.id == occurrence)
            .ok_or(StableSeqRewriteError::MissingOccurrence(occurrence))
    }

    fn resolve_gap(&self, anchor: StableSeqGapAnchor) -> Result<usize, StableSeqRewriteError> {
        if !self.retained_anchor_histories.contains(&anchor.history) {
            return Err(StableSeqRewriteError::ExpiredAnchorHistory(anchor.history));
        }
        let len = self.occurrences.len();
        let start = match (anchor.left, anchor.right) {
            (None, None) if len == 0 => 0,
            (None, Some(right)) => {
                let right = self.occurrence_index(right)?;
                if right != 0 {
                    return Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id));
                }
                0
            }
            (Some(left), None) => {
                let left = self.occurrence_index(left)?;
                if left + 1 != len {
                    return Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id));
                }
                len
            }
            (Some(left), Some(right)) => {
                let left = self.occurrence_index(left)?;
                let right = self.occurrence_index(right)?;
                if left + 1 != right {
                    return Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id));
                }
                right
            }
            (None, None) => return Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id)),
        };
        Ok(start)
    }

    pub fn resolve_intent(
        &self,
        intent: &StableSeqRewriteIntent<T>,
    ) -> Result<SeqSplice<StableSeqOccurrence<T>>, StableSeqRewriteError> {
        if intent.sequence() != self.sequence {
            return Err(StableSeqRewriteError::WrongSequence);
        }
        match intent {
            StableSeqRewriteIntent::Insert {
                anchor,
                occurrence,
                value,
                ..
            } => {
                if self.occurrences.iter().any(|entry| entry.id == *occurrence) {
                    return Err(StableSeqRewriteError::DuplicateOccurrence(*occurrence));
                }
                Ok(SeqSplice {
                    start: self.resolve_gap(*anchor)?,
                    delete_count: 0,
                    insert: vec![StableSeqOccurrence {
                        id: *occurrence,
                        value: value.clone(),
                    }],
                })
            }
            StableSeqRewriteIntent::Replace {
                occurrence, value, ..
            } => Ok(SeqSplice {
                start: self.occurrence_index(*occurrence)?,
                delete_count: 1,
                insert: vec![StableSeqOccurrence {
                    id: *occurrence,
                    value: value.clone(),
                }],
            }),
            StableSeqRewriteIntent::Delete { occurrence, .. } => Ok(SeqSplice {
                start: self.occurrence_index(*occurrence)?,
                delete_count: 1,
                insert: Vec::new(),
            }),
        }
    }
}

fn stable_seq_target_occurrence<T>(intent: &StableSeqRewriteIntent<T>) -> SeqOccurrenceId {
    match intent {
        StableSeqRewriteIntent::Insert { occurrence, .. }
        | StableSeqRewriteIntent::Replace { occurrence, .. }
        | StableSeqRewriteIntent::Delete { occurrence, .. } => *occurrence,
    }
}

fn stable_seq_deleted_occurrence<T>(intent: &StableSeqRewriteIntent<T>) -> Option<SeqOccurrenceId> {
    match intent {
        StableSeqRewriteIntent::Delete { occurrence, .. } => Some(*occurrence),
        _ => None,
    }
}

fn anchor_references(anchor: StableSeqGapAnchor, occurrence: SeqOccurrenceId) -> bool {
    anchor.left == Some(occurrence) || anchor.right == Some(occurrence)
}

/// Fail-closed pair policy for stable sequence intents. Distinct inserts into
/// one semantic gap require an explicit order; rewrites of the same occurrence
/// (or deletion of an occurrence used by the other's anchor) conflict.
#[must_use]
pub fn classify_stable_seq_pair<T>(
    left: &StableSeqRewriteIntent<T>,
    right: &StableSeqRewriteIntent<T>,
) -> StableSeqPairDecision {
    if left.sequence() != right.sequence() {
        return StableSeqPairDecision::CoordinationFree;
    }
    let left_target = stable_seq_target_occurrence(left);
    let right_target = stable_seq_target_occurrence(right);
    if left_target == right_target {
        return StableSeqPairDecision::ConflictingOccurrenceRewrite(left_target);
    }
    if let (
        StableSeqRewriteIntent::Insert {
            anchor: left_anchor,
            ..
        },
        StableSeqRewriteIntent::Insert {
            anchor: right_anchor,
            ..
        },
    ) = (left, right)
        && left_anchor == right_anchor
    {
        return StableSeqPairDecision::SameGapOrderingRequired(*left_anchor);
    }
    if let Some(deleted) = stable_seq_deleted_occurrence(left)
        && let StableSeqRewriteIntent::Insert { anchor, .. } = right
        && anchor_references(*anchor, deleted)
    {
        return StableSeqPairDecision::ConflictingOccurrenceRewrite(deleted);
    }
    if let Some(deleted) = stable_seq_deleted_occurrence(right)
        && let StableSeqRewriteIntent::Insert { anchor, .. } = left
        && anchor_references(*anchor, deleted)
    {
        return StableSeqPairDecision::ConflictingOccurrenceRewrite(deleted);
    }
    StableSeqPairDecision::CoordinationFree
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeqChangeError {
    StartOutOfBounds,
    DeleteOutOfBounds,
}

impl<T: Clone> SeqSplice<T> {
    pub fn apply(&self, old: &[T]) -> Result<Vec<T>, SeqChangeError> {
        if self.start > old.len() {
            return Err(SeqChangeError::StartOutOfBounds);
        }
        let end = self
            .start
            .checked_add(self.delete_count)
            .ok_or(SeqChangeError::DeleteOutOfBounds)?;
        if end > old.len() {
            return Err(SeqChangeError::DeleteOutOfBounds);
        }

        let mut next = Vec::with_capacity(old.len() - self.delete_count + self.insert.len());
        next.extend_from_slice(&old[..self.start]);
        next.extend(self.insert.iter().cloned());
        next.extend_from_slice(&old[end..]);
        Ok(next)
    }

    pub fn into_fine(&self, old: &[T]) -> Result<FineChange<Vec<T>>, SeqChangeError> {
        Ok(FineChange::new(FineChangeKind::Seq, self.apply(old)?))
    }
}

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

impl<T: Clone, I> PreparedRewrite<T, I> {
    #[must_use]
    pub fn apply(&self, old: &T) -> T {
        self.effect.apply(old)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_change_round_trips_and_adapts_to_universal_fine_change() {
        let old = BTreeSet::from([1, 2, 3]);
        let new = BTreeSet::from([2, 3, 4, 5]);
        let delta = SetChange::between(&old, &new);
        assert_eq!(delta.apply(&old), new);
        let universal = Change::Fine(delta.into_fine(&old));
        assert_eq!(universal.apply(&old), new);
    }

    #[test]
    fn sequence_splice_has_exact_checked_semantics_and_adapter() {
        let splice = SeqSplice {
            start: 1,
            delete_count: 2,
            insert: vec![7, 8, 9],
        };
        let old = vec![1, 2, 3, 4];
        let expected = vec![1, 7, 8, 9, 4];
        assert_eq!(splice.apply(&old), Ok(expected.clone()));
        let universal = Change::Fine(splice.into_fine(&old).unwrap());
        assert_eq!(universal.apply(&old), expected);
    }

    #[test]
    fn invalid_splice_is_not_silently_clamped() {
        let splice = SeqSplice {
            start: 2,
            delete_count: 5,
            insert: Vec::<i32>::new(),
        };
        assert_eq!(
            splice.apply(&[1, 2, 3]),
            Err(SeqChangeError::DeleteOutOfBounds)
        );
    }

    #[test]
    fn change_composition_preserves_extensional_endpoint() {
        let first = Change::Fine(FineChange::new(FineChangeKind::Scalar, 2));
        let later = Change::Fine(FineChange::new(FineChangeKind::Scalar, 3));
        assert_eq!(first.compose(later).apply(&1), 3);
    }

    #[test]
    fn prepared_rewrite_keeps_intent_separate_from_effect() {
        let rewrite = PreparedRewrite {
            spec: RewriteSpecId(SemanticId(11)),
            explicit_inputs: vec!["requested"],
            effect: RewriteEffect::Fine(FineChange::new(FineChangeKind::Scalar, 42)),
            law_set: RewriteLawSetId(SemanticId(12)),
        };
        assert_eq!(rewrite.apply(&1), 42);
        assert_eq!(rewrite.spec, RewriteSpecId(SemanticId(11)));
        assert_eq!(rewrite.explicit_inputs, vec!["requested"]);
    }

    fn stable_snapshot() -> StableSeqSnapshot<i32> {
        StableSeqSnapshot {
            sequence: SemanticId(100),
            retained_anchor_histories: [SeqAnchorHistoryId(SemanticId(200))].into_iter().collect(),
            occurrences: vec![
                StableSeqOccurrence {
                    id: SeqOccurrenceId(SemanticId(1)),
                    value: 10,
                },
                StableSeqOccurrence {
                    id: SeqOccurrenceId(SemanticId(2)),
                    value: 20,
                },
                StableSeqOccurrence {
                    id: SeqOccurrenceId(SemanticId(3)),
                    value: 30,
                },
            ],
        }
    }

    fn middle_gap() -> StableSeqGapAnchor {
        StableSeqGapAnchor {
            id: SemanticId(300),
            history: SeqAnchorHistoryId(SemanticId(200)),
            left: Some(SeqOccurrenceId(SemanticId(2))),
            right: Some(SeqOccurrenceId(SemanticId(3))),
        }
    }

    #[test]
    fn stable_seq_anchor_survives_snapshot_index_drift() {
        let intent = StableSeqRewriteIntent::Insert {
            sequence: SemanticId(100),
            anchor: middle_gap(),
            occurrence: SeqOccurrenceId(SemanticId(4)),
            value: 40,
        };
        assert_eq!(stable_snapshot().resolve_intent(&intent).unwrap().start, 2);

        let mut shifted = stable_snapshot();
        shifted.occurrences.insert(
            0,
            StableSeqOccurrence {
                id: SeqOccurrenceId(SemanticId(9)),
                value: 90,
            },
        );
        assert_eq!(shifted.resolve_intent(&intent).unwrap().start, 3);
    }

    #[test]
    fn stable_seq_resolution_fails_typed_for_missing_occurrence_and_expired_anchor() {
        let replace = StableSeqRewriteIntent::Replace {
            sequence: SemanticId(100),
            occurrence: SeqOccurrenceId(SemanticId(99)),
            value: 1,
        };
        assert_eq!(
            stable_snapshot().resolve_intent(&replace),
            Err(StableSeqRewriteError::MissingOccurrence(SeqOccurrenceId(
                SemanticId(99)
            )))
        );

        let mut anchor = middle_gap();
        anchor.history = SeqAnchorHistoryId(SemanticId(999));
        let insert = StableSeqRewriteIntent::Insert {
            sequence: SemanticId(100),
            anchor,
            occurrence: SeqOccurrenceId(SemanticId(4)),
            value: 40,
        };
        assert_eq!(
            stable_snapshot().resolve_intent(&insert),
            Err(StableSeqRewriteError::ExpiredAnchorHistory(
                SeqAnchorHistoryId(SemanticId(999))
            ))
        );
    }

    #[test]
    fn stable_seq_pair_policy_requires_order_for_same_gap_and_conflicts_on_same_occurrence() {
        let insert = |occurrence, value| StableSeqRewriteIntent::Insert {
            sequence: SemanticId(100),
            anchor: middle_gap(),
            occurrence: SeqOccurrenceId(SemanticId(occurrence)),
            value,
        };
        assert_eq!(
            classify_stable_seq_pair(&insert(4, 40), &insert(5, 50)),
            StableSeqPairDecision::SameGapOrderingRequired(middle_gap())
        );
        assert_eq!(
            infer_pair_rewrite_law(
                &insert(4, 40).rewrite_footprint(),
                &insert(5, 50).rewrite_footprint(),
            ),
            PairRewriteLaw::Unknown
        );

        let replace = StableSeqRewriteIntent::Replace {
            sequence: SemanticId(100),
            occurrence: SeqOccurrenceId(SemanticId(2)),
            value: 200,
        };
        let delete = StableSeqRewriteIntent::<i32>::Delete {
            sequence: SemanticId(100),
            occurrence: SeqOccurrenceId(SemanticId(2)),
        };
        assert_eq!(
            classify_stable_seq_pair(&replace, &delete),
            StableSeqPairDecision::ConflictingOccurrenceRewrite(SeqOccurrenceId(SemanticId(2)))
        );
    }

    #[test]
    fn stable_seq_delete_of_anchor_endpoint_conflicts_with_concurrent_insert() {
        let delete = StableSeqRewriteIntent::<i32>::Delete {
            sequence: SemanticId(100),
            occurrence: SeqOccurrenceId(SemanticId(2)),
        };
        let insert = StableSeqRewriteIntent::Insert {
            sequence: SemanticId(100),
            anchor: middle_gap(),
            occurrence: SeqOccurrenceId(SemanticId(4)),
            value: 40,
        };
        assert_eq!(
            classify_stable_seq_pair(&delete, &insert),
            StableSeqPairDecision::ConflictingOccurrenceRewrite(SeqOccurrenceId(SemanticId(2)))
        );
        assert_ne!(
            infer_pair_rewrite_law(&delete.rewrite_footprint(), &insert.rewrite_footprint()),
            PairRewriteLaw::StrongCommute
        );
    }

    #[test]
    fn stable_seq_prepared_rewrite_preserves_anchored_intent_and_semantic_coordinates() {
        let intent = StableSeqRewriteIntent::Insert {
            sequence: SemanticId(100),
            anchor: middle_gap(),
            occurrence: SeqOccurrenceId(SemanticId(4)),
            value: 40,
        };
        let spec = RewriteSpec {
            id: RewriteSpecId(SemanticId(700)),
            law_set: RewriteLawSetId(SemanticId(701)),
            footprint: RewriteFootprint::default(),
        };
        let prepared = spec
            .prepare_stable_seq(&stable_snapshot(), intent.clone())
            .unwrap();
        assert_eq!(prepared.spec, spec.id);
        assert_eq!(prepared.law_set, spec.law_set);
        assert_eq!(prepared.explicit_inputs, vec![intent.clone()]);
        assert_eq!(prepared.apply(&stable_snapshot().occurrences)[2].value, 40);
        assert_eq!(
            intent.write_coordinates(),
            [
                SemanticWriteCoordinate::SeqAnchor {
                    sequence: SemanticId(100),
                    anchor: SemanticId(300),
                },
                SemanticWriteCoordinate::SeqOccurrence {
                    sequence: SemanticId(100),
                    occurrence: SemanticId(4),
                },
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(
            classify_stable_seq_pair(
                &intent,
                &StableSeqRewriteIntent::Insert {
                    sequence: SemanticId(100),
                    anchor: middle_gap(),
                    occurrence: SeqOccurrenceId(SemanticId(5)),
                    value: 50,
                },
            )
            .coordination_decision(),
            PairCoordinationDecision::RequiresCoordination
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticWriteCoordinate {
    ProductField(SemanticId),
    SetClass {
        observable: kernel_types::RevisionObservableId,
        class: kernel_types::EqClassId,
    },
    BagClass {
        observable: kernel_types::RevisionObservableId,
        class: kernel_types::EqClassId,
    },
    MapKeyClass {
        observable: kernel_types::RevisionObservableId,
        class: kernel_types::EqClassId,
    },
    RelationIdentity {
        relation: SemanticId,
        identity: SemanticId,
    },
    SeqAnchor {
        sequence: SemanticId,
        anchor: SemanticId,
    },
    SeqOccurrence {
        sequence: SemanticId,
        occurrence: SemanticId,
    },
}

impl<T> StableSeqRewriteIntent<T> {
    /// Semantic coordinates touched by the stable intent. A gap insertion
    /// writes both the gap-order coordinate and the newly introduced logical
    /// occurrence; occurrence rewrites never degrade to snapshot indices.
    #[must_use]
    pub fn write_coordinates(&self) -> BTreeSet<SemanticWriteCoordinate> {
        match self {
            Self::Insert {
                sequence,
                anchor,
                occurrence,
                ..
            } => [
                SemanticWriteCoordinate::SeqAnchor {
                    sequence: *sequence,
                    anchor: anchor.id,
                },
                SemanticWriteCoordinate::SeqOccurrence {
                    sequence: *sequence,
                    occurrence: occurrence.0,
                },
            ]
            .into_iter()
            .collect(),
            Self::Replace {
                sequence,
                occurrence,
                ..
            }
            | Self::Delete {
                sequence,
                occurrence,
            } => [SemanticWriteCoordinate::SeqOccurrence {
                sequence: *sequence,
                occurrence: occurrence.0,
            }]
            .into_iter()
            .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteActionLaw {
    IdempotentAssign { semantic_value: SemanticId },
    EnsurePresent,
    EnsureAbsent,
    CommutativeAdd { algebra: SemanticId },
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewriteFootprint {
    pub reads: BTreeSet<SemanticWriteCoordinate>,
    pub writes: std::collections::BTreeMap<SemanticWriteCoordinate, RewriteActionLaw>,
    pub invariant_obligations: BTreeSet<SemanticId>,
}

impl<T> StableSeqRewriteIntent<T> {
    /// Conservative footprint for the generic Rewrite-law engine. Specialized
    /// stable-sequence pair classification may strengthen `Unknown` into a
    /// typed same-gap ordering requirement or occurrence conflict, but this
    /// footprint never grants a stronger coordination-free claim.
    #[must_use]
    pub fn rewrite_footprint(&self) -> RewriteFootprint {
        let mut footprint = RewriteFootprint::default();
        match self {
            Self::Insert {
                sequence,
                anchor,
                occurrence,
                ..
            } => {
                for endpoint in [anchor.left, anchor.right].into_iter().flatten() {
                    footprint
                        .reads
                        .insert(SemanticWriteCoordinate::SeqOccurrence {
                            sequence: *sequence,
                            occurrence: endpoint.0,
                        });
                }
                footprint.writes.insert(
                    SemanticWriteCoordinate::SeqAnchor {
                        sequence: *sequence,
                        anchor: anchor.id,
                    },
                    RewriteActionLaw::Opaque,
                );
                footprint.writes.insert(
                    SemanticWriteCoordinate::SeqOccurrence {
                        sequence: *sequence,
                        occurrence: occurrence.0,
                    },
                    RewriteActionLaw::Opaque,
                );
            }
            Self::Replace {
                sequence,
                occurrence,
                ..
            }
            | Self::Delete {
                sequence,
                occurrence,
            } => {
                footprint.writes.insert(
                    SemanticWriteCoordinate::SeqOccurrence {
                        sequence: *sequence,
                        occurrence: occurrence.0,
                    },
                    RewriteActionLaw::Opaque,
                );
            }
        }
        footprint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRewriteLaw {
    StrongCommute,
    SameIdempotentIntent,
    DefiniteIntentConflict,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairCoordinationDecision {
    CoordinationFree,
    IntentConflict,
    RequiresCoordination,
}

impl StableSeqPairDecision {
    #[must_use]
    pub const fn coordination_decision(self) -> PairCoordinationDecision {
        match self {
            Self::CoordinationFree => PairCoordinationDecision::CoordinationFree,
            Self::SameGapOrderingRequired(_) => PairCoordinationDecision::RequiresCoordination,
            Self::ConflictingOccurrenceRewrite(_) => PairCoordinationDecision::IntentConflict,
        }
    }
}

#[must_use]
pub const fn coordination_decision(law: PairRewriteLaw) -> PairCoordinationDecision {
    match law {
        PairRewriteLaw::StrongCommute | PairRewriteLaw::SameIdempotentIntent => {
            PairCoordinationDecision::CoordinationFree
        }
        PairRewriteLaw::DefiniteIntentConflict => PairCoordinationDecision::IntentConflict,
        PairRewriteLaw::Unknown => PairCoordinationDecision::RequiresCoordination,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteResidualDiamond<T, I> {
    pub right_after_left: PreparedRewrite<T, I>,
    pub left_after_right: PreparedRewrite<T, I>,
    common_endpoint: T,
}

impl<T, I> RewriteResidualDiamond<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        &self.common_endpoint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteCubeCoherence<T, I> {
    coherent_residual: PreparedRewrite<T, I>,
}

impl<T, I> RewriteCubeCoherence<T, I> {
    #[must_use]
    pub const fn coherent_residual(&self) -> &PreparedRewrite<T, I> {
        &self.coherent_residual
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteCoherenceError {
    DiamondEndpointMismatch,
    CubeResidualIntentMismatch,
    CubeEndpointMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteFamilyIdentity {
    pub spec: RewriteSpecId,
    pub law_set: RewriteLawSetId,
}

impl<T, I> From<&PreparedRewrite<T, I>> for RewriteFamilyIdentity {
    fn from(rewrite: &PreparedRewrite<T, I>) -> Self {
        Self {
            spec: rewrite.spec,
            law_set: rewrite.law_set,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteResidualFamilyId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteResidualFamilyKey {
    pub left: RewriteFamilyIdentity,
    pub right: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteResidualFamilySpec {
    pub id: RewriteResidualFamilyId,
    pub key: RewriteResidualFamilyKey,
    pub right_after_left: RewriteFamilyIdentity,
    pub left_after_right: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewriteResidualFamilyRegistry {
    by_pair: BTreeMap<RewriteResidualFamilyKey, RewriteResidualFamilySpec>,
    by_id: BTreeMap<RewriteResidualFamilyId, RewriteResidualFamilyKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteSequentialFamilyId(pub SemanticId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RewriteSequentialFamilyKey {
    pub first: RewriteFamilyIdentity,
    pub second: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RewriteSequentialFamilySpec {
    pub id: RewriteSequentialFamilyId,
    pub key: RewriteSequentialFamilyKey,
    pub composite: RewriteFamilyIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewriteSequentialFamilyRegistry {
    by_pair: BTreeMap<RewriteSequentialFamilyKey, RewriteSequentialFamilySpec>,
    by_id: BTreeMap<RewriteSequentialFamilyId, RewriteSequentialFamilyKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteSequentialComposition<T, I> {
    pub first: PreparedRewrite<T, I>,
    pub second: PreparedRewrite<T, I>,
    pub composite: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteSequentialRegistryError {
    FamilyIdConflict,
    PairAlreadyRegistered,
    FamilyNotRegistered,
    InputIdentityMismatch,
    CompositeIdentityMismatch,
    CompositeEndpointMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteResidualCubeWitness<T, I> {
    pub b_after_a: PreparedRewrite<T, I>,
    pub a_after_b: PreparedRewrite<T, I>,
    pub c_after_a: PreparedRewrite<T, I>,
    pub a_after_c: PreparedRewrite<T, I>,
    pub c_after_b: PreparedRewrite<T, I>,
    pub b_after_c: PreparedRewrite<T, I>,
    pub c_after_ab: PreparedRewrite<T, I>,
    pub b_after_ac: PreparedRewrite<T, I>,
    pub c_after_ba: PreparedRewrite<T, I>,
    pub a_after_bc: PreparedRewrite<T, I>,
    pub b_after_ca: PreparedRewrite<T, I>,
    pub a_after_cb: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteResidualCubeCertificate<T, I> {
    pub ab: RewriteResidualDiamond<T, I>,
    pub ac: RewriteResidualDiamond<T, I>,
    pub bc: RewriteResidualDiamond<T, I>,
    pub after_a: RewriteResidualDiamond<T, I>,
    pub after_b: RewriteResidualDiamond<T, I>,
    pub after_c: RewriteResidualDiamond<T, I>,
    pub cube: RewriteCubeCoherence<T, I>,
    common_endpoint: T,
}

impl<T, I> RewriteResidualCubeCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        &self.common_endpoint
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewriteResidualRegistryError {
    FamilyIdConflict,
    PairAlreadyRegistered,
    FamilyNotRegistered,
    InputIdentityMismatch,
    ResidualIdentityMismatch,
    Coherence(RewriteCoherenceError),
}

impl RewriteResidualFamilyRegistry {
    pub fn register(
        &mut self,
        spec: RewriteResidualFamilySpec,
    ) -> Result<(), RewriteResidualRegistryError> {
        if let Some(existing_key) = self.by_id.get(&spec.id) {
            return if *existing_key == spec.key && self.by_pair.get(&spec.key) == Some(&spec) {
                Ok(())
            } else {
                Err(RewriteResidualRegistryError::FamilyIdConflict)
            };
        }
        if self.by_pair.contains_key(&spec.key) {
            return Err(RewriteResidualRegistryError::PairAlreadyRegistered);
        }
        self.by_id.insert(spec.id, spec.key);
        self.by_pair.insert(spec.key, spec);
        Ok(())
    }

    #[must_use]
    pub fn family(&self, key: RewriteResidualFamilyKey) -> Option<&RewriteResidualFamilySpec> {
        self.by_pair.get(&key)
    }

    pub fn certify<T: Clone + PartialEq + Eq, I>(
        &self,
        base: &T,
        left: &PreparedRewrite<T, I>,
        right: &PreparedRewrite<T, I>,
        right_after_left: PreparedRewrite<T, I>,
        left_after_right: PreparedRewrite<T, I>,
    ) -> Result<RewriteResidualDiamond<T, I>, RewriteResidualRegistryError> {
        let key = RewriteResidualFamilyKey {
            left: left.into(),
            right: right.into(),
        };
        let family = self
            .by_pair
            .get(&key)
            .ok_or(RewriteResidualRegistryError::FamilyNotRegistered)?;
        if RewriteFamilyIdentity::from(left) != family.key.left
            || RewriteFamilyIdentity::from(right) != family.key.right
        {
            return Err(RewriteResidualRegistryError::InputIdentityMismatch);
        }
        if RewriteFamilyIdentity::from(&right_after_left) != family.right_after_left
            || RewriteFamilyIdentity::from(&left_after_right) != family.left_after_right
        {
            return Err(RewriteResidualRegistryError::ResidualIdentityMismatch);
        }
        certify_residual_diamond(base, left, right, right_after_left, left_after_right)
            .map_err(RewriteResidualRegistryError::Coherence)
    }

    pub fn certify_cube<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
        &self,
        base: &T,
        a: &PreparedRewrite<T, I>,
        b: &PreparedRewrite<T, I>,
        c: &PreparedRewrite<T, I>,
        witness: RewriteResidualCubeWitness<T, I>,
    ) -> Result<RewriteResidualCubeCertificate<T, I>, RewriteResidualRegistryError> {
        let ab = self.certify(
            base,
            a,
            b,
            witness.b_after_a.clone(),
            witness.a_after_b.clone(),
        )?;
        let ac = self.certify(
            base,
            a,
            c,
            witness.c_after_a.clone(),
            witness.a_after_c.clone(),
        )?;
        let bc = self.certify(
            base,
            b,
            c,
            witness.c_after_b.clone(),
            witness.b_after_c.clone(),
        )?;

        let after_a = self.certify(
            &a.apply(base),
            &witness.b_after_a,
            &witness.c_after_a,
            witness.c_after_ab.clone(),
            witness.b_after_ac,
        )?;
        let after_b = self.certify(
            &b.apply(base),
            &witness.a_after_b,
            &witness.c_after_b,
            witness.c_after_ba.clone(),
            witness.a_after_bc,
        )?;
        let after_c = self.certify(
            &c.apply(base),
            &witness.a_after_c,
            &witness.b_after_c,
            witness.b_after_ca,
            witness.a_after_cb,
        )?;
        let cube = certify_cube_coherence(&witness.c_after_ab, &witness.c_after_ba)
            .map_err(RewriteResidualRegistryError::Coherence)?;
        if after_a.common_endpoint() != after_b.common_endpoint()
            || after_a.common_endpoint() != after_c.common_endpoint()
        {
            return Err(RewriteResidualRegistryError::Coherence(
                RewriteCoherenceError::CubeEndpointMismatch,
            ));
        }
        let common_endpoint = after_a.common_endpoint().clone();

        Ok(RewriteResidualCubeCertificate {
            ab,
            ac,
            bc,
            after_a,
            after_b,
            after_c,
            cube,
            common_endpoint,
        })
    }
}

impl RewriteSequentialFamilyRegistry {
    pub fn register(
        &mut self,
        spec: RewriteSequentialFamilySpec,
    ) -> Result<(), RewriteSequentialRegistryError> {
        if let Some(existing_key) = self.by_id.get(&spec.id) {
            return if *existing_key == spec.key && self.by_pair.get(&spec.key) == Some(&spec) {
                Ok(())
            } else {
                Err(RewriteSequentialRegistryError::FamilyIdConflict)
            };
        }
        if self.by_pair.contains_key(&spec.key) {
            return Err(RewriteSequentialRegistryError::PairAlreadyRegistered);
        }
        self.by_id.insert(spec.id, spec.key);
        self.by_pair.insert(spec.key, spec);
        Ok(())
    }

    pub fn certify<T: Clone + PartialEq + Eq, I: Clone>(
        &self,
        base: &T,
        first: PreparedRewrite<T, I>,
        second: PreparedRewrite<T, I>,
        composite: PreparedRewrite<T, I>,
    ) -> Result<RewriteSequentialComposition<T, I>, RewriteSequentialRegistryError> {
        let key = RewriteSequentialFamilyKey {
            first: (&first).into(),
            second: (&second).into(),
        };
        let family = self
            .by_pair
            .get(&key)
            .ok_or(RewriteSequentialRegistryError::FamilyNotRegistered)?;
        if RewriteFamilyIdentity::from(&first) != family.key.first
            || RewriteFamilyIdentity::from(&second) != family.key.second
        {
            return Err(RewriteSequentialRegistryError::InputIdentityMismatch);
        }
        if RewriteFamilyIdentity::from(&composite) != family.composite {
            return Err(RewriteSequentialRegistryError::CompositeIdentityMismatch);
        }
        let sequential_endpoint = second.apply(&first.apply(base));
        if composite.apply(base) != sequential_endpoint {
            return Err(RewriteSequentialRegistryError::CompositeEndpointMismatch);
        }
        Ok(RewriteSequentialComposition {
            first,
            second,
            composite,
        })
    }
}

pub fn certify_residual_diamond<T: Clone + PartialEq + Eq, I>(
    base: &T,
    left: &PreparedRewrite<T, I>,
    right: &PreparedRewrite<T, I>,
    right_after_left: PreparedRewrite<T, I>,
    left_after_right: PreparedRewrite<T, I>,
) -> Result<RewriteResidualDiamond<T, I>, RewriteCoherenceError> {
    let left_state = left.apply(base);
    let right_state = right.apply(base);
    let via_left = right_after_left.apply(&left_state);
    let via_right = left_after_right.apply(&right_state);
    if via_left != via_right {
        return Err(RewriteCoherenceError::DiamondEndpointMismatch);
    }
    Ok(RewriteResidualDiamond {
        right_after_left,
        left_after_right,
        common_endpoint: via_left,
    })
}

pub fn certify_cube_coherence<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    through_left_then_right: &PreparedRewrite<T, I>,
    through_right_then_left: &PreparedRewrite<T, I>,
) -> Result<RewriteCubeCoherence<T, I>, RewriteCoherenceError> {
    if through_left_then_right != through_right_then_left {
        return Err(RewriteCoherenceError::CubeResidualIntentMismatch);
    }
    Ok(RewriteCubeCoherence {
        coherent_residual: through_left_then_right.clone(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RevisionEffectId(pub u128);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffect<E> {
    pub id: RevisionEffectId,
    pub prerequisites: BTreeSet<RevisionEffectId>,
    pub payload: E,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectIdeal<E> {
    events: BTreeMap<RevisionEffectId, RevisionEffect<E>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectMergeRequirements {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_exclusive: BTreeSet<RevisionEffectId>,
    pub right_exclusive: BTreeSet<RevisionEffectId>,
    pub requires_residual: BTreeSet<(RevisionEffectId, RevisionEffectId)>,
    pub intent_conflicts: BTreeSet<(RevisionEffectId, RevisionEffectId)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectCausalLayerSchedule {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_layers: Vec<BTreeSet<RevisionEffectId>>,
    pub right_layers: Vec<BTreeSet<RevisionEffectId>>,
}

impl RevisionEffectMergeRequirements {
    #[must_use]
    pub fn coordination_free(&self) -> bool {
        self.requires_residual.is_empty() && self.intent_conflicts.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectIdealError {
    DuplicateEffect(RevisionEffectId),
    MissingPrerequisite {
        effect: RevisionEffectId,
        prerequisite: RevisionEffectId,
    },
    CyclicPrerequisites(BTreeSet<RevisionEffectId>),
    EffectIdentityConflict(RevisionEffectId),
    NotSubideal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub diamonds: BTreeMap<(RevisionEffectId, RevisionEffectId), RewriteResidualDiamond<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualCubeLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub cube: RewriteResidualCubeCertificate<T, I>,
}

impl<T, I> RevisionEffectResidualCubeLayerCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        self.cube.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentPairWitness<T, I> {
    pub right_after_left: PreparedRewrite<T, I>,
    pub left_after_right: PreparedRewrite<T, I>,
    pub left_then_right_composite: PreparedRewrite<T, I>,
    pub right_then_left_composite: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentPairCertificate<T, I> {
    pub diamond: RewriteResidualDiamond<T, I>,
    pub left_then_right: RewriteSequentialComposition<T, I>,
    pub right_then_left: RewriteSequentialComposition<T, I>,
    composite: PreparedRewrite<T, I>,
}

impl<T, I> RewriteConcurrentPairCertificate<T, I> {
    #[must_use]
    pub const fn composite(&self) -> &PreparedRewrite<T, I> {
        &self.composite
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentTripleWitness<T, I> {
    pub cube: RewriteResidualCubeWitness<T, I>,
    pub ab_composite: PreparedRewrite<T, I>,
    pub ac_composite: PreparedRewrite<T, I>,
    pub bc_composite: PreparedRewrite<T, I>,
    pub final_composite: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteConcurrentTripleCertificate<T, I> {
    pub cube: RewriteResidualCubeCertificate<T, I>,
    pub ab: RewriteConcurrentPairCertificate<T, I>,
    pub ac: RewriteConcurrentPairCertificate<T, I>,
    pub bc: RewriteConcurrentPairCertificate<T, I>,
    pub final_paths: Vec<RewriteSequentialComposition<T, I>>,
    composite: PreparedRewrite<T, I>,
}

impl<T, I> RewriteConcurrentTripleCertificate<T, I> {
    #[must_use]
    pub const fn composite(&self) -> &PreparedRewrite<T, I> {
        &self.composite
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteConcurrentBranchWitness<T, I> {
    Single,
    Pair(Box<RewriteConcurrentPairWitness<T, I>>),
    Triple(Box<RewriteConcurrentTripleWitness<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RewriteConcurrentBranchCertificate<T, I> {
    Single(PreparedRewrite<T, I>),
    Pair(Box<RewriteConcurrentPairCertificate<T, I>>),
    Triple(Box<RewriteConcurrentTripleCertificate<T, I>>),
}

impl<T, I> RewriteConcurrentBranchCertificate<T, I> {
    #[must_use]
    pub const fn composite(&self) -> &PreparedRewrite<T, I> {
        match self {
            Self::Single(rewrite) => rewrite,
            Self::Pair(certificate) => certificate.composite(),
            Self::Triple(certificate) => certificate.composite(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualNormalizedLayerWitness<T, I> {
    pub left: RewriteConcurrentBranchWitness<T, I>,
    pub right: RewriteConcurrentBranchWitness<T, I>,
    pub right_after_left: PreparedRewrite<T, I>,
    pub left_after_right: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualNormalizedLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub left_normalization: RewriteConcurrentBranchCertificate<T, I>,
    pub right_normalization: RewriteConcurrentBranchCertificate<T, I>,
    pub cross: RewriteResidualDiamond<T, I>,
}

impl<T, I> RevisionEffectResidualNormalizedLayerCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        self.cross.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareLayerWitness<T, I> {
    pub left: RewriteConcurrentPairWitness<T, I>,
    pub right: RewriteConcurrentPairWitness<T, I>,
    pub right_after_left: PreparedRewrite<T, I>,
    pub left_after_right: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareLayerCertificate<T, I> {
    pub common: BTreeSet<RevisionEffectId>,
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub left_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub right_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub cross: RewriteResidualDiamond<T, I>,
}

impl<T, I> RevisionEffectResidualSquareLayerCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        self.cross.common_endpoint()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareChainWitness<T, I> {
    pub first: RevisionEffectResidualSquareLayerWitness<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepWitness<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualSquareChainCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RevisionEffectResidualSquareLayerCertificate<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepCertificate<T, I>>,
    common_endpoint: T,
}

impl<T, I> RevisionEffectResidualSquareChainCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        &self.common_endpoint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedFirstWitness<T, I> {
    Singleton {
        right_after_left: PreparedRewrite<T, I>,
        left_after_right: PreparedRewrite<T, I>,
    },
    Square(Box<RevisionEffectResidualSquareLayerWitness<T, I>>),
    Normalized(Box<RevisionEffectResidualNormalizedLayerWitness<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedFirstCertificate<T, I> {
    Singleton(RewriteResidualDiamond<T, I>),
    Square(Box<RevisionEffectResidualSquareLayerCertificate<T, I>>),
    Normalized(Box<RevisionEffectResidualNormalizedLayerCertificate<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedSquareStepWitness<T, I> {
    pub left: RewriteConcurrentPairWitness<T, I>,
    pub right: RewriteConcurrentPairWitness<T, I>,
    pub transport: RevisionEffectResidualChainStepWitness<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedStepWitness<T, I> {
    Singleton(Box<RevisionEffectResidualChainStepWitness<T, I>>),
    Square(Box<RevisionEffectResidualMixedSquareStepWitness<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedSquareStepCertificate<T, I> {
    pub left_frontier: BTreeSet<RevisionEffectId>,
    pub right_frontier: BTreeSet<RevisionEffectId>,
    pub left_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub right_normalization: RewriteConcurrentPairCertificate<T, I>,
    pub transport: RevisionEffectResidualChainStepCertificate<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualMixedStepCertificate<T, I> {
    Singleton(Box<RevisionEffectResidualChainStepCertificate<T, I>>),
    Square(Box<RevisionEffectResidualMixedSquareStepCertificate<T, I>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedChainWitness<T, I> {
    pub first: RevisionEffectResidualMixedFirstWitness<T, I>,
    pub subsequent: Vec<RevisionEffectResidualMixedStepWitness<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualMixedChainCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RevisionEffectResidualMixedFirstCertificate<T, I>,
    pub subsequent: Vec<RevisionEffectResidualMixedStepCertificate<T, I>>,
    common_endpoint: T,
}

impl<T, I> RevisionEffectResidualMixedChainCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        &self.common_endpoint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectTwoLayerResidualWitness<T, I> {
    pub first_right_after_left: PreparedRewrite<T, I>,
    pub first_left_after_right: PreparedRewrite<T, I>,
    pub right_prefix_after_left_second: PreparedRewrite<T, I>,
    pub left_second_after_right_prefix: PreparedRewrite<T, I>,
    pub left_prefix_after_right_second: PreparedRewrite<T, I>,
    pub right_second_after_left_prefix: PreparedRewrite<T, I>,
    pub second_right_after_left: PreparedRewrite<T, I>,
    pub second_left_after_right: PreparedRewrite<T, I>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectTwoLayerResidualCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RewriteResidualDiamond<T, I>,
    pub left_transport: RewriteResidualDiamond<T, I>,
    pub right_transport: RewriteResidualDiamond<T, I>,
    pub second: RewriteResidualDiamond<T, I>,
    common_endpoint: T,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainStepWitness<T, I> {
    pub right_prefix_after_left: PreparedRewrite<T, I>,
    pub left_after_right_prefix: PreparedRewrite<T, I>,
    pub left_prefix_after_right: PreparedRewrite<T, I>,
    pub right_after_left_prefix: PreparedRewrite<T, I>,
    pub cross_right_after_left: PreparedRewrite<T, I>,
    pub cross_left_after_right: PreparedRewrite<T, I>,
    pub cumulative_right_prefix: Option<PreparedRewrite<T, I>>,
    pub cumulative_left_prefix: Option<PreparedRewrite<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainWitness<T, I> {
    pub first_right_after_left: PreparedRewrite<T, I>,
    pub first_left_after_right: PreparedRewrite<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepWitness<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainStepCertificate<T, I> {
    pub left_transport: RewriteResidualDiamond<T, I>,
    pub right_transport: RewriteResidualDiamond<T, I>,
    pub cross: RewriteResidualDiamond<T, I>,
    pub cumulative_right_prefix: Option<RewriteSequentialComposition<T, I>>,
    pub cumulative_left_prefix: Option<RewriteSequentialComposition<T, I>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionEffectResidualChainCertificate<T, I> {
    pub schedule: RevisionEffectCausalLayerSchedule,
    pub first: RewriteResidualDiamond<T, I>,
    pub subsequent: Vec<RevisionEffectResidualChainStepCertificate<T, I>>,
    common_endpoint: T,
}

impl<T, I> RevisionEffectResidualChainCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        &self.common_endpoint
    }
}

impl<T, I> RevisionEffectTwoLayerResidualCertificate<T, I> {
    #[must_use]
    pub const fn common_endpoint(&self) -> &T {
        &self.common_endpoint
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RevisionEffectResidualLayerError {
    Ideal(RevisionEffectIdealError),
    IntentConflicts(BTreeSet<(RevisionEffectId, RevisionEffectId)>),
    NonFrontierExclusiveEffect(RevisionEffectId),
    MissingResidualPair(RevisionEffectId, RevisionEffectId),
    UnsupportedLayerShape,
    WitnessLayerCountMismatch,
    ConcurrentCompositeIntentMismatch,
    Residual(RewriteResidualRegistryError),
    Sequential(RewriteSequentialRegistryError),
}

impl From<RevisionEffectIdealError> for RevisionEffectResidualLayerError {
    fn from(value: RevisionEffectIdealError) -> Self {
        Self::Ideal(value)
    }
}

impl From<RewriteResidualRegistryError> for RevisionEffectResidualLayerError {
    fn from(value: RewriteResidualRegistryError) -> Self {
        Self::Residual(value)
    }
}

impl From<RewriteSequentialRegistryError> for RevisionEffectResidualLayerError {
    fn from(value: RewriteSequentialRegistryError) -> Self {
        Self::Sequential(value)
    }
}

struct ResidualChainProgress<T, I> {
    left_branch_base: T,
    right_branch_base: T,
    merged_base: T,
    cumulative_right_prefix: PreparedRewrite<T, I>,
    cumulative_left_prefix: PreparedRewrite<T, I>,
}

struct ResidualChainStep<'a, T, I> {
    left_rewrite: &'a PreparedRewrite<T, I>,
    right_rewrite: &'a PreparedRewrite<T, I>,
    witness: RevisionEffectResidualChainStepWitness<T, I>,
    needs_next_prefix: bool,
}

struct ResidualChainRegistries<'a> {
    residual: &'a RewriteResidualFamilyRegistry,
    sequential: &'a RewriteSequentialFamilyRegistry,
}

struct ResidualSquareLayer<T, I> {
    common: BTreeSet<RevisionEffectId>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualSquareLayerWitness<T, I>,
}

struct ResidualSquareNormalization<'a, T, I> {
    ideal: &'a RevisionEffectIdeal<PreparedRewrite<T, I>>,
    frontier: &'a BTreeSet<RevisionEffectId>,
    witness: RewriteConcurrentPairWitness<T, I>,
}

struct ResidualMixedFirstLayer<'a, T, I> {
    left: &'a RevisionEffectIdeal<PreparedRewrite<T, I>>,
    right: &'a RevisionEffectIdeal<PreparedRewrite<T, I>>,
    common: BTreeSet<RevisionEffectId>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualMixedFirstWitness<T, I>,
}

struct ResidualNormalizedMixedFirstLayer<'a, T, I> {
    left: &'a RevisionEffectIdeal<PreparedRewrite<T, I>>,
    right: &'a RevisionEffectIdeal<PreparedRewrite<T, I>>,
    common: BTreeSet<RevisionEffectId>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualNormalizedLayerWitness<T, I>,
}

struct ResidualMixedLayer<'a, T, I> {
    left: &'a RevisionEffectIdeal<PreparedRewrite<T, I>>,
    right: &'a RevisionEffectIdeal<PreparedRewrite<T, I>>,
    left_frontier: BTreeSet<RevisionEffectId>,
    right_frontier: BTreeSet<RevisionEffectId>,
    witness: RevisionEffectResidualMixedStepWitness<T, I>,
    needs_next_prefix: bool,
}

struct ResidualMixedFirstResult<T, I> {
    certificate: RevisionEffectResidualMixedFirstCertificate<T, I>,
    progress: ResidualChainProgress<T, I>,
}

fn certify_concurrent_pair<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    base: &T,
    left: &PreparedRewrite<T, I>,
    right: &PreparedRewrite<T, I>,
    residual_registry: &RewriteResidualFamilyRegistry,
    sequential_registry: &RewriteSequentialFamilyRegistry,
    witness: RewriteConcurrentPairWitness<T, I>,
) -> Result<RewriteConcurrentPairCertificate<T, I>, RevisionEffectResidualLayerError> {
    let diamond = residual_registry.certify(
        base,
        left,
        right,
        witness.right_after_left,
        witness.left_after_right,
    )?;
    let left_then_right = sequential_registry.certify(
        base,
        left.clone(),
        diamond.right_after_left.clone(),
        witness.left_then_right_composite,
    )?;
    let right_then_left = sequential_registry.certify(
        base,
        right.clone(),
        diamond.left_after_right.clone(),
        witness.right_then_left_composite,
    )?;
    if left_then_right.composite != right_then_left.composite {
        return Err(RevisionEffectResidualLayerError::ConcurrentCompositeIntentMismatch);
    }
    Ok(RewriteConcurrentPairCertificate {
        diamond,
        composite: left_then_right.composite.clone(),
        left_then_right,
        right_then_left,
    })
}

pub fn certify_registered_concurrent_triple<
    T: Clone + PartialEq + Eq,
    I: Clone + PartialEq + Eq,
>(
    base: &T,
    a: &PreparedRewrite<T, I>,
    b: &PreparedRewrite<T, I>,
    c: &PreparedRewrite<T, I>,
    residual_registry: &RewriteResidualFamilyRegistry,
    sequential_registry: &RewriteSequentialFamilyRegistry,
    witness: RewriteConcurrentTripleWitness<T, I>,
) -> Result<RewriteConcurrentTripleCertificate<T, I>, RevisionEffectResidualLayerError> {
    let ab = certify_concurrent_pair(
        base,
        a,
        b,
        residual_registry,
        sequential_registry,
        RewriteConcurrentPairWitness {
            right_after_left: witness.cube.b_after_a.clone(),
            left_after_right: witness.cube.a_after_b.clone(),
            left_then_right_composite: witness.ab_composite.clone(),
            right_then_left_composite: witness.ab_composite.clone(),
        },
    )?;
    let ac = certify_concurrent_pair(
        base,
        a,
        c,
        residual_registry,
        sequential_registry,
        RewriteConcurrentPairWitness {
            right_after_left: witness.cube.c_after_a.clone(),
            left_after_right: witness.cube.a_after_c.clone(),
            left_then_right_composite: witness.ac_composite.clone(),
            right_then_left_composite: witness.ac_composite.clone(),
        },
    )?;
    let bc = certify_concurrent_pair(
        base,
        b,
        c,
        residual_registry,
        sequential_registry,
        RewriteConcurrentPairWitness {
            right_after_left: witness.cube.c_after_b.clone(),
            left_after_right: witness.cube.b_after_c.clone(),
            left_then_right_composite: witness.bc_composite.clone(),
            right_then_left_composite: witness.bc_composite.clone(),
        },
    )?;
    let cube = residual_registry.certify_cube(base, a, b, c, witness.cube)?;
    let final_paths = vec![
        sequential_registry.certify(
            base,
            ab.composite().clone(),
            cube.after_a.right_after_left.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify(
            base,
            ab.composite().clone(),
            cube.after_b.right_after_left.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify(
            base,
            ac.composite().clone(),
            cube.after_a.left_after_right.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify(
            base,
            ac.composite().clone(),
            cube.after_c.right_after_left.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify(
            base,
            bc.composite().clone(),
            cube.after_b.left_after_right.clone(),
            witness.final_composite.clone(),
        )?,
        sequential_registry.certify(
            base,
            bc.composite().clone(),
            cube.after_c.left_after_right.clone(),
            witness.final_composite.clone(),
        )?,
    ];
    Ok(RewriteConcurrentTripleCertificate {
        cube,
        ab,
        ac,
        bc,
        final_paths,
        composite: witness.final_composite,
    })
}

fn certify_concurrent_branch<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    base: &T,
    ideal: &RevisionEffectIdeal<PreparedRewrite<T, I>>,
    frontier: &BTreeSet<RevisionEffectId>,
    residual_registry: &RewriteResidualFamilyRegistry,
    sequential_registry: &RewriteSequentialFamilyRegistry,
    witness: RewriteConcurrentBranchWitness<T, I>,
) -> Result<RewriteConcurrentBranchCertificate<T, I>, RevisionEffectResidualLayerError> {
    match (frontier.len(), witness) {
        (1, RewriteConcurrentBranchWitness::Single) => {
            let id = *frontier.first().expect("shape checked");
            Ok(RewriteConcurrentBranchCertificate::Single(
                ideal.events[&id].payload.clone(),
            ))
        }
        (2, RewriteConcurrentBranchWitness::Pair(witness)) => {
            let mut ids = frontier.iter().copied();
            let first = ids.next().expect("shape checked");
            let second = ids.next().expect("shape checked");
            let certificate = certify_concurrent_pair(
                base,
                &ideal.events[&first].payload,
                &ideal.events[&second].payload,
                residual_registry,
                sequential_registry,
                *witness,
            )?;
            Ok(RewriteConcurrentBranchCertificate::Pair(Box::new(
                certificate,
            )))
        }
        (3, RewriteConcurrentBranchWitness::Triple(witness)) => {
            let mut ids = frontier.iter().copied();
            let a = ids.next().expect("shape checked");
            let b = ids.next().expect("shape checked");
            let c = ids.next().expect("shape checked");
            let certificate = certify_registered_concurrent_triple(
                base,
                &ideal.events[&a].payload,
                &ideal.events[&b].payload,
                &ideal.events[&c].payload,
                residual_registry,
                sequential_registry,
                *witness,
            )?;
            Ok(RewriteConcurrentBranchCertificate::Triple(Box::new(
                certificate,
            )))
        }
        _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
    }
}

fn certify_square_normalization<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    base: &T,
    registries: &ResidualChainRegistries<'_>,
    side: ResidualSquareNormalization<'_, T, I>,
) -> Result<RewriteConcurrentPairCertificate<T, I>, RevisionEffectResidualLayerError> {
    if side.frontier.len() != 2 {
        return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
    }
    let mut ids = side.frontier.iter().copied();
    let first = ids.next().expect("shape checked");
    let second = ids.next().expect("shape checked");
    certify_concurrent_pair(
        base,
        &side.ideal.events[&first].payload,
        &side.ideal.events[&second].payload,
        registries.residual,
        registries.sequential,
        side.witness,
    )
}

fn certify_residual_chain_step<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    progress: &mut ResidualChainProgress<T, I>,
    step: ResidualChainStep<'_, T, I>,
    registries: &ResidualChainRegistries<'_>,
) -> Result<RevisionEffectResidualChainStepCertificate<T, I>, RevisionEffectResidualLayerError> {
    let left_transport = registries.residual.certify(
        &progress.left_branch_base,
        step.left_rewrite,
        &progress.cumulative_right_prefix,
        step.witness.right_prefix_after_left,
        step.witness.left_after_right_prefix,
    )?;
    let right_transport = registries.residual.certify(
        &progress.right_branch_base,
        step.right_rewrite,
        &progress.cumulative_left_prefix,
        step.witness.left_prefix_after_right,
        step.witness.right_after_left_prefix,
    )?;
    let cross = registries.residual.certify(
        &progress.merged_base,
        &left_transport.left_after_right,
        &right_transport.left_after_right,
        step.witness.cross_right_after_left,
        step.witness.cross_left_after_right,
    )?;
    progress.left_branch_base = step.left_rewrite.apply(&progress.left_branch_base);
    progress.right_branch_base = step.right_rewrite.apply(&progress.right_branch_base);
    progress.merged_base = cross.common_endpoint().clone();

    let cumulative_right_prefix = if step.needs_next_prefix {
        let composite = step
            .witness
            .cumulative_right_prefix
            .ok_or(RevisionEffectResidualLayerError::WitnessLayerCountMismatch)?;
        let certificate = registries.sequential.certify(
            &progress.left_branch_base,
            left_transport.right_after_left.clone(),
            cross.right_after_left.clone(),
            composite.clone(),
        )?;
        progress.cumulative_right_prefix = composite;
        Some(certificate)
    } else {
        None
    };
    let cumulative_left_prefix = if step.needs_next_prefix {
        let composite = step
            .witness
            .cumulative_left_prefix
            .ok_or(RevisionEffectResidualLayerError::WitnessLayerCountMismatch)?;
        let certificate = registries.sequential.certify(
            &progress.right_branch_base,
            right_transport.right_after_left.clone(),
            cross.left_after_right.clone(),
            composite.clone(),
        )?;
        progress.cumulative_left_prefix = composite;
        Some(certificate)
    } else {
        None
    };
    Ok(RevisionEffectResidualChainStepCertificate {
        left_transport,
        right_transport,
        cross,
        cumulative_right_prefix,
        cumulative_left_prefix,
    })
}

fn certify_mixed_first_layer<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    base: &T,
    registries: &ResidualChainRegistries<'_>,
    layer: ResidualMixedFirstLayer<'_, T, I>,
) -> Result<ResidualMixedFirstResult<T, I>, RevisionEffectResidualLayerError> {
    match layer.witness {
        RevisionEffectResidualMixedFirstWitness::Singleton {
            right_after_left,
            left_after_right,
        } if layer.left_frontier.len() == 1 && layer.right_frontier.len() == 1 => {
            let left_id = *layer.left_frontier.first().expect("shape checked");
            let right_id = *layer.right_frontier.first().expect("shape checked");
            let left_rewrite = &layer.left.events[&left_id].payload;
            let right_rewrite = &layer.right.events[&right_id].payload;
            let first = registries.residual.certify(
                base,
                left_rewrite,
                right_rewrite,
                right_after_left,
                left_after_right,
            )?;
            let progress = ResidualChainProgress {
                left_branch_base: left_rewrite.apply(base),
                right_branch_base: right_rewrite.apply(base),
                merged_base: first.common_endpoint().clone(),
                cumulative_right_prefix: first.right_after_left.clone(),
                cumulative_left_prefix: first.left_after_right.clone(),
            };
            Ok(ResidualMixedFirstResult {
                certificate: RevisionEffectResidualMixedFirstCertificate::Singleton(first),
                progress,
            })
        }
        RevisionEffectResidualMixedFirstWitness::Square(witness)
            if layer.left_frontier.len() == 2 && layer.right_frontier.len() == 2 =>
        {
            let first = layer.left.certify_registered_residual_square_layer(
                layer.right,
                base,
                registries,
                ResidualSquareLayer {
                    common: layer.common,
                    left_frontier: layer.left_frontier,
                    right_frontier: layer.right_frontier,
                    witness: *witness,
                },
            )?;
            let progress = ResidualChainProgress {
                left_branch_base: first.left_normalization.composite().apply(base),
                right_branch_base: first.right_normalization.composite().apply(base),
                merged_base: first.common_endpoint().clone(),
                cumulative_right_prefix: first.cross.right_after_left.clone(),
                cumulative_left_prefix: first.cross.left_after_right.clone(),
            };
            Ok(ResidualMixedFirstResult {
                certificate: RevisionEffectResidualMixedFirstCertificate::Square(Box::new(first)),
                progress,
            })
        }
        RevisionEffectResidualMixedFirstWitness::Normalized(witness)
            if (1..=3).contains(&layer.left_frontier.len())
                && (1..=3).contains(&layer.right_frontier.len()) =>
        {
            certify_normalized_mixed_first_layer(
                base,
                registries,
                ResidualNormalizedMixedFirstLayer {
                    left: layer.left,
                    right: layer.right,
                    common: layer.common,
                    left_frontier: layer.left_frontier,
                    right_frontier: layer.right_frontier,
                    witness: *witness,
                },
            )
        }
        _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
    }
}

fn certify_normalized_mixed_first_layer<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    base: &T,
    registries: &ResidualChainRegistries<'_>,
    layer: ResidualNormalizedMixedFirstLayer<'_, T, I>,
) -> Result<ResidualMixedFirstResult<T, I>, RevisionEffectResidualLayerError> {
    let RevisionEffectResidualNormalizedLayerWitness {
        left,
        right,
        right_after_left,
        left_after_right,
    } = layer.witness;
    let left_normalization = certify_concurrent_branch(
        base,
        layer.left,
        &layer.left_frontier,
        registries.residual,
        registries.sequential,
        left,
    )?;
    let right_normalization = certify_concurrent_branch(
        base,
        layer.right,
        &layer.right_frontier,
        registries.residual,
        registries.sequential,
        right,
    )?;
    let cross = registries.residual.certify(
        base,
        left_normalization.composite(),
        right_normalization.composite(),
        right_after_left,
        left_after_right,
    )?;
    let progress = ResidualChainProgress {
        left_branch_base: left_normalization.composite().apply(base),
        right_branch_base: right_normalization.composite().apply(base),
        merged_base: cross.common_endpoint().clone(),
        cumulative_right_prefix: cross.right_after_left.clone(),
        cumulative_left_prefix: cross.left_after_right.clone(),
    };
    Ok(ResidualMixedFirstResult {
        certificate: RevisionEffectResidualMixedFirstCertificate::Normalized(Box::new(
            RevisionEffectResidualNormalizedLayerCertificate {
                common: layer.common,
                left_frontier: layer.left_frontier,
                right_frontier: layer.right_frontier,
                left_normalization,
                right_normalization,
                cross,
            },
        )),
        progress,
    })
}

fn certify_mixed_chain_step<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>(
    progress: &mut ResidualChainProgress<T, I>,
    registries: &ResidualChainRegistries<'_>,
    layer: ResidualMixedLayer<'_, T, I>,
) -> Result<RevisionEffectResidualMixedStepCertificate<T, I>, RevisionEffectResidualLayerError> {
    match layer.witness {
        RevisionEffectResidualMixedStepWitness::Singleton(witness)
            if layer.left_frontier.len() == 1 && layer.right_frontier.len() == 1 =>
        {
            let left_id = *layer.left_frontier.first().expect("shape checked");
            let right_id = *layer.right_frontier.first().expect("shape checked");
            let certificate = certify_residual_chain_step(
                progress,
                ResidualChainStep {
                    left_rewrite: &layer.left.events[&left_id].payload,
                    right_rewrite: &layer.right.events[&right_id].payload,
                    witness: *witness,
                    needs_next_prefix: layer.needs_next_prefix,
                },
                registries,
            )?;
            Ok(RevisionEffectResidualMixedStepCertificate::Singleton(
                Box::new(certificate),
            ))
        }
        RevisionEffectResidualMixedStepWitness::Square(witness)
            if layer.left_frontier.len() == 2 && layer.right_frontier.len() == 2 =>
        {
            let RevisionEffectResidualMixedSquareStepWitness {
                left,
                right,
                transport,
            } = *witness;
            let left_normalization = certify_square_normalization(
                &progress.left_branch_base,
                registries,
                ResidualSquareNormalization {
                    ideal: layer.left,
                    frontier: &layer.left_frontier,
                    witness: left,
                },
            )?;
            let right_normalization = certify_square_normalization(
                &progress.right_branch_base,
                registries,
                ResidualSquareNormalization {
                    ideal: layer.right,
                    frontier: &layer.right_frontier,
                    witness: right,
                },
            )?;
            let certificate = certify_residual_chain_step(
                progress,
                ResidualChainStep {
                    left_rewrite: left_normalization.composite(),
                    right_rewrite: right_normalization.composite(),
                    witness: transport,
                    needs_next_prefix: layer.needs_next_prefix,
                },
                registries,
            )?;
            Ok(RevisionEffectResidualMixedStepCertificate::Square(
                Box::new(RevisionEffectResidualMixedSquareStepCertificate {
                    left_frontier: layer.left_frontier,
                    right_frontier: layer.right_frontier,
                    left_normalization,
                    right_normalization,
                    transport: certificate,
                }),
            ))
        }
        _ => Err(RevisionEffectResidualLayerError::UnsupportedLayerShape),
    }
}

impl<E: Clone + PartialEq + Eq> RevisionEffectIdeal<E> {
    pub fn new(
        events: impl IntoIterator<Item = RevisionEffect<E>>,
    ) -> Result<Self, RevisionEffectIdealError> {
        let mut by_id = BTreeMap::new();
        for event in events {
            let id = event.id;
            if by_id.insert(id, event).is_some() {
                return Err(RevisionEffectIdealError::DuplicateEffect(id));
            }
        }
        for event in by_id.values() {
            for prerequisite in &event.prerequisites {
                if !by_id.contains_key(prerequisite) {
                    return Err(RevisionEffectIdealError::MissingPrerequisite {
                        effect: event.id,
                        prerequisite: *prerequisite,
                    });
                }
            }
        }
        let mut remaining = by_id
            .iter()
            .map(|(&id, event)| (id, event.prerequisites.len()))
            .collect::<BTreeMap<_, _>>();
        let mut ready = remaining
            .iter()
            .filter_map(|(&id, &count)| (count == 0).then_some(id))
            .collect::<Vec<_>>();
        while let Some(completed) = ready.pop() {
            let Some(count) = remaining.remove(&completed) else {
                continue;
            };
            debug_assert_eq!(count, 0);
            for event in by_id.values() {
                if event.prerequisites.contains(&completed)
                    && let Some(pending) = remaining.get_mut(&event.id)
                {
                    *pending -= 1;
                    if *pending == 0 {
                        ready.push(event.id);
                    }
                }
            }
        }
        if !remaining.is_empty() {
            return Err(RevisionEffectIdealError::CyclicPrerequisites(
                remaining.into_keys().collect(),
            ));
        }
        Ok(Self { events: by_id })
    }

    #[must_use]
    pub const fn events(&self) -> &BTreeMap<RevisionEffectId, RevisionEffect<E>> {
        &self.events
    }

    pub fn common_ideal(&self, other: &Self) -> Result<Self, RevisionEffectIdealError> {
        let mut common = Vec::new();
        for (&id, event) in &self.events {
            let Some(other_event) = other.events.get(&id) else {
                continue;
            };
            if event != other_event {
                return Err(RevisionEffectIdealError::EffectIdentityConflict(id));
            }
            common.push(event.clone());
        }
        Self::new(common)
    }

    pub fn exclusive_from(
        &self,
        common: &Self,
    ) -> Result<Vec<&RevisionEffect<E>>, RevisionEffectIdealError> {
        for (&id, event) in &common.events {
            if self.events.get(&id) != Some(event) {
                return Err(RevisionEffectIdealError::NotSubideal);
            }
        }
        Ok(self
            .events
            .iter()
            .filter_map(|(id, event)| (!common.events.contains_key(id)).then_some(event))
            .collect())
    }

    pub fn union(&self, other: &Self) -> Result<Self, RevisionEffectIdealError> {
        let mut union = self.events.values().cloned().collect::<Vec<_>>();
        for (&id, event) in &other.events {
            if let Some(existing) = self.events.get(&id) {
                if existing != event {
                    return Err(RevisionEffectIdealError::EffectIdentityConflict(id));
                }
            } else {
                union.push(event.clone());
            }
        }
        Self::new(union)
    }

    pub fn merge_requirements(
        &self,
        other: &Self,
        classify: impl Fn(&E, &E) -> PairCoordinationDecision,
    ) -> Result<RevisionEffectMergeRequirements, RevisionEffectIdealError> {
        let common = self.common_ideal(other)?;
        let left = self.exclusive_from(&common)?;
        let right = other.exclusive_from(&common)?;
        let mut requires_residual = BTreeSet::new();
        let mut intent_conflicts = BTreeSet::new();
        for left_event in &left {
            for right_event in &right {
                let pair = (left_event.id, right_event.id);
                match classify(&left_event.payload, &right_event.payload) {
                    PairCoordinationDecision::CoordinationFree => {}
                    PairCoordinationDecision::RequiresCoordination => {
                        requires_residual.insert(pair);
                    }
                    PairCoordinationDecision::IntentConflict => {
                        intent_conflicts.insert(pair);
                    }
                }
            }
        }
        Ok(RevisionEffectMergeRequirements {
            common: common.events.keys().copied().collect(),
            left_exclusive: left.into_iter().map(|event| event.id).collect(),
            right_exclusive: right.into_iter().map(|event| event.id).collect(),
            requires_residual,
            intent_conflicts,
        })
    }

    fn exclusive_causal_layers_from(
        &self,
        common: &Self,
    ) -> Result<Vec<BTreeSet<RevisionEffectId>>, RevisionEffectIdealError> {
        let exclusive = self.exclusive_from(common)?;
        let mut admitted = common.events.keys().copied().collect::<BTreeSet<_>>();
        let mut remaining = exclusive
            .into_iter()
            .map(|event| (event.id, event))
            .collect::<BTreeMap<_, _>>();
        let mut layers = Vec::new();
        while !remaining.is_empty() {
            let layer = remaining
                .iter()
                .filter_map(|(&id, event)| event.prerequisites.is_subset(&admitted).then_some(id))
                .collect::<BTreeSet<_>>();
            if layer.is_empty() {
                return Err(RevisionEffectIdealError::CyclicPrerequisites(
                    remaining.into_keys().collect(),
                ));
            }
            for id in &layer {
                remaining.remove(id);
            }
            admitted.extend(layer.iter().copied());
            layers.push(layer);
        }
        Ok(layers)
    }

    pub fn causal_layer_schedule(
        &self,
        other: &Self,
    ) -> Result<RevisionEffectCausalLayerSchedule, RevisionEffectIdealError> {
        let common = self.common_ideal(other)?;
        Ok(RevisionEffectCausalLayerSchedule {
            common: common.events.keys().copied().collect(),
            left_layers: self.exclusive_causal_layers_from(&common)?,
            right_layers: other.exclusive_causal_layers_from(&common)?,
        })
    }
}

impl<T: Clone + PartialEq + Eq, I: Clone + PartialEq + Eq>
    RevisionEffectIdeal<PreparedRewrite<T, I>>
{
    fn certify_registered_residual_square_layer(
        &self,
        other: &Self,
        base: &T,
        registries: &ResidualChainRegistries<'_>,
        layer: ResidualSquareLayer<T, I>,
    ) -> Result<RevisionEffectResidualSquareLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let ResidualSquareLayer {
            common,
            left_frontier,
            right_frontier,
            witness,
        } = layer;
        if left_frontier.len() != 2 || right_frontier.len() != 2 {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        let mut left_ids = left_frontier.iter().copied();
        let left_a = left_ids.next().expect("shape checked");
        let left_b = left_ids.next().expect("shape checked");
        let mut right_ids = right_frontier.iter().copied();
        let right_a = right_ids.next().expect("shape checked");
        let right_b = right_ids.next().expect("shape checked");

        let left_normalization = certify_concurrent_pair(
            base,
            &self.events[&left_a].payload,
            &self.events[&left_b].payload,
            registries.residual,
            registries.sequential,
            witness.left,
        )?;
        let right_normalization = certify_concurrent_pair(
            base,
            &other.events[&right_a].payload,
            &other.events[&right_b].payload,
            registries.residual,
            registries.sequential,
            witness.right,
        )?;
        let cross = registries.residual.certify(
            base,
            left_normalization.composite(),
            right_normalization.composite(),
            witness.right_after_left,
            witness.left_after_right,
        )?;
        Ok(RevisionEffectResidualSquareLayerCertificate {
            common,
            left_frontier,
            right_frontier,
            left_normalization,
            right_normalization,
            cross,
        })
    }

    pub fn certify_registered_residual_square_frontier(
        &self,
        other: &Self,
        base: &T,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualSquareLayerWitness<T, I>,
    ) -> Result<RevisionEffectResidualSquareLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        if schedule.left_layers.len() != 1
            || schedule.right_layers.len() != 1
            || schedule.left_layers[0].len() != 2
            || schedule.right_layers[0].len() != 2
        {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        self.certify_registered_residual_square_layer(
            other,
            base,
            &ResidualChainRegistries {
                residual: residual_registry,
                sequential: sequential_registry,
            },
            ResidualSquareLayer {
                common: schedule.common,
                left_frontier: schedule.left_layers[0].clone(),
                right_frontier: schedule.right_layers[0].clone(),
                witness,
            },
        )
    }

    pub fn certify_registered_residual_square_chain(
        &self,
        other: &Self,
        base: &T,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualSquareChainWitness<T, I>,
    ) -> Result<RevisionEffectResidualSquareChainCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        let depth = schedule.left_layers.len();
        if depth == 0
            || schedule.right_layers.len() != depth
            || schedule.left_layers[0].len() != 2
            || schedule.right_layers[0].len() != 2
            || schedule.left_layers[1..]
                .iter()
                .any(|layer| layer.len() != 1)
            || schedule.right_layers[1..]
                .iter()
                .any(|layer| layer.len() != 1)
        {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        if witness.subsequent.len() + 1 != depth {
            return Err(RevisionEffectResidualLayerError::WitnessLayerCountMismatch);
        }
        let first = self.certify_registered_residual_square_layer(
            other,
            base,
            &ResidualChainRegistries {
                residual: residual_registry,
                sequential: sequential_registry,
            },
            ResidualSquareLayer {
                common: schedule.common.clone(),
                left_frontier: schedule.left_layers[0].clone(),
                right_frontier: schedule.right_layers[0].clone(),
                witness: witness.first,
            },
        )?;
        let left_composite = first.left_normalization.composite().clone();
        let right_composite = first.right_normalization.composite().clone();
        let mut progress = ResidualChainProgress {
            left_branch_base: left_composite.apply(base),
            right_branch_base: right_composite.apply(base),
            merged_base: first.common_endpoint().clone(),
            cumulative_right_prefix: first.cross.right_after_left.clone(),
            cumulative_left_prefix: first.cross.left_after_right.clone(),
        };
        let mut certificates = Vec::with_capacity(depth.saturating_sub(1));
        for layer_index in 1..depth {
            let left_id = *schedule.left_layers[layer_index]
                .first()
                .expect("shape checked");
            let right_id = *schedule.right_layers[layer_index]
                .first()
                .expect("shape checked");
            certificates.push(certify_residual_chain_step(
                &mut progress,
                ResidualChainStep {
                    left_rewrite: &self.events[&left_id].payload,
                    right_rewrite: &other.events[&right_id].payload,
                    witness: witness.subsequent[layer_index - 1].clone(),
                    needs_next_prefix: layer_index + 1 < depth,
                },
                &ResidualChainRegistries {
                    residual: residual_registry,
                    sequential: sequential_registry,
                },
            )?);
        }
        Ok(RevisionEffectResidualSquareChainCertificate {
            schedule,
            first,
            subsequent: certificates,
            common_endpoint: progress.merged_base,
        })
    }

    pub fn certify_registered_residual_mixed_chain(
        &self,
        other: &Self,
        base: &T,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualMixedChainWitness<T, I>,
    ) -> Result<RevisionEffectResidualMixedChainCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        let depth = schedule.left_layers.len();
        if depth == 0 || schedule.right_layers.len() != depth {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        if witness.subsequent.len() + 1 != depth {
            return Err(RevisionEffectResidualLayerError::WitnessLayerCountMismatch);
        }
        let registries = ResidualChainRegistries {
            residual: residual_registry,
            sequential: sequential_registry,
        };
        let first_result = certify_mixed_first_layer(
            base,
            &registries,
            ResidualMixedFirstLayer {
                left: self,
                right: other,
                common: schedule.common.clone(),
                left_frontier: schedule.left_layers[0].clone(),
                right_frontier: schedule.right_layers[0].clone(),
                witness: witness.first,
            },
        )?;
        let first = first_result.certificate;
        let mut progress = first_result.progress;
        let mut certificates = Vec::with_capacity(depth.saturating_sub(1));
        for (offset, layer_witness) in witness.subsequent.into_iter().enumerate() {
            let layer_index = offset + 1;
            certificates.push(certify_mixed_chain_step(
                &mut progress,
                &registries,
                ResidualMixedLayer {
                    left: self,
                    right: other,
                    left_frontier: schedule.left_layers[layer_index].clone(),
                    right_frontier: schedule.right_layers[layer_index].clone(),
                    witness: layer_witness,
                    needs_next_prefix: layer_index + 1 < depth,
                },
            )?);
        }
        Ok(RevisionEffectResidualMixedChainCertificate {
            schedule,
            first,
            subsequent: certificates,
            common_endpoint: progress.merged_base,
        })
    }

    pub fn certify_registered_residual_cube_frontier(
        &self,
        other: &Self,
        base: &T,
        registry: &RewriteResidualFamilyRegistry,
        witness: RewriteResidualCubeWitness<T, I>,
    ) -> Result<RevisionEffectResidualCubeLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        if schedule.left_layers.len() != 1 || schedule.right_layers.len() != 1 {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        let left_frontier = schedule.left_layers[0].clone();
        let right_frontier = schedule.right_layers[0].clone();
        if !matches!((left_frontier.len(), right_frontier.len()), (2, 1) | (1, 2)) {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }

        let mut ordered = left_frontier
            .iter()
            .map(|id| (0_u8, *id, &self.events[id].payload))
            .chain(
                right_frontier
                    .iter()
                    .map(|id| (1_u8, *id, &other.events[id].payload)),
            )
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(branch, id, _)| (*branch, *id));
        let [(_, _, a), (_, _, b), (_, _, c)] = ordered.as_slice() else {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        };
        let cube = registry.certify_cube(base, a, b, c, witness)?;

        Ok(RevisionEffectResidualCubeLayerCertificate {
            common: schedule.common,
            left_frontier,
            right_frontier,
            cube,
        })
    }

    pub fn certify_registered_residual_normalized_frontier(
        &self,
        other: &Self,
        base: &T,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualNormalizedLayerWitness<T, I>,
    ) -> Result<
        RevisionEffectResidualNormalizedLayerCertificate<T, I>,
        RevisionEffectResidualLayerError,
    > {
        let schedule = self.causal_layer_schedule(other)?;
        if schedule.left_layers.len() != 1 || schedule.right_layers.len() != 1 {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        let left_frontier = schedule.left_layers[0].clone();
        let right_frontier = schedule.right_layers[0].clone();
        let left_normalization = certify_concurrent_branch(
            base,
            self,
            &left_frontier,
            residual_registry,
            sequential_registry,
            witness.left,
        )?;
        let right_normalization = certify_concurrent_branch(
            base,
            other,
            &right_frontier,
            residual_registry,
            sequential_registry,
            witness.right,
        )?;
        let cross = residual_registry.certify(
            base,
            left_normalization.composite(),
            right_normalization.composite(),
            witness.right_after_left,
            witness.left_after_right,
        )?;
        Ok(RevisionEffectResidualNormalizedLayerCertificate {
            common: schedule.common,
            left_frontier,
            right_frontier,
            left_normalization,
            right_normalization,
            cross,
        })
    }

    pub fn certify_registered_residual_chain(
        &self,
        other: &Self,
        base: &T,
        residual_registry: &RewriteResidualFamilyRegistry,
        sequential_registry: &RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualChainWitness<T, I>,
    ) -> Result<RevisionEffectResidualChainCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        let depth = schedule.left_layers.len();
        if depth == 0
            || schedule.right_layers.len() != depth
            || schedule.left_layers.iter().any(|layer| layer.len() != 1)
            || schedule.right_layers.iter().any(|layer| layer.len() != 1)
        {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }
        if witness.subsequent.len() + 1 != depth {
            return Err(RevisionEffectResidualLayerError::WitnessLayerCountMismatch);
        }

        let left_first_id = *schedule.left_layers[0].first().expect("shape checked");
        let right_first_id = *schedule.right_layers[0].first().expect("shape checked");
        let left_first = &self.events[&left_first_id].payload;
        let right_first = &other.events[&right_first_id].payload;
        let first = residual_registry.certify(
            base,
            left_first,
            right_first,
            witness.first_right_after_left,
            witness.first_left_after_right,
        )?;
        let mut progress = ResidualChainProgress {
            left_branch_base: left_first.apply(base),
            right_branch_base: right_first.apply(base),
            merged_base: first.common_endpoint().clone(),
            cumulative_right_prefix: first.right_after_left.clone(),
            cumulative_left_prefix: first.left_after_right.clone(),
        };
        let mut certificates = Vec::with_capacity(depth.saturating_sub(1));

        for layer_index in 1..depth {
            let left_id = *schedule.left_layers[layer_index]
                .first()
                .expect("shape checked");
            let right_id = *schedule.right_layers[layer_index]
                .first()
                .expect("shape checked");
            let left_rewrite = &self.events[&left_id].payload;
            let right_rewrite = &other.events[&right_id].payload;
            certificates.push(certify_residual_chain_step(
                &mut progress,
                ResidualChainStep {
                    left_rewrite,
                    right_rewrite,
                    witness: witness.subsequent[layer_index - 1].clone(),
                    needs_next_prefix: layer_index + 1 < depth,
                },
                &ResidualChainRegistries {
                    residual: residual_registry,
                    sequential: sequential_registry,
                },
            )?);
        }

        Ok(RevisionEffectResidualChainCertificate {
            schedule,
            first,
            subsequent: certificates,
            common_endpoint: progress.merged_base,
        })
    }

    pub fn certify_registered_two_layer_chain(
        &self,
        other: &Self,
        base: &T,
        registry: &RewriteResidualFamilyRegistry,
        witness: RevisionEffectTwoLayerResidualWitness<T, I>,
    ) -> Result<RevisionEffectTwoLayerResidualCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let schedule = self.causal_layer_schedule(other)?;
        if schedule.left_layers.len() != 2
            || schedule.right_layers.len() != 2
            || schedule.left_layers.iter().any(|layer| layer.len() != 1)
            || schedule.right_layers.iter().any(|layer| layer.len() != 1)
        {
            return Err(RevisionEffectResidualLayerError::UnsupportedLayerShape);
        }

        let left_first_id = *schedule.left_layers[0].first().expect("shape checked");
        let left_second_id = *schedule.left_layers[1].first().expect("shape checked");
        let right_first_id = *schedule.right_layers[0].first().expect("shape checked");
        let right_second_id = *schedule.right_layers[1].first().expect("shape checked");
        let left_first = &self.events[&left_first_id].payload;
        let left_second = &self.events[&left_second_id].payload;
        let right_first = &other.events[&right_first_id].payload;
        let right_second = &other.events[&right_second_id].payload;

        let first = registry.certify(
            base,
            left_first,
            right_first,
            witness.first_right_after_left,
            witness.first_left_after_right,
        )?;
        let left_branch_base = left_first.apply(base);
        let right_branch_base = right_first.apply(base);
        let left_transport = registry.certify(
            &left_branch_base,
            left_second,
            &first.right_after_left,
            witness.right_prefix_after_left_second,
            witness.left_second_after_right_prefix,
        )?;
        let right_transport = registry.certify(
            &right_branch_base,
            right_second,
            &first.left_after_right,
            witness.left_prefix_after_right_second,
            witness.right_second_after_left_prefix,
        )?;
        let second = registry.certify(
            first.common_endpoint(),
            &left_transport.left_after_right,
            &right_transport.left_after_right,
            witness.second_right_after_left,
            witness.second_left_after_right,
        )?;
        let common_endpoint = second.common_endpoint().clone();

        Ok(RevisionEffectTwoLayerResidualCertificate {
            schedule,
            first,
            left_transport,
            right_transport,
            second,
            common_endpoint,
        })
    }

    /// Certifies one concurrent branch-exclusive frontier layer against the
    /// registered residual families.
    ///
    /// This deliberately does not pretend that a pair certificate can be
    /// reused for deeper causal suffixes. Every exclusive event admitted by
    /// this method must depend only on the common ideal. Deeper effects must
    /// first be residualized into a new frontier layer by a later consumer.
    pub fn certify_registered_residual_frontier(
        &self,
        other: &Self,
        base: &T,
        registry: &RewriteResidualFamilyRegistry,
        classify: impl Fn(&PreparedRewrite<T, I>, &PreparedRewrite<T, I>) -> PairCoordinationDecision,
        residuals: impl Fn(
            RevisionEffectId,
            RevisionEffectId,
        ) -> Option<(PreparedRewrite<T, I>, PreparedRewrite<T, I>)>,
    ) -> Result<RevisionEffectResidualLayerCertificate<T, I>, RevisionEffectResidualLayerError>
    {
        let requirements = self.merge_requirements(other, classify)?;
        if !requirements.intent_conflicts.is_empty() {
            return Err(RevisionEffectResidualLayerError::IntentConflicts(
                requirements.intent_conflicts,
            ));
        }

        let common = self.common_ideal(other)?;
        let common_ids = common.events.keys().copied().collect::<BTreeSet<_>>();
        let left = self.exclusive_from(&common)?;
        let right = other.exclusive_from(&common)?;

        for event in left.iter().chain(right.iter()) {
            if !event.prerequisites.is_subset(&common_ids) {
                return Err(RevisionEffectResidualLayerError::NonFrontierExclusiveEffect(event.id));
            }
        }

        let left_by_id = left
            .iter()
            .map(|event| (event.id, *event))
            .collect::<BTreeMap<_, _>>();
        let right_by_id = right
            .iter()
            .map(|event| (event.id, *event))
            .collect::<BTreeMap<_, _>>();
        let mut diamonds = BTreeMap::new();
        for &(left_id, right_id) in &requirements.requires_residual {
            let left_event = left_by_id[&left_id];
            let right_event = right_by_id[&right_id];
            let (right_after_left, left_after_right) = residuals(left_id, right_id).ok_or(
                RevisionEffectResidualLayerError::MissingResidualPair(left_id, right_id),
            )?;
            let diamond = registry.certify(
                base,
                &left_event.payload,
                &right_event.payload,
                right_after_left,
                left_after_right,
            )?;
            diamonds.insert((left_id, right_id), diamond);
        }

        Ok(RevisionEffectResidualLayerCertificate {
            common: common_ids,
            left_frontier: left.into_iter().map(|event| event.id).collect(),
            right_frontier: right.into_iter().map(|event| event.id).collect(),
            diamonds,
        })
    }
}

#[cfg(test)]
mod revision_effect_ideal_tests {
    use super::*;

    fn event(
        id: u128,
        prerequisites: &[u128],
        payload: &'static str,
    ) -> RevisionEffect<&'static str> {
        RevisionEffect {
            id: RevisionEffectId(id),
            prerequisites: prerequisites
                .iter()
                .copied()
                .map(RevisionEffectId)
                .collect(),
            payload,
        }
    }

    #[test]
    fn criss_cross_common_history_is_canonical_effect_intersection() {
        let root = event(1, &[], "root");
        let a = event(2, &[1], "a");
        let b = event(3, &[1], "b");
        let left_resolution = event(4, &[2, 3], "left=10");
        let right_resolution = event(5, &[2, 3], "right=20");
        let left =
            RevisionEffectIdeal::new([root.clone(), a.clone(), b.clone(), left_resolution.clone()])
                .unwrap();
        let right = RevisionEffectIdeal::new([
            root.clone(),
            a.clone(),
            b.clone(),
            right_resolution.clone(),
        ])
        .unwrap();

        let common = left.common_ideal(&right).unwrap();
        assert_eq!(
            common.events().keys().copied().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                RevisionEffectId(1),
                RevisionEffectId(2),
                RevisionEffectId(3),
            ])
        );
        assert_eq!(
            left.exclusive_from(&common).unwrap(),
            vec![&left_resolution]
        );
        assert_eq!(
            right.exclusive_from(&common).unwrap(),
            vec![&right_resolution]
        );
        assert_eq!(left.union(&right).unwrap().events().len(), 5);
    }

    #[test]
    fn causal_layer_schedule_peels_deeper_exclusive_suffixes_in_dependency_order() {
        let root = event(1, &[], "root");
        let left = RevisionEffectIdeal::new([
            root.clone(),
            event(2, &[1], "left-a"),
            event(3, &[2], "left-b"),
        ])
        .unwrap();
        let right =
            RevisionEffectIdeal::new([root, event(4, &[1], "right-a"), event(5, &[4], "right-b")])
                .unwrap();
        let schedule = left.causal_layer_schedule(&right).unwrap();
        assert_eq!(schedule.common, BTreeSet::from([RevisionEffectId(1)]));
        assert_eq!(
            schedule.left_layers,
            vec![
                BTreeSet::from([RevisionEffectId(2)]),
                BTreeSet::from([RevisionEffectId(3)])
            ]
        );
        assert_eq!(
            schedule.right_layers,
            vec![
                BTreeSet::from([RevisionEffectId(4)]),
                BTreeSet::from([RevisionEffectId(5)])
            ]
        );
    }

    #[test]
    fn two_layer_chain_consumes_first_residual_then_transports_second_layer() {
        let root = rewrite_event(1, &[], rewrite(1, 0));
        let a1 = rewrite(10, 1);
        let a2 = rewrite(11, 4);
        let b1 = rewrite(20, 2);
        let b2 = rewrite(21, 6);
        let left = RevisionEffectIdeal::new([
            root.clone(),
            rewrite_event(2, &[1], a1.clone()),
            rewrite_event(4, &[2], a2.clone()),
        ])
        .unwrap();
        let right = RevisionEffectIdeal::new([
            root,
            rewrite_event(3, &[1], b1.clone()),
            rewrite_event(5, &[3], b2.clone()),
        ])
        .unwrap();

        let b1_after_a1 = rewrite(30, 3);
        let a1_after_b1 = rewrite(31, 3);
        let b1_prefix_after_a2 = rewrite(32, 5);
        let a2_after_b1_prefix = rewrite(33, 5);
        let a1_prefix_after_b2 = rewrite(34, 7);
        let b2_after_a1_prefix = rewrite(35, 7);
        let b2_after_a2 = rewrite(36, 8);
        let a2_after_b2 = rewrite(37, 8);

        let mut registry = RewriteResidualFamilyRegistry::default();
        register_residual_pair(&mut registry, 800, &a1, &b1, &b1_after_a1, &a1_after_b1);
        register_residual_pair(
            &mut registry,
            801,
            &a2,
            &b1_after_a1,
            &b1_prefix_after_a2,
            &a2_after_b1_prefix,
        );
        register_residual_pair(
            &mut registry,
            802,
            &b2,
            &a1_after_b1,
            &a1_prefix_after_b2,
            &b2_after_a1_prefix,
        );
        register_residual_pair(
            &mut registry,
            803,
            &a2_after_b1_prefix,
            &b2_after_a1_prefix,
            &b2_after_a2,
            &a2_after_b2,
        );

        let certificate = left
            .certify_registered_two_layer_chain(
                &right,
                &0,
                &registry,
                RevisionEffectTwoLayerResidualWitness {
                    first_right_after_left: b1_after_a1,
                    first_left_after_right: a1_after_b1,
                    right_prefix_after_left_second: b1_prefix_after_a2,
                    left_second_after_right_prefix: a2_after_b1_prefix,
                    left_prefix_after_right_second: a1_prefix_after_b2,
                    right_second_after_left_prefix: b2_after_a1_prefix,
                    second_right_after_left: b2_after_a2,
                    second_left_after_right: a2_after_b2,
                },
            )
            .unwrap();
        assert_eq!(certificate.first.common_endpoint(), &3);
        assert_eq!(certificate.left_transport.common_endpoint(), &5);
        assert_eq!(certificate.right_transport.common_endpoint(), &7);
        assert_eq!(certificate.common_endpoint(), &8);
        assert_eq!(certificate.schedule.left_layers.len(), 2);
        assert_eq!(certificate.schedule.right_layers.len(), 2);
    }

    #[test]
    fn two_layer_chain_rejects_deeper_suffix_without_composite_residual_family() {
        let root = rewrite_event(1, &[], rewrite(1, 0));
        let left = RevisionEffectIdeal::new([
            root.clone(),
            rewrite_event(2, &[1], rewrite(10, 1)),
            rewrite_event(4, &[2], rewrite(11, 4)),
            rewrite_event(6, &[4], rewrite(12, 9)),
        ])
        .unwrap();
        let right = RevisionEffectIdeal::new([
            root,
            rewrite_event(3, &[1], rewrite(20, 2)),
            rewrite_event(5, &[3], rewrite(21, 6)),
        ])
        .unwrap();
        let witness = RevisionEffectTwoLayerResidualWitness {
            first_right_after_left: rewrite(30, 3),
            first_left_after_right: rewrite(31, 3),
            right_prefix_after_left_second: rewrite(32, 5),
            left_second_after_right_prefix: rewrite(33, 5),
            left_prefix_after_right_second: rewrite(34, 7),
            right_second_after_left_prefix: rewrite(35, 7),
            second_right_after_left: rewrite(36, 8),
            second_left_after_right: rewrite(37, 8),
        };
        assert_eq!(
            left.certify_registered_two_layer_chain(
                &right,
                &0,
                &RewriteResidualFamilyRegistry::default(),
                witness,
            ),
            Err(RevisionEffectResidualLayerError::UnsupportedLayerShape)
        );
    }

    #[test]
    fn residual_chain_uses_registered_composite_prefix_for_third_layer() {
        let root = rewrite_event(1, &[], rewrite(1, 0));
        let left_rw = [rewrite(10, 1), rewrite(11, 4), rewrite(12, 9)];
        let right_rw = [rewrite(20, 2), rewrite(21, 6), rewrite(22, 10)];
        let left = RevisionEffectIdeal::new([
            root.clone(),
            rewrite_event(2, &[1], left_rw[0].clone()),
            rewrite_event(4, &[2], left_rw[1].clone()),
            rewrite_event(6, &[4], left_rw[2].clone()),
        ])
        .unwrap();
        let right = RevisionEffectIdeal::new([
            root,
            rewrite_event(3, &[1], right_rw[0].clone()),
            rewrite_event(5, &[3], right_rw[1].clone()),
            rewrite_event(7, &[5], right_rw[2].clone()),
        ])
        .unwrap();
        let r = [
            rewrite(30, 3),
            rewrite(31, 3),
            rewrite(32, 5),
            rewrite(33, 5),
            rewrite(34, 7),
            rewrite(35, 7),
            rewrite(36, 8),
            rewrite(37, 8),
            rewrite(38, 8),
            rewrite(39, 8),
            rewrite(40, 11),
            rewrite(41, 11),
            rewrite(42, 12),
            rewrite(43, 12),
            rewrite(44, 13),
            rewrite(45, 13),
        ];
        let mut residuals = RewriteResidualFamilyRegistry::default();
        register_residual_pair(&mut residuals, 800, &left_rw[0], &right_rw[0], &r[0], &r[1]);
        register_residual_pair(&mut residuals, 801, &left_rw[1], &r[0], &r[2], &r[3]);
        register_residual_pair(&mut residuals, 802, &right_rw[1], &r[1], &r[4], &r[5]);
        register_residual_pair(&mut residuals, 803, &r[3], &r[5], &r[6], &r[7]);
        register_residual_pair(&mut residuals, 804, &left_rw[2], &r[8], &r[10], &r[11]);
        register_residual_pair(&mut residuals, 805, &right_rw[2], &r[9], &r[12], &r[13]);
        register_residual_pair(&mut residuals, 806, &r[11], &r[13], &r[14], &r[15]);
        let mut sequential = RewriteSequentialFamilyRegistry::default();
        for (id, first, second, composite) in
            [(900, &r[2], &r[6], &r[8]), (901, &r[4], &r[7], &r[9])]
        {
            sequential
                .register(RewriteSequentialFamilySpec {
                    id: RewriteSequentialFamilyId(SemanticId(id)),
                    key: RewriteSequentialFamilyKey {
                        first: first.into(),
                        second: second.into(),
                    },
                    composite: composite.into(),
                })
                .unwrap();
        }
        let witness = RevisionEffectResidualChainWitness {
            first_right_after_left: r[0].clone(),
            first_left_after_right: r[1].clone(),
            subsequent: vec![
                RevisionEffectResidualChainStepWitness {
                    right_prefix_after_left: r[2].clone(),
                    left_after_right_prefix: r[3].clone(),
                    left_prefix_after_right: r[4].clone(),
                    right_after_left_prefix: r[5].clone(),
                    cross_right_after_left: r[6].clone(),
                    cross_left_after_right: r[7].clone(),
                    cumulative_right_prefix: Some(r[8].clone()),
                    cumulative_left_prefix: Some(r[9].clone()),
                },
                RevisionEffectResidualChainStepWitness {
                    right_prefix_after_left: r[10].clone(),
                    left_after_right_prefix: r[11].clone(),
                    left_prefix_after_right: r[12].clone(),
                    right_after_left_prefix: r[13].clone(),
                    cross_right_after_left: r[14].clone(),
                    cross_left_after_right: r[15].clone(),
                    cumulative_right_prefix: None,
                    cumulative_left_prefix: None,
                },
            ],
        };
        let certificate = left
            .certify_registered_residual_chain(&right, &0, &residuals, &sequential, witness)
            .unwrap();
        assert_eq!(certificate.schedule.left_layers.len(), 3);
        assert_eq!(certificate.subsequent.len(), 2);
        assert_eq!(certificate.common_endpoint(), &13);
        assert!(certificate.subsequent[0].cumulative_right_prefix.is_some());
        assert!(certificate.subsequent[0].cumulative_left_prefix.is_some());
    }

    #[test]
    fn effect_identity_is_exact_and_ideal_must_be_down_closed_and_acyclic() {
        assert_eq!(
            RevisionEffectIdeal::new([event(2, &[1], "orphan")]),
            Err(RevisionEffectIdealError::MissingPrerequisite {
                effect: RevisionEffectId(2),
                prerequisite: RevisionEffectId(1),
            })
        );
        assert!(matches!(
            RevisionEffectIdeal::new([event(1, &[2], "a"), event(2, &[1], "b")]),
            Err(RevisionEffectIdealError::CyclicPrerequisites(_))
        ));
        let left = RevisionEffectIdeal::new([event(1, &[], "left")]).unwrap();
        let right = RevisionEffectIdeal::new([event(1, &[], "right")]).unwrap();
        assert_eq!(
            left.common_ideal(&right),
            Err(RevisionEffectIdealError::EffectIdentityConflict(
                RevisionEffectId(1)
            ))
        );
    }

    fn rewrite(spec: u128, endpoint: i32) -> PreparedRewrite<i32, ()> {
        PreparedRewrite {
            spec: RewriteSpecId(SemanticId(spec)),
            explicit_inputs: Vec::new(),
            effect: RewriteEffect::Replace(endpoint),
            law_set: RewriteLawSetId(SemanticId(900)),
        }
    }

    #[test]
    fn residual_diamond_checks_endpoint_and_cube_checks_exact_residual_intent() {
        let left = rewrite(10, 1);
        let right = rewrite(11, 2);
        let right_after_left = rewrite(12, 3);
        let left_after_right = rewrite(13, 3);
        let diamond = certify_residual_diamond(
            &0,
            &left,
            &right,
            right_after_left.clone(),
            left_after_right,
        )
        .unwrap();
        assert_eq!(diamond.common_endpoint(), &3);
        assert_eq!(
            certify_residual_diamond(&0, &left, &right, right_after_left.clone(), rewrite(13, 4)),
            Err(RewriteCoherenceError::DiamondEndpointMismatch)
        );
        assert!(certify_cube_coherence(&right_after_left, &right_after_left).is_ok());
        assert_eq!(
            certify_cube_coherence(&right_after_left, &rewrite(99, 3)),
            Err(RewriteCoherenceError::CubeResidualIntentMismatch)
        );
    }

    #[test]
    fn residual_family_registry_binds_pair_and_residual_intent_identity() {
        let left = rewrite(10, 1);
        let right = rewrite(11, 2);
        let right_after_left = rewrite(12, 3);
        let left_after_right = rewrite(13, 3);
        let spec = RewriteResidualFamilySpec {
            id: RewriteResidualFamilyId(SemanticId(700)),
            key: RewriteResidualFamilyKey {
                left: (&left).into(),
                right: (&right).into(),
            },
            right_after_left: (&right_after_left).into(),
            left_after_right: (&left_after_right).into(),
        };
        let mut registry = RewriteResidualFamilyRegistry::default();
        registry.register(spec).unwrap();
        assert_eq!(registry.family(spec.key), Some(&spec));
        assert_eq!(
            registry
                .certify(
                    &0,
                    &left,
                    &right,
                    right_after_left.clone(),
                    left_after_right.clone(),
                )
                .unwrap()
                .common_endpoint(),
            &3
        );
        assert_eq!(
            registry.certify(&0, &left, &right, rewrite(99, 3), left_after_right,),
            Err(RewriteResidualRegistryError::ResidualIdentityMismatch)
        );
        assert_eq!(
            registry.register(RewriteResidualFamilySpec {
                id: RewriteResidualFamilyId(SemanticId(701)),
                ..spec
            }),
            Err(RewriteResidualRegistryError::PairAlreadyRegistered)
        );
    }

    #[test]
    fn sequential_family_registry_certifies_exact_composite_identity_and_endpoint() {
        let first = rewrite(40, 1);
        let second = rewrite(41, 3);
        let composite = rewrite(42, 3);
        let mut registry = RewriteSequentialFamilyRegistry::default();
        registry
            .register(RewriteSequentialFamilySpec {
                id: RewriteSequentialFamilyId(SemanticId(900)),
                key: RewriteSequentialFamilyKey {
                    first: (&first).into(),
                    second: (&second).into(),
                },
                composite: (&composite).into(),
            })
            .unwrap();

        let certificate = registry
            .certify(&0, first.clone(), second.clone(), composite.clone())
            .unwrap();
        assert_eq!(certificate.composite, composite);

        assert_eq!(
            registry.certify(&0, first.clone(), second.clone(), rewrite(99, 3)),
            Err(RewriteSequentialRegistryError::CompositeIdentityMismatch)
        );
        assert_eq!(
            registry.certify(&0, first, second, rewrite(42, 4)),
            Err(RewriteSequentialRegistryError::CompositeEndpointMismatch)
        );
    }

    fn register_residual_pair(
        registry: &mut RewriteResidualFamilyRegistry,
        id: u128,
        left: &PreparedRewrite<i32, ()>,
        right: &PreparedRewrite<i32, ()>,
        right_after_left: &PreparedRewrite<i32, ()>,
        left_after_right: &PreparedRewrite<i32, ()>,
    ) {
        registry
            .register(RewriteResidualFamilySpec {
                id: RewriteResidualFamilyId(SemanticId(id)),
                key: RewriteResidualFamilyKey {
                    left: left.into(),
                    right: right.into(),
                },
                right_after_left: right_after_left.into(),
                left_after_right: left_after_right.into(),
            })
            .unwrap();
    }

    fn register_sequential_pair(
        registry: &mut RewriteSequentialFamilyRegistry,
        id: u128,
        first: &PreparedRewrite<i32, ()>,
        second: &PreparedRewrite<i32, ()>,
        composite: &PreparedRewrite<i32, ()>,
    ) {
        registry
            .register(RewriteSequentialFamilySpec {
                id: RewriteSequentialFamilyId(SemanticId(id)),
                key: RewriteSequentialFamilyKey {
                    first: first.into(),
                    second: second.into(),
                },
                composite: composite.into(),
            })
            .unwrap();
    }

    struct SquareFixture {
        left: RevisionEffectIdeal<PreparedRewrite<i32, ()>>,
        right: RevisionEffectIdeal<PreparedRewrite<i32, ()>>,
        residual: RewriteResidualFamilyRegistry,
        sequential: RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualSquareLayerWitness<i32, ()>,
    }

    fn square_fixture() -> SquareFixture {
        let a = rewrite(100, 1);
        let b = rewrite(101, 2);
        let c = rewrite(102, 4);
        let d = rewrite(103, 5);
        let b_after_a = rewrite(110, 3);
        let a_after_b = rewrite(111, 3);
        let d_after_c = rewrite(112, 6);
        let c_after_d = rewrite(113, 6);
        let left_composite = rewrite(120, 3);
        let right_composite = rewrite(121, 6);
        let right_after_left = rewrite(130, 7);
        let left_after_right = rewrite(131, 7);
        let left = RevisionEffectIdeal::new([
            rewrite_event(10, &[], a.clone()),
            rewrite_event(11, &[], b.clone()),
        ])
        .unwrap();
        let right = RevisionEffectIdeal::new([
            rewrite_event(20, &[], c.clone()),
            rewrite_event(21, &[], d.clone()),
        ])
        .unwrap();
        let mut residual = RewriteResidualFamilyRegistry::default();
        register_residual_pair(&mut residual, 1000, &a, &b, &b_after_a, &a_after_b);
        register_residual_pair(&mut residual, 1001, &c, &d, &d_after_c, &c_after_d);
        register_residual_pair(
            &mut residual,
            1002,
            &left_composite,
            &right_composite,
            &right_after_left,
            &left_after_right,
        );
        let mut sequential = RewriteSequentialFamilyRegistry::default();
        register_sequential_pair(&mut sequential, 1100, &a, &b_after_a, &left_composite);
        register_sequential_pair(&mut sequential, 1101, &b, &a_after_b, &left_composite);
        register_sequential_pair(&mut sequential, 1102, &c, &d_after_c, &right_composite);
        register_sequential_pair(&mut sequential, 1103, &d, &c_after_d, &right_composite);
        SquareFixture {
            left,
            right,
            residual,
            sequential,
            witness: RevisionEffectResidualSquareLayerWitness {
                left: RewriteConcurrentPairWitness {
                    right_after_left: b_after_a,
                    left_after_right: a_after_b,
                    left_then_right_composite: left_composite.clone(),
                    right_then_left_composite: left_composite,
                },
                right: RewriteConcurrentPairWitness {
                    right_after_left: d_after_c,
                    left_after_right: c_after_d,
                    left_then_right_composite: right_composite.clone(),
                    right_then_left_composite: right_composite,
                },
                right_after_left,
                left_after_right,
            },
        }
    }

    #[test]
    fn two_by_two_frontier_normalizes_both_orders_before_cross_residual() {
        let fixture = square_fixture();
        let certificate = fixture
            .left
            .certify_registered_residual_square_frontier(
                &fixture.right,
                &0,
                &fixture.residual,
                &fixture.sequential,
                fixture.witness,
            )
            .unwrap();
        assert_eq!(certificate.left_frontier.len(), 2);
        assert_eq!(certificate.right_frontier.len(), 2);
        assert_eq!(certificate.common_endpoint(), &7);
        assert_eq!(
            certificate.left_normalization.left_then_right.composite,
            certificate.left_normalization.right_then_left.composite
        );
        assert_eq!(
            certificate.right_normalization.left_then_right.composite,
            certificate.right_normalization.right_then_left.composite
        );
    }

    #[test]
    fn square_first_layer_carries_exact_residual_prefix_into_singleton_suffix() {
        let SquareFixture {
            left: first_left,
            right: first_right,
            mut residual,
            sequential,
            witness: first,
        } = square_fixture();
        let left_second = rewrite(140, 8);
        let right_second = rewrite(141, 9);
        let right_prefix_after_left = rewrite(142, 10);
        let left_after_right_prefix = rewrite(143, 10);
        let left_prefix_after_right = rewrite(144, 11);
        let right_after_left_prefix = rewrite(145, 11);
        let cross_right_after_left = rewrite(146, 12);
        let cross_left_after_right = rewrite(147, 12);
        let left =
            RevisionEffectIdeal::new(first_left.events().values().cloned().chain([rewrite_event(
                12,
                &[10, 11],
                left_second.clone(),
            )]))
            .unwrap();
        let right = RevisionEffectIdeal::new(
            first_right.events().values().cloned().chain([rewrite_event(
                22,
                &[20, 21],
                right_second.clone(),
            )]),
        )
        .unwrap();
        register_residual_pair(
            &mut residual,
            1003,
            &left_second,
            &first.right_after_left,
            &right_prefix_after_left,
            &left_after_right_prefix,
        );
        register_residual_pair(
            &mut residual,
            1004,
            &right_second,
            &first.left_after_right,
            &left_prefix_after_right,
            &right_after_left_prefix,
        );
        register_residual_pair(
            &mut residual,
            1005,
            &left_after_right_prefix,
            &right_after_left_prefix,
            &cross_right_after_left,
            &cross_left_after_right,
        );
        let certificate = left
            .certify_registered_residual_square_chain(
                &right,
                &0,
                &residual,
                &sequential,
                RevisionEffectResidualSquareChainWitness {
                    first,
                    subsequent: vec![RevisionEffectResidualChainStepWitness {
                        right_prefix_after_left,
                        left_after_right_prefix,
                        left_prefix_after_right,
                        right_after_left_prefix,
                        cross_right_after_left,
                        cross_left_after_right,
                        cumulative_right_prefix: None,
                        cumulative_left_prefix: None,
                    }],
                },
            )
            .unwrap();
        assert_eq!(certificate.schedule.left_layers.len(), 2);
        assert_eq!(certificate.schedule.right_layers.len(), 2);
        assert_eq!(certificate.common_endpoint(), &12);
    }

    struct EmbeddedSquareFixture {
        left: RevisionEffectIdeal<PreparedRewrite<i32, ()>>,
        right: RevisionEffectIdeal<PreparedRewrite<i32, ()>>,
        residual: RewriteResidualFamilyRegistry,
        sequential: RewriteSequentialFamilyRegistry,
        witness: RevisionEffectResidualMixedChainWitness<i32, ()>,
    }

    struct EmbeddedSquareIdeals {
        left: RevisionEffectIdeal<PreparedRewrite<i32, ()>>,
        right: RevisionEffectIdeal<PreparedRewrite<i32, ()>>,
    }

    struct EmbeddedSquareRewrites {
        left_first: PreparedRewrite<i32, ()>,
        right_first: PreparedRewrite<i32, ()>,
        right_after_left: PreparedRewrite<i32, ()>,
        left_after_right: PreparedRewrite<i32, ()>,
        left_a: PreparedRewrite<i32, ()>,
        left_b: PreparedRewrite<i32, ()>,
        left_b_after_a: PreparedRewrite<i32, ()>,
        left_a_after_b: PreparedRewrite<i32, ()>,
        left_composite: PreparedRewrite<i32, ()>,
        right_a: PreparedRewrite<i32, ()>,
        right_b: PreparedRewrite<i32, ()>,
        right_b_after_a: PreparedRewrite<i32, ()>,
        right_a_after_b: PreparedRewrite<i32, ()>,
        right_composite: PreparedRewrite<i32, ()>,
        right_prefix_after_left: PreparedRewrite<i32, ()>,
        left_after_right_prefix: PreparedRewrite<i32, ()>,
        left_prefix_after_right: PreparedRewrite<i32, ()>,
        right_after_left_prefix: PreparedRewrite<i32, ()>,
        cross_right_after_left: PreparedRewrite<i32, ()>,
        cross_left_after_right: PreparedRewrite<i32, ()>,
    }

    fn embedded_square_rewrites() -> EmbeddedSquareRewrites {
        EmbeddedSquareRewrites {
            left_first: rewrite(200, 1),
            right_first: rewrite(201, 2),
            right_after_left: rewrite(202, 3),
            left_after_right: rewrite(203, 3),
            left_a: rewrite(204, 4),
            left_b: rewrite(205, 5),
            left_b_after_a: rewrite(206, 6),
            left_a_after_b: rewrite(207, 6),
            left_composite: rewrite(208, 6),
            right_a: rewrite(209, 7),
            right_b: rewrite(210, 8),
            right_b_after_a: rewrite(211, 9),
            right_a_after_b: rewrite(212, 9),
            right_composite: rewrite(213, 9),
            right_prefix_after_left: rewrite(214, 10),
            left_after_right_prefix: rewrite(215, 10),
            left_prefix_after_right: rewrite(216, 11),
            right_after_left_prefix: rewrite(217, 11),
            cross_right_after_left: rewrite(218, 12),
            cross_left_after_right: rewrite(219, 12),
        }
    }

    fn embedded_square_ideals(r: &EmbeddedSquareRewrites) -> EmbeddedSquareIdeals {
        let left = RevisionEffectIdeal::new([
            rewrite_event(30, &[], r.left_first.clone()),
            rewrite_event(31, &[30], r.left_a.clone()),
            rewrite_event(32, &[30], r.left_b.clone()),
        ])
        .unwrap();
        let right = RevisionEffectIdeal::new([
            rewrite_event(40, &[], r.right_first.clone()),
            rewrite_event(41, &[40], r.right_a.clone()),
            rewrite_event(42, &[40], r.right_b.clone()),
        ])
        .unwrap();
        EmbeddedSquareIdeals { left, right }
    }

    fn embedded_square_registries(
        r: &EmbeddedSquareRewrites,
    ) -> (
        RewriteResidualFamilyRegistry,
        RewriteSequentialFamilyRegistry,
    ) {
        let mut residual = RewriteResidualFamilyRegistry::default();
        register_residual_pair(
            &mut residual,
            1200,
            &r.left_first,
            &r.right_first,
            &r.right_after_left,
            &r.left_after_right,
        );
        register_residual_pair(
            &mut residual,
            1201,
            &r.left_a,
            &r.left_b,
            &r.left_b_after_a,
            &r.left_a_after_b,
        );
        register_residual_pair(
            &mut residual,
            1202,
            &r.right_a,
            &r.right_b,
            &r.right_b_after_a,
            &r.right_a_after_b,
        );
        register_residual_pair(
            &mut residual,
            1203,
            &r.left_composite,
            &r.right_after_left,
            &r.right_prefix_after_left,
            &r.left_after_right_prefix,
        );
        register_residual_pair(
            &mut residual,
            1204,
            &r.right_composite,
            &r.left_after_right,
            &r.left_prefix_after_right,
            &r.right_after_left_prefix,
        );
        register_residual_pair(
            &mut residual,
            1205,
            &r.left_after_right_prefix,
            &r.right_after_left_prefix,
            &r.cross_right_after_left,
            &r.cross_left_after_right,
        );
        let mut sequential = RewriteSequentialFamilyRegistry::default();
        register_sequential_pair(
            &mut sequential,
            1300,
            &r.left_a,
            &r.left_b_after_a,
            &r.left_composite,
        );
        register_sequential_pair(
            &mut sequential,
            1301,
            &r.left_b,
            &r.left_a_after_b,
            &r.left_composite,
        );
        register_sequential_pair(
            &mut sequential,
            1302,
            &r.right_a,
            &r.right_b_after_a,
            &r.right_composite,
        );
        register_sequential_pair(
            &mut sequential,
            1303,
            &r.right_b,
            &r.right_a_after_b,
            &r.right_composite,
        );
        (residual, sequential)
    }

    fn embedded_square_witness(
        r: &EmbeddedSquareRewrites,
    ) -> RevisionEffectResidualMixedChainWitness<i32, ()> {
        RevisionEffectResidualMixedChainWitness {
            first: RevisionEffectResidualMixedFirstWitness::Singleton {
                right_after_left: r.right_after_left.clone(),
                left_after_right: r.left_after_right.clone(),
            },
            subsequent: vec![RevisionEffectResidualMixedStepWitness::Square(Box::new(
                RevisionEffectResidualMixedSquareStepWitness {
                    left: RewriteConcurrentPairWitness {
                        right_after_left: r.left_b_after_a.clone(),
                        left_after_right: r.left_a_after_b.clone(),
                        left_then_right_composite: r.left_composite.clone(),
                        right_then_left_composite: r.left_composite.clone(),
                    },
                    right: RewriteConcurrentPairWitness {
                        right_after_left: r.right_b_after_a.clone(),
                        left_after_right: r.right_a_after_b.clone(),
                        left_then_right_composite: r.right_composite.clone(),
                        right_then_left_composite: r.right_composite.clone(),
                    },
                    transport: RevisionEffectResidualChainStepWitness {
                        right_prefix_after_left: r.right_prefix_after_left.clone(),
                        left_after_right_prefix: r.left_after_right_prefix.clone(),
                        left_prefix_after_right: r.left_prefix_after_right.clone(),
                        right_after_left_prefix: r.right_after_left_prefix.clone(),
                        cross_right_after_left: r.cross_right_after_left.clone(),
                        cross_left_after_right: r.cross_left_after_right.clone(),
                        cumulative_right_prefix: None,
                        cumulative_left_prefix: None,
                    },
                },
            ))],
        }
    }

    fn embedded_square_fixture() -> EmbeddedSquareFixture {
        let rewrites = embedded_square_rewrites();
        let EmbeddedSquareIdeals { left, right } = embedded_square_ideals(&rewrites);
        let (residual, sequential) = embedded_square_registries(&rewrites);
        let witness = embedded_square_witness(&rewrites);
        EmbeddedSquareFixture {
            left,
            right,
            residual,
            sequential,
            witness,
        }
    }

    #[test]
    fn square_layer_after_singleton_prefix_is_residualized_without_order_authority() {
        let fixture = embedded_square_fixture();
        let certificate = fixture
            .left
            .certify_registered_residual_mixed_chain(
                &fixture.right,
                &0,
                &fixture.residual,
                &fixture.sequential,
                fixture.witness,
            )
            .unwrap();
        assert_eq!(certificate.schedule.left_layers.len(), 2);
        assert_eq!(certificate.schedule.left_layers[1].len(), 2);
        assert_eq!(certificate.schedule.right_layers[1].len(), 2);
        assert!(matches!(
            certificate.subsequent[0],
            RevisionEffectResidualMixedStepCertificate::Square { .. }
        ));
        assert_eq!(certificate.common_endpoint(), &12);
    }

    struct CubeFixture {
        registry: RewriteResidualFamilyRegistry,
        a: PreparedRewrite<i32, ()>,
        b: PreparedRewrite<i32, ()>,
        c: PreparedRewrite<i32, ()>,
        witness: RewriteResidualCubeWitness<i32, ()>,
    }

    fn cube_fixture(final_c_spec: u128) -> CubeFixture {
        let a = rewrite(10, 1);
        let b = rewrite(11, 2);
        let c = rewrite(12, 3);
        let witness = RewriteResidualCubeWitness {
            b_after_a: rewrite(20, 4),
            a_after_b: rewrite(21, 4),
            c_after_a: rewrite(22, 5),
            a_after_c: rewrite(23, 5),
            c_after_b: rewrite(24, 6),
            b_after_c: rewrite(25, 6),
            c_after_ab: rewrite(26, 7),
            b_after_ac: rewrite(27, 7),
            c_after_ba: rewrite(final_c_spec, 7),
            a_after_bc: rewrite(28, 7),
            b_after_ca: rewrite(29, 7),
            a_after_cb: rewrite(30, 7),
        };
        let mut registry = RewriteResidualFamilyRegistry::default();
        register_residual_pair(
            &mut registry,
            700,
            &a,
            &b,
            &witness.b_after_a,
            &witness.a_after_b,
        );
        register_residual_pair(
            &mut registry,
            701,
            &a,
            &c,
            &witness.c_after_a,
            &witness.a_after_c,
        );
        register_residual_pair(
            &mut registry,
            702,
            &b,
            &c,
            &witness.c_after_b,
            &witness.b_after_c,
        );
        register_residual_pair(
            &mut registry,
            703,
            &witness.b_after_a,
            &witness.c_after_a,
            &witness.c_after_ab,
            &witness.b_after_ac,
        );
        register_residual_pair(
            &mut registry,
            704,
            &witness.a_after_b,
            &witness.c_after_b,
            &witness.c_after_ba,
            &witness.a_after_bc,
        );
        register_residual_pair(
            &mut registry,
            705,
            &witness.a_after_c,
            &witness.b_after_c,
            &witness.b_after_ca,
            &witness.a_after_cb,
        );
        CubeFixture {
            registry,
            a,
            b,
            c,
            witness,
        }
    }

    fn concurrent_triple_sequential_registry(
        fixture: &CubeFixture,
        ab: &PreparedRewrite<i32, ()>,
        ac: &PreparedRewrite<i32, ()>,
        bc: &PreparedRewrite<i32, ()>,
        final_composite: &PreparedRewrite<i32, ()>,
    ) -> RewriteSequentialFamilyRegistry {
        let mut registry = RewriteSequentialFamilyRegistry::default();
        let paths = [
            (&fixture.a, &fixture.witness.b_after_a, ab),
            (&fixture.b, &fixture.witness.a_after_b, ab),
            (&fixture.a, &fixture.witness.c_after_a, ac),
            (&fixture.c, &fixture.witness.a_after_c, ac),
            (&fixture.b, &fixture.witness.c_after_b, bc),
            (&fixture.c, &fixture.witness.b_after_c, bc),
        ];
        for (offset, (first, second, composite)) in paths.into_iter().enumerate() {
            register_sequential_pair(
                &mut registry,
                1500 + offset as u128,
                first,
                second,
                composite,
            );
        }
        let final_paths = [
            (ab, &fixture.witness.c_after_ab),
            (ac, &fixture.witness.b_after_ac),
            (ac, &fixture.witness.b_after_ca),
            (bc, &fixture.witness.a_after_bc),
            (bc, &fixture.witness.a_after_cb),
        ];
        for (offset, (first, second)) in final_paths.into_iter().enumerate() {
            register_sequential_pair(
                &mut registry,
                1510 + offset as u128,
                first,
                second,
                final_composite,
            );
        }
        registry
    }

    #[test]
    fn concurrent_triple_normalizes_all_six_orders_to_one_exact_composite() {
        let fixture = cube_fixture(26);
        let ab = rewrite(40, 4);
        let ac = rewrite(41, 5);
        let bc = rewrite(42, 6);
        let final_composite = rewrite(43, 7);
        let sequential =
            concurrent_triple_sequential_registry(&fixture, &ab, &ac, &bc, &final_composite);
        let certificate = certify_registered_concurrent_triple(
            &0,
            &fixture.a,
            &fixture.b,
            &fixture.c,
            &fixture.registry,
            &sequential,
            RewriteConcurrentTripleWitness {
                cube: fixture.witness,
                ab_composite: ab,
                ac_composite: ac,
                bc_composite: bc,
                final_composite: final_composite.clone(),
            },
        )
        .unwrap();
        assert_eq!(certificate.final_paths.len(), 6);
        assert_eq!(certificate.composite(), &final_composite);
        assert_eq!(certificate.cube.common_endpoint(), &7);
    }

    #[test]
    fn normalized_frontier_consumes_three_by_one_without_branch_order_authority() {
        let mut fixture = cube_fixture(26);
        let ab = rewrite(40, 4);
        let ac = rewrite(41, 5);
        let bc = rewrite(42, 6);
        let left_composite = rewrite(43, 7);
        let sequential =
            concurrent_triple_sequential_registry(&fixture, &ab, &ac, &bc, &left_composite);
        let right_rewrite = rewrite(44, 8);
        let right_after_left = rewrite(45, 9);
        let left_after_right = rewrite(46, 9);
        register_residual_pair(
            &mut fixture.registry,
            706,
            &left_composite,
            &right_rewrite,
            &right_after_left,
            &left_after_right,
        );
        let left = RevisionEffectIdeal::new([
            rewrite_event(1, &[], fixture.a.clone()),
            rewrite_event(2, &[], fixture.b.clone()),
            rewrite_event(3, &[], fixture.c.clone()),
        ])
        .unwrap();
        let right = RevisionEffectIdeal::new([rewrite_event(4, &[], right_rewrite)]).unwrap();
        let normalized = RevisionEffectResidualNormalizedLayerWitness {
            left: RewriteConcurrentBranchWitness::Triple(Box::new(
                RewriteConcurrentTripleWitness {
                    cube: fixture.witness,
                    ab_composite: ab,
                    ac_composite: ac,
                    bc_composite: bc,
                    final_composite: left_composite.clone(),
                },
            )),
            right: RewriteConcurrentBranchWitness::Single,
            right_after_left,
            left_after_right,
        };
        let certificate = left
            .certify_registered_residual_normalized_frontier(
                &right,
                &0,
                &fixture.registry,
                &sequential,
                normalized.clone(),
            )
            .unwrap();
        assert_eq!(certificate.left_frontier.len(), 3);
        assert_eq!(certificate.right_frontier.len(), 1);
        assert_eq!(certificate.left_normalization.composite(), &left_composite);
        assert_eq!(certificate.common_endpoint(), &9);

        let mixed = left
            .certify_registered_residual_mixed_chain(
                &right,
                &0,
                &fixture.registry,
                &sequential,
                RevisionEffectResidualMixedChainWitness {
                    first: RevisionEffectResidualMixedFirstWitness::Normalized(Box::new(
                        normalized,
                    )),
                    subsequent: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(mixed.common_endpoint(), &9);
        assert!(matches!(
            mixed.first,
            RevisionEffectResidualMixedFirstCertificate::Normalized(_)
        ));
    }

    #[test]
    fn normalized_frontier_consumes_three_by_two_without_branch_order_authority() {
        let mut fixture = cube_fixture(26);
        let ab = rewrite(40, 4);
        let ac = rewrite(41, 5);
        let bc = rewrite(42, 6);
        let left_composite = rewrite(43, 7);
        let mut sequential =
            concurrent_triple_sequential_registry(&fixture, &ab, &ac, &bc, &left_composite);

        let right_a = rewrite(50, 8);
        let right_b = rewrite(51, 10);
        let right_b_after_a = rewrite(52, 11);
        let right_a_after_b = rewrite(53, 11);
        let right_composite = rewrite(54, 11);
        register_residual_pair(
            &mut fixture.registry,
            706,
            &right_a,
            &right_b,
            &right_b_after_a,
            &right_a_after_b,
        );
        register_sequential_pair(
            &mut sequential,
            1520,
            &right_a,
            &right_b_after_a,
            &right_composite,
        );
        register_sequential_pair(
            &mut sequential,
            1521,
            &right_b,
            &right_a_after_b,
            &right_composite,
        );

        let right_after_left = rewrite(55, 12);
        let left_after_right = rewrite(56, 12);
        register_residual_pair(
            &mut fixture.registry,
            707,
            &left_composite,
            &right_composite,
            &right_after_left,
            &left_after_right,
        );

        let left = RevisionEffectIdeal::new([
            rewrite_event(1, &[], fixture.a.clone()),
            rewrite_event(2, &[], fixture.b.clone()),
            rewrite_event(3, &[], fixture.c.clone()),
        ])
        .unwrap();
        let right = RevisionEffectIdeal::new([
            rewrite_event(4, &[], right_a),
            rewrite_event(5, &[], right_b),
        ])
        .unwrap();

        let certificate = left
            .certify_registered_residual_normalized_frontier(
                &right,
                &0,
                &fixture.registry,
                &sequential,
                RevisionEffectResidualNormalizedLayerWitness {
                    left: RewriteConcurrentBranchWitness::Triple(Box::new(
                        RewriteConcurrentTripleWitness {
                            cube: fixture.witness,
                            ab_composite: ab,
                            ac_composite: ac,
                            bc_composite: bc,
                            final_composite: left_composite.clone(),
                        },
                    )),
                    right: RewriteConcurrentBranchWitness::Pair(Box::new(
                        RewriteConcurrentPairWitness {
                            right_after_left: right_b_after_a,
                            left_after_right: right_a_after_b,
                            left_then_right_composite: right_composite.clone(),
                            right_then_left_composite: right_composite.clone(),
                        },
                    )),
                    right_after_left,
                    left_after_right,
                },
            )
            .unwrap();
        assert_eq!(certificate.left_frontier.len(), 3);
        assert_eq!(certificate.right_frontier.len(), 2);
        assert_eq!(certificate.left_normalization.composite(), &left_composite);
        assert_eq!(
            certificate.right_normalization.composite(),
            &right_composite
        );
        assert_eq!(certificate.common_endpoint(), &12);
    }

    #[test]
    fn registered_cube_checks_all_residual_faces_and_exact_tp2_intent() {
        let fixture = cube_fixture(26);
        let certificate = fixture
            .registry
            .certify_cube(&0, &fixture.a, &fixture.b, &fixture.c, fixture.witness)
            .unwrap();
        assert_eq!(certificate.ab.common_endpoint(), &4);
        assert_eq!(certificate.after_a.common_endpoint(), &7);
        assert_eq!(certificate.after_b.common_endpoint(), &7);
        assert_eq!(certificate.after_c.common_endpoint(), &7);
        assert_eq!(certificate.common_endpoint(), &7);
        assert_eq!(
            certificate.cube.coherent_residual().spec,
            RewriteSpecId(SemanticId(26))
        );

        let fixture = cube_fixture(99);
        assert_eq!(
            fixture
                .registry
                .certify_cube(&0, &fixture.a, &fixture.b, &fixture.c, fixture.witness),
            Err(RewriteResidualRegistryError::Coherence(
                RewriteCoherenceError::CubeResidualIntentMismatch
            ))
        );
    }

    #[test]
    fn registered_cube_rejects_disagreeing_upper_face_endpoint() {
        let mut fixture = cube_fixture(26);
        fixture.witness.b_after_ca.effect = RewriteEffect::Replace(8);
        fixture.witness.a_after_cb.effect = RewriteEffect::Replace(8);
        assert_eq!(
            fixture
                .registry
                .certify_cube(&0, &fixture.a, &fixture.b, &fixture.c, fixture.witness,),
            Err(RewriteResidualRegistryError::Coherence(
                RewriteCoherenceError::CubeEndpointMismatch
            ))
        );
    }

    #[test]
    fn non_singleton_frontier_consumes_registered_cube_without_serializing_branch_events() {
        let fixture = cube_fixture(26);
        let left = RevisionEffectIdeal::new(vec![
            rewrite_event(1, &[], fixture.a.clone()),
            rewrite_event(2, &[], fixture.b.clone()),
        ])
        .unwrap();
        let right =
            RevisionEffectIdeal::new(vec![rewrite_event(3, &[], fixture.c.clone())]).unwrap();

        let certificate = left
            .certify_registered_residual_cube_frontier(
                &right,
                &0,
                &fixture.registry,
                fixture.witness,
            )
            .unwrap();
        assert_eq!(
            certificate.left_frontier,
            BTreeSet::from([RevisionEffectId(1), RevisionEffectId(2)])
        );
        assert_eq!(
            certificate.right_frontier,
            BTreeSet::from([RevisionEffectId(3)])
        );
        assert_eq!(certificate.common_endpoint(), &7);
    }

    fn rewrite_event(
        id: u128,
        prerequisites: &[u128],
        rewrite: PreparedRewrite<i32, ()>,
    ) -> RevisionEffect<PreparedRewrite<i32, ()>> {
        RevisionEffect {
            id: RevisionEffectId(id),
            prerequisites: prerequisites
                .iter()
                .copied()
                .map(RevisionEffectId)
                .collect(),
            payload: rewrite,
        }
    }

    #[test]
    fn registered_residual_frontier_consumes_every_required_pair() {
        let root = rewrite_event(1, &[], rewrite(1, 0));
        let left_rewrite = rewrite(10, 1);
        let right_rewrite = rewrite(11, 2);
        let right_after_left = rewrite(12, 3);
        let left_after_right = rewrite(13, 3);
        let left =
            RevisionEffectIdeal::new([root.clone(), rewrite_event(2, &[1], left_rewrite.clone())])
                .unwrap();
        let right = RevisionEffectIdeal::new([root, rewrite_event(3, &[1], right_rewrite.clone())])
            .unwrap();
        let mut registry = RewriteResidualFamilyRegistry::default();
        registry
            .register(RewriteResidualFamilySpec {
                id: RewriteResidualFamilyId(SemanticId(700)),
                key: RewriteResidualFamilyKey {
                    left: (&left_rewrite).into(),
                    right: (&right_rewrite).into(),
                },
                right_after_left: (&right_after_left).into(),
                left_after_right: (&left_after_right).into(),
            })
            .unwrap();

        let certificate = left
            .certify_registered_residual_frontier(
                &right,
                &0,
                &registry,
                |_, _| PairCoordinationDecision::RequiresCoordination,
                |left_id, right_id| {
                    (left_id == RevisionEffectId(2) && right_id == RevisionEffectId(3))
                        .then(|| (right_after_left.clone(), left_after_right.clone()))
                },
            )
            .unwrap();
        assert_eq!(certificate.common, BTreeSet::from([RevisionEffectId(1)]));
        assert_eq!(
            certificate.left_frontier,
            BTreeSet::from([RevisionEffectId(2)])
        );
        assert_eq!(
            certificate.right_frontier,
            BTreeSet::from([RevisionEffectId(3)])
        );
        assert_eq!(
            certificate.diamonds[&(RevisionEffectId(2), RevisionEffectId(3))].common_endpoint(),
            &3
        );
    }

    #[test]
    fn residual_frontier_rejects_missing_pair_and_deeper_causal_suffix() {
        let root = rewrite_event(1, &[], rewrite(1, 0));
        let left_rewrite = rewrite(10, 1);
        let right_rewrite = rewrite(11, 2);
        let left =
            RevisionEffectIdeal::new([root.clone(), rewrite_event(2, &[1], left_rewrite.clone())])
                .unwrap();
        let right =
            RevisionEffectIdeal::new([root.clone(), rewrite_event(3, &[1], right_rewrite.clone())])
                .unwrap();
        assert_eq!(
            left.certify_registered_residual_frontier(
                &right,
                &0,
                &RewriteResidualFamilyRegistry::default(),
                |_, _| PairCoordinationDecision::RequiresCoordination,
                |_, _| None,
            ),
            Err(RevisionEffectResidualLayerError::MissingResidualPair(
                RevisionEffectId(2),
                RevisionEffectId(3),
            ))
        );

        let deeper_left = RevisionEffectIdeal::new([
            root,
            rewrite_event(2, &[1], left_rewrite),
            rewrite_event(4, &[2], rewrite(14, 4)),
        ])
        .unwrap();
        assert_eq!(
            deeper_left.certify_registered_residual_frontier(
                &right,
                &0,
                &RewriteResidualFamilyRegistry::default(),
                |_, _| PairCoordinationDecision::CoordinationFree,
                |_, _| None,
            ),
            Err(RevisionEffectResidualLayerError::NonFrontierExclusiveEffect(RevisionEffectId(4)))
        );
    }

    #[test]
    fn reic_branch_exclusive_events_use_fail_closed_pair_coordination() {
        let root = event(1, &[], "root");
        let left = RevisionEffectIdeal::new([root.clone(), event(2, &[1], "left")]).unwrap();
        let right = RevisionEffectIdeal::new([root, event(3, &[1], "right")]).unwrap();

        let unknown = left
            .merge_requirements(&right, |_, _| {
                PairCoordinationDecision::RequiresCoordination
            })
            .unwrap();
        assert_eq!(unknown.common, BTreeSet::from([RevisionEffectId(1)]));
        assert_eq!(
            unknown.requires_residual,
            BTreeSet::from([(RevisionEffectId(2), RevisionEffectId(3))])
        );
        assert!(!unknown.coordination_free());

        let conflict = left
            .merge_requirements(&right, |_, _| PairCoordinationDecision::IntentConflict)
            .unwrap();
        assert_eq!(
            conflict.intent_conflicts,
            BTreeSet::from([(RevisionEffectId(2), RevisionEffectId(3))])
        );

        let safe = left
            .merge_requirements(&right, |_, _| PairCoordinationDecision::CoordinationFree)
            .unwrap();
        assert!(safe.coordination_free());
    }
}

#[must_use]
pub fn infer_pair_rewrite_law(left: &RewriteFootprint, right: &RewriteFootprint) -> PairRewriteLaw {
    if !left.invariant_obligations.is_empty() || !right.invariant_obligations.is_empty() {
        return PairRewriteLaw::Unknown;
    }
    if left
        .reads
        .iter()
        .any(|coord| right.writes.contains_key(coord))
        || right
            .reads
            .iter()
            .any(|coord| left.writes.contains_key(coord))
    {
        return PairRewriteLaw::Unknown;
    }

    let mut saw_same_idempotent = false;
    for (coordinate, left_action) in &left.writes {
        let Some(right_action) = right.writes.get(coordinate) else {
            continue;
        };
        match (left_action, right_action) {
            (
                RewriteActionLaw::IdempotentAssign {
                    semantic_value: left_value,
                },
                RewriteActionLaw::IdempotentAssign {
                    semantic_value: right_value,
                },
            ) if left_value == right_value => saw_same_idempotent = true,
            (
                RewriteActionLaw::IdempotentAssign { .. },
                RewriteActionLaw::IdempotentAssign { .. },
            )
            | (RewriteActionLaw::EnsurePresent, RewriteActionLaw::EnsureAbsent)
            | (RewriteActionLaw::EnsureAbsent, RewriteActionLaw::EnsurePresent) => {
                return PairRewriteLaw::DefiniteIntentConflict;
            }
            (RewriteActionLaw::EnsurePresent, RewriteActionLaw::EnsurePresent)
            | (RewriteActionLaw::EnsureAbsent, RewriteActionLaw::EnsureAbsent) => {
                saw_same_idempotent = true;
            }
            (
                RewriteActionLaw::CommutativeAdd {
                    algebra: left_algebra,
                },
                RewriteActionLaw::CommutativeAdd {
                    algebra: right_algebra,
                },
            ) if left_algebra == right_algebra => {}
            _ => return PairRewriteLaw::Unknown,
        }
    }

    if saw_same_idempotent && left.writes == right.writes {
        PairRewriteLaw::SameIdempotentIntent
    } else {
        PairRewriteLaw::StrongCommute
    }
}

#[cfg(test)]
mod rewrite_law_tests {
    use super::*;

    fn field(id: u128) -> SemanticWriteCoordinate {
        SemanticWriteCoordinate::ProductField(SemanticId(id))
    }

    #[test]
    fn disjoint_product_fields_strongly_commute_without_cross_guards() {
        let left = RewriteFootprint {
            writes: [(field(1), RewriteActionLaw::Opaque)].into_iter().collect(),
            ..RewriteFootprint::default()
        };
        let right = RewriteFootprint {
            writes: [(field(2), RewriteActionLaw::Opaque)].into_iter().collect(),
            ..RewriteFootprint::default()
        };
        assert_eq!(
            infer_pair_rewrite_law(&left, &right),
            PairRewriteLaw::StrongCommute
        );
        assert_eq!(
            coordination_decision(infer_pair_rewrite_law(&left, &right)),
            PairCoordinationDecision::CoordinationFree
        );
    }

    #[test]
    fn disjoint_writes_do_not_commute_when_guard_reads_other_write() {
        let left = RewriteFootprint {
            reads: [field(2)].into_iter().collect(),
            writes: [(field(1), RewriteActionLaw::Opaque)].into_iter().collect(),
            ..RewriteFootprint::default()
        };
        let right = RewriteFootprint {
            writes: [(field(2), RewriteActionLaw::Opaque)].into_iter().collect(),
            ..RewriteFootprint::default()
        };
        assert_eq!(
            infer_pair_rewrite_law(&left, &right),
            PairRewriteLaw::Unknown
        );
        assert_eq!(
            coordination_decision(infer_pair_rewrite_law(&left, &right)),
            PairCoordinationDecision::RequiresCoordination
        );
    }

    #[test]
    fn identical_assignment_is_idempotent_but_different_assignment_conflicts() {
        let mk = |value| RewriteFootprint {
            writes: [(
                field(1),
                RewriteActionLaw::IdempotentAssign {
                    semantic_value: SemanticId(value),
                },
            )]
            .into_iter()
            .collect(),
            ..RewriteFootprint::default()
        };
        assert_eq!(
            infer_pair_rewrite_law(&mk(7), &mk(7)),
            PairRewriteLaw::SameIdempotentIntent
        );
        assert_eq!(
            infer_pair_rewrite_law(&mk(7), &mk(8)),
            PairRewriteLaw::DefiniteIntentConflict
        );
        assert_eq!(
            coordination_decision(infer_pair_rewrite_law(&mk(7), &mk(8))),
            PairCoordinationDecision::IntentConflict
        );
    }

    #[test]
    fn invariant_obligation_blocks_coordination_free_claim_until_vmf_discharge() {
        let left = RewriteFootprint {
            writes: [(field(1), RewriteActionLaw::Opaque)].into_iter().collect(),
            invariant_obligations: [SemanticId(99)].into_iter().collect(),
            ..RewriteFootprint::default()
        };
        let right = RewriteFootprint {
            writes: [(field(2), RewriteActionLaw::Opaque)].into_iter().collect(),
            ..RewriteFootprint::default()
        };
        assert_eq!(
            infer_pair_rewrite_law(&left, &right),
            PairRewriteLaw::Unknown
        );
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

    /// Resolves stable semantic sequence intent against one authoritative
    /// snapshot and preserves that intent as the explicit input of the
    /// prepared Rewrite. The resulting effect remains extensional while the
    /// durable/concurrent meaning is carried by stable occurrence/gap IDs.
    pub fn prepare_stable_seq<T: Clone>(
        &self,
        snapshot: &StableSeqSnapshot<T>,
        intent: StableSeqRewriteIntent<T>,
    ) -> Result<
        PreparedRewrite<Vec<StableSeqOccurrence<T>>, StableSeqRewriteIntent<T>>,
        StableSeqRewriteError,
    > {
        let splice = snapshot.resolve_intent(&intent)?;
        let endpoint = splice
            .apply(&snapshot.occurrences)
            .map_err(|_| StableSeqRewriteError::ResolvedSpliceInvalid)?;
        Ok(self.prepare(
            vec![intent],
            RewriteEffect::Fine(FineChange::new(FineChangeKind::Seq, endpoint)),
        ))
    }
}

#[cfg(test)]
mod rewrite_spec_tests {
    use super::*;

    #[test]
    fn rewrite_spec_is_authority_for_identity_law_set_and_footprint() {
        let spec = RewriteSpec {
            id: RewriteSpecId(SemanticId(500)),
            law_set: RewriteLawSetId(SemanticId(501)),
            footprint: RewriteFootprint {
                writes: [(
                    SemanticWriteCoordinate::ProductField(SemanticId(10)),
                    RewriteActionLaw::IdempotentAssign {
                        semantic_value: SemanticId(700),
                    },
                )]
                .into_iter()
                .collect(),
                ..RewriteFootprint::default()
            },
        };
        let prepared = spec.prepare(
            vec![SemanticId(900)],
            RewriteEffect::Fine(FineChange::new(FineChangeKind::Scalar, 7_i64)),
        );
        assert_eq!(prepared.spec, spec.id);
        assert_eq!(prepared.law_set, spec.law_set);
        assert_eq!(prepared.explicit_inputs, vec![SemanticId(900)]);
        assert_eq!(
            spec.pair_law_with(&spec),
            PairRewriteLaw::SameIdempotentIntent
        );
    }
}

#[cfg(test)]
mod semantic_collection_change_tests {
    use super::*;

    fn coord(observable: u128, class: u128) -> SemanticClassCoordinate {
        SemanticClassCoordinate {
            observable: RevisionObservableId(observable),
            class: EqClassId(class),
        }
    }

    #[test]
    fn set_change_addresses_semantic_classes_not_representatives() {
        let change = SemanticSetChange {
            observable: RevisionObservableId(1),
            inserted: [(EqClassId(3), "new")].into_iter().collect(),
            removed: [EqClassId(1)].into_iter().collect(),
        };
        let next = change
            .apply_classified([(coord(1, 1), "A"), (coord(1, 2), "B")])
            .unwrap();
        assert_eq!(
            next,
            [(EqClassId(2), "B"), (EqClassId(3), "new")]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn bag_change_updates_multiplicity_per_semantic_class() {
        let change = SemanticBagChange {
            observable: RevisionObservableId(4),
            classes: [(
                EqClassId(7),
                SemanticBagClassChange {
                    representative: None,
                    inserted: 2,
                    removed: 1,
                },
            )]
            .into_iter()
            .collect(),
        };
        let next = change.apply_classified([(coord(4, 7), "A", 3)]).unwrap();
        assert_eq!(next[&EqClassId(7)], ("A", 4));
    }

    #[test]
    fn map_rejects_two_source_keys_in_the_same_semantic_class() {
        let change = SemanticMapChange::<&str, i64> {
            key_observable: RevisionObservableId(9),
            upserted: BTreeMap::new(),
            removed: BTreeSet::new(),
        };
        assert!(matches!(
            change.apply_classified([(coord(9, 2), "A", 1), (coord(9, 2), "a", 2),]),
            Err(SemanticCollectionChangeError::DuplicateMapKeyClass(_))
        ));
    }
}
