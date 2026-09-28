use std::collections::{BTreeMap, BTreeSet};

use kernel_types::SemanticId;

use crate::change::SeqSplice;

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

/// Compiled identity index over one authoritative stable-sequence snapshot.
///
/// The public snapshot remains a plain value object. Callers which resolve a
/// batch of durable intents prepare this view once, validate occurrence
/// identity uniqueness once, and then reuse logarithmic identity lookups for
/// every anchor/occurrence resolution in the batch.
#[derive(Debug)]
pub struct PreparedStableSeqSnapshot<'a, T> {
    snapshot: &'a StableSeqSnapshot<T>,
    occurrence_indices: BTreeMap<SeqOccurrenceId, usize>,
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
    FootprintMismatch,
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
    pub fn resolve_intent(
        &self,
        intent: &StableSeqRewriteIntent<T>,
    ) -> Result<SeqSplice<StableSeqOccurrence<T>>, StableSeqRewriteError> {
        resolve_intent_with_lookup(self, intent, |occurrence| {
            self.occurrences
                .iter()
                .position(|entry| entry.id == occurrence)
        })
    }
}

impl<T> StableSeqSnapshot<T> {
    pub fn prepare(&self) -> Result<PreparedStableSeqSnapshot<'_, T>, StableSeqRewriteError> {
        let mut occurrence_indices = BTreeMap::new();
        for (index, occurrence) in self.occurrences.iter().enumerate() {
            if occurrence_indices.insert(occurrence.id, index).is_some() {
                return Err(StableSeqRewriteError::DuplicateOccurrence(occurrence.id));
            }
        }
        Ok(PreparedStableSeqSnapshot {
            snapshot: self,
            occurrence_indices,
        })
    }
}

impl<T> PreparedStableSeqSnapshot<'_, T> {
    #[must_use]
    pub const fn snapshot(&self) -> &StableSeqSnapshot<T> {
        self.snapshot
    }
}

impl<T: Clone> PreparedStableSeqSnapshot<'_, T> {
    pub fn resolve_intent(
        &self,
        intent: &StableSeqRewriteIntent<T>,
    ) -> Result<SeqSplice<StableSeqOccurrence<T>>, StableSeqRewriteError> {
        resolve_intent_with_lookup(self.snapshot, intent, |occurrence| {
            self.occurrence_indices.get(&occurrence).copied()
        })
    }
}

fn resolve_intent_with_lookup<T: Clone>(
    snapshot: &StableSeqSnapshot<T>,
    intent: &StableSeqRewriteIntent<T>,
    mut occurrence_index: impl FnMut(SeqOccurrenceId) -> Option<usize>,
) -> Result<SeqSplice<StableSeqOccurrence<T>>, StableSeqRewriteError> {
    if intent.sequence() != snapshot.sequence {
        return Err(StableSeqRewriteError::WrongSequence);
    }
    match intent {
        StableSeqRewriteIntent::Insert {
            anchor,
            occurrence,
            value,
            ..
        } => {
            if occurrence_index(*occurrence).is_some() {
                return Err(StableSeqRewriteError::DuplicateOccurrence(*occurrence));
            }
            Ok(SeqSplice {
                start: resolve_gap_with_lookup(snapshot, *anchor, &mut occurrence_index)?,
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
            start: required_occurrence_index(*occurrence, &mut occurrence_index)?,
            delete_count: 1,
            insert: vec![StableSeqOccurrence {
                id: *occurrence,
                value: value.clone(),
            }],
        }),
        StableSeqRewriteIntent::Delete { occurrence, .. } => Ok(SeqSplice {
            start: required_occurrence_index(*occurrence, &mut occurrence_index)?,
            delete_count: 1,
            insert: Vec::new(),
        }),
    }
}

fn required_occurrence_index(
    occurrence: SeqOccurrenceId,
    occurrence_index: &mut impl FnMut(SeqOccurrenceId) -> Option<usize>,
) -> Result<usize, StableSeqRewriteError> {
    occurrence_index(occurrence).ok_or(StableSeqRewriteError::MissingOccurrence(occurrence))
}

fn resolve_gap_with_lookup<T>(
    snapshot: &StableSeqSnapshot<T>,
    anchor: StableSeqGapAnchor,
    occurrence_index: &mut impl FnMut(SeqOccurrenceId) -> Option<usize>,
) -> Result<usize, StableSeqRewriteError> {
    if !snapshot.retained_anchor_histories.contains(&anchor.history) {
        return Err(StableSeqRewriteError::ExpiredAnchorHistory(anchor.history));
    }
    let len = snapshot.occurrences.len();
    match (anchor.left, anchor.right) {
        (None, None) if len == 0 => Ok(0),
        (None, Some(right)) => {
            let right = required_occurrence_index(right, occurrence_index)?;
            if right != 0 {
                return Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id));
            }
            Ok(0)
        }
        (Some(left), None) => {
            let left = required_occurrence_index(left, occurrence_index)?;
            if left + 1 != len {
                return Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id));
            }
            Ok(len)
        }
        (Some(left), Some(right)) => {
            let left = required_occurrence_index(left, occurrence_index)?;
            let right = required_occurrence_index(right, occurrence_index)?;
            if left + 1 != right {
                return Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id));
            }
            Ok(right)
        }
        (None, None) => Err(StableSeqRewriteError::AnchorNoLongerGap(anchor.id)),
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
