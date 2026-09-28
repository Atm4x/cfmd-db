use kernel_schema::ModuleDigest;
use kernel_types::SemanticId;

use crate::equivalence::EquivalenceDomain;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemanticError {
    ModuleUnavailable(ModuleDigest),
    WrongModuleKind(SemanticId),
    TypeMismatch(SemanticId),
    EquivalenceDomainMismatch {
        equivalence: SemanticId,
        expected: EquivalenceDomain,
        actual: EquivalenceDomain,
    },
    DuplicateSetElement {
        equivalence: SemanticId,
    },
    DuplicateMapKey {
        equivalence: SemanticId,
    },
    DuplicateBagElement {
        equivalence: SemanticId,
    },
    DuplicateRelationRow,
    CyclicStructuralEquivalence(SemanticId),
    CyclicStructuralOrdering(SemanticId),
    FreeStructuralRecursion(SemanticId),
    UnguardedStructuralRecursion(SemanticId),
    ImplementationContractMismatch,
    OrderingCompatibilityViolation,
}
