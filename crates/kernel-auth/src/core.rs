use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

const KEY_ID_DOMAIN: &[u8] = b"CFMD-KEY-ID-v1\0";
const KEY_ROTATION_DOMAIN: &[u8] = b"CFMD-TRUST-ROTATION-v1\0";
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Sha256Digest(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyId(pub [u8; 32]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityDigest(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    EmptyTrustRoot,
    DuplicateKey,
    TooManyTrustKeys,
    InvalidVerifyingKey,
    WeakVerifyingKey,
    UnknownSigner,
    InvalidSignature,
    InvalidRotationEpoch,
    ArtifactLengthMismatch,
    ArtifactDigestMismatch,
    SemanticDescriptorDigestMismatch,
    CasMiss,
    CasCorruption,
    ManifestDigestMismatch,
    CheckpointDigestMismatch,
    MetadataDigestMismatch,
    PreparedCapsuleDigestMismatch,
    StoreIdentityMismatch,
    RollbackDetected,
    AnchorForkDetected,
    AnchorGap,
    PreviousGenerationMismatch,
    WalFrameDigestMismatch,
    WalChainGap,
    WalChainLinkMismatch,
    FreshnessEpochMismatch,
    FreshnessCutMismatch,
}

#[must_use]
pub fn sha256(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    Sha256Digest(digest.into())
}

#[must_use]
pub fn key_id(verifying_key: &[u8; 32]) -> KeyId {
    let mut hasher = Sha256::new();
    hasher.update(KEY_ID_DOMAIN);
    hasher.update(verifying_key);
    KeyId(hasher.finalize().into())
}

fn parse_verifying_key(bytes: &[u8; 32]) -> Result<VerifyingKey, AuthError> {
    let key = VerifyingKey::from_bytes(bytes).map_err(|_| AuthError::InvalidVerifyingKey)?;
    if key.is_weak() {
        return Err(AuthError::WeakVerifyingKey);
    }
    Ok(key)
}

pub(crate) fn strict_verify(
    key: &VerifyingKey,
    message: &[u8],
    signature: &[u8; 64],
) -> Result<(), AuthError> {
    let signature = Signature::from_bytes(signature);
    key.verify_strict(message, &signature)
        .map_err(|_| AuthError::InvalidSignature)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustRootSet {
    epoch: u64,
    keys: BTreeMap<KeyId, [u8; 32]>,
}

impl TrustRootSet {
    pub fn bootstrap(epoch: u64, keys: &[[u8; 32]]) -> Result<Self, AuthError> {
        let keys = validated_key_map(keys)?;
        if keys.is_empty() {
            return Err(AuthError::EmptyTrustRoot);
        }
        Ok(Self { epoch, keys })
    }

    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    #[must_use]
    pub fn contains(&self, id: KeyId) -> bool {
        self.keys.contains_key(&id)
    }

    pub(crate) fn verifying_key(&self, id: KeyId) -> Result<VerifyingKey, AuthError> {
        let bytes = self.keys.get(&id).ok_or(AuthError::UnknownSigner)?;
        parse_verifying_key(bytes)
    }

    /// Verifies one already-domain-separated deployment/security message.
    ///
    /// The caller owns the message format and MUST include a stable domain tag;
    /// this helper only centralizes trust-root lookup and strict Ed25519
    /// verification so higher deployment layers never need access to raw trust
    /// keys.
    pub fn verify_message(
        &self,
        signer: KeyId,
        message: &[u8],
        signature: &[u8; 64],
    ) -> Result<(), AuthError> {
        let key = self.verifying_key(signer)?;
        strict_verify(&key, message, signature)
    }

    pub fn apply_rotation(&self, update: &SignedTrustRotation) -> Result<Self, AuthError> {
        if update.current_epoch != self.epoch
            || update.next_epoch
                != self
                    .epoch
                    .checked_add(1)
                    .ok_or(AuthError::InvalidRotationEpoch)?
        {
            return Err(AuthError::InvalidRotationEpoch);
        }
        let next_keys = validated_key_map(&update.next_verifying_keys)?;
        if next_keys.is_empty() {
            return Err(AuthError::EmptyTrustRoot);
        }
        let signer = self.verifying_key(update.signer)?;
        let message = rotation_message(update.current_epoch, update.next_epoch, &next_keys)?;
        strict_verify(&signer, &message, &update.signature)?;
        Ok(Self {
            epoch: update.next_epoch,
            keys: next_keys,
        })
    }
}

fn validated_key_map(keys: &[[u8; 32]]) -> Result<BTreeMap<KeyId, [u8; 32]>, AuthError> {
    let mut result = BTreeMap::new();
    for key_bytes in keys {
        parse_verifying_key(key_bytes)?;
        let id = key_id(key_bytes);
        if result.insert(id, *key_bytes).is_some() {
            return Err(AuthError::DuplicateKey);
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTrustRotation {
    pub current_epoch: u64,
    pub next_epoch: u64,
    pub next_verifying_keys: Vec<[u8; 32]>,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

pub fn sign_trust_rotation(
    signing_key: &SigningKey,
    current_epoch: u64,
    next_epoch: u64,
    next_verifying_keys: Vec<[u8; 32]>,
) -> Result<SignedTrustRotation, AuthError> {
    let next_keys = validated_key_map(&next_verifying_keys)?;
    let message = rotation_message(current_epoch, next_epoch, &next_keys)?;
    let signature = signing_key.sign(&message).to_bytes();
    Ok(SignedTrustRotation {
        current_epoch,
        next_epoch,
        next_verifying_keys,
        signer: key_id(signing_key.verifying_key().as_bytes()),
        signature,
    })
}

fn rotation_message(
    current_epoch: u64,
    next_epoch: u64,
    next_keys: &BTreeMap<KeyId, [u8; 32]>,
) -> Result<Vec<u8>, AuthError> {
    let mut out = Vec::with_capacity(KEY_ROTATION_DOMAIN.len() + 20 + next_keys.len() * 64);
    out.extend_from_slice(KEY_ROTATION_DOMAIN);
    out.extend_from_slice(&current_epoch.to_le_bytes());
    out.extend_from_slice(&next_epoch.to_le_bytes());
    let key_count = u32::try_from(next_keys.len()).map_err(|_| AuthError::TooManyTrustKeys)?;
    out.extend_from_slice(&key_count.to_le_bytes());
    for (id, key) in next_keys {
        out.extend_from_slice(&id.0);
        out.extend_from_slice(key);
    }
    Ok(out)
}
