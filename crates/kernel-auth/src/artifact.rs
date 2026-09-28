use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};
use kernel_semantics::{
    ArtifactAuthenticationSet, ImplementationArtifactDigest, RuntimeProfileDigest,
};

use crate::core::{AuthError, KeyId, Sha256Digest, TrustRootSet, key_id, sha256, strict_verify};

const ARTIFACT_AUTH_DOMAIN: &[u8] = b"CFMD-SEMANTIC-ARTIFACT-AUTH-v1\0";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedArtifactManifest {
    pub artifact_digest: Sha256Digest,
    pub artifact_len: u64,
    pub runtime_profile: RuntimeProfileDigest,
    pub semantic_descriptor_digest: Sha256Digest,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthenticatedArtifactManifest {
    artifact_digest: Sha256Digest,
    artifact_len: u64,
    runtime_profile: RuntimeProfileDigest,
    semantic_descriptor_digest: Sha256Digest,
    signer: KeyId,
}

impl AuthenticatedArtifactManifest {
    #[must_use]
    pub const fn artifact_digest(self) -> Sha256Digest {
        self.artifact_digest
    }

    #[must_use]
    pub const fn artifact_len(self) -> u64 {
        self.artifact_len
    }

    #[must_use]
    pub const fn runtime_profile(self) -> RuntimeProfileDigest {
        self.runtime_profile
    }

    #[must_use]
    pub const fn semantic_descriptor_digest(self) -> Sha256Digest {
        self.semantic_descriptor_digest
    }

    #[must_use]
    pub const fn signer(self) -> KeyId {
        self.signer
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedSemanticArtifact {
    pub artifact_digest: ImplementationArtifactDigest,
    pub runtime_profile: RuntimeProfileDigest,
    pub semantic_descriptor_digest: Sha256Digest,
    pub signer: KeyId,
}

impl VerifiedSemanticArtifact {
    pub fn mark_authenticated(self, authentications: &mut ArtifactAuthenticationSet) {
        authentications.mark_verified(self.artifact_digest);
    }
}

#[must_use]
pub fn sign_artifact_manifest(
    signing_key: &SigningKey,
    artifact: &[u8],
    runtime_profile: RuntimeProfileDigest,
    semantic_descriptor: &[u8],
) -> SignedArtifactManifest {
    let artifact_digest = sha256(artifact);
    let semantic_descriptor_digest = sha256(semantic_descriptor);
    let artifact_len = artifact.len() as u64;
    let message = artifact_message(
        artifact_digest,
        artifact_len,
        runtime_profile,
        semantic_descriptor_digest,
    );
    SignedArtifactManifest {
        artifact_digest,
        artifact_len,
        runtime_profile,
        semantic_descriptor_digest,
        signer: key_id(signing_key.verifying_key().as_bytes()),
        signature: signing_key.sign(&message).to_bytes(),
    }
}

pub fn authenticate_artifact_manifest(
    trust: &TrustRootSet,
    manifest: &SignedArtifactManifest,
    semantic_descriptor: &[u8],
) -> Result<AuthenticatedArtifactManifest, AuthError> {
    let signer = trust.verifying_key(manifest.signer)?;
    let message = artifact_message(
        manifest.artifact_digest,
        manifest.artifact_len,
        manifest.runtime_profile,
        manifest.semantic_descriptor_digest,
    );
    strict_verify(&signer, &message, &manifest.signature)?;
    if sha256(semantic_descriptor) != manifest.semantic_descriptor_digest {
        return Err(AuthError::SemanticDescriptorDigestMismatch);
    }
    Ok(AuthenticatedArtifactManifest {
        artifact_digest: manifest.artifact_digest,
        artifact_len: manifest.artifact_len,
        runtime_profile: manifest.runtime_profile,
        semantic_descriptor_digest: manifest.semantic_descriptor_digest,
        signer: manifest.signer,
    })
}

pub fn verify_artifact_manifest(
    trust: &TrustRootSet,
    manifest: &SignedArtifactManifest,
    artifact: &[u8],
    semantic_descriptor: &[u8],
) -> Result<VerifiedSemanticArtifact, AuthError> {
    let authenticated = authenticate_artifact_manifest(trust, manifest, semantic_descriptor)?;
    verify_authenticated_artifact(authenticated, artifact)
}

fn verify_authenticated_artifact(
    authenticated: AuthenticatedArtifactManifest,
    artifact: &[u8],
) -> Result<VerifiedSemanticArtifact, AuthError> {
    if artifact.len() as u64 != authenticated.artifact_len {
        return Err(AuthError::ArtifactLengthMismatch);
    }
    if sha256(artifact) != authenticated.artifact_digest {
        return Err(AuthError::ArtifactDigestMismatch);
    }
    Ok(VerifiedSemanticArtifact {
        artifact_digest: ImplementationArtifactDigest(authenticated.artifact_digest.0),
        runtime_profile: authenticated.runtime_profile,
        semantic_descriptor_digest: authenticated.semantic_descriptor_digest,
        signer: authenticated.signer,
    })
}

fn artifact_message(
    artifact_digest: Sha256Digest,
    artifact_len: u64,
    runtime_profile: RuntimeProfileDigest,
    semantic_descriptor_digest: Sha256Digest,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(ARTIFACT_AUTH_DOMAIN.len() + 104);
    out.extend_from_slice(ARTIFACT_AUTH_DOMAIN);
    out.extend_from_slice(&artifact_len.to_le_bytes());
    out.extend_from_slice(&artifact_digest.0);
    out.extend_from_slice(&runtime_profile.0);
    out.extend_from_slice(&semantic_descriptor_digest.0);
    out
}

pub trait ArtifactCas {
    /// Load exactly the authenticated object length. Implementations must not
    /// materialize more than `expected_len + 1` bytes while enforcing the bound.
    fn load_exact(
        &self,
        digest: Sha256Digest,
        expected_len: u64,
    ) -> Result<Option<Vec<u8>>, AuthError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryArtifactCas {
    objects: BTreeMap<Sha256Digest, Vec<u8>>,
}

impl MemoryArtifactCas {
    pub fn insert(&mut self, bytes: Vec<u8>) -> Sha256Digest {
        let digest = sha256(&bytes);
        self.objects.insert(digest, bytes);
        digest
    }

    pub fn insert_unchecked_for_test(&mut self, digest: Sha256Digest, bytes: Vec<u8>) {
        self.objects.insert(digest, bytes);
    }
}

impl ArtifactCas for MemoryArtifactCas {
    fn load_exact(
        &self,
        digest: Sha256Digest,
        expected_len: u64,
    ) -> Result<Option<Vec<u8>>, AuthError> {
        let Some(bytes) = self.objects.get(&digest) else {
            return Ok(None);
        };
        if bytes.len() as u64 != expected_len {
            return Err(AuthError::CasCorruption);
        }
        Ok(Some(bytes.clone()))
    }
}

pub fn verify_artifact_from_cas(
    trust: &TrustRootSet,
    cas: &impl ArtifactCas,
    manifest: &SignedArtifactManifest,
    semantic_descriptor: &[u8],
) -> Result<(VerifiedSemanticArtifact, Vec<u8>), AuthError> {
    let authenticated = authenticate_artifact_manifest(trust, manifest, semantic_descriptor)?;
    let artifact = cas
        .load_exact(authenticated.artifact_digest, authenticated.artifact_len)?
        .ok_or(AuthError::CasMiss)?;
    let verified = verify_authenticated_artifact(authenticated, &artifact).map_err(|error| {
        if matches!(
            error,
            AuthError::ArtifactLengthMismatch | AuthError::ArtifactDigestMismatch
        ) {
            AuthError::CasCorruption
        } else {
            error
        }
    })?;
    Ok((verified, artifact))
}
