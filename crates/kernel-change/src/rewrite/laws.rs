use std::collections::BTreeSet;

use kernel_types::SemanticId;

use crate::stable_seq::{StableSeqPairDecision, StableSeqRewriteIntent};

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
    /// Coarse authority coordinate for a whole logical relation. It may
    /// conservatively cover any class-local write in the same relation.
    RelationWhole {
        relation: SemanticId,
    },
    /// Collision-free Γ-canonical relation class coordinate. The byte string
    /// is the canonical tuple encoding produced by the pinned semantic layer.
    RelationClass {
        relation: SemanticId,
        canonical_key: Box<[u8]>,
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

impl RewriteFootprint {
    #[must_use]
    pub fn opaque_relation(relation: SemanticId) -> Self {
        Self {
            writes: [(
                SemanticWriteCoordinate::RelationWhole { relation },
                RewriteActionLaw::Opaque,
            )]
            .into_iter()
            .collect(),
            ..Self::default()
        }
    }

    /// Returns true when this declared footprint is at least as conservative
    /// as `required`. Required reads/obligations must remain visible and every
    /// required write must retain the exact action law; additional coordinates
    /// and obligations may only make coordination more conservative.
    #[must_use]
    pub fn conservatively_covers(&self, required: &Self) -> bool {
        required.reads.iter().all(|coordinate| {
            self.reads.contains(coordinate)
                || relation_whole_coordinate(coordinate)
                    .is_some_and(|whole| self.reads.contains(&whole))
        }) && required
            .invariant_obligations
            .is_subset(&self.invariant_obligations)
            && required.writes.iter().all(|(coordinate, law)| {
                self.writes.get(coordinate) == Some(law)
                    || relation_whole_coordinate(coordinate)
                        .is_some_and(|whole| self.writes.get(&whole) == Some(law))
            })
    }
}

fn relation_whole_coordinate(
    coordinate: &SemanticWriteCoordinate,
) -> Option<SemanticWriteCoordinate> {
    match coordinate {
        SemanticWriteCoordinate::RelationClass { relation, .. }
        | SemanticWriteCoordinate::RelationIdentity { relation, .. } => {
            Some(SemanticWriteCoordinate::RelationWhole {
                relation: *relation,
            })
        }
        _ => None,
    }
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
