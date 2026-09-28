use crate::equivalence::EquivalenceModule;
use crate::error::SemanticError;
use crate::ordering::OrderingModule;
use crate::tokenizer::TokenizerModule;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticContract {
    Equivalence(EquivalenceModule),
    Tokenizer(TokenizerModule),
    Ordering(OrderingModule),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticImplementationArtifact {
    BuiltinEquivalence(EquivalenceModule),
    BuiltinTokenizer(TokenizerModule),
    BuiltinOrdering(OrderingModule),
}

impl SemanticImplementationArtifact {
    #[must_use]
    pub const fn contract(&self) -> SemanticContract {
        match self {
            Self::BuiltinEquivalence(module) => SemanticContract::Equivalence(*module),
            Self::BuiltinTokenizer(module) => SemanticContract::Tokenizer(*module),
            Self::BuiltinOrdering(module) => SemanticContract::Ordering(*module),
        }
    }
}

pub struct SemanticImplementationChecker;

impl kernel_proof::CertificateChecker for SemanticImplementationChecker {
    type Spec = SemanticContract;
    type Certificate = SemanticImplementationArtifact;
    type Error = SemanticError;

    fn check(spec: &Self::Spec, certificate: &Self::Certificate) -> Result<(), Self::Error> {
        (certificate.contract() == *spec)
            .then_some(())
            .ok_or(SemanticError::ImplementationContractMismatch)
    }
}

pub fn certify_implementation(
    contract: &SemanticContract,
    artifact: SemanticImplementationArtifact,
) -> Result<kernel_proof::CheckedCertificate<SemanticImplementationChecker>, SemanticError> {
    kernel_proof::verify_certificate::<SemanticImplementationChecker>(contract, artifact)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct OrderingCompatibilitySpec {
    pub ordering: OrderingModule,
    pub equivalence: EquivalenceModule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderingCompatibilityArtifact {
    BuiltinLaw,
}

pub struct OrderingCompatibilityChecker;

impl kernel_proof::CertificateChecker for OrderingCompatibilityChecker {
    type Spec = OrderingCompatibilitySpec;
    type Certificate = OrderingCompatibilityArtifact;
    type Error = SemanticError;

    fn check(spec: &Self::Spec, _: &Self::Certificate) -> Result<(), Self::Error> {
        builtin_ordering_compatibility_holds(*spec)
            .then_some(())
            .ok_or(SemanticError::OrderingCompatibilityViolation)
    }
}

pub fn certify_ordering_compatibility(
    spec: &OrderingCompatibilitySpec,
    artifact: OrderingCompatibilityArtifact,
) -> Result<kernel_proof::CheckedCertificate<OrderingCompatibilityChecker>, SemanticError> {
    kernel_proof::verify_certificate::<OrderingCompatibilityChecker>(spec, artifact)
}

const fn builtin_ordering_compatibility_holds(spec: OrderingCompatibilitySpec) -> bool {
    matches!(
        (spec.ordering, spec.equivalence),
        (OrderingModule::I64Ascending, EquivalenceModule::I64Exact)
            | (OrderingModule::F64Total, EquivalenceModule::F64Bitwise)
            | (
                OrderingModule::TextBinary
                    | OrderingModule::TextAsciiCaseInsensitive
                    | OrderingModule::TextAsciiCaseInsensitiveThenBinary,
                EquivalenceModule::TextExact
            )
            | (
                OrderingModule::TextAsciiCaseInsensitive,
                EquivalenceModule::TextAsciiCaseInsensitive
            )
    )
}

pub(super) const BUILTIN_ORDERING_COMPATIBILITIES: [OrderingCompatibilitySpec; 6] = [
    OrderingCompatibilitySpec {
        ordering: OrderingModule::I64Ascending,
        equivalence: EquivalenceModule::I64Exact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::F64Total,
        equivalence: EquivalenceModule::F64Bitwise,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextBinary,
        equivalence: EquivalenceModule::TextExact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextAsciiCaseInsensitive,
        equivalence: EquivalenceModule::TextExact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextAsciiCaseInsensitiveThenBinary,
        equivalence: EquivalenceModule::TextExact,
    },
    OrderingCompatibilitySpec {
        ordering: OrderingModule::TextAsciiCaseInsensitive,
        equivalence: EquivalenceModule::TextAsciiCaseInsensitive,
    },
];
