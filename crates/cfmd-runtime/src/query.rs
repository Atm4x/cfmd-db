use crate::{EquivalenceId, Error, ErrorKind, OrderingId, RelationId, Row, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub(crate) inner: kernel_query::RelExpr,
}

impl Query {
    #[must_use]
    pub fn scan(relation: RelationId) -> Self {
        Self {
            inner: kernel_query::RelExpr::Scan(relation.into()),
        }
    }

    #[must_use]
    pub fn filter_eq(self, column: usize, value: Value, equivalence: EquivalenceId) -> Self {
        Self {
            inner: kernel_query::RelExpr::FilterEqConst {
                input: Box::new(self.inner),
                column,
                value: value.into(),
                equivalence: equivalence.into(),
            },
        }
    }

    #[must_use]
    pub fn project(self, columns: impl Into<Vec<usize>>) -> Self {
        Self {
            inner: kernel_query::RelExpr::Project {
                input: Box::new(self.inner),
                columns: columns.into(),
            },
        }
    }

    #[must_use]
    pub fn join_eq(
        self,
        right: Self,
        left_column: usize,
        right_column: usize,
        equivalence: EquivalenceId,
    ) -> Self {
        Self {
            inner: kernel_query::RelExpr::JoinEq {
                left: Box::new(self.inner),
                right: Box::new(right.inner),
                left_column,
                right_column,
                equivalence: equivalence.into(),
            },
        }
    }

    #[must_use]
    pub fn difference(self, right: Self) -> Self {
        Self {
            inner: kernel_query::RelExpr::Difference {
                left: Box::new(self.inner),
                right: Box::new(right.inner),
            },
        }
    }

    #[must_use]
    pub fn anti_join(
        self,
        right: Self,
        left_column: usize,
        right_column: usize,
        equivalence: EquivalenceId,
    ) -> Self {
        Self {
            inner: kernel_query::RelExpr::AntiJoin {
                left: Box::new(self.inner),
                right: Box::new(right.inner),
                left_column,
                right_column,
                equivalence: equivalence.into(),
            },
        }
    }

    #[must_use]
    pub fn group_count(
        self,
        group_column: usize,
        group_equivalence: EquivalenceId,
        result_equivalence: EquivalenceId,
    ) -> Self {
        Self {
            inner: kernel_query::RelExpr::Group {
                input: Box::new(self.inner),
                group_columns: vec![group_column],
                group_equivalences: vec![group_equivalence.into()],
                aggregate: kernel_query::AggregateSpec::Count {
                    result_equivalence: result_equivalence.into(),
                },
            },
        }
    }

    #[must_use]
    pub fn distinct(self, column_equivalences: impl Into<Vec<EquivalenceId>>) -> Self {
        Self {
            inner: kernel_query::RelExpr::Distinct {
                input: Box::new(self.inner),
                column_equivalences: column_equivalences
                    .into()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            },
        }
    }

    #[must_use]
    pub fn top_k_with_ties(
        self,
        column: usize,
        ordering: OrderingId,
        direction: OrderDirection,
        k: usize,
    ) -> Self {
        let direction = match direction {
            OrderDirection::Ascending => kernel_query::OrderDirection::Ascending,
            OrderDirection::Descending => kernel_query::OrderDirection::Descending,
        };
        Self {
            inner: kernel_query::RelExpr::TopKWithTies {
                input: Box::new(self.inner),
                column,
                ordering: ordering.into(),
                direction,
                k,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreparedQuery {
    pub(crate) inner: kernel_query::PreparedRelExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationResult {
    Bag(Vec<Row>),
    Set {
        rows: Vec<Row>,
        column_equivalences: Vec<EquivalenceId>,
    },
}

impl RelationResult {
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        match self {
            Self::Bag(rows) | Self::Set { rows, .. } => rows,
        }
    }
}

impl From<kernel_query::RelationValue> for RelationResult {
    fn from(value: kernel_query::RelationValue) -> Self {
        match value {
            kernel_query::RelationValue::Bag(rows) => Self::Bag(convert_rows(rows)),
            kernel_query::RelationValue::Set {
                rows,
                column_equivalences,
            } => Self::Set {
                rows: convert_rows(rows),
                column_equivalences: column_equivalences
                    .into_iter()
                    .map(|id| EquivalenceId::new(id.raw()))
                    .collect(),
            },
        }
    }
}

fn convert_rows(rows: Vec<kernel_query::Row>) -> Vec<Row> {
    rows.into_iter()
        .map(|row| row.into_iter().map(Into::into).collect())
        .collect()
}

pub(crate) fn query_error(error: &kernel_query::RelQueryError) -> Error {
    Error::new(ErrorKind::Query, format!("query failed: {error:?}"))
}
