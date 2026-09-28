use std::fmt;

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
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
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
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
