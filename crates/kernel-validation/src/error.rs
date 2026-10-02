use kernel_identity::DenseIdentityError;
use kernel_schema::TypeVar;
use kernel_semantics::SemanticError;
use kernel_types::{EntityId, SemanticId};
use kernel_violation::ViolationMeasureError;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnknownField(SemanticId),
    UnknownRelation(SemanticId),
    FieldOwnerMismatch {
        field: SemanticId,
        entity: EntityId,
    },
    FieldRuleTypeMismatch {
        field: SemanticId,
    },
    FieldRuleViolation {
        field: SemanticId,
        entity: EntityId,
        rule_index: usize,
    },
    RelationColumnRuleViolation {
        relation: SemanticId,
        row: usize,
        column: usize,
        rule_index: usize,
    },
    EntityRuleViolation {
        owner: SemanticId,
        entity: EntityId,
        rule_index: usize,
    },
    CapabilityRequiredFieldUndefined {
        capability: SemanticId,
        field: SemanticId,
    },
    CapabilityRequiredFieldContractMismatch {
        capability: SemanticId,
        field: SemanticId,
    },
    MissingCapabilityRequiredField {
        capability: SemanticId,
        field: SemanticId,
        entity: EntityId,
    },
    RelationArityMismatch {
        relation: SemanticId,
        expected: usize,
        actual: usize,
    },
    TypeMismatch,
    ProductShapeMismatch,
    UnknownVariant(SemanticId),
    EquivalenceMismatch {
        expected: SemanticId,
        actual: SemanticId,
    },
    UnboundRecursiveVariable(TypeVar),
    Semantic(SemanticError),
    DenseIdentity(DenseIdentityError),
    ViolationMeasure(ViolationMeasureError),
}

impl From<SemanticError> for ValidationError {
    fn from(value: SemanticError) -> Self {
        Self::Semantic(value)
    }
}

impl From<DenseIdentityError> for ValidationError {
    fn from(value: DenseIdentityError) -> Self {
        Self::DenseIdentity(value)
    }
}

impl From<ViolationMeasureError> for ValidationError {
    fn from(value: ViolationMeasureError) -> Self {
        Self::ViolationMeasure(value)
    }
}
