use kernel_query::{QueryError, QueryTypeError, RelQueryError};
use kernel_semantics::SemanticError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    SourceSemantics(SemanticError),
    TargetSemantics(SemanticError),
    NotDefinitionallyEquivalent,
    UnsupportedStructuralChange,
    UnknownSourceField(kernel_types::SemanticId),
    UnknownTargetField(kernel_types::SemanticId),
    DuplicateTargetField(kernel_types::SemanticId),
    RewriteTargetsPassthroughField(kernel_types::SemanticId),
    OwnerTypeMismatch(kernel_types::SemanticId),
    TransformType(QueryTypeError),
    TransformExecution(QueryError),
    InvalidTarget(kernel_validation::ValidationError),
    SourceRevisionMismatch,
    InvalidRevision(kernel_revision::RevisionError),
    SemanticEnvironmentChangeRequiresTransport,
    SemanticContractChanged(kernel_types::SemanticId),
    NoSemanticLawChange,
    UnknownTargetRelation(kernel_types::SemanticId),
    DuplicateTargetRelation(kernel_types::SemanticId),
    RewriteTargetsPassthroughRelation(kernel_types::SemanticId),
    RelationTransformType(RelQueryError),
    RelationTransformExecution(RelQueryError),
    NotConservativeSemanticExtension,
    IdentitySourceCoverageMismatch,
}

mod core;
mod semantic;
mod typed;

pub use core::*;
pub use semantic::*;
pub use typed::*;

#[cfg(test)]
mod tests;
