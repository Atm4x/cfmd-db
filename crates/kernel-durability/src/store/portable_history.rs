use std::collections::BTreeMap;

use kernel_change::RevisionEffectId;
use kernel_revision::Revision;
use kernel_types::{ClientTransactionId, RevisionId};

use crate::binary_codec::{Cursor, push_len, push_u32, push_u64, push_u128};
use crate::checkpoint;
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::{DurableTransactionIntent, DurableTransactionKey, IdempotencyEpoch};
use crate::metadata;
use crate::runtime::{
    CommittedRevision, DurabilityError, RecoveredAuthorityState, RecoveryScan, TailStatus,
};
use crate::wal_payload::{decode_prepare_payload, encode_prepare_payload};

use super::DurableRevisionStore;
use super::recovery::{HistoricalEpochMaterial, rebuild_semantic_registry};

const PORTABLE_HISTORY_TAG: u32 = 0x4350_4831; // CPH1, pre-release only.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PortableHistoricalEpochClosure {
    pub effect_id: RevisionEffectId,
    pub generation: u64,
    pub metadata: metadata::DurableStoreMetadata,
    pub checkpoint: Revision,
    pub recovery_scan: RecoveryScan,
}

impl PortableHistoricalEpochClosure {
    pub(super) fn from_material(
        effect_id: RevisionEffectId,
        material: &HistoricalEpochMaterial,
    ) -> Self {
        Self {
            effect_id,
            generation: material.generation(),
            metadata: material.metadata().clone(),
            checkpoint: material.checkpoint().clone(),
            recovery_scan: material.recovery_scan().clone(),
        }
    }

    pub(super) fn material(&self) -> Result<HistoricalEpochMaterial, DurabilityError> {
        let registry = rebuild_semantic_registry(&self.metadata)?;
        Ok(HistoricalEpochMaterial::portable(
            self.generation,
            self.checkpoint.clone(),
            self.recovery_scan.clone(),
            registry,
            self.metadata.clone(),
        ))
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, DurabilityError> {
        let metadata = metadata::encode(&self.metadata)?;
        let checkpoint = checkpoint::encode_revision(&self.checkpoint)?;
        let mut out = Vec::new();
        push_u32(&mut out, PORTABLE_HISTORY_TAG);
        push_u128(&mut out, self.effect_id.0);
        push_u64(&mut out, self.generation);
        push_bytes(&mut out, &metadata)?;
        push_bytes(&mut out, &checkpoint)?;
        encode_scan(&mut out, &self.recovery_scan)?;
        Ok(out)
    }

    pub(super) fn decode(bytes: &[u8]) -> Result<Self, DurabilityError> {
        let mut cursor = Cursor::new(bytes);
        if cursor.u32().map_err(codec_corruption)? != PORTABLE_HISTORY_TAG {
            return Err(corruption("unsupported portable historical closure format"));
        }
        let effect_id = RevisionEffectId(cursor.u128().map_err(codec_corruption)?);
        let generation = cursor.u64().map_err(codec_corruption)?;
        let metadata_bytes = take_bytes(&mut cursor)?;
        let metadata = metadata::decode(metadata_bytes).map_err(codec_corruption)?;
        let registry = rebuild_semantic_registry(&metadata)?;
        let checkpoint_bytes = take_bytes(&mut cursor)?;
        let checkpoint = checkpoint::decode_revision(checkpoint_bytes, &registry)?;
        let recovery_scan = decode_scan(&mut cursor)?;
        cursor.finish().map_err(codec_corruption)?;
        Ok(Self {
            effect_id,
            generation,
            metadata,
            checkpoint,
            recovery_scan,
        })
    }
}

fn push_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), DurabilityError> {
    push_len(out, bytes.len())?;
    out.extend_from_slice(bytes);
    Ok(())
}

fn take_bytes<'a>(cursor: &mut Cursor<'a>) -> Result<&'a [u8], DurabilityError> {
    let len = cursor.len().map_err(codec_corruption)?;
    cursor.take(len).map_err(codec_corruption)
}

fn encode_scan(out: &mut Vec<u8>, scan: &RecoveryScan) -> Result<(), DurabilityError> {
    push_u64(out, scan.base_revision().raw());
    push_u64(out, scan.next_lsn());
    push_len(out, scan.committed().len())?;
    for committed in scan.committed() {
        push_u64(out, committed.descriptor.target_revision.raw());
        let descriptor = encode_prepare_payload(&committed.descriptor)?;
        push_bytes(out, &descriptor)?;
        push_u64(out, committed.prepare_lsn);
        out.extend_from_slice(&committed.prepare_payload_crc32c.to_le_bytes());
        push_u64(out, committed.commit_lsn);
    }
    push_len(out, scan.committed_transactions().len())?;
    for (key, committed) in scan.committed_transactions() {
        push_u64(out, key.epoch.raw());
        push_u128(out, key.transaction_id.raw());
        let mut encoded = Vec::new();
        metadata::encode_committed_transaction(&mut encoded, committed)?;
        push_bytes(out, &encoded)?;
    }
    push_len(out, scan.unresolved_prepares().len())?;
    for (lsn, descriptor, crc) in scan.unresolved_prepares() {
        push_u64(out, *lsn);
        out.extend_from_slice(&crc.to_le_bytes());
        push_u64(out, descriptor.target_revision.raw());
        let descriptor = encode_prepare_payload(descriptor)?;
        push_bytes(out, &descriptor)?;
    }
    push_len(out, scan.replication_authority_frames().len())?;
    for frame in scan.replication_authority_frames() {
        push_bytes(out, frame)?;
    }
    Ok(())
}

fn decode_scan(cursor: &mut Cursor<'_>) -> Result<RecoveryScan, DurabilityError> {
    let base_revision = RevisionId::new(cursor.u64().map_err(codec_corruption)?);
    let next_lsn = cursor.u64().map_err(codec_corruption)?;
    let committed_len = cursor.len().map_err(codec_corruption)?;
    let mut committed = Vec::with_capacity(cursor.bounded_capacity(committed_len));
    for _ in 0..committed_len {
        let target = RevisionId::new(cursor.u64().map_err(codec_corruption)?);
        let descriptor_bytes = take_bytes(cursor)?;
        let descriptor =
            decode_prepare_payload(target, descriptor_bytes).map_err(codec_corruption)?;
        let prepare_lsn = cursor.u64().map_err(codec_corruption)?;
        let prepare_payload_crc32c = cursor.u32().map_err(codec_corruption)?;
        let commit_lsn = cursor.u64().map_err(codec_corruption)?;
        committed.push(CommittedRevision {
            descriptor,
            prepare_lsn,
            prepare_payload_crc32c,
            commit_lsn,
        });
    }
    let tx_len = cursor.len().map_err(codec_corruption)?;
    let mut committed_transactions = BTreeMap::new();
    for _ in 0..tx_len {
        let epoch = IdempotencyEpoch::new(cursor.u64().map_err(codec_corruption)?);
        let transaction_id = ClientTransactionId::new(cursor.u128().map_err(codec_corruption)?);
        let bytes = take_bytes(cursor)?;
        let mut tx_cursor = Cursor::new(bytes);
        let committed_tx =
            metadata::decode_committed_transaction(&mut tx_cursor).map_err(codec_corruption)?;
        tx_cursor.finish().map_err(codec_corruption)?;
        committed_transactions.insert(
            DurableTransactionKey::new(epoch, transaction_id),
            committed_tx,
        );
    }
    let unresolved_len = cursor.len().map_err(codec_corruption)?;
    let mut unresolved_prepares = Vec::with_capacity(cursor.bounded_capacity(unresolved_len));
    for _ in 0..unresolved_len {
        let lsn = cursor.u64().map_err(codec_corruption)?;
        let crc = cursor.u32().map_err(codec_corruption)?;
        let target = RevisionId::new(cursor.u64().map_err(codec_corruption)?);
        let bytes = take_bytes(cursor)?;
        let descriptor = decode_prepare_payload(target, bytes).map_err(codec_corruption)?;
        unresolved_prepares.push((lsn, descriptor, crc));
    }
    let replication_len = cursor.len().map_err(codec_corruption)?;
    let mut replication_authority_frames =
        Vec::with_capacity(cursor.bounded_capacity(replication_len));
    for _ in 0..replication_len {
        replication_authority_frames.push(take_bytes(cursor)?.to_vec());
    }
    Ok(RecoveryScan::recovered(
        base_revision,
        committed,
        0,
        next_lsn,
        TailStatus::Clean,
        RecoveredAuthorityState {
            committed_transactions,
            unresolved_prepares,
            replication_authority_frames,
        },
    ))
}

fn codec_corruption(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

fn corruption(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

impl DurableRevisionStore {
    /// Makes every retained historical epoch self-contained before the physical
    /// durable backend is retired. A volatile owner cannot depend on directory
    /// or single-file bytes that cease to be part of its authority.
    pub(super) fn materialize_retained_history_for_volatile(
        &mut self,
    ) -> Result<(), DurabilityError> {
        let missing = self
            .historical_epoch_anchors
            .keys()
            .copied()
            .filter(|effect_id| !self.portable_historical_epochs.contains_key(effect_id))
            .collect::<Vec<_>>();
        for effect_id in missing {
            let material = self
                .historical_epoch_material(effect_id)?
                .ok_or(DurabilityError::Protocol {
                    offset: 0,
                    reason: "retained historical epoch has no materialization authority before volatile transition",
                })?;
            let closure = PortableHistoricalEpochClosure::from_material(effect_id, &material);
            self.portable_historical_epochs
                .insert(effect_id, closure.encode()?);
        }
        Ok(())
    }

    fn portable_history_metadata_snapshot(&self) -> metadata::DurableStoreMetadata {
        metadata::DurableStoreMetadata {
            external_freshness: self
                .external_freshness
                .as_ref()
                .map(super::freshness::ExternalFreshnessState::metadata_binding),
            current_idempotency_epoch: self.current_idempotency_epoch,
            minimum_retry_epoch: self.minimum_retry_epoch,
            materializations: self.materialization_specs.clone(),
            physical_artifacts: self.physical_artifact_specs.clone(),
            artifact_cores: self.artifact_cores.clone(),
            migration_complements: self.migration_complements.clone(),
            historical_epoch_anchors: self.historical_epoch_anchors.clone(),
            committed_transactions: self.committed_transactions.clone(),
            semantic_modules: self.semantic_registry.builtin_module_specs(),
            next_revision_effect_id: self.next_revision_effect_id,
            causal_coverage_root: Some(self.causal_coverage_root),
            revision_effects: self.revision_effects.clone(),
            revision_effect_frontiers: self.revision_effect_frontiers.clone(),
            checkpoint_realization: None,
        }
    }

    /// Captures source-independent retained migration history directly from a
    /// volatile authority cut. The same portable closure is later consumed by
    /// single-file staging; no filesystem generation is invented for RAM mode.
    pub(super) fn capture_volatile_historical_epochs(
        &mut self,
        descriptors: &[DurableRevisionDescriptor],
    ) -> Result<(), DurabilityError> {
        if !self.backend.is_volatile() {
            return Ok(());
        }
        let migration_effects = descriptors
            .iter()
            .filter(|descriptor| {
                matches!(
                    descriptor.intent,
                    DurableTransactionIntent::SchemaMigration { .. }
                )
            })
            .map(|descriptor| {
                descriptor
                    .revision_effect_id
                    .map(|effect_id| (effect_id, descriptor.source_revision))
                    .ok_or(DurabilityError::Protocol {
                        offset: 0,
                        reason: "volatile migration descriptor has no revision effect id",
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if migration_effects.is_empty() {
            return Ok(());
        }

        let scan = self
            .wal
            .volatile_recovery_scan(self.checkpoint.id())?
            .ok_or(DurabilityError::Protocol {
                offset: 0,
                reason: "volatile historical capture requires volatile WAL authority",
            })?;
        let metadata = self.portable_history_metadata_snapshot();
        for (effect_id, source_revision) in migration_effects {
            let source_present = source_revision == self.checkpoint.id()
                || scan
                    .committed()
                    .iter()
                    .any(|committed| committed.descriptor.target_revision == source_revision);
            if !source_present {
                return Err(DurabilityError::Protocol {
                    offset: 0,
                    reason: "volatile historical closure does not contain migration source revision",
                });
            }
            let closure = PortableHistoricalEpochClosure {
                effect_id,
                generation: self.generation,
                metadata: metadata.clone(),
                checkpoint: self.checkpoint.clone(),
                recovery_scan: scan.clone(),
            };
            self.portable_historical_epochs
                .insert(effect_id, closure.encode()?);
        }
        Ok(())
    }
}
