use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use crate::{EquivalenceId, Error, ErrorKind, OrderingId, RelationId, Row, Value};

static NEXT_QUERY_NODE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryNodeId(u64);

impl QueryNodeId {
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuerySource {
    file: &'static str,
    line: u32,
    column: u32,
}

impl QuerySource {
    #[must_use]
    pub const fn file(self) -> &'static str {
        self.file
    }
    #[must_use]
    pub const fn line(self) -> u32 {
        self.line
    }
    #[must_use]
    pub const fn column(self) -> u32 {
        self.column
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum QueryNodeKind {
    Scan,
    FilterEq,
    FilterEqColumns,
    FilterOrder,
    Project,
    JoinEq,
    Difference,
    Union,
    AntiJoin,
    GroupCount,
    GroupExactF64Sum,
    Distinct,
    TopKWithTies,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryNode {
    id: QueryNodeId,
    kind: QueryNodeKind,
    source: QuerySource,
    parents: Vec<QueryNodeId>,
}

impl QueryNode {
    #[must_use]
    pub const fn id(&self) -> QueryNodeId {
        self.id
    }
    #[must_use]
    pub const fn kind(&self) -> QueryNodeKind {
        self.kind
    }
    #[must_use]
    pub const fn source(&self) -> QuerySource {
        self.source
    }
    #[must_use]
    pub fn parents(&self) -> &[QueryNodeId] {
        &self.parents
    }
}

fn caller_source(location: &'static std::panic::Location<'static>) -> QuerySource {
    QuerySource {
        file: location.file(),
        line: location.line(),
        column: location.column(),
    }
}

fn merge_nodes(target: &mut Vec<QueryNode>, source: Vec<QueryNode>) {
    for node in source {
        if !target.iter().any(|existing| existing.id == node.id) {
            target.push(node);
        }
    }
}

fn next_node(kind: QueryNodeKind, source: QuerySource, parents: Vec<QueryNodeId>) -> QueryNode {
    QueryNode {
        id: QueryNodeId(NEXT_QUERY_NODE_ID.fetch_add(1, AtomicOrdering::Relaxed)),
        kind,
        source,
        parents,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderComparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub(crate) inner: kernel_query::RelExpr,
    nodes: Vec<QueryNode>,
    root: QueryNodeId,
}

impl Query {
    #[track_caller]
    #[must_use]
    pub fn scan(relation: RelationId) -> Self {
        let node = next_node(
            QueryNodeKind::Scan,
            caller_source(std::panic::Location::caller()),
            Vec::new(),
        );
        Self {
            inner: kernel_query::RelExpr::Scan(relation.into()),
            nodes: vec![node.clone()],
            root: node.id,
        }
    }

    #[must_use]
    pub const fn node_id(&self) -> QueryNodeId {
        self.root
    }

    #[must_use]
    pub fn source(&self) -> QuerySource {
        self.nodes
            .last()
            .expect("query trace always has a root node")
            .source
    }

    #[must_use]
    pub fn nodes(&self) -> &[QueryNode] {
        &self.nodes
    }

    fn unary(
        mut self,
        inner: kernel_query::RelExpr,
        kind: QueryNodeKind,
        source: QuerySource,
    ) -> Self {
        let node = next_node(kind, source, vec![self.root]);
        self.root = node.id;
        self.nodes.push(node);
        self.inner = inner;
        self
    }

    #[track_caller]
    #[must_use]
    pub fn filter_eq(self, column: usize, value: Value, equivalence: EquivalenceId) -> Self {
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::FilterEqConst {
                input,
                column,
                value: value.into(),
                equivalence: equivalence.into(),
            },
            QueryNodeKind::FilterEq,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub fn filter_eq_columns(
        self,
        left_column: usize,
        right_column: usize,
        equivalence: EquivalenceId,
    ) -> Self {
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence: equivalence.into(),
            },
            QueryNodeKind::FilterEqColumns,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub fn filter_order(
        self,
        column: usize,
        value: Value,
        ordering: OrderingId,
        comparison: OrderComparison,
    ) -> Self {
        let input = Box::new(self.inner.clone());
        let comparison = match comparison {
            OrderComparison::Less => kernel_query::OrderComparison::Less,
            OrderComparison::LessOrEqual => kernel_query::OrderComparison::LessOrEqual,
            OrderComparison::Greater => kernel_query::OrderComparison::Greater,
            OrderComparison::GreaterOrEqual => kernel_query::OrderComparison::GreaterOrEqual,
        };
        self.unary(
            kernel_query::RelExpr::FilterOrderConst {
                input,
                column,
                value: value.into(),
                ordering: ordering.into(),
                comparison,
            },
            QueryNodeKind::FilterOrder,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub fn project(self, columns: impl Into<Vec<usize>>) -> Self {
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::Project {
                input,
                columns: columns.into(),
            },
            QueryNodeKind::Project,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub(crate) fn project_preserving_multiplicity(self, columns: impl Into<Vec<usize>>) -> Self {
        let input = Box::new(kernel_query::RelExpr::PromoteToBag(Box::new(
            self.inner.clone(),
        )));
        self.unary(
            kernel_query::RelExpr::Project {
                input,
                columns: columns.into(),
            },
            QueryNodeKind::Project,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub fn join_eq(
        mut self,
        right: Self,
        left_column: usize,
        right_column: usize,
        equivalence: EquivalenceId,
    ) -> Self {
        let left_root = self.root;
        let right_root = right.root;
        let inner = kernel_query::RelExpr::JoinEq {
            left: Box::new(self.inner.clone()),
            right: Box::new(right.inner),
            left_column,
            right_column,
            equivalence: equivalence.into(),
        };
        merge_nodes(&mut self.nodes, right.nodes);
        let node = next_node(
            QueryNodeKind::JoinEq,
            caller_source(std::panic::Location::caller()),
            vec![left_root, right_root],
        );
        self.root = node.id;
        self.nodes.push(node);
        self.inner = inner;
        self
    }

    #[track_caller]
    #[must_use]
    pub fn difference(mut self, right: Self) -> Self {
        let left_root = self.root;
        let right_root = right.root;
        let inner = kernel_query::RelExpr::Difference {
            left: Box::new(self.inner.clone()),
            right: Box::new(right.inner),
        };
        merge_nodes(&mut self.nodes, right.nodes);
        let node = next_node(
            QueryNodeKind::Difference,
            caller_source(std::panic::Location::caller()),
            vec![left_root, right_root],
        );
        self.root = node.id;
        self.nodes.push(node);
        self.inner = inner;
        self
    }

    #[track_caller]
    #[must_use]
    pub fn union(mut self, right: Self) -> Self {
        let left_root = self.root;
        let right_root = right.root;
        let inner = kernel_query::RelExpr::Union {
            left: Box::new(self.inner.clone()),
            right: Box::new(right.inner),
        };
        merge_nodes(&mut self.nodes, right.nodes);
        let node = next_node(
            QueryNodeKind::Union,
            caller_source(std::panic::Location::caller()),
            vec![left_root, right_root],
        );
        self.root = node.id;
        self.nodes.push(node);
        self.inner = inner;
        self
    }

    #[track_caller]
    #[must_use]
    pub fn anti_join(
        mut self,
        right: Self,
        left_column: usize,
        right_column: usize,
        equivalence: EquivalenceId,
    ) -> Self {
        let left_root = self.root;
        let right_root = right.root;
        let inner = kernel_query::RelExpr::AntiJoin {
            left: Box::new(self.inner.clone()),
            right: Box::new(right.inner),
            left_column,
            right_column,
            equivalence: equivalence.into(),
        };
        merge_nodes(&mut self.nodes, right.nodes);
        let node = next_node(
            QueryNodeKind::AntiJoin,
            caller_source(std::panic::Location::caller()),
            vec![left_root, right_root],
        );
        self.root = node.id;
        self.nodes.push(node);
        self.inner = inner;
        self
    }

    #[track_caller]
    #[must_use]
    pub fn group_count(
        self,
        group_columns: impl Into<Vec<usize>>,
        group_equivalences: impl Into<Vec<EquivalenceId>>,
        result_equivalence: EquivalenceId,
    ) -> Self {
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::Group {
                input,
                group_columns: group_columns.into(),
                group_equivalences: group_equivalences
                    .into()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                aggregate: kernel_query::AggregateSpec::Count {
                    result_equivalence: result_equivalence.into(),
                },
            },
            QueryNodeKind::GroupCount,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub fn group_exact_f64_sum(
        self,
        group_columns: impl Into<Vec<usize>>,
        group_equivalences: impl Into<Vec<EquivalenceId>>,
        value_column: usize,
        result_equivalence: EquivalenceId,
    ) -> Self {
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::Group {
                input,
                group_columns: group_columns.into(),
                group_equivalences: group_equivalences
                    .into()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                aggregate: kernel_query::AggregateSpec::ExactF64Sum {
                    value_column,
                    result_equivalence: result_equivalence.into(),
                },
            },
            QueryNodeKind::GroupExactF64Sum,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub fn count(self, result_equivalence: EquivalenceId) -> Self {
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::Group {
                input,
                group_columns: Vec::new(),
                group_equivalences: Vec::new(),
                aggregate: kernel_query::AggregateSpec::Count {
                    result_equivalence: result_equivalence.into(),
                },
            },
            QueryNodeKind::GroupCount,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
    #[must_use]
    pub fn distinct(self, column_equivalences: impl Into<Vec<EquivalenceId>>) -> Self {
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::Distinct {
                input,
                column_equivalences: column_equivalences
                    .into()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
            },
            QueryNodeKind::Distinct,
            caller_source(std::panic::Location::caller()),
        )
    }

    #[track_caller]
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
        let input = Box::new(self.inner.clone());
        self.unary(
            kernel_query::RelExpr::TopKWithTies {
                input,
                column,
                ordering: ordering.into(),
                direction,
                k,
            },
            QueryNodeKind::TopKWithTies,
            caller_source(std::panic::Location::caller()),
        )
    }
}

#[derive(Debug, Clone)]
pub struct PreparedQuery {
    pub(crate) inner: kernel_query::PreparedRelExpr,
    pub(crate) node: QueryNodeId,
    pub(crate) source: QuerySource,
}

impl PreparedQuery {
    #[must_use]
    pub const fn node_id(&self) -> QueryNodeId {
        self.node
    }

    #[must_use]
    pub const fn source(&self) -> QuerySource {
        self.source
    }
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

pub(crate) fn query_error_at(query: &Query, error: &kernel_query::RelQueryError) -> Error {
    query_error(error).with_query(query.node_id(), query.source())
}
