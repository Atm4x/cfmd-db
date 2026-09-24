#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use kernel_semantics::{
    ArtifactAuthenticationSet, ImplementationArtifactDigest, RuntimeProfileDigest,
};
use sha2::{Digest, Sha256};

const KEY_ID_DOMAIN: &[u8] = b"CFMD-KEY-ID-v1\0";
const ARTIFACT_AUTH_DOMAIN: &[u8] = b"CFMD-SEMANTIC-ARTIFACT-AUTH-v1\0";
const KEY_ROTATION_DOMAIN: &[u8] = b"CFMD-TRUST-ROTATION-v1\0";
const GENERATION_AUTH_DOMAIN: &[u8] = b"CFMD-DURABLE-GENERATION-AUTH-v1\0";
const GENERATION_RECORD_DIGEST_DOMAIN: &[u8] = b"CFMD-DURABLE-GENERATION-RECORD-v1\0";
const WAL_FRAME_AUTH_DOMAIN: &[u8] = b"CFMD-DURABLE-WAL-FRAME-AUTH-v1\0";
const WAL_RECORD_DIGEST_DOMAIN: &[u8] = b"CFMD-DURABLE-WAL-RECORD-v1\0";
const FRESHNESS_CUT_AUTH_DOMAIN: &[u8] = b"CFMD-EXTERNAL-FRESHNESS-CUT-v1\0";
const FRESHNESS_CUT_DIGEST_DOMAIN: &[u8] = b"CFMD-EXTERNAL-FRESHNESS-RECORD-v1\0";

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

fn strict_verify(
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

    fn verifying_key(&self, id: KeyId) -> Result<VerifyingKey, AuthError> {
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

pub fn verify_artifact_manifest(
    trust: &TrustRootSet,
    manifest: &SignedArtifactManifest,
    artifact: &[u8],
    semantic_descriptor: &[u8],
) -> Result<VerifiedSemanticArtifact, AuthError> {
    if artifact.len() as u64 != manifest.artifact_len {
        return Err(AuthError::ArtifactLengthMismatch);
    }
    if sha256(artifact) != manifest.artifact_digest {
        return Err(AuthError::ArtifactDigestMismatch);
    }
    if sha256(semantic_descriptor) != manifest.semantic_descriptor_digest {
        return Err(AuthError::SemanticDescriptorDigestMismatch);
    }
    let signer = trust.verifying_key(manifest.signer)?;
    let message = artifact_message(
        manifest.artifact_digest,
        manifest.artifact_len,
        manifest.runtime_profile,
        manifest.semantic_descriptor_digest,
    );
    strict_verify(&signer, &message, &manifest.signature)?;
    Ok(VerifiedSemanticArtifact {
        artifact_digest: ImplementationArtifactDigest(manifest.artifact_digest.0),
        runtime_profile: manifest.runtime_profile,
        semantic_descriptor_digest: manifest.semantic_descriptor_digest,
        signer: manifest.signer,
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
    fn load(&self, digest: Sha256Digest) -> Option<Vec<u8>>;
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
    fn load(&self, digest: Sha256Digest) -> Option<Vec<u8>> {
        self.objects.get(&digest).cloned()
    }
}

pub fn verify_artifact_from_cas(
    trust: &TrustRootSet,
    cas: &impl ArtifactCas,
    manifest: &SignedArtifactManifest,
    semantic_descriptor: &[u8],
) -> Result<(VerifiedSemanticArtifact, Vec<u8>), AuthError> {
    let artifact = cas
        .load(manifest.artifact_digest)
        .ok_or(AuthError::CasMiss)?;
    if sha256(&artifact) != manifest.artifact_digest {
        return Err(AuthError::CasCorruption);
    }
    let verified = verify_artifact_manifest(trust, manifest, &artifact, semantic_descriptor)?;
    Ok((verified, artifact))
}

#[derive(Debug, Clone, Copy)]
pub struct GenerationComponents<'a> {
    pub manifest: &'a [u8],
    pub checkpoint: &'a [u8],
    pub metadata: &'a [u8],
    pub prepared_capsule: Option<&'a [u8]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedGenerationRecord {
    pub store_id: [u8; 32],
    pub generation: u64,
    pub previous: Option<AuthorityDigest>,
    pub manifest_digest: Sha256Digest,
    pub checkpoint_digest: Sha256Digest,
    pub metadata_digest: Sha256Digest,
    pub prepared_capsule_digest: Option<Sha256Digest>,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedGeneration {
    pub store_id: [u8; 32],
    pub generation: u64,
    pub previous: Option<AuthorityDigest>,
    pub auth_digest: AuthorityDigest,
    pub signer: KeyId,
}

#[must_use]
pub fn sign_generation_record(
    signing_key: &SigningKey,
    store_id: [u8; 32],
    generation: u64,
    previous: Option<AuthorityDigest>,
    components: GenerationComponents<'_>,
) -> SignedGenerationRecord {
    let record = UnsignedGenerationRecord {
        store_id,
        generation,
        previous,
        manifest_digest: sha256(components.manifest),
        checkpoint_digest: sha256(components.checkpoint),
        metadata_digest: sha256(components.metadata),
        prepared_capsule_digest: components.prepared_capsule.map(sha256),
    };
    let message = generation_message(&record);
    SignedGenerationRecord {
        store_id,
        generation,
        previous,
        manifest_digest: record.manifest_digest,
        checkpoint_digest: record.checkpoint_digest,
        metadata_digest: record.metadata_digest,
        prepared_capsule_digest: record.prepared_capsule_digest,
        signer: key_id(signing_key.verifying_key().as_bytes()),
        signature: signing_key.sign(&message).to_bytes(),
    }
}

pub fn verify_generation_record(
    trust: &TrustRootSet,
    record: &SignedGenerationRecord,
    components: GenerationComponents<'_>,
) -> Result<VerifiedGeneration, AuthError> {
    if sha256(components.manifest) != record.manifest_digest {
        return Err(AuthError::ManifestDigestMismatch);
    }
    if sha256(components.checkpoint) != record.checkpoint_digest {
        return Err(AuthError::CheckpointDigestMismatch);
    }
    if sha256(components.metadata) != record.metadata_digest {
        return Err(AuthError::MetadataDigestMismatch);
    }
    if components.prepared_capsule.map(sha256) != record.prepared_capsule_digest {
        return Err(AuthError::PreparedCapsuleDigestMismatch);
    }
    let unsigned = UnsignedGenerationRecord {
        store_id: record.store_id,
        generation: record.generation,
        previous: record.previous,
        manifest_digest: record.manifest_digest,
        checkpoint_digest: record.checkpoint_digest,
        metadata_digest: record.metadata_digest,
        prepared_capsule_digest: record.prepared_capsule_digest,
    };
    let signer = trust.verifying_key(record.signer)?;
    let message = generation_message(&unsigned);
    strict_verify(&signer, &message, &record.signature)?;
    Ok(VerifiedGeneration {
        store_id: record.store_id,
        generation: record.generation,
        previous: record.previous,
        auth_digest: generation_record_digest(record),
        signer: record.signer,
    })
}

#[derive(Debug, Clone, Copy)]
struct UnsignedGenerationRecord {
    store_id: [u8; 32],
    generation: u64,
    previous: Option<AuthorityDigest>,
    manifest_digest: Sha256Digest,
    checkpoint_digest: Sha256Digest,
    metadata_digest: Sha256Digest,
    prepared_capsule_digest: Option<Sha256Digest>,
}

fn generation_message(record: &UnsignedGenerationRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(GENERATION_AUTH_DOMAIN.len() + 178);
    out.extend_from_slice(GENERATION_AUTH_DOMAIN);
    out.extend_from_slice(&record.store_id);
    out.extend_from_slice(&record.generation.to_le_bytes());
    push_optional_authority_digest(&mut out, record.previous);
    out.extend_from_slice(&record.manifest_digest.0);
    out.extend_from_slice(&record.checkpoint_digest.0);
    out.extend_from_slice(&record.metadata_digest.0);
    push_optional_sha256(&mut out, record.prepared_capsule_digest);
    out
}

fn push_optional_authority_digest(out: &mut Vec<u8>, digest: Option<AuthorityDigest>) {
    if let Some(digest) = digest {
        out.push(1);
        out.extend_from_slice(&digest.0);
    } else {
        out.push(0);
        out.extend_from_slice(&[0; 32]);
    }
}

fn push_optional_sha256(out: &mut Vec<u8>, digest: Option<Sha256Digest>) {
    if let Some(digest) = digest {
        out.push(1);
        out.extend_from_slice(&digest.0);
    } else {
        out.push(0);
        out.extend_from_slice(&[0; 32]);
    }
}

#[must_use]
pub fn generation_record_digest(record: &SignedGenerationRecord) -> AuthorityDigest {
    let unsigned = UnsignedGenerationRecord {
        store_id: record.store_id,
        generation: record.generation,
        previous: record.previous,
        manifest_digest: record.manifest_digest,
        checkpoint_digest: record.checkpoint_digest,
        metadata_digest: record.metadata_digest,
        prepared_capsule_digest: record.prepared_capsule_digest,
    };
    let body = generation_message(&unsigned);
    let mut hasher = Sha256::new();
    hasher.update(GENERATION_RECORD_DIGEST_DOMAIN);
    hasher.update(&body);
    hasher.update(record.signer.0);
    hasher.update(record.signature);
    AuthorityDigest(hasher.finalize().into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedWalFrameRecord {
    pub store_id: [u8; 32],
    pub generation: u64,
    pub lsn: u64,
    pub previous: AuthorityDigest,
    pub frame_digest: Sha256Digest,
    pub signer: KeyId,
    pub signature: [u8; 64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedWalFrame {
    pub store_id: [u8; 32],
    pub generation: u64,
    pub lsn: u64,
    pub previous: AuthorityDigest,
    pub auth_digest: AuthorityDigest,
    pub signer: KeyId,
}

#[must_use]
pub fn sign_wal_frame_record(
    signing_key: &SigningKey,
    store_id: [u8; 32],
    generation: u64,
    lsn: u64,
    previous: AuthorityDigest,
    frame_bytes: &[u8],
) -> SignedWalFrameRecord {
    let frame_digest = sha256(frame_bytes);
    let message = wal_frame_message(store_id, generation, lsn, previous, frame_digest);
    SignedWalFrameRecord {
        store_id,
        generation,
        lsn,
        previous,
        frame_digest,
        signer: key_id(signing_key.verifying_key().as_bytes()),
        signature: signing_key.sign(&message).to_bytes(),
    }
}

pub fn verify_wal_frame_record(
    trust: &TrustRootSet,
    record: &SignedWalFrameRecord,
    frame_bytes: &[u8],
) -> Result<VerifiedWalFrame, AuthError> {
    if sha256(frame_bytes) != record.frame_digest {
        return Err(AuthError::WalFrameDigestMismatch);
    }
    let signer = trust.verifying_key(record.signer)?;
    let message = wal_frame_message(
        record.store_id,
        record.generation,
        record.lsn,
        record.previous,
        record.frame_digest,
    );
    strict_verify(&signer, &message, &record.signature)?;
    Ok(VerifiedWalFrame {
        store_id: record.store_id,
        generation: record.generation,
        lsn: record.lsn,
        previous: record.previous,
        auth_digest: wal_record_digest(record),
        signer: record.signer,
    })
}

fn wal_frame_message(
    store_id: [u8; 32],
    generation: u64,
    lsn: u64,
    previous: AuthorityDigest,
    frame_digest: Sha256Digest,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(WAL_FRAME_AUTH_DOMAIN.len() + 112);
    out.extend_from_slice(WAL_FRAME_AUTH_DOMAIN);
    out.extend_from_slice(&store_id);
    out.extend_from_slice(&generation.to_le_bytes());
    out.extend_from_slice(&lsn.to_le_bytes());
    out.extend_from_slice(&previous.0);
    out.extend_from_slice(&frame_digest.0);
    out
}

#[must_use]
pub fn wal_record_digest(record: &SignedWalFrameRecord) -> AuthorityDigest {
    let body = wal_frame_message(
        record.store_id,
        record.generation,
        record.lsn,
        record.previous,
        record.frame_digest,
    );
    let mut hasher = Sha256::new();
    hasher.update(WAL_RECORD_DIGEST_DOMAIN);
    hasher.update(&body);
    hasher.update(record.signer.0);
    hasher.update(record.signature);
    AuthorityDigest(hasher.finalize().into())
}

pub fn verify_wal_extension(
    anchor: AuthorityDigest,
    expected_store_id: [u8; 32],
    generation: u64,
    first_lsn: u64,
    frames: &[VerifiedWalFrame],
) -> Result<AuthorityDigest, AuthError> {
    let mut previous = anchor;
    let mut expected_lsn = first_lsn;
    for frame in frames {
        if frame.store_id != expected_store_id {
            return Err(AuthError::StoreIdentityMismatch);
        }
        if frame.generation != generation || frame.lsn != expected_lsn {
            return Err(AuthError::WalChainGap);
        }
        if frame.previous != previous {
            return Err(AuthError::WalChainLinkMismatch);
        }
        previous = frame.auth_digest;
        expected_lsn = expected_lsn.checked_add(1).ok_or(AuthError::WalChainGap)?;
    }
    Ok(previous)
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_semantics::{
        SemanticContractIdentity, SemanticDeploymentError, SemanticDeploymentRegistry,
        SemanticExecutionPolicy, SemanticImplementationPackage,
    };
    use std::collections::BTreeSet;

    fn signing(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn trust(signers: &[&SigningKey]) -> TrustRootSet {
        let keys = signers
            .iter()
            .map(|key| key.verifying_key().to_bytes())
            .collect::<Vec<_>>();
        TrustRootSet::bootstrap(1, &keys).unwrap()
    }

    #[test]
    fn semantic_artifact_authentication_bridges_into_existing_deployment_boundary() {
        let signer = signing(7);
        let trust = trust(&[&signer]);
        let runtime = RuntimeProfileDigest([0xA5; 32]);
        let artifact = b"external semantic implementation bytes";
        let descriptor = b"opaque-contract-descriptor-v1";
        let manifest = sign_artifact_manifest(&signer, artifact, runtime, descriptor);
        let verified = verify_artifact_manifest(&trust, &manifest, artifact, descriptor).unwrap();

        let contract = SemanticContractIdentity::OpaqueArtifact {
            artifact: verified.artifact_digest,
            runtime,
        };
        let package = SemanticImplementationPackage {
            contract,
            artifact_digest: verified.artifact_digest,
            runtime_profile: runtime,
            refinement: None,
            executable: None,
        };
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(package);
        let policy = SemanticExecutionPolicy {
            allowed_runtime_profiles: BTreeSet::from([runtime]),
            revoked_artifacts: BTreeSet::new(),
            require_authentication: true,
        };

        let unauthenticated = deployment.authorize_artifact(
            verified.artifact_digest,
            &policy,
            &ArtifactAuthenticationSet::default(),
        );
        assert_eq!(
            unauthenticated,
            Err(SemanticDeploymentError::UnauthenticatedArtifact)
        );

        let mut authentications = ArtifactAuthenticationSet::default();
        verified.mark_authenticated(&mut authentications);
        assert!(
            deployment
                .authorize_artifact(verified.artifact_digest, &policy, &authentications)
                .is_ok()
        );
    }

    #[test]
    fn artifact_tamper_descriptor_tamper_and_signature_tamper_are_rejected() {
        let signer = signing(9);
        let trust = trust(&[&signer]);
        let runtime = RuntimeProfileDigest([2; 32]);
        let artifact = b"artifact";
        let descriptor = b"descriptor";
        let manifest = sign_artifact_manifest(&signer, artifact, runtime, descriptor);

        assert_eq!(
            verify_artifact_manifest(&trust, &manifest, b"artifacU", descriptor),
            Err(AuthError::ArtifactDigestMismatch)
        );
        assert_eq!(
            verify_artifact_manifest(&trust, &manifest, artifact, b"descriptoS"),
            Err(AuthError::SemanticDescriptorDigestMismatch)
        );
        let mut forged = manifest.clone();
        forged.signature[5] ^= 0x80;
        assert_eq!(
            verify_artifact_manifest(&trust, &forged, artifact, descriptor),
            Err(AuthError::InvalidSignature)
        );
    }

    #[test]
    fn cas_detects_missing_and_corrupted_objects() {
        let signer = signing(10);
        let trust = trust(&[&signer]);
        let runtime = RuntimeProfileDigest([3; 32]);
        let artifact = b"artifact".to_vec();
        let descriptor = b"descriptor";
        let manifest = sign_artifact_manifest(&signer, &artifact, runtime, descriptor);
        let empty = MemoryArtifactCas::default();
        assert_eq!(
            verify_artifact_from_cas(&trust, &empty, &manifest, descriptor),
            Err(AuthError::CasMiss)
        );
        let mut corrupt = MemoryArtifactCas::default();
        corrupt.insert_unchecked_for_test(manifest.artifact_digest, b"wrong".to_vec());
        assert_eq!(
            verify_artifact_from_cas(&trust, &corrupt, &manifest, descriptor),
            Err(AuthError::CasCorruption)
        );
        let mut valid = MemoryArtifactCas::default();
        assert_eq!(valid.insert(artifact), manifest.artifact_digest);
        assert!(verify_artifact_from_cas(&trust, &valid, &manifest, descriptor).is_ok());
    }

    #[test]
    fn signed_key_rotation_replaces_trust_without_backdoor_accepting_retired_key() {
        let old = signing(11);
        let replacement = signing(12);
        let initial = trust(&[&old]);
        let add_replacement = sign_trust_rotation(
            &old,
            1,
            2,
            vec![
                old.verifying_key().to_bytes(),
                replacement.verifying_key().to_bytes(),
            ],
        )
        .unwrap();
        let dual = initial.apply_rotation(&add_replacement).unwrap();
        assert!(dual.contains(key_id(old.verifying_key().as_bytes())));
        assert!(dual.contains(key_id(replacement.verifying_key().as_bytes())));

        let retire_old = sign_trust_rotation(
            &replacement,
            2,
            3,
            vec![replacement.verifying_key().to_bytes()],
        )
        .unwrap();
        let current = dual.apply_rotation(&retire_old).unwrap();
        assert!(!current.contains(key_id(old.verifying_key().as_bytes())));
        assert!(current.contains(key_id(replacement.verifying_key().as_bytes())));

        let runtime = RuntimeProfileDigest([4; 32]);
        let old_manifest = sign_artifact_manifest(&old, b"old", runtime, b"descriptor");
        assert_eq!(
            verify_artifact_manifest(&current, &old_manifest, b"old", b"descriptor"),
            Err(AuthError::UnknownSigner)
        );
        let new_manifest = sign_artifact_manifest(&replacement, b"new", runtime, b"descriptor");
        assert!(verify_artifact_manifest(&current, &new_manifest, b"new", b"descriptor").is_ok());
    }

    #[test]
    fn stale_or_wrongly_signed_rotation_is_rejected() {
        let trusted = signing(13);
        let outsider = signing(14);
        let initial = trust(&[&trusted]);
        let stale =
            sign_trust_rotation(&trusted, 0, 1, vec![trusted.verifying_key().to_bytes()]).unwrap();
        assert_eq!(
            initial.apply_rotation(&stale),
            Err(AuthError::InvalidRotationEpoch)
        );
        let outsider_update =
            sign_trust_rotation(&outsider, 1, 2, vec![outsider.verifying_key().to_bytes()])
                .unwrap();
        assert_eq!(
            initial.apply_rotation(&outsider_update),
            Err(AuthError::UnknownSigner)
        );
    }

    #[test]
    fn weak_root_key_is_rejected() {
        let identity_point = {
            let mut bytes = [0_u8; 32];
            bytes[0] = 1;
            bytes
        };
        assert_eq!(
            TrustRootSet::bootstrap(1, &[identity_point]),
            Err(AuthError::WeakVerifyingKey)
        );
    }

    fn components<'a>(
        manifest: &'a [u8],
        checkpoint: &'a [u8],
        metadata: &'a [u8],
    ) -> GenerationComponents<'a> {
        GenerationComponents {
            manifest,
            checkpoint,
            metadata,
            prepared_capsule: None,
        }
    }

    #[test]
    fn durable_generation_authentication_binds_all_published_components() {
        let signer = signing(20);
        let trust = trust(&[&signer]);
        let store_id = [0x55; 32];
        let record = sign_generation_record(
            &signer,
            store_id,
            8,
            None,
            components(b"manifest", b"checkpoint", b"metadata"),
        );
        let verified = verify_generation_record(
            &trust,
            &record,
            components(b"manifest", b"checkpoint", b"metadata"),
        )
        .unwrap();
        assert_eq!(verified.generation, 8);
        assert_eq!(
            verify_generation_record(
                &trust,
                &record,
                components(b"manifest", b"CHECKPOINT", b"metadata"),
            ),
            Err(AuthError::CheckpointDigestMismatch)
        );
    }

    #[test]
    fn generation_signature_prevents_cross_store_transplant() {
        let signer = signing(21);
        let trust = trust(&[&signer]);
        let mut record =
            sign_generation_record(&signer, [1; 32], 1, None, components(b"m", b"c", b"d"));
        record.store_id = [2; 32];
        assert_eq!(
            verify_generation_record(&trust, &record, components(b"m", b"c", b"d")),
            Err(AuthError::InvalidSignature)
        );
    }

    #[test]
    fn external_anchor_detects_rollback_fork_and_accepts_one_generation_catchup() {
        let signer = signing(22);
        let trust = trust(&[&signer]);
        let store_id = [0xA0; 32];
        let first_record = sign_generation_record(
            &signer,
            store_id,
            10,
            None,
            components(b"m10", b"c10", b"d10"),
        );
        let first =
            verify_generation_record(&trust, &first_record, components(b"m10", b"c10", b"d10"))
                .unwrap();
        let anchor = AnchoredGeneration::from(first);

        let second_record = sign_generation_record(
            &signer,
            store_id,
            11,
            Some(first.auth_digest),
            components(b"m11", b"c11", b"d11"),
        );
        let second =
            verify_generation_record(&trust, &second_record, components(b"m11", b"c11", b"d11"))
                .unwrap();
        assert_eq!(
            compare_with_anchor(Some(anchor), second),
            Ok(FreshnessDisposition::CatchUpOne)
        );
        assert_eq!(
            compare_with_anchor(Some(AnchoredGeneration::from(second)), first),
            Err(AuthError::RollbackDetected)
        );

        let fork_record = sign_generation_record(
            &signer,
            store_id,
            10,
            None,
            components(b"fork", b"c10", b"d10"),
        );
        let fork =
            verify_generation_record(&trust, &fork_record, components(b"fork", b"c10", b"d10"))
                .unwrap();
        assert_eq!(
            compare_with_anchor(Some(anchor), fork),
            Err(AuthError::AnchorForkDetected)
        );
    }

    #[test]
    fn wal_auth_chain_binds_exact_frame_bytes_and_contiguous_lsn() {
        let signer = signing(24);
        let trust = trust(&[&signer]);
        let store_id = [0xC0; 32];
        let generation = 7;
        let root = AuthorityDigest([0x11; 32]);
        let prepare_record =
            sign_wal_frame_record(&signer, store_id, generation, 41, root, b"prepare-frame");
        let prepare = verify_wal_frame_record(&trust, &prepare_record, b"prepare-frame").unwrap();
        let commit_record = sign_wal_frame_record(
            &signer,
            store_id,
            generation,
            42,
            prepare.auth_digest,
            b"commit-frame",
        );
        let commit = verify_wal_frame_record(&trust, &commit_record, b"commit-frame").unwrap();
        assert_eq!(
            verify_wal_extension(root, store_id, generation, 41, &[prepare, commit]),
            Ok(commit.auth_digest)
        );
        assert_eq!(
            verify_wal_frame_record(&trust, &prepare_record, b"PREPARE-frame"),
            Err(AuthError::WalFrameDigestMismatch)
        );
    }

    #[test]
    fn wal_auth_chain_rejects_missing_or_relinked_frame() {
        let signer = signing(25);
        let trust = trust(&[&signer]);
        let store_id = [0xD0; 32];
        let root = AuthorityDigest([0x22; 32]);
        let first_record = sign_wal_frame_record(&signer, store_id, 9, 1, root, b"frame-1");
        let first = verify_wal_frame_record(&trust, &first_record, b"frame-1").unwrap();
        let third_record =
            sign_wal_frame_record(&signer, store_id, 9, 3, first.auth_digest, b"frame-3");
        let third = verify_wal_frame_record(&trust, &third_record, b"frame-3").unwrap();
        assert_eq!(
            verify_wal_extension(root, store_id, 9, 1, &[first, third]),
            Err(AuthError::WalChainGap)
        );
        let wrong_link_record = sign_wal_frame_record(
            &signer,
            store_id,
            9,
            2,
            AuthorityDigest([0xEE; 32]),
            b"frame-2",
        );
        let wrong_link = verify_wal_frame_record(&trust, &wrong_link_record, b"frame-2").unwrap();
        assert_eq!(
            verify_wal_extension(root, store_id, 9, 1, &[first, wrong_link]),
            Err(AuthError::WalChainLinkMismatch)
        );
    }

    #[test]
    fn anchor_gap_is_not_silently_accepted() {
        let signer = signing(23);
        let trust = trust(&[&signer]);
        let store_id = [0xB0; 32];
        let anchor = AnchoredGeneration {
            store_id,
            generation: 3,
            auth_digest: AuthorityDigest([7; 32]),
        };
        let record = sign_generation_record(
            &signer,
            store_id,
            5,
            Some(anchor.auth_digest),
            components(b"m5", b"c5", b"d5"),
        );
        let local =
            verify_generation_record(&trust, &record, components(b"m5", b"c5", b"d5")).unwrap();
        assert_eq!(
            compare_with_anchor(Some(anchor), local),
            Err(AuthError::AnchorGap)
        );
    }
}
