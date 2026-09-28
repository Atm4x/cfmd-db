use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};

use crate::core::{
    AuthError, AuthorityDigest, KeyId, Sha256Digest, TrustRootSet, key_id, sha256, strict_verify,
};

const GENERATION_AUTH_DOMAIN: &[u8] = b"CFMD-DURABLE-GENERATION-AUTH-v1\0";
const GENERATION_RECORD_DIGEST_DOMAIN: &[u8] = b"CFMD-DURABLE-GENERATION-RECORD-v1\0";
const WAL_FRAME_AUTH_DOMAIN: &[u8] = b"CFMD-DURABLE-WAL-FRAME-AUTH-v1\0";
const WAL_RECORD_DIGEST_DOMAIN: &[u8] = b"CFMD-DURABLE-WAL-RECORD-v1\0";
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

pub(crate) fn push_optional_authority_digest(out: &mut Vec<u8>, digest: Option<AuthorityDigest>) {
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
    let signer = trust.verifying_key(record.signer)?;
    let message = wal_frame_message(
        record.store_id,
        record.generation,
        record.lsn,
        record.previous,
        record.frame_digest,
    );
    strict_verify(&signer, &message, &record.signature)?;
    if sha256(frame_bytes) != record.frame_digest {
        return Err(AuthError::WalFrameDigestMismatch);
    }
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
