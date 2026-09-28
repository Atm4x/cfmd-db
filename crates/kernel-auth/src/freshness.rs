use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use crate::core::{AuthError, AuthorityDigest, KeyId, TrustRootSet, key_id, strict_verify};
use crate::durable::{VerifiedGeneration, push_optional_authority_digest};

const FRESHNESS_CUT_AUTH_DOMAIN: &[u8] = b"CFMD-EXTERNAL-FRESHNESS-CUT-v1\0";
const FRESHNESS_CUT_DIGEST_DOMAIN: &[u8] = b"CFMD-EXTERNAL-FRESHNESS-RECORD-v1\0";
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedFreshnessCut {
    pub cut: FreshnessCut,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedFreshnessCut {
    pub cut: FreshnessCut,
    pub signer: KeyId,
    pub record_digest: AuthorityDigest,
}

#[must_use]
pub fn sign_freshness_cut(signing_key: &SigningKey, cut: FreshnessCut) -> SignedFreshnessCut {
    let message = freshness_cut_message(cut);
    SignedFreshnessCut {
        cut,
        signer: key_id(signing_key.verifying_key().as_bytes()),
        signature: signing_key.sign(&message).to_bytes(),
    }
}

pub fn verify_freshness_cut(
    trust: &TrustRootSet,
    record: &SignedFreshnessCut,
) -> Result<VerifiedFreshnessCut, AuthError> {
    if record.cut.trust_root_epoch != trust.epoch() {
        return Err(AuthError::FreshnessEpochMismatch);
    }
    let signer = trust.verifying_key(record.signer)?;
    strict_verify(
        &signer,
        &freshness_cut_message(record.cut),
        &record.signature,
    )?;
    Ok(VerifiedFreshnessCut {
        cut: record.cut,
        signer: record.signer,
        record_digest: freshness_record_digest(record),
    })
}

#[must_use]
pub fn freshness_record_digest(record: &SignedFreshnessCut) -> AuthorityDigest {
    let mut hasher = Sha256::new();
    hasher.update(FRESHNESS_CUT_DIGEST_DOMAIN);
    hasher.update(freshness_cut_message(record.cut));
    hasher.update(record.signer.0);
    hasher.update(record.signature);
    AuthorityDigest(hasher.finalize().into())
}

fn freshness_cut_message(cut: FreshnessCut) -> Vec<u8> {
    let mut out = Vec::with_capacity(FRESHNESS_CUT_AUTH_DOMAIN.len() + 193);
    out.extend_from_slice(FRESHNESS_CUT_AUTH_DOMAIN);
    out.extend_from_slice(&cut.store_id);
    out.extend_from_slice(&cut.generation.to_le_bytes());
    push_optional_authority_digest(&mut out, cut.previous_generation);
    out.extend_from_slice(&cut.generation_digest.0);
    out.extend_from_slice(&cut.wal_lsn.to_le_bytes());
    out.extend_from_slice(&cut.wal_digest.0);
    out.extend_from_slice(&cut.trust_root_epoch.to_le_bytes());
    out.extend_from_slice(&cut.deployment_policy_epoch.to_le_bytes());
    out
}

pub trait AuthenticatedFreshnessAnchor {
    type Error;

    fn read_signed(
        &mut self,
        store_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessCut>, Self::Error>;

    /// Atomically advances from the exact previously observed signed record.
    /// The external authority signs the new cut; callers never provide or
    /// persist its private key inside the database process.
    fn compare_and_advance_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    ) -> Result<SignedFreshnessCut, Self::Error>;
}
