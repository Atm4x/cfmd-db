use std::fmt;

use crate::query::{QueryNodeId, QuerySource};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    Recovery,
    Query,
    InvalidPlan,
    InvalidSchema,
    TypeMismatch,
    Cardinality,
    NotFound,
    StaleRevision,
    TransactionConflict,
    HistoryRebaseConflict,
    InvariantViolation,
    NonReversibleHistory,
    WatchUnavailable,
    WatchClosed,
    ResourceLimit,
    PermissionDenied,
    SessionRevoked,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    query_node: Option<QueryNodeId>,
    query_source: Option<QuerySource>,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            query_node: None,
            query_source: None,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn query_node(&self) -> Option<QueryNodeId> {
        self.query_node
    }

    #[must_use]
    pub const fn query_source(&self) -> Option<QuerySource> {
        self.query_source
    }

    pub(crate) const fn with_query(mut self, node: QueryNodeId, source: QuerySource) -> Self {
        self.query_node = Some(node);
        self.query_source = Some(source);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
