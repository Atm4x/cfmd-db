#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
#[cfg(target_os = "linux")]
use std::io::{Read, Write};
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::{Command, Stdio};
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

use ed25519_dalek::{Signer, SigningKey};
use kernel_auth::{
    ArtifactCas, AuthError, KeyId, Sha256Digest, SignedArtifactManifest, TrustRootSet,
    VerifiedSemanticArtifact, key_id, sha256, verify_artifact_from_cas,
};
use kernel_semantics::{
    ImplementationArtifactDigest, RuntimeProfileDigest, SemanticContractIdentity,
};

const RUNTIME_PROFILE_DOMAIN: &[u8] = b"CFMD-SEMANTIC-RUNTIME-PROFILE-v1\0";
const DEPLOYMENT_POLICY_DOMAIN: &[u8] = b"CFMD-SEMANTIC-DEPLOYMENT-POLICY-v1\0";
const PACKAGE_MAGIC: &[u8; 8] = b"CFMDSPK1";
const PACKAGE_VERSION: u16 = 1;
const ABI_VERSION: u16 = 1;

pub const MAX_SEMANTIC_DESCRIPTOR_BYTES: usize = 256 * 1024;
pub const MAX_REFINEMENT_PROOF_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ABI_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PACKAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationClass {
    DeterministicWasm,
    SandboxedProcess,
}

impl IsolationClass {
    const fn tag(self) -> u8 {
        match self {
            Self::DeterministicWasm => 1,
            Self::SandboxedProcess => 2,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, DeploymentError> {
        match tag {
            1 => Ok(Self::DeterministicWasm),
            2 => Ok(Self::SandboxedProcess),
            _ => Err(DeploymentError::MalformedPackage),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeProfileSpec {
    pub isolation: IsolationClass,
    pub abi_version: u16,
    pub max_request_bytes: u32,
    pub max_response_bytes: u32,
    pub max_fuel: u64,
}

impl RuntimeProfileSpec {
    pub fn new(
        isolation: IsolationClass,
        max_request_bytes: u32,
        max_response_bytes: u32,
        max_fuel: u64,
    ) -> Result<Self, DeploymentError> {
        let profile = Self {
            isolation,
            abi_version: ABI_VERSION,
            max_request_bytes,
            max_response_bytes,
            max_fuel,
        };
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(self) -> Result<(), DeploymentError> {
        if self.abi_version != ABI_VERSION
            || self.max_request_bytes == 0
            || self.max_response_bytes == 0
            || usize::try_from(self.max_request_bytes)
                .map_or(true, |value| value > MAX_ABI_FRAME_BYTES)
            || usize::try_from(self.max_response_bytes)
                .map_or(true, |value| value > MAX_ABI_FRAME_BYTES)
            || self.max_fuel == 0
        {
            return Err(DeploymentError::InvalidRuntimeProfile);
        }
        Ok(())
    }

    #[must_use]
    pub fn digest(self) -> RuntimeProfileDigest {
        let mut bytes = Vec::with_capacity(RUNTIME_PROFILE_DOMAIN.len() + 19);
        bytes.extend_from_slice(RUNTIME_PROFILE_DOMAIN);
        bytes.push(self.isolation.tag());
        bytes.extend_from_slice(&self.abi_version.to_le_bytes());
        bytes.extend_from_slice(&self.max_request_bytes.to_le_bytes());
        bytes.extend_from_slice(&self.max_response_bytes.to_le_bytes());
        bytes.extend_from_slice(&self.max_fuel.to_le_bytes());
        RuntimeProfileDigest(sha256(&bytes).0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RefinementCheckerId(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RefinementFormatId(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefinementProofEnvelope {
    pub checker: RefinementCheckerId,
    pub format: RefinementFormatId,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticPackageEnvelope {
    pub manifest: SignedArtifactManifest,
    pub runtime_profile: RuntimeProfileSpec,
    pub semantic_descriptor: Vec<u8>,
    pub refinement: Option<RefinementProofEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedRefinement {
    pub checker: RefinementCheckerId,
    pub proof_digest: Sha256Digest,
}

pub trait RefinementChecker {
    fn checker_id(&self) -> RefinementCheckerId;

    fn check(
        &self,
        contract: SemanticContractIdentity,
        artifact: ImplementationArtifactDigest,
        runtime: RuntimeProfileDigest,
        semantic_descriptor: &[u8],
        format: RefinementFormatId,
        proof: &[u8],
    ) -> Result<(), DeploymentError>;
}

pub trait RefinementCheckerRegistry {
    fn checker(&self, id: RefinementCheckerId) -> Option<&dyn RefinementChecker>;
}

#[derive(Default)]
pub struct MemoryCheckerRegistry {
    checkers: BTreeMap<RefinementCheckerId, Box<dyn RefinementChecker>>,
}

impl std::fmt::Debug for MemoryCheckerRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryCheckerRegistry")
            .field("checker_count", &self.checkers.len())
            .finish()
    }
}

impl MemoryCheckerRegistry {
    pub fn insert(&mut self, checker: Box<dyn RefinementChecker>) {
        self.checkers.insert(checker.checker_id(), checker);
    }
}

impl RefinementCheckerRegistry for MemoryCheckerRegistry {
    fn checker(&self, id: RefinementCheckerId) -> Option<&dyn RefinementChecker> {
        self.checkers.get(&id).map(Box::as_ref)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentPolicy {
    pub epoch: u64,
    pub allowed_runtime_profiles: BTreeSet<RuntimeProfileDigest>,
    pub revoked_artifacts: BTreeSet<ImplementationArtifactDigest>,
    pub allowed_refinement_checkers: BTreeSet<RefinementCheckerId>,
}

impl DeploymentPolicy {
    pub fn bootstrap(
        epoch: u64,
        allowed_runtime_profiles: BTreeSet<RuntimeProfileDigest>,
        allowed_refinement_checkers: BTreeSet<RefinementCheckerId>,
    ) -> Result<Self, DeploymentError> {
        if epoch == 0 || allowed_runtime_profiles.is_empty() {
            return Err(DeploymentError::InvalidPolicy);
        }
        Ok(Self {
            epoch,
            allowed_runtime_profiles,
            revoked_artifacts: BTreeSet::new(),
            allowed_refinement_checkers,
        })
    }

    pub fn apply_signed_update(
        &self,
        trust: &TrustRootSet,
        update: &SignedDeploymentPolicy,
    ) -> Result<Self, DeploymentError> {
        if update.current_epoch != self.epoch
            || update.next_epoch
                != self
                    .epoch
                    .checked_add(1)
                    .ok_or(DeploymentError::InvalidPolicyEpoch)?
        {
            return Err(DeploymentError::InvalidPolicyEpoch);
        }
        let candidate = update.as_policy()?;
        let message = deployment_policy_message(update)?;
        trust
            .verify_message(update.signer, &message, &update.signature)
            .map_err(DeploymentError::Authentication)?;
        Ok(candidate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedDeploymentPolicy {
    pub current_epoch: u64,
    pub next_epoch: u64,
    pub allowed_runtime_profiles: Vec<RuntimeProfileDigest>,
    pub revoked_artifacts: Vec<ImplementationArtifactDigest>,
    pub allowed_refinement_checkers: Vec<RefinementCheckerId>,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

impl SignedDeploymentPolicy {
    fn as_policy(&self) -> Result<DeploymentPolicy, DeploymentError> {
        if !is_strictly_sorted(&self.allowed_runtime_profiles)
            || !is_strictly_sorted(&self.revoked_artifacts)
            || !is_strictly_sorted(&self.allowed_refinement_checkers)
        {
            return Err(DeploymentError::NonCanonicalPolicy);
        }
        let runtime_profiles: BTreeSet<_> = self.allowed_runtime_profiles.iter().copied().collect();
        let revoked_artifacts: BTreeSet<_> = self.revoked_artifacts.iter().copied().collect();
        let checkers: BTreeSet<_> = self.allowed_refinement_checkers.iter().copied().collect();
        if runtime_profiles.len() != self.allowed_runtime_profiles.len()
            || revoked_artifacts.len() != self.revoked_artifacts.len()
            || checkers.len() != self.allowed_refinement_checkers.len()
            || runtime_profiles.is_empty()
            || self.next_epoch == 0
        {
            return Err(DeploymentError::InvalidPolicy);
        }
        Ok(DeploymentPolicy {
            epoch: self.next_epoch,
            allowed_runtime_profiles: runtime_profiles,
            revoked_artifacts,
            allowed_refinement_checkers: checkers,
        })
    }
}

fn is_strictly_sorted<T: Ord>(values: &[T]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}

pub fn sign_deployment_policy(
    signing_key: &SigningKey,
    current_epoch: u64,
    next_policy: DeploymentPolicy,
) -> Result<SignedDeploymentPolicy, DeploymentError> {
    if next_policy.epoch
        != current_epoch
            .checked_add(1)
            .ok_or(DeploymentError::InvalidPolicyEpoch)?
    {
        return Err(DeploymentError::InvalidPolicyEpoch);
    }
    let mut update = SignedDeploymentPolicy {
        current_epoch,
        next_epoch: next_policy.epoch,
        allowed_runtime_profiles: next_policy.allowed_runtime_profiles.into_iter().collect(),
        revoked_artifacts: next_policy.revoked_artifacts.into_iter().collect(),
        allowed_refinement_checkers: next_policy
            .allowed_refinement_checkers
            .into_iter()
            .collect(),
        signer: key_id(signing_key.verifying_key().as_bytes()),
        signature: [0; 64],
    };
    let message = deployment_policy_message(&update)?;
    update.signature = signing_key.sign(&message).to_bytes();
    Ok(update)
}

fn deployment_policy_message(update: &SignedDeploymentPolicy) -> Result<Vec<u8>, DeploymentError> {
    let runtime_count = u32::try_from(update.allowed_runtime_profiles.len())
        .map_err(|_| DeploymentError::InvalidPolicy)?;
    let revoked_count = u32::try_from(update.revoked_artifacts.len())
        .map_err(|_| DeploymentError::InvalidPolicy)?;
    let checker_count = u32::try_from(update.allowed_refinement_checkers.len())
        .map_err(|_| DeploymentError::InvalidPolicy)?;
    let mut out = Vec::with_capacity(
        DEPLOYMENT_POLICY_DOMAIN.len()
            + 28
            + update.allowed_runtime_profiles.len() * 32
            + update.revoked_artifacts.len() * 32
            + update.allowed_refinement_checkers.len() * 32,
    );
    out.extend_from_slice(DEPLOYMENT_POLICY_DOMAIN);
    out.extend_from_slice(&update.current_epoch.to_le_bytes());
    out.extend_from_slice(&update.next_epoch.to_le_bytes());
    out.extend_from_slice(&runtime_count.to_le_bytes());
    for profile in &update.allowed_runtime_profiles {
        out.extend_from_slice(&profile.0);
    }
    out.extend_from_slice(&revoked_count.to_le_bytes());
    for artifact in &update.revoked_artifacts {
        out.extend_from_slice(&artifact.0);
    }
    out.extend_from_slice(&checker_count.to_le_bytes());
    for checker in &update.allowed_refinement_checkers {
        out.extend_from_slice(&checker.0);
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiOperation {
    Equivalence,
    Ordering,
    Tokenize,
}

impl AbiOperation {
    const fn tag(self) -> u8 {
        match self {
            Self::Equivalence => 1,
            Self::Ordering => 2,
            Self::Tokenize => 3,
        }
    }

    fn from_tag(tag: u8) -> Result<Self, DeploymentError> {
        match tag {
            1 => Ok(Self::Equivalence),
            2 => Ok(Self::Ordering),
            3 => Ok(Self::Tokenize),
            _ => Err(DeploymentError::MalformedAbiFrame),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiRequest {
    pub operation: AbiOperation,
    pub fuel_limit: u64,
    pub payload: Vec<u8>,
}

impl AbiRequest {
    pub fn encode(&self, profile: RuntimeProfileSpec) -> Result<Vec<u8>, DeploymentError> {
        profile.validate()?;
        if self.fuel_limit == 0 || self.fuel_limit > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len =
            u32::try_from(self.payload.len()).map_err(|_| DeploymentError::AbiFrameTooLarge)?;
        let mut out = Vec::with_capacity(15 + self.payload.len());
        out.extend_from_slice(&ABI_VERSION.to_le_bytes());
        out.push(self.operation.tag());
        out.extend_from_slice(&self.fuel_limit.to_le_bytes());
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&self.payload);
        if out.len() > usize::try_from(profile.max_request_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8], profile: RuntimeProfileSpec) -> Result<Self, DeploymentError> {
        profile.validate()?;
        if bytes.len() > usize::try_from(profile.max_request_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        let mut cursor = Cursor::new(bytes);
        let version = cursor.u16()?;
        if version != ABI_VERSION {
            return Err(DeploymentError::AbiVersionMismatch);
        }
        let operation = AbiOperation::from_tag(cursor.u8()?)?;
        let fuel_limit = cursor.u64()?;
        if fuel_limit == 0 || fuel_limit > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len = cursor.u32_as_usize()?;
        let payload = cursor.bytes(payload_len)?.to_vec();
        cursor.finish()?;
        Ok(Self {
            operation,
            fuel_limit,
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbiResponse {
    pub consumed_fuel: u64,
    pub payload: Vec<u8>,
}

impl AbiResponse {
    pub fn encode(&self, profile: RuntimeProfileSpec) -> Result<Vec<u8>, DeploymentError> {
        profile.validate()?;
        if self.consumed_fuel > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len =
            u32::try_from(self.payload.len()).map_err(|_| DeploymentError::AbiFrameTooLarge)?;
        let mut out = Vec::with_capacity(14 + self.payload.len());
        out.extend_from_slice(&ABI_VERSION.to_le_bytes());
        out.extend_from_slice(&self.consumed_fuel.to_le_bytes());
        out.extend_from_slice(&payload_len.to_le_bytes());
        out.extend_from_slice(&self.payload);
        if out.len() > usize::try_from(profile.max_response_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8], profile: RuntimeProfileSpec) -> Result<Self, DeploymentError> {
        profile.validate()?;
        if bytes.len() > usize::try_from(profile.max_response_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        let mut cursor = Cursor::new(bytes);
        if cursor.u16()? != ABI_VERSION {
            return Err(DeploymentError::AbiVersionMismatch);
        }
        let consumed_fuel = cursor.u64()?;
        if consumed_fuel > profile.max_fuel {
            return Err(DeploymentError::FuelLimitExceeded);
        }
        let payload_len = cursor.u32_as_usize()?;
        let payload = cursor.bytes(payload_len)?.to_vec();
        cursor.finish()?;
        Ok(Self {
            consumed_fuel,
            payload,
        })
    }
}

pub trait SandboxedRuntime {
    fn profile(&self) -> RuntimeProfileSpec;

    fn invoke(
        &mut self,
        artifact: &[u8],
        encoded_request: &[u8],
    ) -> Result<Vec<u8>, DeploymentError>;
}

/// Distribution is deliberately an untrusted byte source. A filesystem,
/// HTTP/object-store adapter or package mirror only has to return the package
/// envelope selected by artifact identity; cryptographic authentication and
/// policy checks happen after retrieval.
pub trait PackageRepository {
    fn load_package(&self, artifact: ImplementationArtifactDigest) -> Option<Vec<u8>>;
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
    fn load_package(&self, artifact: ImplementationArtifactDigest) -> Option<Vec<u8>> {
        self.packages.get(&artifact).cloned()
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
    fn load_package(&self, artifact: ImplementationArtifactDigest) -> Option<Vec<u8>> {
        let bytes = fs::read(self.path_for(artifact)).ok()?;
        (bytes.len() <= MAX_PACKAGE_BYTES).then_some(bytes)
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
    fn load(&self, digest: Sha256Digest) -> Option<Vec<u8>> {
        fs::read(self.path_for(digest)).ok()
    }
}

fn hex_digest(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxNamespaceProcessRuntime {
    profile: RuntimeProfileSpec,
    unshare_program: PathBuf,
    timeout_program: PathBuf,
}

#[cfg(target_os = "linux")]
impl LinuxNamespaceProcessRuntime {
    pub fn new(profile: RuntimeProfileSpec) -> Result<Self, DeploymentError> {
        profile.validate()?;
        if profile.isolation != IsolationClass::SandboxedProcess {
            return Err(DeploymentError::InvalidRuntimeProfile);
        }
        Ok(Self {
            profile,
            unshare_program: PathBuf::from("/usr/bin/unshare"),
            timeout_program: PathBuf::from("/usr/bin/timeout"),
        })
    }

    #[must_use]
    pub fn with_programs(
        mut self,
        unshare: impl Into<PathBuf>,
        timeout: impl Into<PathBuf>,
    ) -> Self {
        self.unshare_program = unshare.into();
        self.timeout_program = timeout.into();
        self
    }

    fn invoke_file(
        &self,
        executable: &Path,
        encoded_request: &[u8],
    ) -> Result<Vec<u8>, DeploymentError> {
        let timeout_ms = self.profile.max_fuel.max(1);
        let mut child = Command::new(&self.timeout_program)
            .arg("--signal=KILL")
            .arg(format!("{}.{:03}s", timeout_ms / 1_000, timeout_ms % 1_000))
            .arg(&self.unshare_program)
            .args([
                "--user",
                "--map-root-user",
                "--mount",
                "--pid",
                "--fork",
                "--kill-child",
                "--net",
                "--ipc",
                "--uts",
                "--propagation",
                "private",
            ])
            .arg(executable)
            .env_clear()
            .current_dir(executable.parent().ok_or(DeploymentError::RuntimeFailure)?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| DeploymentError::RuntimeFailure)?;
        let mut stdin = child.stdin.take().ok_or(DeploymentError::RuntimeFailure)?;
        stdin
            .write_all(encoded_request)
            .map_err(|_| DeploymentError::RuntimeFailure)?;
        drop(stdin);
        let limit = u64::from(self.profile.max_response_bytes) + 1;
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or(DeploymentError::RuntimeFailure)?
            .take(limit)
            .read_to_end(&mut output)
            .map_err(|_| DeploymentError::RuntimeFailure)?;
        let status = child.wait().map_err(|_| DeploymentError::RuntimeFailure)?;
        if !status.success() {
            return Err(DeploymentError::RuntimeFailure);
        }
        if output.len() > usize::try_from(self.profile.max_response_bytes).unwrap_or(usize::MAX) {
            return Err(DeploymentError::AbiFrameTooLarge);
        }
        Ok(output)
    }
}

#[cfg(target_os = "linux")]
impl SandboxedRuntime for LinuxNamespaceProcessRuntime {
    fn profile(&self) -> RuntimeProfileSpec {
        self.profile
    }

    fn invoke(
        &mut self,
        artifact: &[u8],
        encoded_request: &[u8],
    ) -> Result<Vec<u8>, DeploymentError> {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let id = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("cfmd-semantic-sandbox-{}-{id}", std::process::id()));
        fs::create_dir(&dir).map_err(|_| DeploymentError::RuntimeFailure)?;
        let executable = dir.join("artifact");
        let result = (|| {
            fs::write(&executable, artifact).map_err(|_| DeploymentError::RuntimeFailure)?;
            let mut permissions = fs::metadata(&executable)
                .map_err(|_| DeploymentError::RuntimeFailure)?
                .permissions();
            permissions.set_mode(0o500);
            fs::set_permissions(&executable, permissions)
                .map_err(|_| DeploymentError::RuntimeFailure)?;
            self.invoke_file(&executable, encoded_request)
        })();
        let _ = fs::remove_dir_all(&dir);
        result
    }
}

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
        .load_package(lookup.artifact)
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

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8], DeploymentError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(DeploymentError::MalformedPackage)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(DeploymentError::MalformedPackage)?;
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, DeploymentError> {
        Ok(*self
            .bytes(1)?
            .first()
            .ok_or(DeploymentError::MalformedPackage)?)
    }

    fn u16(&mut self) -> Result<u16, DeploymentError> {
        let bytes: [u8; 2] = self
            .bytes(2)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, DeploymentError> {
        let bytes: [u8; 4] = self
            .bytes(4)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u32_as_usize(&mut self) -> Result<usize, DeploymentError> {
        usize::try_from(self.u32()?).map_err(|_| DeploymentError::MalformedPackage)
    }

    fn u64(&mut self) -> Result<u64, DeploymentError> {
        let bytes: [u8; 8] = self
            .bytes(8)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn array_32(&mut self) -> Result<[u8; 32], DeploymentError> {
        self.bytes(32)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)
    }

    fn array_64(&mut self) -> Result<[u8; 64], DeploymentError> {
        self.bytes(64)?
            .try_into()
            .map_err(|_| DeploymentError::MalformedPackage)
    }

    fn finish(self) -> Result<(), DeploymentError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(DeploymentError::MalformedPackage)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_auth::{MemoryArtifactCas, sign_artifact_manifest};
    use kernel_semantics::{BuiltinSemanticModuleSpec, EquivalenceModule};

    const CHECKER_ID: RefinementCheckerId = RefinementCheckerId([0x33; 32]);
    const FORMAT_ID: RefinementFormatId = RefinementFormatId([0x44; 32]);

    struct ExactProofChecker;

    impl RefinementChecker for ExactProofChecker {
        fn checker_id(&self) -> RefinementCheckerId {
            CHECKER_ID
        }

        fn check(
            &self,
            _contract: SemanticContractIdentity,
            artifact: ImplementationArtifactDigest,
            runtime: RuntimeProfileDigest,
            semantic_descriptor: &[u8],
            format: RefinementFormatId,
            proof: &[u8],
        ) -> Result<(), DeploymentError> {
            let mut expected = Vec::new();
            expected.extend_from_slice(b"proof-v1\0");
            expected.extend_from_slice(&artifact.0);
            expected.extend_from_slice(&runtime.0);
            expected.extend_from_slice(&sha256(semantic_descriptor).0);
            if format == FORMAT_ID && proof == expected {
                Ok(())
            } else {
                Err(DeploymentError::RefinementRejected)
            }
        }
    }

    fn proof_bytes(
        artifact: ImplementationArtifactDigest,
        runtime: RuntimeProfileDigest,
        descriptor: &[u8],
    ) -> Vec<u8> {
        let mut proof = Vec::new();
        proof.extend_from_slice(b"proof-v1\0");
        proof.extend_from_slice(&artifact.0);
        proof.extend_from_slice(&runtime.0);
        proof.extend_from_slice(&sha256(descriptor).0);
        proof
    }

    fn signing_key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn runtime_profile() -> RuntimeProfileSpec {
        RuntimeProfileSpec::new(IsolationClass::SandboxedProcess, 4096, 4096, 10_000).unwrap()
    }

    fn defined_contract() -> SemanticContractIdentity {
        let spec = BuiltinSemanticModuleSpec::Equivalence {
            module: EquivalenceModule::TextExact,
            implementation_revision: 1,
        };
        SemanticContractIdentity::Defined(spec.contract())
    }

    fn package_fixture() -> (
        TrustRootSet,
        DeploymentPolicy,
        MemoryArtifactCas,
        SemanticPackageEnvelope,
        MemoryCheckerRegistry,
    ) {
        let signer = signing_key(7);
        let trust = TrustRootSet::bootstrap(1, &[*signer.verifying_key().as_bytes()]).unwrap();
        let runtime_profile = runtime_profile();
        let runtime_digest = runtime_profile.digest();
        let artifact = b"opaque executable bytes".to_vec();
        let descriptor = b"canonical semantic descriptor".to_vec();
        let manifest = sign_artifact_manifest(&signer, &artifact, runtime_digest, &descriptor);
        let artifact_id = ImplementationArtifactDigest(manifest.artifact_digest.0);
        let proof = RefinementProofEnvelope {
            checker: CHECKER_ID,
            format: FORMAT_ID,
            bytes: proof_bytes(artifact_id, runtime_digest, &descriptor),
        };
        let package = SemanticPackageEnvelope {
            manifest,
            runtime_profile,
            semantic_descriptor: descriptor,
            refinement: Some(proof),
        };
        let mut cas = MemoryArtifactCas::default();
        cas.insert(artifact);
        let policy = DeploymentPolicy::bootstrap(
            1,
            BTreeSet::from([runtime_digest]),
            BTreeSet::from([CHECKER_ID]),
        )
        .unwrap();
        let mut checkers = MemoryCheckerRegistry::default();
        checkers.insert(Box::new(ExactProofChecker));
        (trust, policy, cas, package, checkers)
    }

    #[test]
    fn canonical_package_roundtrip_is_exact() {
        let (_, _, _, package, _) = package_fixture();
        let encoded = package.encode().unwrap();
        let decoded = SemanticPackageEnvelope::decode(&encoded).unwrap();
        assert_eq!(decoded, package);
        assert_eq!(decoded.encode().unwrap(), encoded);
    }

    #[test]
    fn defined_contract_requires_authenticated_artifact_and_checked_refinement() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        let verified = verify_external_package(
            &trust,
            &policy,
            &cas,
            package,
            defined_contract(),
            b"canonical semantic descriptor",
            &checkers,
        )
        .unwrap();
        assert_eq!(
            verified.checked_refinement.as_ref().unwrap().checker,
            CHECKER_ID
        );
    }

    #[test]
    fn proof_tamper_and_unknown_checker_are_rejected() {
        let (trust, policy, cas, mut package, checkers) = package_fixture();
        package.refinement.as_mut().unwrap().bytes[0] ^= 1;
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package.clone(),
                defined_contract(),
                b"canonical semantic descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::RefinementRejected
        );
        package.refinement.as_mut().unwrap().checker = RefinementCheckerId([0x99; 32]);
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                defined_contract(),
                b"canonical semantic descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::RefinementCheckerForbidden
        );
    }

    #[test]
    fn runtime_profile_is_cryptographically_bound_and_policy_checked() {
        let (trust, policy, cas, mut package, checkers) = package_fixture();
        package.runtime_profile.max_fuel += 1;
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                defined_contract(),
                b"canonical semantic descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::RuntimeProfileForbidden
        );
    }

    #[test]
    fn signed_policy_update_is_monotone_and_can_revoke_artifact() {
        let (trust, policy, _, package, _) = package_fixture();
        let signer = signing_key(7);
        let artifact = ImplementationArtifactDigest(package.manifest.artifact_digest.0);
        let next = DeploymentPolicy {
            epoch: 2,
            allowed_runtime_profiles: policy.allowed_runtime_profiles.clone(),
            revoked_artifacts: BTreeSet::from([artifact]),
            allowed_refinement_checkers: policy.allowed_refinement_checkers.clone(),
        };
        let update = sign_deployment_policy(&signer, 1, next.clone()).unwrap();
        assert_eq!(policy.apply_signed_update(&trust, &update).unwrap(), next);

        let stale = SignedDeploymentPolicy {
            current_epoch: 0,
            ..update
        };
        assert_eq!(
            policy.apply_signed_update(&trust, &stale).unwrap_err(),
            DeploymentError::InvalidPolicyEpoch
        );
    }

    #[derive(Debug)]
    struct EchoSandbox {
        profile: RuntimeProfileSpec,
    }

    impl SandboxedRuntime for EchoSandbox {
        fn profile(&self) -> RuntimeProfileSpec {
            self.profile
        }

        fn invoke(
            &mut self,
            _artifact: &[u8],
            encoded_request: &[u8],
        ) -> Result<Vec<u8>, DeploymentError> {
            let request = AbiRequest::decode(encoded_request, self.profile)?;
            AbiResponse {
                consumed_fuel: 10,
                payload: request.payload,
            }
            .encode(self.profile)
        }
    }

    #[test]
    fn sandbox_abi_is_bounded_and_runtime_profile_pinned() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        let verified = verify_external_package(
            &trust,
            &policy,
            &cas,
            package,
            defined_contract(),
            b"canonical semantic descriptor",
            &checkers,
        )
        .unwrap();
        let request = AbiRequest {
            operation: AbiOperation::Equivalence,
            fuel_limit: 100,
            payload: b"hello".to_vec(),
        };
        let mut runtime = EchoSandbox {
            profile: runtime_profile(),
        };
        assert_eq!(
            verified.invoke(&mut runtime, &request).unwrap().payload,
            b"hello"
        );

        runtime.profile.max_fuel += 1;
        assert_eq!(
            verified.invoke(&mut runtime, &request).unwrap_err(),
            DeploymentError::RuntimeProfileMismatch
        );
    }

    #[test]
    fn abi_parser_rejects_trailing_garbage_and_fuel_escape() {
        let profile = runtime_profile();
        let request = AbiRequest {
            operation: AbiOperation::Ordering,
            fuel_limit: 100,
            payload: vec![1, 2, 3],
        };
        let mut encoded = request.encode(profile).unwrap();
        encoded.push(9);
        assert_eq!(
            AbiRequest::decode(&encoded, profile).unwrap_err(),
            DeploymentError::MalformedPackage
        );

        let oversized_fuel = AbiRequest {
            operation: AbiOperation::Ordering,
            fuel_limit: profile.max_fuel + 1,
            payload: Vec::new(),
        };
        assert_eq!(
            oversized_fuel.encode(profile).unwrap_err(),
            DeploymentError::FuelLimitExceeded
        );
    }

    #[test]
    fn caller_semantic_descriptor_is_part_of_the_authority_cut() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        assert_eq!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                defined_contract(),
                b"different durable descriptor",
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::SemanticDescriptorMismatch
        );
    }

    #[test]
    fn untrusted_repository_cannot_swap_package_for_requested_artifact() {
        let (trust, policy, cas, package, checkers) = package_fixture();
        let requested = ImplementationArtifactDigest(package.manifest.artifact_digest.0);
        let mut repository = MemoryPackageRepository::default();
        repository.insert(
            ImplementationArtifactDigest([0xA5; 32]),
            package.encode().unwrap(),
        );
        assert_eq!(
            verify_external_package_from_repository(
                &trust,
                &policy,
                &repository,
                &cas,
                PackageLookup {
                    artifact: requested,
                    contract: defined_contract(),
                    expected_semantic_descriptor: b"canonical semantic descriptor",
                },
                &checkers,
            )
            .unwrap_err(),
            DeploymentError::PackageUnavailable
        );

        repository.insert(requested, package.encode().unwrap());
        assert!(
            verify_external_package_from_repository(
                &trust,
                &policy,
                &repository,
                &cas,
                PackageLookup {
                    artifact: requested,
                    contract: defined_contract(),
                    expected_semantic_descriptor: b"canonical semantic descriptor",
                },
                &checkers,
            )
            .is_ok()
        );
    }

    #[test]
    fn deployment_policy_encoding_rejects_noncanonical_order() {
        let (trust, policy, _, _, _) = package_fixture();
        let signer = signing_key(7);
        let mut next = DeploymentPolicy {
            epoch: 2,
            allowed_runtime_profiles: policy.allowed_runtime_profiles.clone(),
            revoked_artifacts: BTreeSet::from([
                ImplementationArtifactDigest([1; 32]),
                ImplementationArtifactDigest([2; 32]),
            ]),
            allowed_refinement_checkers: policy.allowed_refinement_checkers.clone(),
        };
        let mut update = sign_deployment_policy(&signer, 1, next.clone()).unwrap();
        update.revoked_artifacts.reverse();
        let message = deployment_policy_message(&update).unwrap();
        update.signature = signer.sign(&message).to_bytes();
        assert_eq!(
            policy.apply_signed_update(&trust, &update).unwrap_err(),
            DeploymentError::NonCanonicalPolicy
        );

        next.revoked_artifacts.clear();
        assert!(sign_deployment_policy(&signer, 1, next).is_ok());
    }

    #[test]
    fn opaque_contract_requires_exact_authenticated_artifact_runtime_pair() {
        let (trust, policy, cas, mut package, checkers) = package_fixture();
        package.refinement = None;
        let contract = SemanticContractIdentity::OpaqueArtifact {
            artifact: ImplementationArtifactDigest(package.manifest.artifact_digest.0),
            runtime: package.runtime_profile.digest(),
        };
        assert!(
            verify_external_package(
                &trust,
                &policy,
                &cas,
                package,
                contract,
                b"canonical semantic descriptor",
                &checkers,
            )
            .is_ok()
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_runtime_tests {
    use super::*;

    #[test]
    fn linux_namespace_runtime_executes_abi_out_of_process() {
        let profile = RuntimeProfileSpec::new(IsolationClass::SandboxedProcess, 4096, 4096, 5_000)
            .expect("profile");
        let mut runtime = LinuxNamespaceProcessRuntime::new(profile).expect("runtime");
        let artifact = br"#!/usr/bin/python3
import sys
sys.stdin.buffer.read()
out=(1).to_bytes(2,'little')+(1).to_bytes(8,'little')+(2).to_bytes(4,'little')+b'ok'
sys.stdout.buffer.write(out)
";
        let request = AbiRequest {
            operation: AbiOperation::Equivalence,
            fuel_limit: 100,
            payload: b"request".to_vec(),
        };
        let raw = runtime
            .invoke(artifact, &request.encode(profile).expect("request"))
            .expect("sandbox");
        let response = AbiResponse::decode(&raw, profile).expect("response");
        assert_eq!(response.payload, b"ok");
        assert_eq!(response.consumed_fuel, 1);
    }

    #[test]
    fn linux_namespace_runtime_cannot_reach_host_loopback_listener() {
        use std::net::TcpListener;
        use std::thread;
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        listener.set_nonblocking(true).expect("nonblocking");
        let port = listener.local_addr().expect("addr").port();
        let profile = RuntimeProfileSpec::new(IsolationClass::SandboxedProcess, 4096, 4096, 5_000)
            .expect("profile");
        let mut runtime = LinuxNamespaceProcessRuntime::new(profile).expect("runtime");
        let artifact = format!(
            "#!/usr/bin/python3\nimport socket,sys\ns=socket.socket()\ns.settimeout(0.25)\ntry:\n s.connect(('127.0.0.1',{port}))\n payload=b'bad'\nexcept Exception:\n payload=b'ok'\nout=(1).to_bytes(2,'little')+(1).to_bytes(8,'little')+len(payload).to_bytes(4,'little')+payload\nsys.stdout.buffer.write(out)\n"
        );
        let request = AbiRequest {
            operation: AbiOperation::Equivalence,
            fuel_limit: 100,
            payload: Vec::new(),
        };
        let raw = runtime
            .invoke(
                artifact.as_bytes(),
                &request.encode(profile).expect("request"),
            )
            .expect("sandbox");
        let response = AbiResponse::decode(&raw, profile).expect("response");
        assert_eq!(response.payload, b"ok");
        thread::sleep(std::time::Duration::from_millis(20));
        assert!(
            matches!(listener.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
    }
}

#[cfg(test)]
mod filesystem_adapter_tests {
    use super::*;

    #[test]
    fn filesystem_repository_and_cas_are_untrusted_byte_sources_only() {
        let root = std::env::temp_dir().join(format!("cfmd-deployment-fs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("root");

        let artifact_bytes = b"artifact-bytes";
        let artifact_sha = sha256(artifact_bytes);
        fs::write(
            root.join(format!("{}.artifact", hex_digest(&artifact_sha.0))),
            artifact_bytes,
        )
        .expect("artifact");
        let cas = FilesystemArtifactCas::new(&root);
        assert_eq!(
            cas.load(artifact_sha).as_deref(),
            Some(artifact_bytes.as_slice())
        );

        let artifact = ImplementationArtifactDigest([0x4a; 32]);
        let package_bytes = b"untrusted-package-bytes";
        fs::write(
            root.join(format!("{}.cfmdspk", hex_digest(&artifact.0))),
            package_bytes,
        )
        .expect("package");
        let repository = FilesystemPackageRepository::new(&root);
        assert_eq!(
            repository.load_package(artifact).as_deref(),
            Some(package_bytes.as_slice())
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}
