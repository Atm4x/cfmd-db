use crate::{ProtocolValue, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotTarget {
    Head,
    Revision(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolQuery {
    Scan {
        relation: u128,
    },
    FilterEq {
        input: Box<Self>,
        column: usize,
        value: ProtocolValue,
        equivalence: u128,
    },
    Project {
        input: Box<Self>,
        columns: Vec<usize>,
    },
    JoinEq {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: u128,
    },
    Difference {
        left: Box<Self>,
        right: Box<Self>,
    },
    AntiJoin {
        left: Box<Self>,
        right: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: u128,
    },
    GroupCount {
        input: Box<Self>,
        group_column: usize,
        group_equivalence: u128,
        result_equivalence: u128,
    },
    Distinct {
        input: Box<Self>,
        column_equivalences: Vec<u128>,
    },
    TopKWithTies {
        input: Box<Self>,
        column: usize,
        ordering: u128,
        direction: OrderDirection,
        k: usize,
    },
}

impl ProtocolQuery {
    pub(crate) fn into_runtime(self) -> cfmd_runtime::Query {
        match self {
            Self::Scan { relation } => {
                cfmd_runtime::Query::scan(cfmd_runtime::RelationId::new(relation))
            }
            Self::FilterEq {
                input,
                column,
                value,
                equivalence,
            } => input.into_runtime().filter_eq(
                column,
                value.into(),
                cfmd_runtime::EquivalenceId::new(equivalence),
            ),
            Self::Project { input, columns } => input.into_runtime().project(columns),
            Self::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => left.into_runtime().join_eq(
                right.into_runtime(),
                left_column,
                right_column,
                cfmd_runtime::EquivalenceId::new(equivalence),
            ),
            Self::Difference { left, right } => {
                left.into_runtime().difference(right.into_runtime())
            }
            Self::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => left.into_runtime().anti_join(
                right.into_runtime(),
                left_column,
                right_column,
                cfmd_runtime::EquivalenceId::new(equivalence),
            ),
            Self::GroupCount {
                input,
                group_column,
                group_equivalence,
                result_equivalence,
            } => input.into_runtime().group_count(
                group_column,
                cfmd_runtime::EquivalenceId::new(group_equivalence),
                cfmd_runtime::EquivalenceId::new(result_equivalence),
            ),
            Self::Distinct {
                input,
                column_equivalences,
            } => input.into_runtime().distinct(
                column_equivalences
                    .into_iter()
                    .map(cfmd_runtime::EquivalenceId::new)
                    .collect::<Vec<_>>(),
            ),
            Self::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            } => input.into_runtime().top_k_with_ties(
                column,
                cfmd_runtime::OrderingId::new(ordering),
                match direction {
                    OrderDirection::Ascending => cfmd_runtime::OrderDirection::Ascending,
                    OrderDirection::Descending => cfmd_runtime::OrderDirection::Descending,
                },
                k,
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRequest {
    pub target: SnapshotTarget,
    pub query: ProtocolQuery,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryResponse {
    pub revision: u64,
    pub rows: Vec<Row>,
    pub is_set: bool,
    pub column_equivalences: Vec<u128>,
}

impl QueryResponse {
    pub(crate) fn from_runtime(
        revision: cfmd_runtime::RevisionId,
        result: cfmd_runtime::RelationResult,
    ) -> Self {
        let (rows, is_set, column_equivalences) = match result {
            cfmd_runtime::RelationResult::Bag(rows) => (rows, false, Vec::new()),
            cfmd_runtime::RelationResult::Set {
                rows,
                column_equivalences,
            } => (
                rows,
                true,
                column_equivalences
                    .into_iter()
                    .map(cfmd_runtime::EquivalenceId::raw)
                    .collect(),
            ),
        };
        Self {
            revision: revision.raw(),
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(Into::into).collect())
                .collect(),
            is_set,
            column_equivalences,
        }
    }
}
