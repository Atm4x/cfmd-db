use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use kernel_auth::{
    AuthorityDigest, FreshnessCut, Sha256Digest, SignedFreshnessCut, TrustRootSet,
    VerifiedFreshnessCut, sha256, verify_freshness_cut,
};
use sha2::{Digest, Sha256};

use crate::binary_codec::{crc32c, read_u16, read_u32, read_u64};
use crate::domain::DurableExternalFreshnessBinding;
use crate::runtime::DurabilityError;
use crate::wal::WAL_FRESHNESS_PREFIX_DOMAIN;
use crate::wal_frame::{HEADER_LEN, MAGIC, MAX_PAYLOAD_LEN, validate_frame_header};

use super::checkpoint_storage::{
    CHECKPOINT_CHUNK_DESCRIPTOR_LEN, CHECKPOINT_HEADER_LEN, CHECKPOINT_MAGIC,
    read_checkpoint_root_bounded, validate_checkpoint_chunk_descriptors,
};
use super::format_registry::{DurableFormatRegistry, LEGACY_CHECKPOINT_FORMAT_VERSION};
use super::generation_layout::{
    checkpoint_chunk_path, checkpoint_path, manifest_path, metadata_path, prepared_capsule_path,
};
use super::{DurableGenerationReceipt, DurableRevisionStore};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFreshnessConfig {
    pub store_id: [u8; 32],
    pub trust_roots: TrustRootSet,
    pub deployment_policy_epoch: u64,
}

pub trait ExternalFreshnessAuthority: std::fmt::Debug + Send {
    fn read_signed(
        &mut self,
        store_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessCut>, DurabilityError>;

    fn compare_and_advance_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessCut,
    ) -> Result<SignedFreshnessCut, DurabilityError>;
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PendingExternalFreshnessAdvance {
    expected_record: AuthorityDigest,
    next: FreshnessCut,
}

#[derive(Debug)]
pub(super) struct ExternalFreshnessState {
    config: ExternalFreshnessConfig,
    current: Option<VerifiedFreshnessCut>,
    authority: Box<dyn ExternalFreshnessAuthority>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct FreshnessGenerationMaterial {
    pub(super) generation: u64,
    pub(super) binding: DurableExternalFreshnessBinding,
    pub(super) generation_digest: AuthorityDigest,
}

#[derive(Debug, Clone)]
pub(super) struct WalFreshnessSource {
    pub(super) path: PathBuf,
    pub(super) start_offset: u64,
    pub(super) end_offset: u64,
    pub(super) first_lsn: u64,
}

#[derive(Debug, Clone)]
pub(super) struct FreshnessRecoveryMaterial {
    pub(super) generation: FreshnessGenerationMaterial,
    pub(super) wal: WalFreshnessSource,
}

impl WalFreshnessSource {
    fn head(&self) -> Result<(u64, AuthorityDigest), DurabilityError> {
        wal_freshness_prefix(self, None)
    }

    fn digest_at_lsn(&self, target_lsn: u64) -> Result<AuthorityDigest, DurabilityError> {
        if target_lsn < self.first_lsn {
            return Ok(wal_prefix_digest(&[]));
        }
        wal_freshness_prefix(self, Some(target_lsn)).map(|(_, digest)| digest)
    }
}

impl ExternalFreshnessState {
    fn unanchored(
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Self {
        Self {
            config,
            current: None,
            authority,
        }
    }

    pub(super) fn anchored(
        config: ExternalFreshnessConfig,
        current: VerifiedFreshnessCut,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Self {
        Self {
            config,
            current: Some(current),
            authority,
        }
    }

    pub(super) fn recover_preflight(
        local: &FreshnessRecoveryMaterial,
        config: ExternalFreshnessConfig,
        mut authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<(Self, Option<PendingExternalFreshnessAdvance>), DurabilityError> {
        let signed = authority
            .read_signed(config.store_id)?
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority has no record for durable store",
            })?;
        let current = verify_freshness_cut(&config.trust_roots, &signed).map_err(|_| {
            DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness record authentication failed",
            }
        })?;
        if current.cut.store_id != config.store_id
            || current.cut.deployment_policy_epoch != config.deployment_policy_epoch
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness policy identity mismatch",
            });
        }
        let next = preflight_external_freshness(local, &config, current)?;
        let pending = (next != current.cut).then_some(PendingExternalFreshnessAdvance {
            expected_record: current.record_digest,
            next,
        });
        Ok((Self::anchored(config, current, authority), pending))
    }

    pub(super) fn complete_recovery_advance(
        &mut self,
        pending: Option<PendingExternalFreshnessAdvance>,
    ) -> Result<(), DurabilityError> {
        let Some(pending) = pending else {
            return Ok(());
        };
        let advanced = self
            .authority
            .compare_and_advance_signed(Some(pending.expected_record), pending.next)?;
        self.current = Some(verify_returned_freshness_cut(
            &self.config,
            pending.next,
            &advanced,
        )?);
        Ok(())
    }

    pub(super) fn advance(
        &mut self,
        material: FreshnessGenerationMaterial,
        wal_lsn: u64,
        wal_digest: AuthorityDigest,
    ) -> Result<(), DurabilityError> {
        advance_external_freshness_state(material, wal_lsn, wal_digest, self)
    }

    pub(super) fn metadata_binding(&self) -> DurableExternalFreshnessBinding {
        DurableExternalFreshnessBinding {
            store_id: self.config.store_id,
            previous_generation_digest: self.current.map(|current| current.cut.generation_digest),
            trust_root_epoch: self.config.trust_roots.epoch(),
            deployment_policy_epoch: self.config.deployment_policy_epoch,
        }
    }
}

fn verify_returned_freshness_cut(
    config: &ExternalFreshnessConfig,
    expected: FreshnessCut,
    signed: &SignedFreshnessCut,
) -> Result<VerifiedFreshnessCut, DurabilityError> {
    let verified = verify_freshness_cut(&config.trust_roots, signed).map_err(|_| {
        DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority returned an invalid signature",
        }
    })?;
    if verified.cut != expected
        || verified.cut.store_id != config.store_id
        || verified.cut.deployment_policy_epoch != config.deployment_policy_epoch
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority signed another cut",
        });
    }
    Ok(verified)
}

fn preflight_external_freshness(
    local: &FreshnessRecoveryMaterial,
    config: &ExternalFreshnessConfig,
    anchored: VerifiedFreshnessCut,
) -> Result<FreshnessCut, DurabilityError> {
    let generation = local.generation.generation;
    let binding = local.generation.binding;
    if binding.store_id != config.store_id
        || binding.trust_root_epoch != config.trust_roots.epoch()
        || binding.deployment_policy_epoch != config.deployment_policy_epoch
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "published external freshness binding is stale or belongs to another store",
        });
    }
    if generation < anchored.cut.generation {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local durable generation was rolled back behind external freshness authority",
        });
    }
    if generation > anchored.cut.generation.saturating_add(1) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local durable generation jumped beyond external freshness authority",
        });
    }
    if generation == anchored.cut.generation.saturating_add(1)
        && binding.previous_generation_digest != Some(anchored.cut.generation_digest)
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local generation does not extend externally anchored predecessor",
        });
    }
    let generation_digest = local.generation.generation_digest;
    if generation == anchored.cut.generation && generation_digest != anchored.cut.generation_digest
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "same-generation durable store fork detected by external freshness authority",
        });
    }
    let (wal_lsn, wal_digest) = local.wal.head()?;
    if generation == anchored.cut.generation {
        if wal_lsn < anchored.cut.wal_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "local WAL was truncated behind external freshness authority",
            });
        }
        let anchored_prefix = local.wal.digest_at_lsn(anchored.cut.wal_lsn)?;
        if anchored_prefix != anchored.cut.wal_digest {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "local WAL prefix forks from external freshness authority",
            });
        }
    }
    Ok(FreshnessCut {
        store_id: config.store_id,
        generation,
        previous_generation: binding.previous_generation_digest,
        generation_digest,
        wal_lsn,
        wal_digest,
        trust_root_epoch: config.trust_roots.epoch(),
        deployment_policy_epoch: config.deployment_policy_epoch,
    })
}

fn advance_external_freshness_state(
    material: FreshnessGenerationMaterial,
    wal_lsn: u64,
    wal_digest: AuthorityDigest,
    state: &mut ExternalFreshnessState,
) -> Result<(), DurabilityError> {
    let generation = material.generation;
    let binding = material.binding;
    let generation_digest = state.current.map_or(material.generation_digest, |current| {
        if current.cut.generation == generation {
            current.cut.generation_digest
        } else {
            material.generation_digest
        }
    });
    let cut = FreshnessCut {
        store_id: state.config.store_id,
        generation,
        previous_generation: binding.previous_generation_digest,
        generation_digest,
        wal_lsn,
        wal_digest,
        trust_root_epoch: state.config.trust_roots.epoch(),
        deployment_policy_epoch: state.config.deployment_policy_epoch,
    };
    let expected = state.current.map(|current| current.record_digest);
    let signed = state.authority.compare_and_advance_signed(expected, cut)?;
    state.current = Some(verify_returned_freshness_cut(&state.config, cut, &signed)?);
    Ok(())
}

fn sha256_file(path: &Path) -> Result<Sha256Digest, DurabilityError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Sha256Digest(hasher.finalize().into()))
}

pub(super) fn generation_material_digest(
    directory: &Path,
    generation: u64,
) -> Result<AuthorityDigest, DurabilityError> {
    let checkpoint = checkpoint_path(directory, generation);
    let checkpoint_root = read_checkpoint_root_bounded(&checkpoint)?;
    let mut material = Vec::new();
    material.extend_from_slice(b"CFMD-GENERATION-MATERIAL-v1\0");
    for path in [
        manifest_path(directory, generation),
        checkpoint,
        metadata_path(directory, generation),
    ] {
        material.extend_from_slice(&sha256_file(&path)?.0);
    }
    let prepared = prepared_capsule_path(directory, generation);
    if prepared.is_file() {
        material.push(1);
        material.extend_from_slice(&sha256_file(&prepared)?.0);
    } else {
        material.push(0);
    }

    for ordinal in checkpoint_chunk_ordinals(&checkpoint_root)? {
        material.extend_from_slice(
            &sha256_file(&checkpoint_chunk_path(directory, generation, ordinal))?.0,
        );
    }
    Ok(AuthorityDigest(sha256(&material).0))
}

fn checkpoint_chunk_ordinals(root: &[u8]) -> Result<std::ops::Range<usize>, DurabilityError> {
    if root.len() < CHECKPOINT_HEADER_LEN {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint header truncated while deriving generation authority",
        });
    }
    if root[..4] != CHECKPOINT_MAGIC {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint magic mismatch while deriving generation authority",
        });
    }
    let version = read_u16(&root[4..6]);
    if version == LEGACY_CHECKPOINT_FORMAT_VERSION {
        return Ok(0..0);
    }
    DurableFormatRegistry::require_checkpoint_current(version)?;
    if crc32c(&root[..28]) != read_u32(&root[28..32]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root header mismatch while deriving generation authority",
        });
    }
    let chunk_count = usize::from(read_u16(&root[6..8]));
    let descriptor_len = chunk_count
        .checked_mul(CHECKPOINT_CHUNK_DESCRIPTOR_LEN)
        .ok_or(DurabilityError::PayloadTooLarge)?;
    if root.len() != CHECKPOINT_HEADER_LEN + descriptor_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root descriptor length mismatch while deriving generation authority",
        });
    }
    let descriptors = &root[CHECKPOINT_HEADER_LEN..];
    if crc32c(descriptors) != read_u32(&root[24..28]) {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "checkpoint root descriptor checksum mismatch while deriving generation authority",
        });
    }
    let logical_len =
        usize::try_from(read_u64(&root[8..16])).map_err(|_| DurabilityError::PayloadTooLarge)?;
    validate_checkpoint_chunk_descriptors(descriptors, chunk_count, logical_len)?;
    Ok(0..chunk_count)
}

fn wal_freshness_prefix(
    source: &WalFreshnessSource,
    target_lsn: Option<u64>,
) -> Result<(u64, AuthorityDigest), DurabilityError> {
    if source.first_lsn == 0 {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "WAL first LSN must be nonzero",
        });
    }
    if source.end_offset < source.start_offset {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "external freshness WAL region is inverted",
        });
    }
    let mut file = File::open(&source.path)?;
    let file_len = file.metadata()?.len();
    if source.end_offset > file_len {
        return Err(DurabilityError::Corruption {
            offset: 0,
            reason: "external freshness WAL region exceeds file length",
        });
    }
    file.seek(SeekFrom::Start(source.start_offset))?;
    let mut hasher = Sha256::new();
    hasher.update(WAL_FRESHNESS_PREFIX_DOMAIN);
    let mut offset = 0_u64;
    let mut expected_lsn = source.first_lsn;
    let region_len = source.end_offset - source.start_offset;

    while offset < region_len {
        let remaining = region_len - offset;
        if remaining < u64::try_from(HEADER_LEN).expect("WAL header length fits u64") {
            break;
        }
        let offset_usize = usize::try_from(offset).map_err(|_| DurabilityError::PayloadTooLarge)?;
        let mut header = [0_u8; HEADER_LEN];
        file.read_exact(&mut header)?;
        if header[..4] != MAGIC {
            return Err(DurabilityError::Corruption {
                offset: offset_usize,
                reason: "non-frame bytes large enough to hide a complete frame",
            });
        }
        validate_frame_header(&header, offset_usize, expected_lsn)?;
        let payload_len =
            usize::try_from(read_u32(&header[8..12])).map_err(|_| DurabilityError::Corruption {
                offset: offset_usize,
                reason: "payload length overflow",
            })?;
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(DurabilityError::Corruption {
                offset: offset_usize,
                reason: "payload length exceeds hard limit",
            });
        }
        let frame_len = HEADER_LEN
            .checked_add(payload_len)
            .ok_or(DurabilityError::Corruption {
                offset: offset_usize,
                reason: "frame length overflow",
            })?;
        if remaining < u64::try_from(frame_len).map_err(|_| DurabilityError::PayloadTooLarge)? {
            break;
        }
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(payload_len)
            .map_err(|_| DurabilityError::PayloadTooLarge)?;
        payload.resize(payload_len, 0);
        file.read_exact(&mut payload)?;
        if crc32c(&payload) != read_u32(&header[28..32]) {
            return Err(DurabilityError::Corruption {
                offset: offset_usize,
                reason: "payload checksum mismatch",
            });
        }
        hasher.update(header);
        hasher.update(&payload);
        offset = offset
            .checked_add(u64::try_from(frame_len).map_err(|_| DurabilityError::PayloadTooLarge)?)
            .ok_or(DurabilityError::PayloadTooLarge)?;
        let lsn = read_u64(&header[12..20]);
        if target_lsn == Some(lsn) {
            return Ok((lsn, AuthorityDigest(hasher.finalize().into())));
        }
        expected_lsn = expected_lsn
            .checked_add(1)
            .ok_or(DurabilityError::LsnExhausted)?;
    }

    if target_lsn.is_some() {
        return Err(DurabilityError::Protocol {
            offset: usize::try_from(offset).unwrap_or(usize::MAX),
            reason: "external freshness WAL head is beyond valid local WAL prefix",
        });
    }
    Ok((
        expected_lsn.saturating_sub(1),
        AuthorityDigest(hasher.finalize().into()),
    ))
}

fn wal_prefix_digest(bytes: &[u8]) -> AuthorityDigest {
    let mut hasher = Sha256::new();
    hasher.update(WAL_FRESHNESS_PREFIX_DOMAIN);
    hasher.update(bytes);
    AuthorityDigest(hasher.finalize().into())
}

impl DurableRevisionStore {
    pub(super) fn advance_external_freshness_wal(&mut self) -> Result<(), DurabilityError> {
        self.advance_external_freshness_generation_with_digest(
            self.generation,
            self.wal.last_lsn(),
            self.wal.freshness_digest(),
        )
    }

    pub(super) fn advance_external_freshness_generation_with_digest(
        &mut self,
        generation: u64,
        wal_lsn: u64,
        wal_digest: AuthorityDigest,
    ) -> Result<(), DurabilityError> {
        let Some(mut freshness) = self.external_freshness.take() else {
            return Ok(());
        };
        let material = self.backend.freshness_generation_material(generation);
        let result = material.and_then(|material| freshness.advance(material, wal_lsn, wal_digest));
        self.external_freshness = Some(freshness);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub fn adopt_external_freshness(
        &mut self,
        config: ExternalFreshnessConfig,
        mut authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<DurableGenerationReceipt, DurabilityError> {
        if !self.backend.capabilities().external_freshness {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durability backend does not support external freshness",
            });
        }
        if self.external_freshness.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority is already active",
            });
        }
        if authority.read_signed(config.store_id)?.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority already contains this store id",
            });
        }
        self.external_freshness = Some(ExternalFreshnessState::unanchored(config, authority));
        let result = self.rotate_checkpoint(&self.checkpoint.clone());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
