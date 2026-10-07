use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use crate::core::{AuthError, AuthorityDigest, KeyId, TrustRootSet, key_id, strict_verify};
use crate::durable::{VerifiedGeneration, push_optional_authority_digest};

const FRESHNESS_AUTH_DOMAIN: &[u8] = b"CFMD-PERSISTENCE-LINEAGE-AUTHORITY-v2\0";
const FRESHNESS_DIGEST_DOMAIN: &[u8] = b"CFMD-PERSISTENCE-LINEAGE-RECORD-v2\0";
const VOLATILE_FENCE_NONCE_DOMAIN: &[u8] = b"CFMD-VOLATILE-FENCE-NONCE-v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchoredGeneration {
    pub store_id: [u8; 32],
    pub generation: u64,
    pub auth_digest: AuthorityDigest,
}

impl From<VerifiedGeneration> for AnchoredGeneration {
    fn from(value: VerifiedGeneration) -> Self {
        Self {
            store_id: value.store_id,
            generation: value.generation,
            auth_digest: value.auth_digest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessDisposition {
    Bootstrap,
    Exact,
    CatchUpOne,
}

pub fn compare_with_anchor(
    anchor: Option<AnchoredGeneration>,
    local: VerifiedGeneration,
) -> Result<FreshnessDisposition, AuthError> {
    let Some(anchor) = anchor else {
        return Ok(FreshnessDisposition::Bootstrap);
    };
    if anchor.store_id != local.store_id {
        return Err(AuthError::StoreIdentityMismatch);
    }
    if local.generation < anchor.generation {
        return Err(AuthError::RollbackDetected);
    }
    if local.generation == anchor.generation {
        return if local.auth_digest == anchor.auth_digest {
            Ok(FreshnessDisposition::Exact)
        } else {
            Err(AuthError::AnchorForkDetected)
        };
    }
    if local.generation != anchor.generation.saturating_add(1) {
        return Err(AuthError::AnchorGap);
    }
    if local.previous != Some(anchor.auth_digest) {
        return Err(AuthError::PreviousGenerationMismatch);
    }
    Ok(FreshnessDisposition::CatchUpOne)
}

pub trait FreshnessAnchor {
    type Error;

    fn read(&self, store_id: [u8; 32]) -> Result<Option<AnchoredGeneration>, Self::Error>;

    fn compare_and_advance(
        &mut self,
        expected: Option<AnchoredGeneration>,
        next: AnchoredGeneration,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FreshnessCut {
    pub store_id: [u8; 32],
    pub generation: u64,
    pub previous_generation: Option<AuthorityDigest>,
    pub generation_digest: AuthorityDigest,
    pub wal_lsn: u64,
    pub wal_digest: AuthorityDigest,
    pub trust_root_epoch: u64,
    pub deployment_policy_epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolatileFence {
    pub lineage_id: [u8; 32],
    pub predecessor_record_digest: AuthorityDigest,
    pub source_generation_digest: AuthorityDigest,
    pub trust_root_epoch: u64,
    pub deployment_policy_epoch: u64,
    pub transition_nonce: AuthorityDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessAuthorityState {
    DurableCut(FreshnessCut),
    VolatileFence(VolatileFence),
}

impl FreshnessAuthorityState {
    #[must_use]
    pub const fn lineage_id(self) -> [u8; 32] {
        match self {
            Self::DurableCut(cut) => cut.store_id,
            Self::VolatileFence(fence) => fence.lineage_id,
        }
    }

    #[must_use]
    pub const fn trust_root_epoch(self) -> u64 {
        match self {
            Self::DurableCut(cut) => cut.trust_root_epoch,
            Self::VolatileFence(fence) => fence.trust_root_epoch,
        }
    }

    #[must_use]
    pub const fn deployment_policy_epoch(self) -> u64 {
        match self {
            Self::DurableCut(cut) => cut.deployment_policy_epoch,
            Self::VolatileFence(fence) => fence.deployment_policy_epoch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedFreshnessAuthority {
    pub state: FreshnessAuthorityState,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedFreshnessAuthority {
    pub state: FreshnessAuthorityState,
    pub signer: KeyId,
    pub record_digest: AuthorityDigest,
}

impl SignedFreshnessAuthority {
    #[must_use]
    pub const fn durable_cut(&self) -> Option<FreshnessCut> {
        match self.state {
            FreshnessAuthorityState::DurableCut(cut) => Some(cut),
            FreshnessAuthorityState::VolatileFence(_) => None,
        }
    }
}

impl VerifiedFreshnessAuthority {
    #[must_use]
    pub const fn durable_cut(self) -> Option<FreshnessCut> {
        match self.state {
            FreshnessAuthorityState::DurableCut(cut) => Some(cut),
            FreshnessAuthorityState::VolatileFence(_) => None,
        }
    }

    #[must_use]
    pub const fn volatile_fence(self) -> Option<VolatileFence> {
        match self.state {
            FreshnessAuthorityState::DurableCut(_) => None,
            FreshnessAuthorityState::VolatileFence(fence) => Some(fence),
        }
    }
}

#[must_use]
pub fn volatile_fence_nonce(
    predecessor_record_digest: AuthorityDigest,
    source_generation_digest: AuthorityDigest,
) -> AuthorityDigest {
    let mut hasher = Sha256::new();
    hasher.update(VOLATILE_FENCE_NONCE_DOMAIN);
    hasher.update(predecessor_record_digest.0);
    hasher.update(source_generation_digest.0);
    AuthorityDigest(hasher.finalize().into())
}

#[must_use]
pub fn sign_freshness_authority(
    signing_key: &SigningKey,
    state: FreshnessAuthorityState,
) -> SignedFreshnessAuthority {
    let message = freshness_authority_message(state);
    SignedFreshnessAuthority {
        state,
        signer: key_id(signing_key.verifying_key().as_bytes()),
        signature: signing_key.sign(&message).to_bytes(),
    }
}

pub fn verify_freshness_authority(
    trust: &TrustRootSet,
    record: &SignedFreshnessAuthority,
) -> Result<VerifiedFreshnessAuthority, AuthError> {
    if record.state.trust_root_epoch() != trust.epoch() {
        return Err(AuthError::FreshnessEpochMismatch);
    }
    let signer = trust.verifying_key(record.signer)?;
    strict_verify(
        &signer,
        &freshness_authority_message(record.state),
        &record.signature,
    )?;
    Ok(VerifiedFreshnessAuthority {
        state: record.state,
        signer: record.signer,
        record_digest: freshness_authority_record_digest(record),
    })
}

#[must_use]
pub fn freshness_authority_record_digest(record: &SignedFreshnessAuthority) -> AuthorityDigest {
    let mut hasher = Sha256::new();
    hasher.update(FRESHNESS_DIGEST_DOMAIN);
    hasher.update(freshness_authority_message(record.state));
    hasher.update(record.signer.0);
    hasher.update(record.signature);
    AuthorityDigest(hasher.finalize().into())
}

fn freshness_authority_message(state: FreshnessAuthorityState) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRESHNESS_AUTH_DOMAIN.len() + 256);
    out.extend_from_slice(FRESHNESS_AUTH_DOMAIN);
    match state {
        FreshnessAuthorityState::DurableCut(cut) => {
            out.push(0);
            out.extend_from_slice(&cut.store_id);
            out.extend_from_slice(&cut.generation.to_le_bytes());
            push_optional_authority_digest(&mut out, cut.previous_generation);
            out.extend_from_slice(&cut.generation_digest.0);
            out.extend_from_slice(&cut.wal_lsn.to_le_bytes());
            out.extend_from_slice(&cut.wal_digest.0);
            out.extend_from_slice(&cut.trust_root_epoch.to_le_bytes());
            out.extend_from_slice(&cut.deployment_policy_epoch.to_le_bytes());
        }
        FreshnessAuthorityState::VolatileFence(fence) => {
            out.push(1);
            out.extend_from_slice(&fence.lineage_id);
            out.extend_from_slice(&fence.predecessor_record_digest.0);
            out.extend_from_slice(&fence.source_generation_digest.0);
            out.extend_from_slice(&fence.trust_root_epoch.to_le_bytes());
            out.extend_from_slice(&fence.deployment_policy_epoch.to_le_bytes());
            out.extend_from_slice(&fence.transition_nonce.0);
        }
    }
    out
}
