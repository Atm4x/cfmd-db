use std::collections::{BTreeMap, BTreeSet};

use kernel_change::RevisionEffectId;
use kernel_semantics::BuiltinSemanticModuleSpec;
use kernel_types::{ClientTransactionId, RevisionId};

use crate::binary_codec::{Cursor, push_len, push_u64, push_u128};
use crate::descriptor::{
    DurableArtifactCore, DurableMaterializationSpec, DurablePhysicalArtifactSpec,
};
use crate::domain::{
    DurableExternalFreshnessBinding, DurableMigrationComplement, DurableRevisionEffectRecord,
    DurableTransactionIntent, DurableTransactionKey, IdempotencyEpoch,
};
use crate::runtime::CodecError;

use super::artifact_codec::{
    decode_artifact_cores, decode_materialization_specs, decode_migration_complements,
    decode_physical_artifact_specs, encode_artifact_cores, encode_materialization_specs,
    encode_migration_complements, encode_physical_artifact_specs,
};
use super::intent_codec::{
    decode_semantic_module_spec, decode_transaction_intent, encode_semantic_module_spec,
    encode_transaction_intent,
};

const METADATA_CODEC_VERSION: u16 = 13;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct DurableStoreMetadata {
    pub external_freshness: Option<DurableExternalFreshnessBinding>,
    pub current_idempotency_epoch: IdempotencyEpoch,
    pub minimum_retry_epoch: IdempotencyEpoch,
    pub materializations: Vec<DurableMaterializationSpec>,
    pub physical_artifacts: Vec<DurablePhysicalArtifactSpec>,
    pub artifact_cores: Vec<DurableArtifactCore>,
    pub migration_complements: Vec<DurableMigrationComplement>,
    pub committed_transactions: BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
    pub semantic_modules: Vec<BuiltinSemanticModuleSpec>,
    pub causal_coverage_root: Option<RevisionId>,
    pub revision_effects: BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    pub revision_effect_frontiers: BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
}

type DecodedRevisionEffectState = (
    Option<RevisionId>,
    BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
);

pub(crate) fn encode(metadata: &DurableStoreMetadata) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.extend_from_slice(&METADATA_CODEC_VERSION.to_le_bytes());
    if metadata.minimum_retry_epoch > metadata.current_idempotency_epoch {
        return Err(CodecError::CollectionTooLarge);
    }
    push_u64(&mut out, metadata.current_idempotency_epoch.raw());
    push_u64(&mut out, metadata.minimum_retry_epoch.raw());
    encode_materialization_specs(&mut out, &metadata.materializations)?;
    encode_physical_artifact_specs(&mut out, &metadata.physical_artifacts)?;
    encode_artifact_cores(&mut out, &metadata.artifact_cores)?;
    encode_migration_complements(&mut out, &metadata.migration_complements)?;
    push_len(&mut out, metadata.committed_transactions.len())?;
    for (key, intent) in &metadata.committed_transactions {
        if key.epoch < metadata.minimum_retry_epoch
            || key.epoch > metadata.current_idempotency_epoch
        {
            return Err(CodecError::CollectionTooLarge);
        }
        push_u64(&mut out, key.epoch.raw());
        push_u128(&mut out, key.transaction_id.raw());
        encode_transaction_intent(&mut out, intent)?;
    }
    let mut modules = metadata.semantic_modules.clone();
    modules.sort_by_key(|spec| spec.digest());
    modules.dedup_by_key(|spec| spec.digest());
    push_len(&mut out, modules.len())?;
    for spec in modules {
        encode_semantic_module_spec(&mut out, spec);
    }
    encode_revision_effect_state(
        &mut out,
        metadata.causal_coverage_root,
        &metadata.revision_effects,
        &metadata.revision_effect_frontiers,
    )?;
    encode_external_freshness(&mut out, metadata.external_freshness);
    Ok(out)
}

fn decode_metadata_header(
    cursor: &mut Cursor<'_>,
) -> Result<(u16, IdempotencyEpoch, IdempotencyEpoch), &'static str> {
    let version = cursor.u16()?;
    if !matches!(version, 1..=9 | 11..=METADATA_CODEC_VERSION) {
        return Err("unsupported durable metadata codec version");
    }
    let (current, minimum) = if version >= 12 {
        (
            IdempotencyEpoch::new(cursor.u64()?),
            IdempotencyEpoch::new(cursor.u64()?),
        )
    } else {
        (IdempotencyEpoch::ZERO, IdempotencyEpoch::ZERO)
    };
    if minimum > current {
        return Err("minimum retry epoch exceeds current idempotency epoch");
    }
    Ok((version, current, minimum))
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DurableStoreMetadata, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let (version, current_idempotency_epoch, minimum_retry_epoch) =
        decode_metadata_header(&mut cursor)?;
    let materializations = decode_materialization_specs(&mut cursor)?;
    let physical_artifacts = if version >= 5 {
        decode_physical_artifact_specs(&mut cursor)?
    } else {
        Vec::new()
    };
    let artifact_cores = if version >= 11 {
        decode_artifact_cores(&mut cursor)?
    } else {
        Vec::new()
    };
    let migration_complements = if version >= 7 {
        decode_migration_complements(&mut cursor)?
    } else {
        Vec::new()
    };

    let transaction_count = cursor.len()?;
    let mut committed_transactions = BTreeMap::new();
    let mut previous_transaction = None;
    for _ in 0..transaction_count {
        let epoch = if version >= 12 {
            IdempotencyEpoch::new(cursor.u64()?)
        } else {
            IdempotencyEpoch::ZERO
        };
        let transaction_id = ClientTransactionId::new(cursor.u128()?);
        let key = DurableTransactionKey::new(epoch, transaction_id);
        if previous_transaction.is_some_and(|prior: DurableTransactionKey| prior >= key) {
            return Err("transaction retry keys are not strictly sorted and unique");
        }
        if epoch < minimum_retry_epoch || epoch > current_idempotency_epoch {
            return Err("transaction retry epoch is outside durable retry window");
        }
        previous_transaction = Some(key);
        let intent = if version == 1 {
            DurableTransactionIntent::LegacyTargetOnly {
                target_revision: RevisionId::new(cursor.u64()?),
            }
        } else {
            decode_transaction_intent(&mut cursor)?
        };
        committed_transactions.insert(key, intent);
    }
    let semantic_modules = if version == 1 {
        Vec::new()
    } else {
        let count = cursor.len()?;
        let mut modules = Vec::with_capacity(cursor.bounded_capacity(count));
        let mut previous = None;
        for _ in 0..count {
            let spec = decode_semantic_module_spec(&mut cursor)?;
            let digest = spec.digest();
            if previous.is_some_and(|prior| prior >= digest) {
                return Err("semantic module digests are not strictly sorted and unique");
            }
            previous = Some(digest);
            modules.push(spec);
        }
        modules
    };
    let (causal_coverage_root, revision_effects, revision_effect_frontiers) = if version >= 9 {
        decode_revision_effect_state(&mut cursor, version, &committed_transactions)?
    } else {
        (None, BTreeMap::new(), BTreeMap::new())
    };
    let external_freshness = if version >= 13 {
        decode_external_freshness(&mut cursor)?
    } else {
        None
    };
    cursor.finish()?;
    Ok(DurableStoreMetadata {
        external_freshness,
        current_idempotency_epoch,
        minimum_retry_epoch,
        materializations,
        physical_artifacts,
        artifact_cores,
        migration_complements,
        committed_transactions,
        semantic_modules,
        causal_coverage_root,
        revision_effects,
        revision_effect_frontiers,
    })
}

fn encode_external_freshness(out: &mut Vec<u8>, binding: Option<DurableExternalFreshnessBinding>) {
    let Some(binding) = binding else {
        out.push(0);
        return;
    };
    out.push(1);
    out.extend_from_slice(&binding.store_id);
    if let Some(digest) = binding.previous_generation_digest {
        out.push(1);
        out.extend_from_slice(&digest.0);
    } else {
        out.push(0);
        out.extend_from_slice(&[0; 32]);
    }
    push_u64(out, binding.trust_root_epoch);
    push_u64(out, binding.deployment_policy_epoch);
}

fn decode_external_freshness(
    cursor: &mut Cursor<'_>,
) -> Result<Option<DurableExternalFreshnessBinding>, &'static str> {
    if cursor.u8()? == 0 {
        return Ok(None);
    }
    let mut store_id = [0_u8; 32];
    store_id.copy_from_slice(cursor.take(32)?);
    let previous_generation_digest = if cursor.u8()? == 0 {
        let zero = cursor.take(32)?;
        if zero.iter().any(|byte| *byte != 0) {
            return Err("external freshness none digest is nonzero");
        }
        None
    } else {
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(cursor.take(32)?);
        Some(kernel_auth::AuthorityDigest(digest))
    };
    Ok(Some(DurableExternalFreshnessBinding {
        store_id,
        previous_generation_digest,
        trust_root_epoch: cursor.u64()?,
        deployment_policy_epoch: cursor.u64()?,
    }))
}

fn encode_revision_effect_state(
    out: &mut Vec<u8>,
    causal_coverage_root: Option<RevisionId>,
    effects: &BTreeMap<RevisionEffectId, DurableRevisionEffectRecord>,
    frontiers: &BTreeMap<RevisionId, BTreeSet<RevisionEffectId>>,
) -> Result<(), CodecError> {
    match causal_coverage_root {
        Some(root) => {
            out.push(1);
            push_u64(out, root.raw());
        }
        None => out.push(0),
    }
    push_len(out, effects.len())?;
    for (&id, effect) in effects {
        effect
            .validate_identity()
            .map_err(|_| CodecError::CollectionTooLarge)?;
        if id != effect.id {
            return Err(CodecError::CollectionTooLarge);
        }
        push_u128(out, id.0);
        push_len(out, effect.prerequisites.len())?;
        for prerequisite in &effect.prerequisites {
            push_u128(out, prerequisite.0);
        }
        push_u64(out, effect.transaction_epoch.raw());
        push_u128(out, effect.transaction_id.raw());
        encode_transaction_intent(out, &effect.intent)?;
        push_u64(out, effect.source_revision.raw());
        push_u64(out, effect.target_revision.raw());
    }
    push_len(out, frontiers.len())?;
    for (&revision, frontier) in frontiers {
        push_u64(out, revision.raw());
        push_len(out, frontier.len())?;
        for effect in frontier {
            push_u128(out, effect.0);
        }
    }
    Ok(())
}

fn decode_revision_effect_state(
    cursor: &mut Cursor<'_>,
    metadata_version: u16,
    transactions: &BTreeMap<DurableTransactionKey, DurableTransactionIntent>,
) -> Result<DecodedRevisionEffectState, &'static str> {
    let causal_coverage_root = match cursor.u8()? {
        0 => None,
        1 => Some(RevisionId::new(cursor.u64()?)),
        _ => return Err("invalid causal coverage root tag"),
    };
    let effect_count = cursor.len()?;
    let mut effects = BTreeMap::new();
    let mut previous_effect = None;
    for _ in 0..effect_count {
        let id = RevisionEffectId(cursor.u128()?);
        if previous_effect.is_some_and(|previous| previous >= id) {
            return Err("revision effect ids are not strictly sorted and unique");
        }
        previous_effect = Some(id);
        let prerequisite_count = cursor.len()?;
        let mut prerequisites = BTreeSet::new();
        let mut previous_prerequisite = None;
        for _ in 0..prerequisite_count {
            let prerequisite = RevisionEffectId(cursor.u128()?);
            if previous_prerequisite.is_some_and(|previous| previous >= prerequisite) {
                return Err("revision effect prerequisites are not strictly sorted and unique");
            }
            previous_prerequisite = Some(prerequisite);
            prerequisites.insert(prerequisite);
        }
        let transaction_epoch = if metadata_version >= 12 {
            IdempotencyEpoch::new(cursor.u64()?)
        } else {
            IdempotencyEpoch::ZERO
        };
        let transaction_id = ClientTransactionId::new(cursor.u128()?);
        let intent = if metadata_version >= 12 {
            decode_transaction_intent(cursor)?
        } else {
            transactions
                .get(&DurableTransactionKey::new(
                    transaction_epoch,
                    transaction_id,
                ))
                .cloned()
                .unwrap_or(DurableTransactionIntent::LegacyTargetOnly {
                    target_revision: RevisionId::new(0),
                })
        };
        let source_revision = RevisionId::new(cursor.u64()?);
        let target_revision = RevisionId::new(cursor.u64()?);
        let intent = if matches!(intent, DurableTransactionIntent::LegacyTargetOnly { target_revision: revision } if revision.raw() == 0)
        {
            DurableTransactionIntent::LegacyTargetOnly { target_revision }
        } else {
            intent
        };
        let effect = DurableRevisionEffectRecord {
            id,
            prerequisites,
            transaction_epoch,
            transaction_id,
            intent,
            source_revision,
            target_revision,
        };
        effect.validate_identity()?;
        effects.insert(id, effect);
    }
    let frontier_count = cursor.len()?;
    let mut frontiers = BTreeMap::new();
    let mut previous_revision = None;
    for _ in 0..frontier_count {
        let revision = RevisionId::new(cursor.u64()?);
        if previous_revision.is_some_and(|previous| previous >= revision) {
            return Err("revision effect frontier revisions are not strictly sorted and unique");
        }
        previous_revision = Some(revision);
        let effect_count = cursor.len()?;
        let mut frontier = BTreeSet::new();
        let mut previous_effect = None;
        for _ in 0..effect_count {
            let effect = RevisionEffectId(cursor.u128()?);
            if previous_effect.is_some_and(|previous| previous >= effect) {
                return Err("revision frontier effect ids are not strictly sorted and unique");
            }
            previous_effect = Some(effect);
            frontier.insert(effect);
        }
        frontiers.insert(revision, frontier);
    }
    Ok((causal_coverage_root, effects, frontiers))
}
