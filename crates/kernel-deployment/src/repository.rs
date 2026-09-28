use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use kernel_auth::{ArtifactCas, AuthError, Sha256Digest};
use kernel_semantics::{ImplementationArtifactDigest, SemanticContractIdentity};

use crate::DeploymentError;
/// Distribution is deliberately an untrusted byte source. A filesystem,
/// HTTP/object-store adapter or package mirror only has to return the package
/// envelope selected by artifact identity; cryptographic authentication and
/// policy checks happen after retrieval.
pub trait PackageRepository {
    /// Retrieve a package without materializing more than `max_bytes + 1` bytes.
    fn load_package_bounded(
        &self,
        artifact: ImplementationArtifactDigest,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, DeploymentError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageLookup<'a> {
    pub artifact: ImplementationArtifactDigest,
    pub contract: SemanticContractIdentity,
    pub expected_semantic_descriptor: &'a [u8],
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPackageRepository {
    packages: BTreeMap<ImplementationArtifactDigest, Vec<u8>>,
}

impl MemoryPackageRepository {
    pub fn insert(&mut self, artifact: ImplementationArtifactDigest, package_bytes: Vec<u8>) {
        self.packages.insert(artifact, package_bytes);
    }
}

impl PackageRepository for MemoryPackageRepository {
    fn load_package_bounded(
        &self,
        artifact: ImplementationArtifactDigest,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, DeploymentError> {
        let Some(bytes) = self.packages.get(&artifact) else {
            return Ok(None);
        };
        if bytes.len() > max_bytes {
            return Err(DeploymentError::PackageTooLarge);
        }
        Ok(Some(bytes.clone()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemPackageRepository {
    root: PathBuf,
}

impl FilesystemPackageRepository {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, artifact: ImplementationArtifactDigest) -> PathBuf {
        self.root
            .join(format!("{}.cfmdspk", hex_digest(&artifact.0)))
    }
}

impl PackageRepository for FilesystemPackageRepository {
    fn load_package_bounded(
        &self,
        artifact: ImplementationArtifactDigest,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, DeploymentError> {
        let Ok(file) = File::open(self.path_for(artifact)) else {
            return Ok(None);
        };
        let limit = u64::try_from(max_bytes)
            .map_err(|_| DeploymentError::PackageTooLarge)?
            .saturating_add(1);
        let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
        file.take(limit)
            .read_to_end(&mut bytes)
            .map_err(|_| DeploymentError::PackageUnavailable)?;
        if bytes.len() > max_bytes {
            return Err(DeploymentError::PackageTooLarge);
        }
        Ok(Some(bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilesystemArtifactCas {
    root: PathBuf,
}

impl FilesystemArtifactCas {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path_for(&self, digest: Sha256Digest) -> PathBuf {
        self.root
            .join(format!("{}.artifact", hex_digest(&digest.0)))
    }
}

impl ArtifactCas for FilesystemArtifactCas {
    fn load_exact(
        &self,
        digest: Sha256Digest,
        expected_len: u64,
    ) -> Result<Option<Vec<u8>>, AuthError> {
        let Ok(file) = File::open(self.path_for(digest)) else {
            return Ok(None);
        };
        let limit = expected_len.saturating_add(1);
        let initial_capacity = usize::try_from(expected_len)
            .unwrap_or(usize::MAX)
            .min(64 * 1024);
        let mut bytes = Vec::with_capacity(initial_capacity);
        file.take(limit)
            .read_to_end(&mut bytes)
            .map_err(|_| AuthError::CasCorruption)?;
        if bytes.len() as u64 != expected_len {
            return Err(AuthError::CasCorruption);
        }
        Ok(Some(bytes))
    }
}

pub(crate) fn hex_digest(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}
