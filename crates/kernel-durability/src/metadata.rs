use std::collections::{BTreeMap, BTreeSet};

use kernel_change::RevisionEffectId;

use kernel_lens::{ArchiveProofId, ComplementRetention, LensSpecId, SemanticManifestId};
use kernel_query::{AggregateSpec, OrderDirection, RelExpr};
use kernel_semantics::{
    BuiltinSemanticModuleSpec, EquivalenceModule, OrderingModule, TokenizerModule,
};
use kernel_types::{
    ClientTransactionId, MaterializationId, RevisionId, SemanticId, SemanticRevision,
};

use super::{
    CodecError, Cursor, DurableArtifactCore, DurableExternalFreshnessBinding,
    DurableMaterializationSpec, DurableMigrationComplement, DurablePhysicalArtifactSpec,
    DurableRelationLayoutKind, DurableRevisionEffectRecord, DurableSemanticKeyPart,
    DurableTransactionIntent, DurableTransactionKey, IdempotencyEpoch,
    PHYSICAL_ARTIFACT_RECIPE_VERSION, canonical_physical_artifact_specs, encode_value, push_bytes,
    push_len, push_u32, push_u64, push_u128,
};

const METADATA_CODEC_VERSION: u16 = 13;
const MAX_QUERY_DEPTH: usize = 128;

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
        let mut modules = Vec::with_capacity(count);
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

pub(crate) fn encode_migration_complements(
    out: &mut Vec<u8>,
    complements: &[DurableMigrationComplement],
) -> Result<(), CodecError> {
    push_len(out, complements.len())?;
    let mut previous_target = None;
    for complement in complements {
        complement
            .validate()
            .map_err(|_| CodecError::CollectionTooLarge)?;
        if previous_target.is_some_and(|target| target != complement.source_schema) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous_target = Some(complement.target_schema);
        push_u64(out, complement.source_schema.raw());
        push_u64(out, complement.target_schema.raw());
        push_u128(out, complement.lens_spec.0.raw());
        push_u128(out, complement.semantic_pins.0.raw());
        push_u32(out, complement.encoding_version);
        match complement.retention {
            ComplementRetention::Forever => out.push(0),
            ComplementRetention::UntilRevision(revision) => {
                out.push(1);
                push_u64(out, revision.raw());
            }
            ComplementRetention::UntilEpoch(epoch) => {
                out.push(2);
                push_u64(out, epoch);
            }
            ComplementRetention::ExternalArchive(proof) => {
                out.push(3);
                push_u128(out, proof.0.raw());
            }
            ComplementRetention::Forget => out.push(4),
        }
        out.push(u8::from(complement.released));
        match &complement.local_complement {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                encode_value(out, value, 0)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn decode_migration_complements(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DurableMigrationComplement>, &'static str> {
    let count = cursor.len()?;
    let mut complements = Vec::with_capacity(count);
    let mut previous_target = None;
    for _ in 0..count {
        let source_schema = kernel_types::SchemaRevisionId::new(cursor.u64()?);
        let target_schema = kernel_types::SchemaRevisionId::new(cursor.u64()?);
        if previous_target.is_some_and(|target| target != source_schema) {
            return Err("migration complement chain is discontinuous");
        }
        previous_target = Some(target_schema);
        let lens_spec = LensSpecId(SemanticId::new(cursor.u128()?));
        let semantic_pins = SemanticManifestId(SemanticId::new(cursor.u128()?));
        let encoding_version = cursor.u32()?;
        let retention = match cursor.u8()? {
            0 => ComplementRetention::Forever,
            1 => ComplementRetention::UntilRevision(RevisionId::new(cursor.u64()?)),
            2 => ComplementRetention::UntilEpoch(cursor.u64()?),
            3 => ComplementRetention::ExternalArchive(ArchiveProofId(SemanticId::new(
                cursor.u128()?,
            ))),
            4 => ComplementRetention::Forget,
            _ => return Err("invalid migration complement retention tag"),
        };
        let released = match cursor.u8()? {
            0 => false,
            1 => true,
            _ => return Err("invalid migration complement release tag"),
        };
        let local_complement = match cursor.u8()? {
            0 => None,
            1 => Some(cursor.value(0)?),
            _ => return Err("invalid migration complement payload tag"),
        };
        let complement = DurableMigrationComplement {
            source_schema,
            target_schema,
            lens_spec,
            semantic_pins,
            encoding_version,
            retention,
            local_complement,
            released,
        };
        complement.validate()?;
        complements.push(complement);
    }
    Ok(complements)
}

pub(crate) fn encode_materialization_specs(
    out: &mut Vec<u8>,
    specs: &[DurableMaterializationSpec],
) -> Result<(), CodecError> {
    push_len(out, specs.len())?;
    let mut previous = None;
    for spec in specs {
        if previous.is_some_and(|id: MaterializationId| id >= spec.id) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous = Some(spec.id);
        push_u128(out, spec.id.raw());
        encode_rel_expr(out, &spec.query, 0)?;
    }
    Ok(())
}

pub(crate) fn decode_materialization_specs(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DurableMaterializationSpec>, &'static str> {
    let materialization_count = cursor.len()?;
    let mut materializations = Vec::with_capacity(materialization_count);
    let mut previous = None;
    for _ in 0..materialization_count {
        let id = MaterializationId::new(cursor.u128()?);
        if previous.is_some_and(|prior: MaterializationId| prior >= id) {
            return Err("materialization ids are not strictly sorted and unique");
        }
        previous = Some(id);
        materializations.push(DurableMaterializationSpec {
            id,
            query: decode_rel_expr(cursor, 0)?,
        });
    }
    Ok(materializations)
}

fn encode_artifact_cores(
    out: &mut Vec<u8>,
    cores: &[DurableArtifactCore],
) -> Result<(), CodecError> {
    let mut cores = cores.to_vec();
    cores.sort();
    cores.dedup();
    push_len(out, cores.len())?;
    for core in cores {
        match core {
            DurableArtifactCore::ObservableAtom {
                source_revision,
                relation,
                key_parts,
                encoded_keys_by_ordinal,
            } => {
                out.push(0);
                push_u64(out, source_revision.raw());
                push_u128(out, relation.raw());
                push_len(out, key_parts.len())?;
                for part in key_parts {
                    push_len(out, part.column)?;
                    push_u128(out, part.equivalence.raw());
                }
                push_len(out, encoded_keys_by_ordinal.len())?;
                for tuple in encoded_keys_by_ordinal {
                    kernel_semantics::decode_canonical_eq_key_tuple(&tuple)
                        .map_err(|_| CodecError::CollectionTooLarge)?;
                    push_bytes(out, &tuple)?;
                }
            }
        }
    }
    Ok(())
}

fn decode_artifact_cores(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DurableArtifactCore>, &'static str> {
    let count = cursor.len()?;
    let mut cores = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let core = match cursor.u8()? {
            0 => {
                let source_revision = RevisionId::new(cursor.u64()?);
                let relation = SemanticId::new(cursor.u128()?);
                let key_count = cursor.len()?;
                let mut key_parts = Vec::with_capacity(key_count);
                for _ in 0..key_count {
                    key_parts.push(DurableSemanticKeyPart {
                        column: cursor.len()?,
                        equivalence: SemanticId::new(cursor.u128()?),
                    });
                }
                let row_count = cursor.len()?;
                let mut encoded_keys_by_ordinal = Vec::with_capacity(row_count);
                for _ in 0..row_count {
                    let len = cursor.len()?;
                    let tuple = cursor.take(len)?.to_vec();
                    let keys = kernel_semantics::decode_canonical_eq_key_tuple(&tuple)
                        .map_err(|_| "invalid canonical observable artifact core")?;
                    if keys.len() != key_parts.len() {
                        return Err("observable artifact core key arity mismatch");
                    }
                    encoded_keys_by_ordinal.push(tuple);
                }
                DurableArtifactCore::ObservableAtom {
                    source_revision,
                    relation,
                    key_parts,
                    encoded_keys_by_ordinal,
                }
            }
            _ => return Err("unknown durable artifact core tag"),
        };
        if previous.as_ref().is_some_and(|prior| prior >= &core) {
            return Err("durable artifact cores are not strictly sorted and unique");
        }
        previous = Some(core.clone());
        cores.push(core);
    }
    Ok(cores)
}

fn encode_physical_artifact_specs(
    out: &mut Vec<u8>,
    specs: &[DurablePhysicalArtifactSpec],
) -> Result<(), CodecError> {
    out.extend_from_slice(&PHYSICAL_ARTIFACT_RECIPE_VERSION.to_le_bytes());
    let specs = canonical_physical_artifact_specs(specs);
    push_len(out, specs.len())?;
    for spec in specs {
        match spec {
            DurablePhysicalArtifactSpec::RelationLayout {
                relation,
                layout_id,
                kind,
            } => {
                out.push(3);
                push_u128(out, relation.raw());
                push_u128(out, layout_id);
                out.push(match kind {
                    DurableRelationLayoutKind::RowStore => 0,
                    DurableRelationLayoutKind::ValueColumnar => 1,
                    DurableRelationLayoutKind::I64Columnar => 2,
                    DurableRelationLayoutKind::TypedColumnar => 3,
                });
            }
            DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column,
                equivalence,
                advisor_managed,
            } => {
                out.push(4);
                push_u128(out, relation.raw());
                push_u64(
                    out,
                    u64::try_from(key_column).map_err(|_| CodecError::CollectionTooLarge)?,
                );
                push_u128(out, equivalence.raw());
                out.push(u8::from(advisor_managed));
            }
            DurablePhysicalArtifactSpec::SemanticIndex {
                relation,
                key_parts,
                advisor_managed,
            } => {
                out.push(0);
                encode_semantic_artifact_key(out, relation, &key_parts, advisor_managed)?;
            }
            DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                relation,
                key_parts,
                advisor_managed,
            } => {
                out.push(1);
                encode_semantic_artifact_key(out, relation, &key_parts, advisor_managed)?;
            }
            DurablePhysicalArtifactSpec::SemanticStatistics {
                relation,
                key_parts,
                advisor_managed,
            } => {
                out.push(2);
                encode_semantic_artifact_key(out, relation, &key_parts, advisor_managed)?;
            }
            DurablePhysicalArtifactSpec::ObservableAtom {
                relation,
                key_parts,
                advisor_managed,
            } => {
                out.push(5);
                encode_semantic_artifact_key(out, relation, &key_parts, advisor_managed)?;
            }
        }
    }
    Ok(())
}

fn encode_semantic_artifact_key(
    out: &mut Vec<u8>,
    relation: SemanticId,
    key_parts: &[DurableSemanticKeyPart],
    advisor_managed: bool,
) -> Result<(), CodecError> {
    push_u128(out, relation.raw());
    push_len(out, key_parts.len())?;
    for part in key_parts {
        push_u64(
            out,
            u64::try_from(part.column).map_err(|_| CodecError::CollectionTooLarge)?,
        );
        push_u128(out, part.equivalence.raw());
    }
    out.push(u8::from(advisor_managed));
    Ok(())
}

fn decode_physical_artifact_specs(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<DurablePhysicalArtifactSpec>, &'static str> {
    let recipe_version = cursor.u16()?;
    if !(1..=PHYSICAL_ARTIFACT_RECIPE_VERSION).contains(&recipe_version) {
        return Err("unsupported physical artifact recipe version");
    }
    let count = cursor.len()?;
    let mut specs = Vec::with_capacity(count);
    for _ in 0..count {
        let tag = cursor.u8()?;
        let spec = match tag {
            0 => {
                let (relation, key_parts, advisor_managed) = decode_semantic_artifact_key(cursor)?;
                DurablePhysicalArtifactSpec::SemanticIndex {
                    relation,
                    key_parts,
                    advisor_managed,
                }
            }
            1 => {
                let (relation, key_parts, advisor_managed) = decode_semantic_artifact_key(cursor)?;
                DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                    relation,
                    key_parts,
                    advisor_managed,
                }
            }
            2 => {
                let (relation, key_parts, advisor_managed) = decode_semantic_artifact_key(cursor)?;
                DurablePhysicalArtifactSpec::SemanticStatistics {
                    relation,
                    key_parts,
                    advisor_managed,
                }
            }
            3 if recipe_version >= 2 => {
                let relation = SemanticId::new(cursor.u128()?);
                let layout_id = cursor.u128()?;
                let kind = match cursor.u8()? {
                    0 => DurableRelationLayoutKind::RowStore,
                    1 => DurableRelationLayoutKind::ValueColumnar,
                    2 => DurableRelationLayoutKind::I64Columnar,
                    3 => DurableRelationLayoutKind::TypedColumnar,
                    _ => return Err("invalid durable relation layout kind"),
                };
                DurablePhysicalArtifactSpec::RelationLayout {
                    relation,
                    layout_id,
                    kind,
                }
            }
            4 if recipe_version >= 2 => {
                let relation = SemanticId::new(cursor.u128()?);
                let key_column = usize::try_from(cursor.u64()?)
                    .map_err(|_| "physical artifact column overflow")?;
                let equivalence = SemanticId::new(cursor.u128()?);
                let advisor_managed = match cursor.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err("invalid physical artifact ownership tag"),
                };
                DurablePhysicalArtifactSpec::I64Index {
                    relation,
                    key_column,
                    equivalence,
                    advisor_managed,
                }
            }
            5 if recipe_version >= 3 => {
                let (relation, key_parts, advisor_managed) = decode_semantic_artifact_key(cursor)?;
                DurablePhysicalArtifactSpec::ObservableAtom {
                    relation,
                    key_parts,
                    advisor_managed,
                }
            }
            _ => return Err("invalid physical artifact spec tag"),
        };
        specs.push(spec);
    }
    if specs.windows(2).any(|pair| pair[0] >= pair[1]) {
        return Err("physical artifact specs are not strictly sorted and unique");
    }
    Ok(specs)
}

fn decode_semantic_artifact_key(
    cursor: &mut Cursor<'_>,
) -> Result<(SemanticId, Vec<DurableSemanticKeyPart>, bool), &'static str> {
    let relation = SemanticId::new(cursor.u128()?);
    let count = cursor.len()?;
    if count == 0 {
        return Err("physical artifact semantic key is empty");
    }
    let mut key_parts = Vec::with_capacity(count);
    for _ in 0..count {
        let column =
            usize::try_from(cursor.u64()?).map_err(|_| "physical artifact column overflow")?;
        let equivalence = SemanticId::new(cursor.u128()?);
        key_parts.push(DurableSemanticKeyPart {
            column,
            equivalence,
        });
    }
    let advisor_managed = match cursor.u8()? {
        0 => false,
        1 => true,
        _ => return Err("invalid physical artifact ownership tag"),
    };
    Ok((relation, key_parts, advisor_managed))
}

pub(crate) fn encode_transaction_intent(
    out: &mut Vec<u8>,
    intent: &DurableTransactionIntent,
) -> Result<(), CodecError> {
    match intent {
        DurableTransactionIntent::RelationRewriteExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            semantic_modules,
        } => {
            out.push(3);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            super::encode_relation_mutations(out, relation_mutations)?;
            super::encode_relation_rewrite_intents(out, rewrite_intents)?;
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::RelationResolutionExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            causal_parents,
            semantic_modules,
        } => {
            out.push(5);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            super::encode_relation_mutations(out, relation_mutations)?;
            super::encode_relation_rewrite_intents(out, rewrite_intents)?;
            push_len(out, causal_parents.len())?;
            for parent in causal_parents {
                push_u64(out, parent.raw());
            }
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::RelationDataExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            semantic_modules,
        } => {
            out.push(2);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            super::encode_relation_mutations(out, relation_mutations)?;
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::Exact {
            target_revision,
            encoded_target_revision,
            materializations,
            semantic_modules,
        } => {
            out.push(1);
            push_u64(out, target_revision.raw());
            push_bytes(out, encoded_target_revision)?;
            match materializations {
                None => out.push(0),
                Some(specs) => {
                    out.push(1);
                    encode_materialization_specs(out, specs)?;
                }
            }
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::SchemaMigrationExact {
            source_revision,
            target_revision,
            encoded_target_revision,
            migration_complement,
            semantic_modules,
        } => {
            out.push(4);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_bytes(out, encoded_target_revision)?;
            encode_migration_complements(out, std::slice::from_ref(migration_complement))?;
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::LegacyTargetOnly { target_revision } => {
            out.push(0);
            push_u64(out, target_revision.raw());
        }
    }
    Ok(())
}

pub(crate) fn decode_transaction_intent(
    cursor: &mut Cursor<'_>,
) -> Result<DurableTransactionIntent, &'static str> {
    match cursor.u8()? {
        0 => Ok(DurableTransactionIntent::LegacyTargetOnly {
            target_revision: RevisionId::new(cursor.u64()?),
        }),
        1 => {
            let target_revision = RevisionId::new(cursor.u64()?);
            let len = cursor.len()?;
            let encoded_target_revision = cursor.take(len)?.to_vec();
            let materializations = match cursor.u8()? {
                0 => None,
                1 => Some(decode_materialization_specs(cursor)?),
                _ => return Err("invalid transaction intent materialization tag"),
            };
            let semantic_modules = decode_semantic_module_specs(cursor)?;
            Ok(DurableTransactionIntent::Exact {
                target_revision,
                encoded_target_revision,
                materializations,
                semantic_modules,
            })
        }
        2 => Ok(DurableTransactionIntent::RelationDataExact {
            source_revision: RevisionId::new(cursor.u64()?),
            target_revision: RevisionId::new(cursor.u64()?),
            semantic_revision: SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            ),
            relation_mutations: super::decode_relation_mutations(cursor)?,
            semantic_modules: decode_semantic_module_specs(cursor)?,
        }),
        3 => {
            let source_revision = RevisionId::new(cursor.u64()?);
            let target_revision = RevisionId::new(cursor.u64()?);
            let semantic_revision = SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            );
            let relation_mutations = super::decode_relation_mutations(cursor)?;
            let rewrite_intents = super::decode_relation_rewrite_intents(cursor)?;
            if relation_mutations.len() != rewrite_intents.len()
                || relation_mutations
                    .iter()
                    .zip(&rewrite_intents)
                    .any(|(mutation, intent)| mutation.relation != intent.relation)
            {
                return Err("relation rewrite intents do not match relation mutations");
            }
            Ok(DurableTransactionIntent::RelationRewriteExact {
                source_revision,
                target_revision,
                semantic_revision,
                relation_mutations,
                rewrite_intents,
                semantic_modules: decode_semantic_module_specs(cursor)?,
            })
        }
        4 => {
            let source_revision = RevisionId::new(cursor.u64()?);
            let target_revision = RevisionId::new(cursor.u64()?);
            let len = cursor.len()?;
            let encoded_target_revision = cursor.take(len)?.to_vec();
            let mut complements = decode_migration_complements(cursor)?;
            if complements.len() != 1 {
                return Err("schema migration intent must carry exactly one complement");
            }
            Ok(DurableTransactionIntent::SchemaMigrationExact {
                source_revision,
                target_revision,
                encoded_target_revision,
                migration_complement: complements.remove(0),
                semantic_modules: decode_semantic_module_specs(cursor)?,
            })
        }
        5 => decode_relation_resolution_intent(cursor),
        _ => Err("invalid transaction intent tag"),
    }
}

fn decode_relation_resolution_intent(
    cursor: &mut Cursor<'_>,
) -> Result<DurableTransactionIntent, &'static str> {
    let source_revision = RevisionId::new(cursor.u64()?);
    let target_revision = RevisionId::new(cursor.u64()?);
    let semantic_revision = SemanticRevision::new(
        kernel_types::SchemaRevisionId::new(cursor.u64()?),
        kernel_types::SemanticEnvId::new(cursor.u64()?),
    );
    let relation_mutations = super::decode_relation_mutations(cursor)?;
    let rewrite_intents = super::decode_relation_rewrite_intents(cursor)?;
    if relation_mutations.len() != rewrite_intents.len()
        || relation_mutations
            .iter()
            .zip(&rewrite_intents)
            .any(|(mutation, intent)| mutation.relation != intent.relation)
    {
        return Err("relation resolution intents do not match relation mutations");
    }
    let count = cursor.len()?;
    if count < 2 {
        return Err("relation resolution must have at least two causal parents");
    }
    let mut causal_parents = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let parent = RevisionId::new(cursor.u64()?);
        if previous.is_some_and(|prior| prior >= parent) {
            return Err("relation resolution causal parents are not strictly sorted");
        }
        previous = Some(parent);
        causal_parents.push(parent);
    }
    if causal_parents.binary_search(&source_revision).is_err() {
        return Err("relation resolution causal parents omit source revision");
    }
    Ok(DurableTransactionIntent::RelationResolutionExact {
        source_revision,
        target_revision,
        semantic_revision,
        relation_mutations,
        rewrite_intents,
        causal_parents,
        semantic_modules: decode_semantic_module_specs(cursor)?,
    })
}

pub(crate) fn encode_semantic_module_specs(
    out: &mut Vec<u8>,
    specs: &[BuiltinSemanticModuleSpec],
) -> Result<(), CodecError> {
    let mut specs = specs.to_vec();
    specs.sort_by_key(|spec| spec.digest());
    specs.dedup_by_key(|spec| spec.digest());
    push_len(out, specs.len())?;
    for spec in specs {
        encode_semantic_module_spec(out, spec);
    }
    Ok(())
}

pub(crate) fn decode_semantic_module_specs(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<BuiltinSemanticModuleSpec>, &'static str> {
    let count = cursor.len()?;
    let mut modules = Vec::with_capacity(count);
    let mut previous = None;
    for _ in 0..count {
        let spec = decode_semantic_module_spec(cursor)?;
        let digest = spec.digest();
        if previous.is_some_and(|prior| prior >= digest) {
            return Err("semantic module digests are not strictly sorted and unique");
        }
        previous = Some(digest);
        modules.push(spec);
    }
    Ok(modules)
}

fn encode_semantic_module_spec(out: &mut Vec<u8>, spec: BuiltinSemanticModuleSpec) {
    match spec {
        BuiltinSemanticModuleSpec::Equivalence {
            module,
            implementation_revision,
        } => {
            out.push(0);
            encode_equivalence_module(out, module);
            push_u64(out, implementation_revision);
        }
        BuiltinSemanticModuleSpec::Tokenizer {
            module,
            implementation_revision,
        } => {
            out.push(1);
            out.push(match module {
                TokenizerModule::AsciiWhitespace => 0,
                TokenizerModule::AsciiWhitespaceLowercase => 1,
            });
            push_u64(out, implementation_revision);
        }
        BuiltinSemanticModuleSpec::Ordering {
            module,
            implementation_revision,
        } => {
            out.push(2);
            encode_ordering_module(out, module);
            push_u64(out, implementation_revision);
        }
    }
}

fn decode_semantic_module_spec(
    cursor: &mut Cursor<'_>,
) -> Result<BuiltinSemanticModuleSpec, &'static str> {
    match cursor.u8()? {
        0 => Ok(BuiltinSemanticModuleSpec::Equivalence {
            module: decode_equivalence_module(cursor)?,
            implementation_revision: cursor.u64()?,
        }),
        1 => Ok(BuiltinSemanticModuleSpec::Tokenizer {
            module: match cursor.u8()? {
                0 => TokenizerModule::AsciiWhitespace,
                1 => TokenizerModule::AsciiWhitespaceLowercase,
                _ => return Err("invalid builtin tokenizer module tag"),
            },
            implementation_revision: cursor.u64()?,
        }),
        2 => Ok(BuiltinSemanticModuleSpec::Ordering {
            module: decode_ordering_module(cursor)?,
            implementation_revision: cursor.u64()?,
        }),
        _ => Err("invalid builtin semantic module kind"),
    }
}

fn encode_ordering_module(out: &mut Vec<u8>, module: OrderingModule) {
    match module {
        OrderingModule::I64Ascending => out.push(0),
        OrderingModule::F64Total => out.push(1),
        OrderingModule::TextBinary => out.push(2),
        OrderingModule::TextAsciiCaseInsensitive => out.push(3),
        OrderingModule::TextAsciiCaseInsensitiveThenBinary => out.push(4),
        OrderingModule::UnitExact => out.push(5),
        OrderingModule::BoolAscending => out.push(6),
        OrderingModule::LiveEntityIdAscending(entity_type) => {
            out.push(7);
            push_u128(out, entity_type.raw());
        }
        OrderingModule::HistoricalEntityIdAscending(entity_type) => {
            out.push(8);
            push_u128(out, entity_type.raw());
        }
    }
}

fn decode_ordering_module(cursor: &mut Cursor<'_>) -> Result<OrderingModule, &'static str> {
    match cursor.u8()? {
        0 => Ok(OrderingModule::I64Ascending),
        1 => Ok(OrderingModule::F64Total),
        2 => Ok(OrderingModule::TextBinary),
        3 => Ok(OrderingModule::TextAsciiCaseInsensitive),
        4 => Ok(OrderingModule::TextAsciiCaseInsensitiveThenBinary),
        5 => Ok(OrderingModule::UnitExact),
        6 => Ok(OrderingModule::BoolAscending),
        7 => Ok(OrderingModule::LiveEntityIdAscending(SemanticId::new(
            cursor.u128()?,
        ))),
        8 => Ok(OrderingModule::HistoricalEntityIdAscending(
            SemanticId::new(cursor.u128()?),
        )),
        _ => Err("invalid builtin ordering module tag"),
    }
}

fn encode_equivalence_module(out: &mut Vec<u8>, module: EquivalenceModule) {
    match module {
        EquivalenceModule::UnitExact => out.push(0),
        EquivalenceModule::BoolExact => out.push(1),
        EquivalenceModule::I64Exact => out.push(2),
        EquivalenceModule::F64Bitwise => out.push(3),
        EquivalenceModule::TextExact => out.push(4),
        EquivalenceModule::TextAsciiCaseInsensitive => out.push(5),
        EquivalenceModule::LiveEntityIdExact(entity_type) => {
            out.push(6);
            push_u128(out, entity_type.raw());
        }
        EquivalenceModule::HistoricalEntityIdExact(entity_type) => {
            out.push(7);
            push_u128(out, entity_type.raw());
        }
    }
}

fn decode_equivalence_module(cursor: &mut Cursor<'_>) -> Result<EquivalenceModule, &'static str> {
    match cursor.u8()? {
        0 => Ok(EquivalenceModule::UnitExact),
        1 => Ok(EquivalenceModule::BoolExact),
        2 => Ok(EquivalenceModule::I64Exact),
        3 => Ok(EquivalenceModule::F64Bitwise),
        4 => Ok(EquivalenceModule::TextExact),
        5 => Ok(EquivalenceModule::TextAsciiCaseInsensitive),
        6 => Ok(EquivalenceModule::LiveEntityIdExact(SemanticId::new(
            cursor.u128()?,
        ))),
        7 => Ok(EquivalenceModule::HistoricalEntityIdExact(SemanticId::new(
            cursor.u128()?,
        ))),
        _ => Err("invalid builtin equivalence module tag"),
    }
}

fn encode_rel_expr(out: &mut Vec<u8>, expr: &RelExpr, depth: usize) -> Result<(), CodecError> {
    if depth > MAX_QUERY_DEPTH {
        return Err(CodecError::ValueNestingTooDeep);
    }
    match expr {
        RelExpr::Scan(relation) => {
            out.push(0);
            push_u128(out, relation.raw());
        }
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => {
            out.push(1);
            encode_rel_expr(out, input, depth + 1)?;
            push_usize(out, *column)?;
            encode_value(out, value, 0)?;
            push_u128(out, equivalence.raw());
        }
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => encode_filter_columns(out, input, *left_column, *right_column, *equivalence, depth)?,
        RelExpr::Project { input, columns } => {
            out.push(2);
            encode_rel_expr(out, input, depth + 1)?;
            push_len(out, columns.len())?;
            for column in columns {
                push_usize(out, *column)?;
            }
        }
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            out.push(3);
            encode_rel_expr(out, left, depth + 1)?;
            encode_rel_expr(out, right, depth + 1)?;
            push_usize(out, *left_column)?;
            push_usize(out, *right_column)?;
            push_u128(out, equivalence.raw());
        }
        RelExpr::Difference { left, right } => {
            out.push(9);
            encode_rel_expr(out, left, depth + 1)?;
            encode_rel_expr(out, right, depth + 1)?;
        }
        RelExpr::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            out.push(10);
            encode_rel_expr(out, left, depth + 1)?;
            encode_rel_expr(out, right, depth + 1)?;
            push_usize(out, *left_column)?;
            push_usize(out, *right_column)?;
            push_u128(out, equivalence.raw());
        }
        RelExpr::Distinct {
            input,
            column_equivalences,
        } => {
            out.push(4);
            encode_rel_expr(out, input, depth + 1)?;
            encode_semantic_ids(out, column_equivalences)?;
        }
        RelExpr::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } => encode_rel_group(
            out,
            input,
            group_columns,
            group_equivalences,
            aggregate,
            depth,
        )?,
        RelExpr::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } => encode_rel_top_k(out, input, *column, *ordering, *direction, *k, depth)?,
        RelExpr::PromoteToBag(input) => encode_promote_to_bag(out, input, depth)?,
    }
    Ok(())
}

fn encode_promote_to_bag(
    out: &mut Vec<u8>,
    input: &RelExpr,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(7);
    encode_rel_expr(out, input, depth + 1)
}

fn encode_filter_columns(
    out: &mut Vec<u8>,
    input: &RelExpr,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(8);
    encode_rel_expr(out, input, depth + 1)?;
    push_usize(out, left_column)?;
    push_usize(out, right_column)?;
    push_u128(out, equivalence.raw());
    Ok(())
}

fn encode_rel_group(
    out: &mut Vec<u8>,
    input: &RelExpr,
    group_columns: &[usize],
    group_equivalences: &[SemanticId],
    aggregate: &AggregateSpec,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(5);
    encode_rel_expr(out, input, depth + 1)?;
    push_len(out, group_columns.len())?;
    for column in group_columns {
        push_usize(out, *column)?;
    }
    encode_semantic_ids(out, group_equivalences)?;
    encode_aggregate(out, aggregate)
}

fn encode_rel_top_k(
    out: &mut Vec<u8>,
    input: &RelExpr,
    column: usize,
    ordering: SemanticId,
    direction: OrderDirection,
    k: usize,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(6);
    encode_rel_expr(out, input, depth + 1)?;
    push_usize(out, column)?;
    push_u128(out, ordering.raw());
    out.push(match direction {
        OrderDirection::Ascending => 0,
        OrderDirection::Descending => 1,
    });
    push_usize(out, k)
}

fn decode_rel_expr(cursor: &mut Cursor<'_>, depth: usize) -> Result<RelExpr, &'static str> {
    if depth > MAX_QUERY_DEPTH {
        return Err("query nesting exceeds hard limit");
    }
    match cursor.u8()? {
        0 => Ok(RelExpr::Scan(SemanticId::new(cursor.u128()?))),
        1 => Ok(RelExpr::FilterEqConst {
            input: Box::new(decode_rel_expr(cursor, depth + 1)?),
            column: decode_usize(cursor)?,
            value: cursor.value(0)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        2 => {
            let input = Box::new(decode_rel_expr(cursor, depth + 1)?);
            let count = cursor.len()?;
            let mut columns = Vec::with_capacity(count);
            for _ in 0..count {
                columns.push(decode_usize(cursor)?);
            }
            Ok(RelExpr::Project { input, columns })
        }
        3 => Ok(RelExpr::JoinEq {
            left: Box::new(decode_rel_expr(cursor, depth + 1)?),
            right: Box::new(decode_rel_expr(cursor, depth + 1)?),
            left_column: decode_usize(cursor)?,
            right_column: decode_usize(cursor)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        4 => Ok(RelExpr::Distinct {
            input: Box::new(decode_rel_expr(cursor, depth + 1)?),
            column_equivalences: decode_semantic_ids(cursor)?,
        }),
        5 => {
            let input = Box::new(decode_rel_expr(cursor, depth + 1)?);
            let count = cursor.len()?;
            let mut group_columns = Vec::with_capacity(count);
            for _ in 0..count {
                group_columns.push(decode_usize(cursor)?);
            }
            Ok(RelExpr::Group {
                input,
                group_columns,
                group_equivalences: decode_semantic_ids(cursor)?,
                aggregate: decode_aggregate(cursor)?,
            })
        }
        6 => {
            let input = Box::new(decode_rel_expr(cursor, depth + 1)?);
            let column = decode_usize(cursor)?;
            let ordering = SemanticId::new(cursor.u128()?);
            let direction = match cursor.u8()? {
                0 => OrderDirection::Ascending,
                1 => OrderDirection::Descending,
                _ => return Err("invalid order direction"),
            };
            let k = decode_usize(cursor)?;
            Ok(RelExpr::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            })
        }
        7 => Ok(RelExpr::PromoteToBag(Box::new(decode_rel_expr(
            cursor,
            depth + 1,
        )?))),
        8 => Ok(RelExpr::FilterEqColumns {
            input: Box::new(decode_rel_expr(cursor, depth + 1)?),
            left_column: decode_usize(cursor)?,
            right_column: decode_usize(cursor)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        9 => Ok(RelExpr::Difference {
            left: Box::new(decode_rel_expr(cursor, depth + 1)?),
            right: Box::new(decode_rel_expr(cursor, depth + 1)?),
        }),
        10 => Ok(RelExpr::AntiJoin {
            left: Box::new(decode_rel_expr(cursor, depth + 1)?),
            right: Box::new(decode_rel_expr(cursor, depth + 1)?),
            left_column: decode_usize(cursor)?,
            right_column: decode_usize(cursor)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        _ => Err("unknown durable relation expression tag"),
    }
}

fn encode_aggregate(out: &mut Vec<u8>, aggregate: &AggregateSpec) -> Result<(), CodecError> {
    match aggregate {
        AggregateSpec::Count { result_equivalence } => {
            out.push(0);
            push_u128(out, result_equivalence.raw());
        }
        AggregateSpec::ExactF64Sum {
            value_column,
            result_equivalence,
        } => {
            out.push(1);
            push_usize(out, *value_column)?;
            push_u128(out, result_equivalence.raw());
        }
    }
    Ok(())
}

fn decode_aggregate(cursor: &mut Cursor<'_>) -> Result<AggregateSpec, &'static str> {
    match cursor.u8()? {
        0 => Ok(AggregateSpec::Count {
            result_equivalence: SemanticId::new(cursor.u128()?),
        }),
        1 => Ok(AggregateSpec::ExactF64Sum {
            value_column: decode_usize(cursor)?,
            result_equivalence: SemanticId::new(cursor.u128()?),
        }),
        _ => Err("unknown durable aggregate tag"),
    }
}

fn encode_semantic_ids(out: &mut Vec<u8>, ids: &[SemanticId]) -> Result<(), CodecError> {
    push_len(out, ids.len())?;
    for id in ids {
        push_u128(out, id.raw());
    }
    Ok(())
}

fn decode_semantic_ids(cursor: &mut Cursor<'_>) -> Result<Vec<SemanticId>, &'static str> {
    let count = cursor.len()?;
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(SemanticId::new(cursor.u128()?));
    }
    Ok(ids)
}

fn push_usize(out: &mut Vec<u8>, value: usize) -> Result<(), CodecError> {
    let value = u64::try_from(value).map_err(|_| CodecError::LengthOverflow)?;
    push_u64(out, value);
    Ok(())
}

fn decode_usize(cursor: &mut Cursor<'_>) -> Result<usize, &'static str> {
    usize::try_from(cursor.u64()?).map_err(|_| "usize value overflow")
}

#[cfg(test)]
mod tests {
    use kernel_model::Value;
    use kernel_query::{AggregateSpec, OrderDirection, RelExpr};
    use kernel_types::{ClientTransactionId, MaterializationId, RevisionId, SemanticId};

    use super::*;

    fn current_test_transactions() -> BTreeMap<DurableTransactionKey, DurableTransactionIntent> {
        [
            (
                DurableTransactionKey::new(IdempotencyEpoch::ZERO, ClientTransactionId::new(11)),
                DurableTransactionIntent::Exact {
                    target_revision: RevisionId::new(12),
                    encoded_target_revision: vec![1, 2, 3],
                    materializations: None,
                    semantic_modules: Vec::new(),
                },
            ),
            (
                DurableTransactionKey::new(IdempotencyEpoch::ZERO, ClientTransactionId::new(13)),
                DurableTransactionIntent::RelationDataExact {
                    source_revision: RevisionId::new(12),
                    target_revision: RevisionId::new(14),
                    semantic_revision: kernel_types::SemanticRevision::new(
                        kernel_types::SchemaRevisionId::new(3),
                        kernel_types::SemanticEnvId::new(4),
                    ),
                    relation_mutations: vec![crate::DurableRelationMutation {
                        relation: SemanticId::new(9),
                        inserted: vec![vec![kernel_model::Value::I64(5)]],
                        removed: Vec::new(),
                    }],
                    semantic_modules: Vec::new(),
                },
            ),
        ]
        .into()
    }

    #[test]
    fn metadata_roundtrip_covers_all_current_query_variants_and_transactions() {
        let eq = SemanticId::new(90);
        let left = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(SemanticId::new(1))),
            column: 0,
            value: Value::Text("A".into()),
            equivalence: eq,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Group {
                input: Box::new(RelExpr::Distinct {
                    input: Box::new(RelExpr::FilterEqColumns {
                        input: Box::new(RelExpr::JoinEq {
                            left: Box::new(RelExpr::Project {
                                input: Box::new(left),
                                columns: vec![0, 1],
                            }),
                            right: Box::new(RelExpr::PromoteToBag(Box::new(RelExpr::Scan(
                                SemanticId::new(2),
                            )))),
                            left_column: 0,
                            right_column: 0,
                            equivalence: eq,
                        }),
                        left_column: 1,
                        right_column: 3,
                        equivalence: eq,
                    }),
                    column_equivalences: vec![eq, eq],
                }),
                group_columns: vec![0],
                group_equivalences: vec![eq],
                aggregate: AggregateSpec::ExactF64Sum {
                    value_column: 1,
                    result_equivalence: eq,
                },
            }),
            column: 0,
            ordering: SemanticId::new(91),
            direction: OrderDirection::Descending,
            k: 5,
        };
        let metadata = DurableStoreMetadata {
            external_freshness: None,
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            materializations: vec![
                DurableMaterializationSpec {
                    id: MaterializationId::new(7),
                    query,
                },
                DurableMaterializationSpec {
                    id: MaterializationId::new(8),
                    query: RelExpr::Difference {
                        left: Box::new(RelExpr::Scan(SemanticId::new(3))),
                        right: Box::new(RelExpr::Scan(SemanticId::new(4))),
                    },
                },
                DurableMaterializationSpec {
                    id: MaterializationId::new(9),
                    query: RelExpr::AntiJoin {
                        left: Box::new(RelExpr::Scan(SemanticId::new(5))),
                        right: Box::new(RelExpr::Scan(SemanticId::new(6))),
                        left_column: 0,
                        right_column: 1,
                        equivalence: eq,
                    },
                },
            ],
            physical_artifacts: current_physical_artifact_specs(eq),
            migration_complements: Vec::new(),
            committed_transactions: current_test_transactions(),
            semantic_modules: Vec::new(),
            ..DurableStoreMetadata::default()
        };
        assert_eq!(decode(&encode(&metadata).unwrap()).unwrap(), metadata);
    }

    #[test]
    fn relation_rewrite_transaction_intent_roundtrips_in_metadata() {
        let transaction_id = ClientTransactionId::new(15);
        let intent = DurableTransactionIntent::RelationRewriteExact {
            source_revision: RevisionId::new(14),
            target_revision: RevisionId::new(16),
            semantic_revision: kernel_types::SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(3),
                kernel_types::SemanticEnvId::new(4),
            ),
            relation_mutations: vec![crate::DurableRelationMutation {
                relation: SemanticId::new(10),
                inserted: vec![vec![kernel_model::Value::I64(6)]],
                removed: Vec::new(),
            }],
            rewrite_intents: vec![crate::DurableRelationRewriteIntent {
                relation: SemanticId::new(10),
                rewrite_spec: SemanticId::new(110),
                law_set: SemanticId::new(111),
            }],
            semantic_modules: Vec::new(),
        };
        let metadata = DurableStoreMetadata {
            external_freshness: None,
            current_idempotency_epoch: IdempotencyEpoch::ZERO,
            minimum_retry_epoch: IdempotencyEpoch::ZERO,
            materializations: Vec::new(),
            physical_artifacts: Vec::new(),
            artifact_cores: Vec::new(),
            migration_complements: Vec::new(),
            committed_transactions: BTreeMap::from([(
                DurableTransactionKey::new(IdempotencyEpoch::ZERO, transaction_id),
                intent.clone(),
            )]),
            semantic_modules: Vec::new(),
            causal_coverage_root: Some(RevisionId::new(14)),
            revision_effects: BTreeMap::from([(
                RevisionEffectId(transaction_id.raw()),
                DurableRevisionEffectRecord {
                    id: RevisionEffectId(transaction_id.raw()),
                    prerequisites: BTreeSet::new(),
                    transaction_epoch: IdempotencyEpoch::ZERO,
                    transaction_id,
                    intent,
                    source_revision: RevisionId::new(14),
                    target_revision: RevisionId::new(16),
                },
            )]),
            revision_effect_frontiers: BTreeMap::from([
                (RevisionId::new(14), BTreeSet::new()),
                (
                    RevisionId::new(16),
                    BTreeSet::from([RevisionEffectId(transaction_id.raw())]),
                ),
            ]),
        };
        let bytes = encode(&metadata).unwrap();
        assert_eq!(decode(&bytes).unwrap(), metadata);
    }

    fn current_physical_artifact_specs(eq: SemanticId) -> Vec<DurablePhysicalArtifactSpec> {
        vec![
            DurablePhysicalArtifactSpec::RelationLayout {
                relation: SemanticId::new(1),
                layout_id: 0x1234,
                kind: DurableRelationLayoutKind::TypedColumnar,
            },
            DurablePhysicalArtifactSpec::I64Index {
                relation: SemanticId::new(1),
                key_column: 0,
                equivalence: eq,
                advisor_managed: false,
            },
            DurablePhysicalArtifactSpec::SemanticIndex {
                relation: SemanticId::new(1),
                key_parts: vec![DurableSemanticKeyPart {
                    column: 0,
                    equivalence: eq,
                }],
                advisor_managed: true,
            },
            DurablePhysicalArtifactSpec::SemanticQuotientFactor {
                relation: SemanticId::new(2),
                key_parts: vec![DurableSemanticKeyPart {
                    column: 1,
                    equivalence: eq,
                }],
                advisor_managed: false,
            },
            DurablePhysicalArtifactSpec::SemanticStatistics {
                relation: SemanticId::new(2),
                key_parts: vec![DurableSemanticKeyPart {
                    column: 0,
                    equivalence: eq,
                }],
                advisor_managed: true,
            },
            DurablePhysicalArtifactSpec::ObservableAtom {
                relation: SemanticId::new(3),
                key_parts: vec![DurableSemanticKeyPart {
                    column: 0,
                    equivalence: eq,
                }],
                advisor_managed: false,
            },
        ]
    }

    #[test]
    fn physical_artifact_recipe_v1_remains_decodable() {
        let relation = SemanticId::new(41);
        let equivalence = SemanticId::new(42);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        push_len(&mut bytes, 1).unwrap();
        bytes.push(0);
        encode_semantic_artifact_key(
            &mut bytes,
            relation,
            &[DurableSemanticKeyPart {
                column: 3,
                equivalence,
            }],
            true,
        )
        .unwrap();

        let decoded = decode_physical_artifact_specs(&mut Cursor::new(&bytes)).unwrap();
        assert_eq!(
            decoded,
            vec![DurablePhysicalArtifactSpec::SemanticIndex {
                relation,
                key_parts: vec![DurableSemanticKeyPart {
                    column: 3,
                    equivalence,
                }],
                advisor_managed: true,
            }]
        );
    }

    #[test]
    fn physical_artifact_recipe_v2_remains_decodable_after_observable_atom_recipe() {
        let relation = SemanticId::new(51);
        let equivalence = SemanticId::new(52);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&2_u16.to_le_bytes());
        push_len(&mut bytes, 1).unwrap();
        bytes.push(4);
        push_u128(&mut bytes, relation.raw());
        push_u64(&mut bytes, 2);
        push_u128(&mut bytes, equivalence.raw());
        bytes.push(1);

        let decoded = decode_physical_artifact_specs(&mut Cursor::new(&bytes)).unwrap();
        assert_eq!(
            decoded,
            vec![DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column: 2,
                equivalence,
                advisor_managed: true,
            }]
        );
    }

    #[test]
    fn physical_artifact_recipe_collapse_preserves_manual_pin() {
        let eq = SemanticId::new(78);
        let mut bytes = Vec::new();
        encode_physical_artifact_specs(
            &mut bytes,
            &[
                DurablePhysicalArtifactSpec::SemanticIndex {
                    relation: SemanticId::new(77),
                    key_parts: vec![DurableSemanticKeyPart {
                        column: 0,
                        equivalence: eq,
                    }],
                    advisor_managed: true,
                },
                DurablePhysicalArtifactSpec::SemanticIndex {
                    relation: SemanticId::new(77),
                    key_parts: vec![DurableSemanticKeyPart {
                        column: 0,
                        equivalence: eq,
                    }],
                    advisor_managed: false,
                },
            ],
        )
        .unwrap();
        let mut cursor = Cursor::new(&bytes);
        let decoded = decode_physical_artifact_specs(&mut cursor).unwrap();
        cursor.finish().unwrap();
        assert_eq!(
            decoded,
            vec![DurablePhysicalArtifactSpec::SemanticIndex {
                relation: SemanticId::new(77),
                key_parts: vec![DurableSemanticKeyPart {
                    column: 0,
                    equivalence: eq,
                }],
                advisor_managed: false,
            }]
        );
    }

    #[test]
    fn i64_recipe_collapse_preserves_manual_pin() {
        let relation = SemanticId::new(79);
        let equivalence = SemanticId::new(80);
        let mut bytes = Vec::new();
        encode_physical_artifact_specs(
            &mut bytes,
            &[
                DurablePhysicalArtifactSpec::I64Index {
                    relation,
                    key_column: 2,
                    equivalence,
                    advisor_managed: true,
                },
                DurablePhysicalArtifactSpec::I64Index {
                    relation,
                    key_column: 2,
                    equivalence,
                    advisor_managed: false,
                },
            ],
        )
        .unwrap();
        let decoded = decode_physical_artifact_specs(&mut Cursor::new(&bytes)).unwrap();
        assert_eq!(
            decoded,
            vec![DurablePhysicalArtifactSpec::I64Index {
                relation,
                key_column: 2,
                equivalence,
                advisor_managed: false,
            }]
        );
    }

    #[test]
    fn metadata_v4_decodes_without_physical_artifact_manifest() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&4_u16.to_le_bytes());
        encode_materialization_specs(&mut bytes, &[]).unwrap();
        push_len(&mut bytes, 0).unwrap();
        push_len(&mut bytes, 0).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert!(decoded.materializations.is_empty());
        assert!(decoded.physical_artifacts.is_empty());
        assert!(decoded.committed_transactions.is_empty());
        assert!(decoded.semantic_modules.is_empty());
    }

    #[test]
    fn metadata_v6_decodes_without_migration_complement_ledger() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&6_u16.to_le_bytes());
        encode_materialization_specs(&mut bytes, &[]).unwrap();
        encode_physical_artifact_specs(&mut bytes, &[]).unwrap();
        push_len(&mut bytes, 0).unwrap();
        push_len(&mut bytes, 0).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert!(decoded.materializations.is_empty());
        assert!(decoded.physical_artifacts.is_empty());
        assert!(decoded.migration_complements.is_empty());
        assert!(decoded.committed_transactions.is_empty());
        assert!(decoded.semantic_modules.is_empty());
    }

    #[test]
    fn physical_artifact_recipe_version_is_fail_closed() {
        let mut bytes = Vec::new();
        encode_physical_artifact_specs(&mut bytes, &[]).unwrap();
        bytes[..2].copy_from_slice(&(PHYSICAL_ARTIFACT_RECIPE_VERSION + 1).to_le_bytes());
        let mut cursor = Cursor::new(&bytes);
        assert_eq!(
            decode_physical_artifact_specs(&mut cursor),
            Err("unsupported physical artifact recipe version")
        );
    }
}
