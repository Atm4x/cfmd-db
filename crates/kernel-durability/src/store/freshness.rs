use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use kernel_auth::{
    AuthorityDigest, FreshnessAuthorityState, FreshnessCut, Sha256Digest, SignedFreshnessAuthority,
    TrustRootSet, VerifiedFreshnessAuthority, VolatileFence, freshness_authority_record_digest,
    sha256, verify_freshness_authority, volatile_fence_nonce,
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
use super::format_registry::DurableFormatRegistry;
use super::generation_layout::{
    checkpoint_chunk_path, checkpoint_path, manifest_path, metadata_path, prepared_capsule_path,
    realization_path,
};
use super::{DurableGenerationReceipt, DurableRevisionStore};
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFreshnessConfig {
    pub store_id: [u8; 32],
    pub trust_roots: TrustRootSet,
    pub deployment_policy_epoch: u64,
}

impl ExternalFreshnessConfig {
    pub fn bootstrap(
        store_id: [u8; 32],
        trust_root_epoch: u64,
        verifying_keys: &[[u8; 32]],
        deployment_policy_epoch: u64,
    ) -> Result<Self, DurabilityError> {
        let trust_roots =
            TrustRootSet::bootstrap(trust_root_epoch, verifying_keys).map_err(|_| {
                DurabilityError::Protocol {
                    offset: 0,
                    reason: "invalid external freshness trust-root configuration",
                }
            })?;
        Ok(Self {
            store_id,
            trust_roots,
            deployment_policy_epoch,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalFreshnessHandoffKind {
    DurableCut,
    VolatileFence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalFreshnessHandoff {
    pub source_store_id: [u8; 32],
    pub source_record_digest: AuthorityDigest,
    pub source_generation_digest: AuthorityDigest,
    pub trust_root_epoch: u64,
    pub deployment_policy_epoch: u64,
    pub kind: ExternalFreshnessHandoffKind,
}

pub trait ExternalFreshnessAuthority: std::fmt::Debug + Send {
    fn read_signed(
        &mut self,
        lineage_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessAuthority>, DurabilityError>;

    fn compare_and_set_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError>;

    fn compare_and_rebind_signed(
        &mut self,
        _source_lineage_id: [u8; 32],
        _expected_source_record: AuthorityDigest,
        _next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError> {
        Err(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority does not support atomic store rebind",
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PendingExternalFreshnessAdvance {
    expected_record: AuthorityDigest,
    next: FreshnessCut,
}

#[derive(Debug)]
pub(super) struct ExternalFreshnessState {
    config: ExternalFreshnessConfig,
    current: Option<VerifiedFreshnessAuthority>,
    initial_previous_generation_digest: Option<AuthorityDigest>,
    authority: Box<dyn ExternalFreshnessAuthority>,
}

#[derive(Debug)]
struct ExternalFreshnessRebindAuthority {
    inner: Box<dyn ExternalFreshnessAuthority>,
    source: ExternalFreshnessHandoff,
    armed: bool,
}

#[derive(Debug)]
struct ExternalFreshnessBootstrapAuthority {
    inner: Box<dyn ExternalFreshnessAuthority>,
    armed: bool,
}

impl ExternalFreshnessAuthority for ExternalFreshnessBootstrapAuthority {
    fn read_signed(
        &mut self,
        lineage_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessAuthority>, DurabilityError> {
        self.inner.read_signed(lineage_id)
    }

    fn compare_and_set_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError> {
        if self.armed {
            let FreshnessAuthorityState::DurableCut(cut) = next else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness bootstrap must publish a durable root",
                });
            };
            if expected_record.is_some() || cut.previous_generation.is_some() {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness bootstrap must publish a fresh root",
                });
            }
            match self.inner.compare_and_set_signed(None, next) {
                Ok(signed) => {
                    self.armed = false;
                    return Ok(signed);
                }
                Err(error) => {
                    if let Some(target) = self.inner.read_signed(cut.store_id)? {
                        self.armed = false;
                        return Ok(target);
                    }
                    return Err(error);
                }
            }
        }
        self.inner.compare_and_set_signed(expected_record, next)
    }

    fn compare_and_rebind_signed(
        &mut self,
        source_lineage_id: [u8; 32],
        expected_source_record: AuthorityDigest,
        next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError> {
        self.inner
            .compare_and_rebind_signed(source_lineage_id, expected_source_record, next)
    }
}

impl ExternalFreshnessAuthority for ExternalFreshnessRebindAuthority {
    fn read_signed(
        &mut self,
        lineage_id: [u8; 32],
    ) -> Result<Option<SignedFreshnessAuthority>, DurabilityError> {
        self.inner.read_signed(lineage_id)
    }

    fn compare_and_set_signed(
        &mut self,
        expected_record: Option<AuthorityDigest>,
        next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError> {
        if self.armed {
            let FreshnessAuthorityState::DurableCut(cut) = next else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness rebind target must be durable",
                });
            };
            if expected_record.is_some()
                || cut.previous_generation != Some(self.source.source_generation_digest)
            {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness rebind target does not extend source trust cut",
                });
            }
            match self.inner.compare_and_rebind_signed(
                self.source.source_store_id,
                self.source.source_record_digest,
                next,
            ) {
                Ok(signed) => {
                    self.armed = false;
                    return Ok(signed);
                }
                Err(error) => {
                    let source = self.inner.read_signed(self.source.source_store_id)?;
                    let target = self.inner.read_signed(cut.store_id)?;
                    let source_is_original = source.as_ref().is_some_and(|record| {
                        freshness_authority_record_digest(record)
                            == self.source.source_record_digest
                    });
                    if !source_is_original && let Some(target) = target {
                        self.armed = false;
                        return Ok(target);
                    }
                    if source_is_original && target.is_none() {
                        return Err(error);
                    }
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "external freshness rebind outcome is ambiguous",
                    });
                }
            }
        }
        self.inner.compare_and_set_signed(expected_record, next)
    }

    fn compare_and_rebind_signed(
        &mut self,
        source_lineage_id: [u8; 32],
        expected_source_record: AuthorityDigest,
        next: FreshnessAuthorityState,
    ) -> Result<SignedFreshnessAuthority, DurabilityError> {
        self.inner
            .compare_and_rebind_signed(source_lineage_id, expected_source_record, next)
    }
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
            initial_previous_generation_digest: None,
            authority,
        }
    }

    pub(super) fn prepare_bootstrap_target(
        config: ExternalFreshnessConfig,
        mut authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<Self, DurabilityError> {
        if authority.read_signed(config.store_id)?.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness bootstrap target store id already exists",
            });
        }
        Ok(Self::unanchored(
            config,
            Box::new(ExternalFreshnessBootstrapAuthority {
                inner: authority,
                armed: true,
            }),
        ))
    }

    pub(super) fn anchored(
        config: ExternalFreshnessConfig,
        current: VerifiedFreshnessAuthority,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Self {
        Self {
            config,
            current: Some(current),
            initial_previous_generation_digest: None,
            authority,
        }
    }

    fn rebind_target(
        config: ExternalFreshnessConfig,
        source: ExternalFreshnessHandoff,
        authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Self {
        Self {
            config,
            current: None,
            initial_previous_generation_digest: Some(source.source_generation_digest),
            authority: Box::new(ExternalFreshnessRebindAuthority {
                inner: authority,
                source,
                armed: true,
            }),
        }
    }

    pub(super) fn prepare_rebind_target(
        config: ExternalFreshnessConfig,
        source: ExternalFreshnessHandoff,
        mut authority: Box<dyn ExternalFreshnessAuthority>,
    ) -> Result<Self, DurabilityError> {
        if config.store_id == source.source_store_id {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness rebind requires a distinct target store id",
            });
        }
        if source.kind != ExternalFreshnessHandoffKind::DurableCut {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness rebind requires a durable source cut",
            });
        }
        if config.trust_roots.epoch() != source.trust_root_epoch
            || config.deployment_policy_epoch != source.deployment_policy_epoch
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness rebind changed trust or deployment policy epoch",
            });
        }
        if authority.read_signed(config.store_id)?.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness rebind target store id already exists",
            });
        }
        let source_signed =
            authority
                .read_signed(source.source_store_id)?
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness rebind source trust cut is missing",
                })?;
        let verified =
            verify_freshness_authority(&config.trust_roots, &source_signed).map_err(|_| {
                DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness rebind source authentication failed",
                }
            })?;
        let verified_cut = verified.durable_cut().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness rebind source is not a durable cut",
        })?;
        if verified.record_digest != source.source_record_digest
            || verified_cut.store_id != source.source_store_id
            || verified_cut.generation_digest != source.source_generation_digest
            || verified_cut.deployment_policy_epoch != source.deployment_policy_epoch
            || freshness_authority_record_digest(&source_signed) != source.source_record_digest
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness rebind source trust cut changed",
            });
        }
        Ok(Self::rebind_target(config, source, authority))
    }

    pub(super) fn handoff(&self) -> Result<ExternalFreshnessHandoff, DurabilityError> {
        let current = self.current.ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority has no committed lineage state for handoff",
        })?;
        let (source_generation_digest, kind) = match current.state {
            FreshnessAuthorityState::DurableCut(cut) => (
                cut.generation_digest,
                ExternalFreshnessHandoffKind::DurableCut,
            ),
            FreshnessAuthorityState::VolatileFence(fence) => (
                fence.source_generation_digest,
                ExternalFreshnessHandoffKind::VolatileFence,
            ),
        };
        Ok(ExternalFreshnessHandoff {
            source_store_id: self.config.store_id,
            source_record_digest: current.record_digest,
            source_generation_digest,
            trust_root_epoch: self.config.trust_roots.epoch(),
            deployment_policy_epoch: self.config.deployment_policy_epoch,
            kind,
        })
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
        let current = verify_freshness_authority(&config.trust_roots, &signed).map_err(|_| {
            DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness record authentication failed",
            }
        })?;
        let current_cut = current.durable_cut().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "durable source was retired by an external volatile fence",
        })?;
        if current_cut.store_id != config.store_id
            || current_cut.deployment_policy_epoch != config.deployment_policy_epoch
        {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness policy identity mismatch",
            });
        }
        let next = preflight_external_freshness(local, &config, current)?;
        let pending = (next != current_cut).then_some(PendingExternalFreshnessAdvance {
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
        let advanced = self.authority.compare_and_set_signed(
            Some(pending.expected_record),
            FreshnessAuthorityState::DurableCut(pending.next),
        )?;
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

    pub(super) fn is_volatile_fence(&self) -> bool {
        self.current.is_some_and(|current| {
            matches!(current.state, FreshnessAuthorityState::VolatileFence(_))
        })
    }

    pub(super) fn remote_matches_current(&mut self) -> Result<bool, DurabilityError> {
        let Some(current) = self.current else {
            return Ok(false);
        };
        let Some(observed) = self.authority.read_signed(self.config.store_id)? else {
            return Ok(false);
        };
        let verified = verify_freshness_authority(&self.config.trust_roots, &observed).map_err(
            |_| DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority returned an unauthenticated lineage state",
            },
        )?;
        Ok(verified.record_digest == current.record_digest && verified.state == current.state)
    }

    pub(super) fn publish_volatile_fence(&mut self) -> Result<(), DurabilityError> {
        let current = self.current.ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority has no durable cut to fence",
        })?;
        let cut = current.durable_cut().ok_or(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority is already volatile-fenced",
        })?;
        let fence = VolatileFence {
            lineage_id: self.config.store_id,
            predecessor_record_digest: current.record_digest,
            source_generation_digest: cut.generation_digest,
            trust_root_epoch: self.config.trust_roots.epoch(),
            deployment_policy_epoch: self.config.deployment_policy_epoch,
            transition_nonce: volatile_fence_nonce(current.record_digest, cut.generation_digest),
        };
        let expected_state = FreshnessAuthorityState::VolatileFence(fence);
        let signed = match self
            .authority
            .compare_and_set_signed(Some(current.record_digest), expected_state)
        {
            Ok(signed) => signed,
            Err(error) => {
                let Some(observed) = self.authority.read_signed(self.config.store_id)? else {
                    return Err(error);
                };
                if observed.state == expected_state {
                    observed
                } else if freshness_authority_record_digest(&observed) == current.record_digest {
                    return Err(error);
                } else {
                    return Err(DurabilityError::Protocol {
                        offset: 0,
                        reason: "external volatile-fence CAS outcome is ambiguous",
                    });
                }
            }
        };
        let verified =
            verify_freshness_authority(&self.config.trust_roots, &signed).map_err(|_| {
                DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness authority returned an invalid volatile fence",
                }
            })?;
        if verified.state != expected_state {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "external freshness authority signed another volatile fence",
            });
        }
        self.current = Some(verified);
        Ok(())
    }

    pub(super) fn metadata_binding(&self) -> DurableExternalFreshnessBinding {
        let current_generation = self.current.map(|current| match current.state {
            FreshnessAuthorityState::DurableCut(cut) => cut.generation_digest,
            FreshnessAuthorityState::VolatileFence(fence) => fence.source_generation_digest,
        });
        DurableExternalFreshnessBinding {
            store_id: self.config.store_id,
            previous_generation_digest: current_generation
                .or(self.initial_previous_generation_digest),
            trust_root_epoch: self.config.trust_roots.epoch(),
            deployment_policy_epoch: self.config.deployment_policy_epoch,
        }
    }
}

fn verify_returned_freshness_cut(
    config: &ExternalFreshnessConfig,
    expected: FreshnessCut,
    signed: &SignedFreshnessAuthority,
) -> Result<VerifiedFreshnessAuthority, DurabilityError> {
    let verified = verify_freshness_authority(&config.trust_roots, signed).map_err(|_| {
        DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority returned an invalid signature",
        }
    })?;
    if verified.state != FreshnessAuthorityState::DurableCut(expected)
        || expected.store_id != config.store_id
        || expected.deployment_policy_epoch != config.deployment_policy_epoch
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "external freshness authority signed another durable cut",
        });
    }
    Ok(verified)
}

fn preflight_external_freshness(
    local: &FreshnessRecoveryMaterial,
    config: &ExternalFreshnessConfig,
    anchored: VerifiedFreshnessAuthority,
) -> Result<FreshnessCut, DurabilityError> {
    let anchored_cut = anchored.durable_cut().ok_or(DurabilityError::Protocol {
        offset: 0,
        reason: "durable source was retired by an external volatile fence",
    })?;
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
    if generation < anchored_cut.generation {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local durable generation was rolled back behind external freshness authority",
        });
    }
    if generation > anchored_cut.generation.saturating_add(1) {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local durable generation jumped beyond external freshness authority",
        });
    }
    if generation == anchored_cut.generation.saturating_add(1)
        && binding.previous_generation_digest != Some(anchored_cut.generation_digest)
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "local generation does not extend externally anchored predecessor",
        });
    }
    let generation_digest = local.generation.generation_digest;
    if generation == anchored_cut.generation && generation_digest != anchored_cut.generation_digest
    {
        return Err(DurabilityError::Protocol {
            offset: 0,
            reason: "same-generation durable store fork detected by external freshness authority",
        });
    }
    let (wal_lsn, wal_digest) = local.wal.head()?;
    if generation == anchored_cut.generation {
        if wal_lsn < anchored_cut.wal_lsn {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "local WAL was truncated behind external freshness authority",
            });
        }
        let anchored_prefix = local.wal.digest_at_lsn(anchored_cut.wal_lsn)?;
        if anchored_prefix != anchored_cut.wal_digest {
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
    let generation_digest =
        state
            .current
            .map_or(material.generation_digest, |current| match current.state {
                FreshnessAuthorityState::DurableCut(cut) if cut.generation == generation => {
                    cut.generation_digest
                }
                _ => material.generation_digest,
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
    let next_state = FreshnessAuthorityState::DurableCut(cut);
    let signed = match state.authority.compare_and_set_signed(expected, next_state) {
        Ok(signed) => signed,
        Err(error) => {
            let Some(observed) = state.authority.read_signed(state.config.store_id)? else {
                return Err(error);
            };
            if observed.state == next_state {
                observed
            } else if observed.state.lineage_id().eq(&state.config.store_id)
                && Some(freshness_authority_record_digest(&observed)) == expected
            {
                return Err(error);
            } else {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "external freshness CAS outcome is ambiguous",
                });
            }
        }
    };
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
    let realization = realization_path(directory, generation);
    if realization.is_file() {
        material.push(1);
        material.extend_from_slice(&sha256_file(&realization)?.0);
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
    #[must_use]
    pub const fn is_volatile(&self) -> bool {
        self.backend.is_volatile()
    }

    pub(super) fn advance_external_freshness_wal(&mut self) -> Result<(), DurabilityError> {
        if self.backend.is_volatile() {
            if self
                .external_freshness
                .as_ref()
                .is_some_and(ExternalFreshnessState::is_volatile_fence)
            {
                return Ok(());
            }
            if self.external_freshness.is_some() {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "volatile backend carries non-fenced external freshness authority",
                });
            }
            return Ok(());
        }
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
    pub fn demote_to_volatile(&mut self) -> Result<(), DurabilityError> {
        if self.poisoned {
            return Err(DurabilityError::Poisoned);
        }
        if self.backend.is_volatile() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "durability backend is already volatile",
            });
        }
        if self.streaming_checkpoint.is_some() {
            return Err(DurabilityError::Protocol {
                offset: 0,
                reason: "cannot demote while a streaming checkpoint is active",
            });
        }
        self.materialize_retained_history_for_volatile()?;
        if let Some(freshness) = self.external_freshness.as_mut() {
            freshness.publish_volatile_fence()?;
        }
        let protection_floor = self.backend.protection_profile();
        let next_lsn = self.wal.next_lsn();
        self.wal = crate::wal::RuntimeRevisionWal::volatile_at_lsn(next_lsn);
        self.backend = super::backend::DurabilityBackend::volatile(protection_floor);
        Ok(())
    }

    pub fn adopt_external_freshness(
        &mut self,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
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
        self.external_freshness = Some(ExternalFreshnessState::prepare_bootstrap_target(
            config, authority,
        )?);
        let result = self.rotate_checkpoint(&self.checkpoint.clone());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    pub fn adopt_external_freshness_rebind(
        &mut self,
        config: ExternalFreshnessConfig,
        authority: Box<dyn ExternalFreshnessAuthority>,
        source: ExternalFreshnessHandoff,
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
        self.external_freshness = Some(ExternalFreshnessState::prepare_rebind_target(
            config, source, authority,
        )?);
        let result = self.rotate_checkpoint(&self.checkpoint.clone());
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
