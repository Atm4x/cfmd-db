use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signer, SigningKey};
use kernel_auth::{KeyId, Sha256Digest, SignedArtifactManifest, TrustRootSet, key_id, sha256};
use kernel_semantics::{
    ImplementationArtifactDigest, RuntimeProfileDigest, SemanticContractIdentity,
};

use crate::{DeploymentError, MAX_ABI_FRAME_BYTES};

const RUNTIME_PROFILE_DOMAIN: &[u8] = b"CFMD-SEMANTIC-RUNTIME-PROFILE-v1\0";
const DEPLOYMENT_POLICY_DOMAIN: &[u8] = b"CFMD-SEMANTIC-DEPLOYMENT-POLICY-v1\0";
use crate::limits::ABI_VERSION;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationClass {
    DeterministicWasm,
    SandboxedProcess,
}

impl IsolationClass {
    pub(crate) const fn tag(self) -> u8 {
        match self {
            Self::DeterministicWasm => 1,
            Self::SandboxedProcess => 2,
        }
    }

    pub(crate) fn from_tag(tag: u8) -> Result<Self, DeploymentError> {
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

pub(crate) fn deployment_policy_message(
    update: &SignedDeploymentPolicy,
) -> Result<Vec<u8>, DeploymentError> {
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
