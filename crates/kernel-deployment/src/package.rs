use kernel_auth::{AuthError, KeyId, Sha256Digest, SignedArtifactManifest};
use kernel_semantics::RuntimeProfileDigest;

use crate::{
    IsolationClass, MAX_PACKAGE_BYTES, MAX_REFINEMENT_PROOF_BYTES, MAX_SEMANTIC_DESCRIPTOR_BYTES,
    RefinementCheckerId, RefinementFormatId, RefinementProofEnvelope, RuntimeProfileSpec,
    SemanticPackageEnvelope,
};

const PACKAGE_MAGIC: &[u8; 8] = b"CFMDSPK1";
const PACKAGE_VERSION: u16 = 1;

use crate::cursor::Cursor;

impl SemanticPackageEnvelope {
    pub fn encode(&self) -> Result<Vec<u8>, DeploymentError> {
        self.runtime_profile.validate()?;
        if self.semantic_descriptor.len() > MAX_SEMANTIC_DESCRIPTOR_BYTES {
            return Err(DeploymentError::SemanticDescriptorTooLarge);
        }
        if self
            .refinement
            .as_ref()
            .is_some_and(|proof| proof.bytes.len() > MAX_REFINEMENT_PROOF_BYTES)
        {
            return Err(DeploymentError::RefinementProofTooLarge);
        }
        let descriptor_len = u32::try_from(self.semantic_descriptor.len())
            .map_err(|_| DeploymentError::PackageTooLarge)?;
        let proof_len = self.refinement.as_ref().map_or(Ok(0), |proof| {
            u32::try_from(proof.bytes.len()).map_err(|_| DeploymentError::PackageTooLarge)
        })?;
        let mut out = Vec::new();
        out.extend_from_slice(PACKAGE_MAGIC);
        out.extend_from_slice(&PACKAGE_VERSION.to_le_bytes());
        encode_runtime_profile(&mut out, self.runtime_profile);
        encode_manifest(&mut out, &self.manifest);
        out.extend_from_slice(&descriptor_len.to_le_bytes());
        out.extend_from_slice(&self.semantic_descriptor);
        match &self.refinement {
            Some(proof) => {
                out.push(1);
                out.extend_from_slice(&proof.checker.0);
                out.extend_from_slice(&proof.format.0);
                out.extend_from_slice(&proof_len.to_le_bytes());
                out.extend_from_slice(&proof.bytes);
            }
            None => out.push(0),
        }
        if out.len() > MAX_PACKAGE_BYTES {
            return Err(DeploymentError::PackageTooLarge);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, DeploymentError> {
        if bytes.len() > MAX_PACKAGE_BYTES {
            return Err(DeploymentError::PackageTooLarge);
        }
        let mut cursor = Cursor::new(bytes);
        if cursor.bytes(PACKAGE_MAGIC.len())? != PACKAGE_MAGIC {
            return Err(DeploymentError::MalformedPackage);
        }
        if cursor.u16()? != PACKAGE_VERSION {
            return Err(DeploymentError::PackageVersionMismatch);
        }
        let runtime_profile = decode_runtime_profile(&mut cursor)?;
        runtime_profile.validate()?;
        let manifest = decode_manifest(&mut cursor)?;
        let descriptor_len = cursor.u32_as_usize()?;
        if descriptor_len > MAX_SEMANTIC_DESCRIPTOR_BYTES {
            return Err(DeploymentError::SemanticDescriptorTooLarge);
        }
        let semantic_descriptor = cursor.bytes(descriptor_len)?.to_vec();
        let refinement = match cursor.u8()? {
            0 => None,
            1 => {
                let checker = RefinementCheckerId(cursor.array_32()?);
                let format = RefinementFormatId(cursor.array_32()?);
                let proof_len = cursor.u32_as_usize()?;
                if proof_len > MAX_REFINEMENT_PROOF_BYTES {
                    return Err(DeploymentError::RefinementProofTooLarge);
                }
                Some(RefinementProofEnvelope {
                    checker,
                    format,
                    bytes: cursor.bytes(proof_len)?.to_vec(),
                })
            }
            _ => return Err(DeploymentError::MalformedPackage),
        };
        cursor.finish()?;
        Ok(Self {
            manifest,
            runtime_profile,
            semantic_descriptor,
            refinement,
        })
    }
}

fn encode_runtime_profile(out: &mut Vec<u8>, profile: RuntimeProfileSpec) {
    out.push(profile.isolation.tag());
    out.extend_from_slice(&profile.abi_version.to_le_bytes());
    out.extend_from_slice(&profile.max_request_bytes.to_le_bytes());
    out.extend_from_slice(&profile.max_response_bytes.to_le_bytes());
    out.extend_from_slice(&profile.max_fuel.to_le_bytes());
}

fn decode_runtime_profile(cursor: &mut Cursor<'_>) -> Result<RuntimeProfileSpec, DeploymentError> {
    Ok(RuntimeProfileSpec {
        isolation: IsolationClass::from_tag(cursor.u8()?)?,
        abi_version: cursor.u16()?,
        max_request_bytes: cursor.u32()?,
        max_response_bytes: cursor.u32()?,
        max_fuel: cursor.u64()?,
    })
}

fn encode_manifest(out: &mut Vec<u8>, manifest: &SignedArtifactManifest) {
    out.extend_from_slice(&manifest.artifact_digest.0);
    out.extend_from_slice(&manifest.artifact_len.to_le_bytes());
    out.extend_from_slice(&manifest.runtime_profile.0);
    out.extend_from_slice(&manifest.semantic_descriptor_digest.0);
    out.extend_from_slice(&manifest.signer.0);
    out.extend_from_slice(&manifest.signature);
}

fn decode_manifest(cursor: &mut Cursor<'_>) -> Result<SignedArtifactManifest, DeploymentError> {
    Ok(SignedArtifactManifest {
        artifact_digest: Sha256Digest(cursor.array_32()?),
        artifact_len: cursor.u64()?,
        runtime_profile: RuntimeProfileDigest(cursor.array_32()?),
        semantic_descriptor_digest: Sha256Digest(cursor.array_32()?),
        signer: KeyId(cursor.array_32()?),
        signature: cursor.array_64()?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeploymentError {
    Authentication(AuthError),
    InvalidRuntimeProfile,
    RuntimeProfileForbidden,
    RuntimeProfileMismatch,
    ArtifactRevoked,
    MissingRefinementProof,
    RefinementProofTooLarge,
    RefinementCheckerForbidden,
    RefinementCheckerUnavailable,
    RefinementRejected,
    SemanticDescriptorTooLarge,
    SemanticDescriptorMismatch,
    PackageTooLarge,
    PackageVersionMismatch,
    PackageUnavailable,
    PackageArtifactMismatch,
    MalformedPackage,
    InvalidPolicy,
    InvalidPolicyEpoch,
    NonCanonicalPolicy,
    OpaqueIdentityMismatch,
    AbiVersionMismatch,
    AbiFrameTooLarge,
    MalformedAbiFrame,
    FuelLimitExceeded,
    RuntimeFailure,
}
