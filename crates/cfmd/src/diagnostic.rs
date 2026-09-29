use crate::{Error, ErrorKind, QueryNodeId, QuerySource};

/// Stable product-facing diagnostic category.
///
/// This deliberately belongs to the `cfmd` facade rather than an internal runtime/kernel crate so
/// CLI, IDE and language bindings can depend on one taxonomy even when internals evolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticCode {
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
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Severity {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    code: DiagnosticCode,
    severity: Severity,
    message: String,
    query_node: Option<QueryNodeId>,
    query_source: Option<QuerySource>,
}

impl Diagnostic {
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }

    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
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
}

pub trait ErrorDiagnosticExt {
    #[must_use]
    fn diagnostic(&self) -> Diagnostic;
}

impl ErrorDiagnosticExt for Error {
    fn diagnostic(&self) -> Diagnostic {
        Diagnostic {
            code: code_for(self.kind()),
            severity: Severity::Error,
            message: self.message().to_owned(),
            query_node: self.query_node(),
            query_source: self.query_source(),
        }
    }
}

const fn code_for(kind: ErrorKind) -> DiagnosticCode {
    match kind {
        ErrorKind::Recovery => DiagnosticCode::Recovery,
        ErrorKind::Query => DiagnosticCode::Query,
        ErrorKind::InvalidPlan => DiagnosticCode::InvalidPlan,
        ErrorKind::InvalidSchema => DiagnosticCode::InvalidSchema,
        ErrorKind::TypeMismatch => DiagnosticCode::TypeMismatch,
        ErrorKind::Cardinality => DiagnosticCode::Cardinality,
        ErrorKind::NotFound => DiagnosticCode::NotFound,
        ErrorKind::StaleRevision => DiagnosticCode::StaleRevision,
        ErrorKind::TransactionConflict => DiagnosticCode::TransactionConflict,
        ErrorKind::HistoryRebaseConflict => DiagnosticCode::HistoryRebaseConflict,
        ErrorKind::InvariantViolation => DiagnosticCode::InvariantViolation,
        ErrorKind::NonReversibleHistory => DiagnosticCode::NonReversibleHistory,
        ErrorKind::WatchUnavailable => DiagnosticCode::WatchUnavailable,
        ErrorKind::WatchClosed => DiagnosticCode::WatchClosed,
        ErrorKind::ResourceLimit => DiagnosticCode::ResourceLimit,
        ErrorKind::PermissionDenied => DiagnosticCode::PermissionDenied,
        ErrorKind::SessionRevoked => DiagnosticCode::SessionRevoked,
        ErrorKind::Internal => DiagnosticCode::Internal,
        _ => DiagnosticCode::Unknown,
    }
}
