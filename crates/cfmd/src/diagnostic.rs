use crate::{Error, ErrorKind, MigrationDiagnostic, QueryNodeId, QuerySource, RecoveryDiagnostic};

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
    ContractNotRepresentable,
    Cardinality,
    NotFound,
    StaleRevision,
    TransactionConflict,
    FormationProofUnavailable,
    FormationProofInvalidated,
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

impl DiagnosticCode {
    /// Stable transport/binding spelling for this product diagnostic category.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recovery => "Recovery",
            Self::Query => "Query",
            Self::InvalidPlan => "InvalidPlan",
            Self::InvalidSchema => "InvalidSchema",
            Self::TypeMismatch => "TypeMismatch",
            Self::ContractNotRepresentable => "ContractNotRepresentable",
            Self::Cardinality => "Cardinality",
            Self::NotFound => "NotFound",
            Self::StaleRevision => "StaleRevision",
            Self::TransactionConflict => "TransactionConflict",
            Self::FormationProofUnavailable => "FormationProofUnavailable",
            Self::FormationProofInvalidated => "FormationProofInvalidated",
            Self::HistoryRebaseConflict => "HistoryRebaseConflict",
            Self::InvariantViolation => "InvariantViolation",
            Self::NonReversibleHistory => "NonReversibleHistory",
            Self::WatchUnavailable => "WatchUnavailable",
            Self::WatchClosed => "WatchClosed",
            Self::ResourceLimit => "ResourceLimit",
            Self::PermissionDenied => "PermissionDenied",
            Self::SessionRevoked => "SessionRevoked",
            Self::Internal => "Internal",
            Self::Unknown => "Unknown",
        }
    }
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
    migration: Option<MigrationDiagnostic>,
    recovery: Option<RecoveryDiagnostic>,
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

    /// Exact migration/schema-bridge diagnostic attached by the runtime, when applicable.
    #[must_use]
    pub const fn migration(&self) -> Option<&MigrationDiagnostic> {
        self.migration.as_ref()
    }

    /// Exact recovery/deployment failure projection, when this diagnostic came from persistence.
    #[must_use]
    pub const fn recovery(&self) -> Option<RecoveryDiagnostic> {
        self.recovery
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
            migration: self.migration_diagnostic().cloned(),
            recovery: self.recovery_diagnostic(),
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
        ErrorKind::ContractNotRepresentable => DiagnosticCode::ContractNotRepresentable,
        ErrorKind::Cardinality => DiagnosticCode::Cardinality,
        ErrorKind::NotFound => DiagnosticCode::NotFound,
        ErrorKind::StaleRevision => DiagnosticCode::StaleRevision,
        ErrorKind::TransactionConflict => DiagnosticCode::TransactionConflict,
        ErrorKind::FormationProofUnavailable => DiagnosticCode::FormationProofUnavailable,
        ErrorKind::FormationProofInvalidated => DiagnosticCode::FormationProofInvalidated,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formation_proof_diagnostics_are_stable_product_categories() {
        assert_eq!(
            code_for(ErrorKind::FormationProofUnavailable),
            DiagnosticCode::FormationProofUnavailable
        );
        assert_eq!(
            code_for(ErrorKind::FormationProofInvalidated),
            DiagnosticCode::FormationProofInvalidated
        );
        assert_ne!(
            code_for(ErrorKind::FormationProofInvalidated),
            DiagnosticCode::TransactionConflict
        );
        assert_eq!(
            DiagnosticCode::FormationProofUnavailable.as_str(),
            "FormationProofUnavailable"
        );
        assert_eq!(
            DiagnosticCode::FormationProofInvalidated.as_str(),
            "FormationProofInvalidated"
        );
    }
}
