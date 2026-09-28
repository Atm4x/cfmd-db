use kernel_schema::ModuleDigest;

use crate::contracts::{SemanticContract, SemanticImplementationArtifact};
use crate::equivalence::EquivalenceModule;
use crate::ordering::OrderingModule;
use crate::tokenizer::TokenizerModule;

/// Durable description of one semantic implementation provided by the CFMD
/// binary itself. The descriptor is intentionally limited to builtin
/// implementation families; arbitrary executable code is never smuggled
/// through durable storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinSemanticModuleSpec {
    Equivalence {
        module: EquivalenceModule,
        implementation_revision: u64,
    },
    Tokenizer {
        module: TokenizerModule,
        implementation_revision: u64,
    },
    Ordering {
        module: OrderingModule,
        implementation_revision: u64,
    },
}

impl BuiltinSemanticModuleSpec {
    #[must_use]
    pub const fn contract(self) -> SemanticContract {
        match self {
            Self::Equivalence { module, .. } => SemanticContract::Equivalence(module),
            Self::Tokenizer { module, .. } => SemanticContract::Tokenizer(module),
            Self::Ordering { module, .. } => SemanticContract::Ordering(module),
        }
    }

    #[must_use]
    pub const fn artifact(self) -> SemanticImplementationArtifact {
        match self {
            Self::Equivalence { module, .. } => {
                SemanticImplementationArtifact::BuiltinEquivalence(module)
            }
            Self::Tokenizer { module, .. } => {
                SemanticImplementationArtifact::BuiltinTokenizer(module)
            }
            Self::Ordering { module, .. } => {
                SemanticImplementationArtifact::BuiltinOrdering(module)
            }
        }
    }

    #[must_use]
    pub fn digest(self) -> ModuleDigest {
        match self {
            Self::Equivalence {
                module,
                implementation_revision,
            } => module.implementation_digest(implementation_revision),
            Self::Tokenizer {
                module,
                implementation_revision,
            } => module.implementation_digest(implementation_revision),
            Self::Ordering {
                module,
                implementation_revision,
            } => module.implementation_digest(implementation_revision),
        }
    }
}
