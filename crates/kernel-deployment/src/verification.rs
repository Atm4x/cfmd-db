use kernel_auth::{
    ArtifactCas, TrustRootSet, VerifiedSemanticArtifact, sha256, verify_artifact_from_cas,
};
use kernel_semantics::{ImplementationArtifactDigest, SemanticContractIdentity};

use crate::{
    AbiRequest, AbiResponse, CheckedRefinement, DeploymentError, DeploymentPolicy,
    MAX_PACKAGE_BYTES, MAX_REFINEMENT_PROOF_BYTES, MAX_SEMANTIC_DESCRIPTOR_BYTES, PackageLookup,
    PackageRepository, RefinementCheckerRegistry, RuntimeProfileSpec, SandboxedRuntime,
    SemanticPackageEnvelope,
};
pub struct ExternalDeploymentAuthority<'a, R, C, K> {
    trust: &'a TrustRootSet,
    policy: &'a DeploymentPolicy,
    repository: &'a R,
    cas: &'a C,
    checkers: &'a K,
}

impl<'a, R, C, K> ExternalDeploymentAuthority<'a, R, C, K>
where
    R: PackageRepository,
    C: ArtifactCas,
    K: RefinementCheckerRegistry,
{
    #[must_use]
    pub const fn new(
        trust: &'a TrustRootSet,
        policy: &'a DeploymentPolicy,
        repository: &'a R,
        cas: &'a C,
        checkers: &'a K,
    ) -> Self {
        Self {
            trust,
            policy,
            repository,
            cas,
            checkers,
        }
    }

    pub fn verify(
        &self,
        lookup: PackageLookup<'_>,
    ) -> Result<VerifiedExternalPackage, DeploymentError> {
        verify_external_package_from_repository(
            self.trust,
            self.policy,
            self.repository,
            self.cas,
            lookup,
            self.checkers,
        )
    }

    pub fn verify_and_invoke(
        &self,
        lookup: PackageLookup<'_>,
        runtime: &mut impl SandboxedRuntime,
        request: &AbiRequest,
    ) -> Result<AbiResponse, DeploymentError> {
        self.verify(lookup)?.invoke(runtime, request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExternalPackage {
    pub artifact: Vec<u8>,
    pub verified_artifact: VerifiedSemanticArtifact,
    pub runtime_profile: RuntimeProfileSpec,
    pub semantic_descriptor: Vec<u8>,
    pub checked_refinement: Option<CheckedRefinement>,
}

impl VerifiedExternalPackage {
    pub fn invoke(
        &self,
        runtime: &mut impl SandboxedRuntime,
        request: &AbiRequest,
    ) -> Result<AbiResponse, DeploymentError> {
        let actual = runtime.profile();
        actual.validate()?;
        if actual.digest() != self.verified_artifact.runtime_profile
            || actual != self.runtime_profile
        {
            return Err(DeploymentError::RuntimeProfileMismatch);
        }
        let encoded_request = request.encode(actual)?;
        let encoded_response = runtime.invoke(&self.artifact, &encoded_request)?;
        AbiResponse::decode(&encoded_response, actual)
    }
}

pub fn verify_external_package(
    trust: &TrustRootSet,
    policy: &DeploymentPolicy,
    cas: &impl ArtifactCas,
    package: SemanticPackageEnvelope,
    contract: SemanticContractIdentity,
    expected_semantic_descriptor: &[u8],
    checkers: &impl RefinementCheckerRegistry,
) -> Result<VerifiedExternalPackage, DeploymentError> {
    package.runtime_profile.validate()?;
    let runtime_digest = package.runtime_profile.digest();
    if runtime_digest != package.manifest.runtime_profile
        || !policy.allowed_runtime_profiles.contains(&runtime_digest)
    {
        return Err(DeploymentError::RuntimeProfileForbidden);
    }
    if package.semantic_descriptor.len() > MAX_SEMANTIC_DESCRIPTOR_BYTES {
        return Err(DeploymentError::SemanticDescriptorTooLarge);
    }
    if package.semantic_descriptor != expected_semantic_descriptor {
        return Err(DeploymentError::SemanticDescriptorMismatch);
    }
    let (verified_artifact, artifact) =
        verify_artifact_from_cas(trust, cas, &package.manifest, &package.semantic_descriptor)
            .map_err(DeploymentError::Authentication)?;
    if policy
        .revoked_artifacts
        .contains(&verified_artifact.artifact_digest)
    {
        return Err(DeploymentError::ArtifactRevoked);
    }

    let checked_refinement = match contract {
        SemanticContractIdentity::Defined(_) => {
            let proof = package
                .refinement
                .as_ref()
                .ok_or(DeploymentError::MissingRefinementProof)?;
            if proof.bytes.len() > MAX_REFINEMENT_PROOF_BYTES {
                return Err(DeploymentError::RefinementProofTooLarge);
            }
            if !policy.allowed_refinement_checkers.contains(&proof.checker) {
                return Err(DeploymentError::RefinementCheckerForbidden);
            }
            let checker = checkers
                .checker(proof.checker)
                .ok_or(DeploymentError::RefinementCheckerUnavailable)?;
            checker.check(
                contract,
                verified_artifact.artifact_digest,
                runtime_digest,
                &package.semantic_descriptor,
                proof.format,
                &proof.bytes,
            )?;
            Some(CheckedRefinement {
                checker: proof.checker,
                proof_digest: sha256(&proof.bytes),
            })
        }
        SemanticContractIdentity::OpaqueArtifact { artifact, runtime } => {
            if artifact != verified_artifact.artifact_digest || runtime != runtime_digest {
                return Err(DeploymentError::OpaqueIdentityMismatch);
            }
            None
        }
    };

    Ok(VerifiedExternalPackage {
        artifact,
        verified_artifact,
        runtime_profile: package.runtime_profile,
        semantic_descriptor: package.semantic_descriptor,
        checked_refinement,
    })
}

pub fn verify_external_package_from_repository(
    trust: &TrustRootSet,
    policy: &DeploymentPolicy,
    repository: &impl PackageRepository,
    cas: &impl ArtifactCas,
    lookup: PackageLookup<'_>,
    checkers: &impl RefinementCheckerRegistry,
) -> Result<VerifiedExternalPackage, DeploymentError> {
    let bytes = repository
        .load_package_bounded(lookup.artifact, MAX_PACKAGE_BYTES)?
        .ok_or(DeploymentError::PackageUnavailable)?;
    let package = SemanticPackageEnvelope::decode(&bytes)?;
    if ImplementationArtifactDigest(package.manifest.artifact_digest.0) != lookup.artifact {
        return Err(DeploymentError::PackageArtifactMismatch);
    }
    verify_external_package(
        trust,
        policy,
        cas,
        package,
        lookup.contract,
        lookup.expected_semantic_descriptor,
        checkers,
    )
}
