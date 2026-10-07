use std::fmt;

use crate::query::{QueryNodeId, QuerySource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RecoveryOperation {
    Open,
    Backup,
    VerifyBackup,
    RestoreBackup,
    Fork,
    AuthorityTransfer,
    PersistenceTransition,
    ProtectionReconfigure,
}

impl RecoveryOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Backup => "Backup",
            Self::VerifyBackup => "VerifyBackup",
            Self::RestoreBackup => "RestoreBackup",
            Self::Fork => "Fork",
            Self::AuthorityTransfer => "AuthorityTransfer",
            Self::PersistenceTransition => "PersistenceTransition",
            Self::ProtectionReconfigure => "ProtectionReconfigure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RecoveryAuthority {
    Storage,
    DurableBytes,
    SingleFileFormat,
    ManifestFormat,
    CheckpointFileFormat,
    MetadataFileFormat,
    PhysicalState,
    Revision,
    MigrationTransport,
    BaseRevision,
    SemanticRevision,
    DurableHead,
}

impl RecoveryAuthority {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Storage => "Storage",
            Self::DurableBytes => "DurableBytes",
            Self::SingleFileFormat => "SingleFileFormat",
            Self::ManifestFormat => "ManifestFormat",
            Self::CheckpointFileFormat => "CheckpointFileFormat",
            Self::MetadataFileFormat => "MetadataFileFormat",
            Self::PhysicalState => "PhysicalState",
            Self::Revision => "Revision",
            Self::MigrationTransport => "MigrationTransport",
            Self::BaseRevision => "BaseRevision",
            Self::SemanticRevision => "SemanticRevision",
            Self::DurableHead => "DurableHead",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RecoveryReason {
    Io,
    Poisoned,
    SequenceExhausted,
    PayloadTooLarge,
    UnsupportedFormat,
    Encoding,
    Corruption,
    ProtocolViolation,
    PhysicalStateRejected,
    RevisionRejected,
    MigrationTransportRejected,
    BaseRevisionMismatch,
    SemanticRevisionMismatch,
    DurableHeadMismatch,
    RestoredRevisionMismatch,
    PathUnavailable,
}

impl RecoveryReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Io => "Io",
            Self::Poisoned => "Poisoned",
            Self::SequenceExhausted => "SequenceExhausted",
            Self::PayloadTooLarge => "PayloadTooLarge",
            Self::UnsupportedFormat => "UnsupportedFormat",
            Self::Encoding => "Encoding",
            Self::Corruption => "Corruption",
            Self::ProtocolViolation => "ProtocolViolation",
            Self::PhysicalStateRejected => "PhysicalStateRejected",
            Self::RevisionRejected => "RevisionRejected",
            Self::MigrationTransportRejected => "MigrationTransportRejected",
            Self::BaseRevisionMismatch => "BaseRevisionMismatch",
            Self::SemanticRevisionMismatch => "SemanticRevisionMismatch",
            Self::DurableHeadMismatch => "DurableHeadMismatch",
            Self::RestoredRevisionMismatch => "RestoredRevisionMismatch",
            Self::PathUnavailable => "PathUnavailable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecoveryDiagnostic {
    operation: RecoveryOperation,
    authority: RecoveryAuthority,
    reason: RecoveryReason,
    byte_offset: Option<usize>,
    format_version: Option<u16>,
}

impl RecoveryDiagnostic {
    #[must_use]
    pub const fn operation(self) -> RecoveryOperation {
        self.operation
    }

    #[must_use]
    pub const fn authority(self) -> RecoveryAuthority {
        self.authority
    }

    #[must_use]
    pub const fn reason(self) -> RecoveryReason {
        self.reason
    }

    #[must_use]
    pub const fn byte_offset(self) -> Option<usize> {
        self.byte_offset
    }

    #[must_use]
    pub const fn format_version(self) -> Option<u16> {
        self.format_version
    }

    pub(crate) const fn new(
        operation: RecoveryOperation,
        authority: RecoveryAuthority,
        reason: RecoveryReason,
        byte_offset: Option<usize>,
        format_version: Option<u16>,
    ) -> Self {
        Self {
            operation,
            authority,
            reason,
            byte_offset,
            format_version,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    message: String,
    query_node: Option<QueryNodeId>,
    query_source: Option<QuerySource>,
    migration_diagnostic: Option<Box<crate::MigrationDiagnostic>>,
    recovery_diagnostic: Option<RecoveryDiagnostic>,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            query_node: None,
            query_source: None,
            migration_diagnostic: None,
            recovery_diagnostic: None,
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

    /// Structured migration/schema-bridge diagnostic when this error was produced by exact
    /// migration representability analysis. Frontends should inspect this value instead of
    /// parsing `message()`.
    #[must_use]
    pub fn migration_diagnostic(&self) -> Option<&crate::MigrationDiagnostic> {
        self.migration_diagnostic.as_deref()
    }

    /// Structured projection of an existing durability/recovery failure.
    /// This never performs recovery or infers a replacement state.
    #[must_use]
    pub const fn recovery_diagnostic(&self) -> Option<RecoveryDiagnostic> {
        self.recovery_diagnostic
    }

    pub(crate) fn with_migration_diagnostic(
        mut self,
        diagnostic: crate::MigrationDiagnostic,
    ) -> Self {
        self.migration_diagnostic = Some(Box::new(diagnostic));
        self
    }

    pub(crate) const fn with_recovery_diagnostic(mut self, diagnostic: RecoveryDiagnostic) -> Self {
        self.recovery_diagnostic = Some(diagnostic);
        self
    }

    pub(crate) fn from_durability(
        operation: RecoveryOperation,
        message: impl Into<String>,
        error: &kernel_durability::DurabilityError,
    ) -> Self {
        Self::new(ErrorKind::Recovery, message)
            .with_recovery_diagnostic(recovery_from_durability(operation, error))
    }

    pub(crate) fn from_runtime_recovery(
        operation: RecoveryOperation,
        message: impl Into<String>,
        error: &kernel_plan::RuntimeRecoveryError,
    ) -> Self {
        let diagnostic = match error {
            kernel_plan::RuntimeRecoveryError::Durability(error) => {
                recovery_from_durability(operation, error)
            }
            kernel_plan::RuntimeRecoveryError::Runtime(_) => RecoveryDiagnostic::new(
                operation,
                RecoveryAuthority::PhysicalState,
                RecoveryReason::PhysicalStateRejected,
                None,
                None,
            ),
            kernel_plan::RuntimeRecoveryError::Revision(_) => RecoveryDiagnostic::new(
                operation,
                RecoveryAuthority::Revision,
                RecoveryReason::RevisionRejected,
                None,
                None,
            ),
            kernel_plan::RuntimeRecoveryError::MigrationTransport(_) => RecoveryDiagnostic::new(
                operation,
                RecoveryAuthority::MigrationTransport,
                RecoveryReason::MigrationTransportRejected,
                None,
                None,
            ),
            kernel_plan::RuntimeRecoveryError::BaseRevisionMismatch => RecoveryDiagnostic::new(
                operation,
                RecoveryAuthority::BaseRevision,
                RecoveryReason::BaseRevisionMismatch,
                None,
                None,
            ),
            kernel_plan::RuntimeRecoveryError::SemanticRevisionMismatch => RecoveryDiagnostic::new(
                operation,
                RecoveryAuthority::SemanticRevision,
                RecoveryReason::SemanticRevisionMismatch,
                None,
                None,
            ),
            kernel_plan::RuntimeRecoveryError::DurableHeadMismatch => RecoveryDiagnostic::new(
                operation,
                RecoveryAuthority::DurableHead,
                RecoveryReason::DurableHeadMismatch,
                None,
                None,
            ),
        };
        Self::new(ErrorKind::Recovery, message).with_recovery_diagnostic(diagnostic)
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

fn recovery_from_durability(
    operation: RecoveryOperation,
    error: &kernel_durability::DurabilityError,
) -> RecoveryDiagnostic {
    use kernel_durability::{DurabilityError, DurableFormatComponent};
    match error {
        DurabilityError::Io(_) => RecoveryDiagnostic::new(
            operation,
            RecoveryAuthority::Storage,
            RecoveryReason::Io,
            None,
            None,
        ),
        DurabilityError::Poisoned => RecoveryDiagnostic::new(
            operation,
            RecoveryAuthority::Storage,
            RecoveryReason::Poisoned,
            None,
            None,
        ),
        DurabilityError::LsnExhausted => RecoveryDiagnostic::new(
            operation,
            RecoveryAuthority::DurableBytes,
            RecoveryReason::SequenceExhausted,
            None,
            None,
        ),
        DurabilityError::PayloadTooLarge => RecoveryDiagnostic::new(
            operation,
            RecoveryAuthority::DurableBytes,
            RecoveryReason::PayloadTooLarge,
            None,
            None,
        ),
        DurabilityError::UnsupportedDurableFormat { component, version } => {
            let authority = match component {
                DurableFormatComponent::SingleFile => RecoveryAuthority::SingleFileFormat,
                DurableFormatComponent::Manifest => RecoveryAuthority::ManifestFormat,
                DurableFormatComponent::CheckpointFile => RecoveryAuthority::CheckpointFileFormat,
                DurableFormatComponent::MetadataFile => RecoveryAuthority::MetadataFileFormat,
            };
            RecoveryDiagnostic::new(
                operation,
                authority,
                RecoveryReason::UnsupportedFormat,
                None,
                Some(*version),
            )
        }
        DurabilityError::Encode(_) => RecoveryDiagnostic::new(
            operation,
            RecoveryAuthority::DurableBytes,
            RecoveryReason::Encoding,
            None,
            None,
        ),
        DurabilityError::Corruption { offset, .. } => RecoveryDiagnostic::new(
            operation,
            RecoveryAuthority::DurableBytes,
            RecoveryReason::Corruption,
            Some(*offset),
            None,
        ),
        DurabilityError::Protocol { offset, .. } => RecoveryDiagnostic::new(
            operation,
            RecoveryAuthority::DurableBytes,
            RecoveryReason::ProtocolViolation,
            Some(*offset),
            None,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_format_preserves_component_and_version() {
        let error = kernel_durability::DurabilityError::UnsupportedDurableFormat {
            component: kernel_durability::DurableFormatComponent::Manifest,
            version: 7,
        };
        let diagnostic = recovery_from_durability(RecoveryOperation::Open, &error);
        assert_eq!(diagnostic.operation(), RecoveryOperation::Open);
        assert_eq!(diagnostic.authority(), RecoveryAuthority::ManifestFormat);
        assert_eq!(diagnostic.reason(), RecoveryReason::UnsupportedFormat);
        assert_eq!(diagnostic.byte_offset(), None);
        assert_eq!(diagnostic.format_version(), Some(7));
    }

    #[test]
    fn corruption_preserves_exact_byte_offset() {
        let error = kernel_durability::DurabilityError::Corruption {
            offset: 4097,
            reason: "test corruption",
        };
        let diagnostic = recovery_from_durability(RecoveryOperation::VerifyBackup, &error);
        assert_eq!(diagnostic.authority(), RecoveryAuthority::DurableBytes);
        assert_eq!(diagnostic.reason(), RecoveryReason::Corruption);
        assert_eq!(diagnostic.byte_offset(), Some(4097));
    }
}
