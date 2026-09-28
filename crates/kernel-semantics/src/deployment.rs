use std::collections::{BTreeMap, BTreeSet};

use kernel_schema::ModuleDigest;

use crate::contracts::{SemanticContract, SemanticImplementationArtifact, certify_implementation};
use crate::implementation_descriptor::BuiltinSemanticModuleSpec;
use crate::registry::SemanticRegistry;

/// Stable identity of one executable implementation artifact. This is
/// deliberately distinct from semantic contract identity: an authenticated
/// artifact can still implement the wrong contract. Current builtins derive
/// this identity from their durable implementation descriptor; external
/// package byte hashing belongs to the deployment layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImplementationArtifactDigest(pub [u8; 32]);

/// Identity of the execution ABI/sandbox/runtime assumptions under which a
/// semantic implementation is certified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RuntimeProfileDigest(pub [u8; 32]);

pub const BUILTIN_RUNTIME_PROFILE: RuntimeProfileDigest = RuntimeProfileDigest([0xCF; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticContractIdentity {
    /// A complete semantic specification. A different executable artifact may
    /// run it only after checked refinement to this same contract.
    Defined(SemanticContract),
    /// No independent complete specification exists; exact artifact bytes and
    /// runtime therefore participate in semantic identity.
    OpaqueArtifact {
        artifact: ImplementationArtifactDigest,
        runtime: RuntimeProfileDigest,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticRefinementCertificate {
    Builtin(SemanticImplementationArtifact),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticExecutableArtifact {
    Builtin(BuiltinSemanticModuleSpec),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticImplementationPackage {
    pub contract: SemanticContractIdentity,
    pub artifact_digest: ImplementationArtifactDigest,
    pub runtime_profile: RuntimeProfileDigest,
    pub refinement: Option<SemanticRefinementCertificate>,
    pub executable: Option<SemanticExecutableArtifact>,
}

impl SemanticImplementationPackage {
    #[must_use]
    pub fn builtin(spec: BuiltinSemanticModuleSpec) -> Self {
        Self {
            contract: SemanticContractIdentity::Defined(spec.contract()),
            artifact_digest: ImplementationArtifactDigest(spec.digest().0),
            runtime_profile: BUILTIN_RUNTIME_PROFILE,
            refinement: Some(SemanticRefinementCertificate::Builtin(spec.artifact())),
            executable: Some(SemanticExecutableArtifact::Builtin(spec)),
        }
    }
}

/// Evidence that an external deployment/security layer authenticated exactly
/// these artifact digests. The semantic kernel consumes this evidence but does
/// not decide trust roots, signature suites or key rotation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtifactAuthenticationSet {
    verified: BTreeSet<ImplementationArtifactDigest>,
}

impl ArtifactAuthenticationSet {
    pub fn mark_verified(&mut self, artifact: ImplementationArtifactDigest) {
        self.verified.insert(artifact);
    }

    #[must_use]
    pub fn contains(&self, artifact: ImplementationArtifactDigest) -> bool {
        self.verified.contains(&artifact)
    }

    #[must_use]
    pub fn trusted_builtins(specs: &[BuiltinSemanticModuleSpec]) -> Self {
        Self {
            verified: specs
                .iter()
                .map(|spec| ImplementationArtifactDigest(spec.digest().0))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticExecutionPolicy {
    pub allowed_runtime_profiles: BTreeSet<RuntimeProfileDigest>,
    pub revoked_artifacts: BTreeSet<ImplementationArtifactDigest>,
    pub require_authentication: bool,
}

impl SemanticExecutionPolicy {
    #[must_use]
    pub fn trusted_builtin_only() -> Self {
        Self {
            allowed_runtime_profiles: BTreeSet::from([BUILTIN_RUNTIME_PROFILE]),
            revoked_artifacts: BTreeSet::new(),
            require_authentication: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticDeploymentError {
    ArtifactDoesNotMatchOpaqueContract,
    RuntimeDoesNotMatchOpaqueContract,
    MissingRefinementCertificate,
    RefinementContractMismatch,
    UnauthenticatedArtifact,
    RuntimeProfileForbidden,
    RevokedArtifact,
    ContractUnavailable,
    ExecutableUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionAuthorization {
    package: SemanticImplementationPackage,
}

impl ExecutionAuthorization {
    #[must_use]
    pub const fn contract(&self) -> SemanticContractIdentity {
        self.package.contract
    }

    #[must_use]
    pub const fn artifact_digest(&self) -> ImplementationArtifactDigest {
        self.package.artifact_digest
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticExecutionCapability {
    pub authorized: Vec<ExecutionAuthorization>,
    pub unavailable: Vec<(SemanticContractIdentity, SemanticDeploymentError)>,
}

impl SemanticExecutionCapability {
    #[must_use]
    pub fn executable(&self) -> bool {
        self.unavailable.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SemanticDeploymentRegistry {
    packages: BTreeMap<ImplementationArtifactDigest, SemanticImplementationPackage>,
}

impl SemanticDeploymentRegistry {
    pub fn register(&mut self, package: SemanticImplementationPackage) {
        self.packages.insert(package.artifact_digest, package);
    }

    #[must_use]
    pub fn from_builtin_specs(specs: &[BuiltinSemanticModuleSpec]) -> Self {
        let mut registry = Self::default();
        for &spec in specs {
            registry.register(SemanticImplementationPackage::builtin(spec));
        }
        registry
    }

    pub fn authorize_contract(
        &self,
        contract: SemanticContractIdentity,
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> Result<ExecutionAuthorization, SemanticDeploymentError> {
        let mut matched = false;
        let mut first_error = None;
        for package in self
            .packages
            .values()
            .filter(|package| package.contract == contract)
        {
            matched = true;
            match authorize_package(package, policy, authentications) {
                Ok(authorization) => return Ok(authorization),
                Err(error) => first_error.get_or_insert(error),
            };
        }
        if matched {
            Err(first_error.unwrap_or(SemanticDeploymentError::ContractUnavailable))
        } else {
            Err(SemanticDeploymentError::ContractUnavailable)
        }
    }

    pub fn authorize_artifact(
        &self,
        artifact: ImplementationArtifactDigest,
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> Result<ExecutionAuthorization, SemanticDeploymentError> {
        let package = self
            .packages
            .get(&artifact)
            .ok_or(SemanticDeploymentError::ContractUnavailable)?;
        authorize_package(package, policy, authentications)
    }

    pub fn authorize_required(
        &self,
        required: &[SemanticContractIdentity],
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> Result<Vec<ExecutionAuthorization>, SemanticDeploymentError> {
        let capability = self.execution_capability(required, policy, authentications);
        if let Some((_, error)) = capability.unavailable.first() {
            return Err(*error);
        }
        Ok(capability.authorized)
    }

    #[must_use]
    pub fn execution_capability(
        &self,
        required: &[SemanticContractIdentity],
        policy: &SemanticExecutionPolicy,
        authentications: &ArtifactAuthenticationSet,
    ) -> SemanticExecutionCapability {
        let mut authorized = Vec::new();
        let mut unavailable = Vec::new();
        for &contract in required {
            match self.authorize_contract(contract, policy, authentications) {
                Ok(authorization) => authorized.push(authorization),
                Err(error) => unavailable.push((contract, error)),
            }
        }
        SemanticExecutionCapability {
            authorized,
            unavailable,
        }
    }

    pub fn install_authorized_builtin(
        authorization: &ExecutionAuthorization,
        registry: &mut SemanticRegistry,
    ) -> Result<ModuleDigest, SemanticDeploymentError> {
        match authorization.package.executable {
            Some(SemanticExecutableArtifact::Builtin(spec)) => {
                Ok(registry.install_builtin_module_spec(spec))
            }
            None => Err(SemanticDeploymentError::ExecutableUnavailable),
        }
    }
}

fn authorize_package(
    package: &SemanticImplementationPackage,
    policy: &SemanticExecutionPolicy,
    authentications: &ArtifactAuthenticationSet,
) -> Result<ExecutionAuthorization, SemanticDeploymentError> {
    match package.contract {
        SemanticContractIdentity::Defined(contract) => {
            let Some(SemanticRefinementCertificate::Builtin(artifact)) = package.refinement else {
                return Err(SemanticDeploymentError::MissingRefinementCertificate);
            };
            certify_implementation(&contract, artifact)
                .map_err(|_| SemanticDeploymentError::RefinementContractMismatch)?;
            if let Some(SemanticExecutableArtifact::Builtin(spec)) = package.executable
                && (spec.artifact() != artifact
                    || ImplementationArtifactDigest(spec.digest().0) != package.artifact_digest
                    || package.runtime_profile != BUILTIN_RUNTIME_PROFILE)
            {
                return Err(SemanticDeploymentError::RefinementContractMismatch);
            }
        }
        SemanticContractIdentity::OpaqueArtifact { artifact, runtime } => {
            if package.artifact_digest != artifact {
                return Err(SemanticDeploymentError::ArtifactDoesNotMatchOpaqueContract);
            }
            if package.runtime_profile != runtime {
                return Err(SemanticDeploymentError::RuntimeDoesNotMatchOpaqueContract);
            }
        }
    }
    if policy.require_authentication && !authentications.contains(package.artifact_digest) {
        return Err(SemanticDeploymentError::UnauthenticatedArtifact);
    }
    if !policy
        .allowed_runtime_profiles
        .contains(&package.runtime_profile)
    {
        return Err(SemanticDeploymentError::RuntimeProfileForbidden);
    }
    if policy.revoked_artifacts.contains(&package.artifact_digest) {
        return Err(SemanticDeploymentError::RevokedArtifact);
    }
    Ok(ExecutionAuthorization {
        package: package.clone(),
    })
}
