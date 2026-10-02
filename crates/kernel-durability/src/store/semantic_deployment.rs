use kernel_semantics::{
    ArtifactAuthenticationSet, ImplementationArtifactDigest, SemanticContractIdentity,
    SemanticDeploymentRegistry, SemanticExecutionPolicy, SemanticRegistry,
};

use crate::domain::DurableTransactionIntent;
use crate::runtime::DurabilityError;

pub(super) fn install_semantic_module_packages(
    registry: &mut SemanticRegistry,
    semantic_modules: &[kernel_semantics::BuiltinSemanticModuleSpec],
) -> Result<(), DurabilityError> {
    let deployment = SemanticDeploymentRegistry::from_builtin_specs(semantic_modules);
    let policy = SemanticExecutionPolicy::trusted_builtin_only();
    let authentications = ArtifactAuthenticationSet::trusted_builtins(semantic_modules);
    for spec in semantic_modules {
        let artifact = ImplementationArtifactDigest(spec.digest().0);
        let authorization = deployment
            .authorize_artifact(artifact, &policy, &authentications)
            .map_err(|_| DurabilityError::Protocol {
                offset: 0,
                reason: "semantic implementation package is not execution-authorized",
            })?;
        if authorization.contract() != SemanticContractIdentity::Defined(spec.contract()) {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "semantic implementation package contract changed during authorization",
            });
        }
        let installed =
            SemanticDeploymentRegistry::install_authorized_builtin(&authorization, registry)
                .map_err(|_| DurabilityError::Protocol {
                    offset: 0,
                    reason: "authorized semantic package has no builtin executable artifact",
                })?;
        if installed != spec.digest() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "authorized semantic package installed a different artifact digest",
            });
        }
    }
    Ok(())
}

pub(super) fn install_intent_semantic_modules(
    registry: &mut SemanticRegistry,
    intent: &DurableTransactionIntent,
) -> Result<(), DurabilityError> {
    let semantic_modules = match intent {
        DurableTransactionIntent::RelationDataExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::RelationDataResidualExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::RelationRewriteExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::RelationResolutionExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::MixedRevisionExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::MixedRevisionResidualExact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::Exact {
            semantic_modules, ..
        }
        | DurableTransactionIntent::SchemaMigrationExact {
            semantic_modules, ..
        } => semantic_modules,
        DurableTransactionIntent::LegacyTargetOnly { .. } => return Ok(()),
    };
    install_semantic_module_packages(registry, semantic_modules)
}
