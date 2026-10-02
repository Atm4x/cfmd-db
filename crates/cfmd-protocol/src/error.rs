use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolErrorCode {
    Recovery,
    Query,
    InvalidRequest,
    InvalidSchema,
    TypeMismatch,
    Cardinality,
    NotFound,
    StaleRevision,
    TransactionConflict,
    HistoryConflict,
    InvariantViolation,
    NonReversibleHistory,
    WatchUnavailable,
    WatchClosed,
    ResourceLimit,
    PermissionDenied,
    SessionClosed,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    code: ProtocolErrorCode,
    message: String,
}

impl ProtocolError {
    #[must_use]
    pub fn new(code: ProtocolErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn code(&self) -> ProtocolErrorCode {
        self.code
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for ProtocolError {}

impl From<cfmd_runtime::Error> for ProtocolError {
    fn from(value: cfmd_runtime::Error) -> Self {
        use cfmd_runtime::ErrorKind;
        let (code, message) = match value.kind() {
            ErrorKind::Recovery => (
                ProtocolErrorCode::Recovery,
                "database recovery operation failed".to_owned(),
            ),
            ErrorKind::Query => (ProtocolErrorCode::Query, value.message().to_owned()),
            ErrorKind::InvalidPlan => (
                ProtocolErrorCode::InvalidRequest,
                value.message().to_owned(),
            ),
            ErrorKind::InvalidSchema => {
                (ProtocolErrorCode::InvalidSchema, value.message().to_owned())
            }
            ErrorKind::TypeMismatch => {
                (ProtocolErrorCode::TypeMismatch, value.message().to_owned())
            }
            ErrorKind::Cardinality => (ProtocolErrorCode::Cardinality, value.message().to_owned()),
            ErrorKind::NotFound => (ProtocolErrorCode::NotFound, value.message().to_owned()),
            ErrorKind::StaleRevision => {
                (ProtocolErrorCode::StaleRevision, value.message().to_owned())
            }
            ErrorKind::TransactionConflict => (
                ProtocolErrorCode::TransactionConflict,
                value.message().to_owned(),
            ),
            ErrorKind::HistoryRebaseConflict => (
                ProtocolErrorCode::HistoryConflict,
                value.message().to_owned(),
            ),
            ErrorKind::InvariantViolation => (
                ProtocolErrorCode::InvariantViolation,
                "operation violated a database invariant".to_owned(),
            ),
            ErrorKind::NonReversibleHistory => (
                ProtocolErrorCode::NonReversibleHistory,
                value.message().to_owned(),
            ),
            ErrorKind::WatchUnavailable => (
                ProtocolErrorCode::WatchUnavailable,
                value.message().to_owned(),
            ),
            ErrorKind::WatchClosed => (ProtocolErrorCode::WatchClosed, value.message().to_owned()),
            ErrorKind::ResourceLimit => {
                (ProtocolErrorCode::ResourceLimit, value.message().to_owned())
            }
            ErrorKind::PermissionDenied => (
                ProtocolErrorCode::PermissionDenied,
                "permission denied".to_owned(),
            ),
            ErrorKind::SessionRevoked => (
                ProtocolErrorCode::SessionClosed,
                "hosted session is closed".to_owned(),
            ),
            ErrorKind::Internal | _ => (
                ProtocolErrorCode::Internal,
                "internal database error".to_owned(),
            ),
        };
        Self::new(code, message)
    }
}

pub type Result<T> = std::result::Result<T, ProtocolError>;
