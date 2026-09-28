use kernel_change::RevisionEffectId;
use kernel_semantics::BuiltinSemanticModuleSpec;
use kernel_types::{ClientTransactionId, RevisionId, SemanticRevision};

use crate::binary_codec::{
    Cursor, push_bytes, push_len, push_u16, push_u64, push_u128, read_u32, read_u64,
};
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::{
    DurableMigrationComplement, DurableRelationMutation, DurableRelationResolution,
    DurableRevisionChange, DurableTransactionIntent, IdempotencyEpoch,
};
use crate::metadata::{
    self, decode_relation_mutations, decode_relation_rewrite_intents, encode_relation_mutations,
    encode_relation_rewrite_intents,
};
use crate::runtime::CodecError;

pub const MUTATION_CODEC_VERSION: u16 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CommitRecord {
    pub(crate) target_revision: RevisionId,
    pub(crate) prepare_lsn: u64,
    pub(crate) prepare_payload_crc32c: u32,
}

pub(crate) fn encode_commit_payload(record: &CommitRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    crate::binary_codec::push_u64(&mut out, record.prepare_lsn);
    crate::binary_codec::push_u32(&mut out, record.prepare_payload_crc32c);
    out
}

pub(crate) fn decode_commit_payload(
    target_revision: RevisionId,
    payload: &[u8],
) -> Result<CommitRecord, &'static str> {
    if payload.len() != 12 {
        return Err("commit payload length mismatch");
    }
    Ok(CommitRecord {
        target_revision,
        prepare_lsn: read_u64(&payload[0..8]),
        prepare_payload_crc32c: read_u32(&payload[8..12]),
    })
}

fn encode_schema_migration_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    encoded_target_revision: &[u8],
    migration_complement: &DurableMigrationComplement,
    semantic_modules: &[BuiltinSemanticModuleSpec],
) -> Result<(), CodecError> {
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
    {
        return Err(CodecError::CollectionTooLarge);
    }
    let DurableRevisionChange::FullRevision {
        encoded_target_revision: change_target,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if encoded_target_revision != change_target {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(4);
    push_bytes(out, encoded_target_revision)?;
    metadata::encode_migration_complements(out, std::slice::from_ref(migration_complement))?;
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    Ok(())
}

fn encode_relation_data_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    semantic_revision: SemanticRevision,
    relation_mutations: &[DurableRelationMutation],
    semantic_modules: &[BuiltinSemanticModuleSpec],
) -> Result<(), CodecError> {
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
        || semantic_revision != *change_semantics
        || relation_mutations != change_mutations
    {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(2);
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    push_u64(out, semantic_revision.schema.raw());
    push_u64(out, semantic_revision.environment.raw());
    encode_relation_mutations(out, relation_mutations)?;
    Ok(())
}

fn encode_relation_resolution_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    source_revision: RevisionId,
    target_revision: RevisionId,
    semantic_revision: SemanticRevision,
    resolution: &DurableRelationResolution,
    semantic_modules: &[BuiltinSemanticModuleSpec],
) -> Result<(), CodecError> {
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if source_revision != descriptor.source_revision
        || target_revision != descriptor.target_revision
        || semantic_revision != *change_semantics
        || resolution.relation_mutations != *change_mutations
    {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(5);
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    push_u64(out, semantic_revision.schema.raw());
    push_u64(out, semantic_revision.environment.raw());
    encode_relation_mutations(out, &resolution.relation_mutations)?;
    encode_relation_rewrite_intents(out, &resolution.rewrite_intents)?;
    push_len(out, resolution.causal_parents.len())?;
    for parent in &resolution.causal_parents {
        push_u64(out, parent.raw());
    }
    Ok(())
}

fn encode_relation_rewrite_prepare(
    out: &mut Vec<u8>,
    descriptor: &DurableRevisionDescriptor,
    intent: &DurableTransactionIntent,
) -> Result<(), CodecError> {
    let DurableTransactionIntent::RelationRewriteExact {
        source_revision,
        target_revision,
        semantic_revision,
        relation_mutations,
        rewrite_intents,
        semantic_modules,
    } = intent
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    let DurableRevisionChange::RelationData {
        semantic_revision: change_semantics,
        relation_mutations: change_mutations,
    } = &descriptor.change
    else {
        return Err(CodecError::CollectionTooLarge);
    };
    if *source_revision != descriptor.source_revision
        || *target_revision != descriptor.target_revision
        || *semantic_revision != *change_semantics
        || relation_mutations != change_mutations
    {
        return Err(CodecError::CollectionTooLarge);
    }
    out.push(3);
    metadata::encode_semantic_module_specs(out, semantic_modules)?;
    push_u64(out, semantic_revision.schema.raw());
    push_u64(out, semantic_revision.environment.raw());
    encode_relation_mutations(out, relation_mutations)?;
    encode_relation_rewrite_intents(out, rewrite_intents)?;
    Ok(())
}

fn encode_prepare_identity_prefix(out: &mut Vec<u8>, descriptor: &DurableRevisionDescriptor) {
    let identity_bound = descriptor.idempotency_epoch != IdempotencyEpoch::ZERO
        || descriptor.revision_effect_id.is_some();
    push_u16(
        out,
        if identity_bound {
            MUTATION_CODEC_VERSION
        } else {
            8
        },
    );
    push_u128(out, descriptor.transaction_id.raw());
    if identity_bound {
        push_u64(out, descriptor.idempotency_epoch.raw());
        match descriptor.revision_effect_id {
            Some(id) => {
                out.push(1);
                push_u128(out, id.0);
            }
            None => out.push(0),
        }
    }
    push_u64(out, descriptor.source_revision.raw());
}

pub(crate) fn encode_prepare_payload(
    descriptor: &DurableRevisionDescriptor,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    encode_prepare_identity_prefix(&mut out, descriptor);
    match &descriptor.intent {
        DurableTransactionIntent::RelationDataExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            semantic_modules,
        } => encode_relation_data_prepare(
            &mut out,
            descriptor,
            *source_revision,
            *target_revision,
            *semantic_revision,
            relation_mutations,
            semantic_modules,
        )?,
        intent @ DurableTransactionIntent::RelationRewriteExact { .. } => {
            encode_relation_rewrite_prepare(&mut out, descriptor, intent)?;
        }
        DurableTransactionIntent::RelationResolutionExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            causal_parents,
            semantic_modules,
        } => encode_relation_resolution_prepare(
            &mut out,
            descriptor,
            *source_revision,
            *target_revision,
            *semantic_revision,
            &DurableRelationResolution {
                relation_mutations: relation_mutations.clone(),
                rewrite_intents: rewrite_intents.clone(),
                causal_parents: causal_parents.clone(),
            },
            semantic_modules,
        )?,
        DurableTransactionIntent::Exact {
            target_revision,
            encoded_target_revision,
            materializations,
            semantic_modules,
        } => {
            if *target_revision != descriptor.target_revision {
                return Err(CodecError::CollectionTooLarge);
            }
            out.push(1);
            push_bytes(&mut out, encoded_target_revision)?;
            match materializations {
                None => out.push(0),
                Some(specs) => {
                    out.push(1);
                    metadata::encode_materialization_specs(&mut out, specs)?;
                }
            }
            metadata::encode_semantic_module_specs(&mut out, semantic_modules)?;
            match &descriptor.change {
                DurableRevisionChange::FullRevision { .. } => out.push(1),
                DurableRevisionChange::FullRevisionAndMaterializations { .. } => out.push(2),
                DurableRevisionChange::RelationData { .. } => {
                    return Err(CodecError::CollectionTooLarge);
                }
            }
        }
        DurableTransactionIntent::SchemaMigrationExact {
            source_revision,
            target_revision,
            encoded_target_revision,
            migration_complement,
            semantic_modules,
        } => encode_schema_migration_prepare(
            &mut out,
            descriptor,
            *source_revision,
            *target_revision,
            encoded_target_revision,
            migration_complement,
            semantic_modules,
        )?,
        DurableTransactionIntent::LegacyTargetOnly { .. } => {
            return Err(CodecError::CollectionTooLarge);
        }
    }
    Ok(out)
}

pub(crate) fn decode_prepare_payload(
    target_revision: RevisionId,
    payload: &[u8],
) -> Result<DurableRevisionDescriptor, &'static str> {
    let mut cursor = Cursor::new(payload);
    let version = cursor.u16()?;
    let transaction_id = ClientTransactionId::new(cursor.u128()?);
    let (idempotency_epoch, revision_effect_id) = if version >= 9 {
        let epoch = IdempotencyEpoch::new(cursor.u64()?);
        let effect = match cursor.u8()? {
            0 => None,
            1 => Some(RevisionEffectId(cursor.u128()?)),
            _ => return Err("invalid revision effect identity tag"),
        };
        (epoch, effect)
    } else {
        (IdempotencyEpoch::ZERO, None)
    };
    let source_revision = RevisionId::new(cursor.u64()?);
    let (intent, change) = match version {
        2 => (
            DurableTransactionIntent::LegacyTargetOnly { target_revision },
            DurableRevisionChange::RelationData {
                semantic_revision: SemanticRevision::new(
                    kernel_types::SchemaRevisionId::new(cursor.u64()?),
                    kernel_types::SemanticEnvId::new(cursor.u64()?),
                ),
                relation_mutations: decode_relation_mutations(&mut cursor)?,
            },
        ),
        3 => {
            let change = match cursor.u8()? {
                0 => DurableRevisionChange::RelationData {
                    semantic_revision: SemanticRevision::new(
                        kernel_types::SchemaRevisionId::new(cursor.u64()?),
                        kernel_types::SemanticEnvId::new(cursor.u64()?),
                    ),
                    relation_mutations: decode_relation_mutations(&mut cursor)?,
                },
                1 => {
                    let len = cursor.len()?;
                    DurableRevisionChange::FullRevision {
                        encoded_target_revision: cursor.take(len)?.to_vec(),
                    }
                }
                _ => return Err("unknown durable revision change tag"),
            };
            let intent = match &change {
                DurableRevisionChange::FullRevision {
                    encoded_target_revision,
                } => DurableTransactionIntent::Exact {
                    target_revision,
                    encoded_target_revision: encoded_target_revision.clone(),
                    materializations: None,
                    semantic_modules: Vec::new(),
                },
                DurableRevisionChange::RelationData { .. }
                | DurableRevisionChange::FullRevisionAndMaterializations { .. } => {
                    DurableTransactionIntent::LegacyTargetOnly { target_revision }
                }
            };
            (intent, change)
        }
        4 => decode_v4_prepare_payload(&mut cursor, target_revision)?,
        5 | 6 | 7 | 8 | MUTATION_CODEC_VERSION => {
            decode_current_prepare_payload(&mut cursor, source_revision, target_revision)?
        }
        _ => return Err("unsupported mutation codec version"),
    };
    cursor.finish()?;
    Ok(DurableRevisionDescriptor {
        idempotency_epoch,
        revision_effect_id,
        transaction_id,
        source_revision,
        target_revision,
        intent,
        change,
    })
}

fn decode_v4_prepare_payload(
    cursor: &mut Cursor<'_>,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    if cursor.u8()? != 1 {
        return Err("new durable transaction intent is not exact");
    }
    let encoded_target_revision = {
        let len = cursor.len()?;
        cursor.take(len)?.to_vec()
    };
    let materializations = match cursor.u8()? {
        0 => None,
        1 => Some(metadata::decode_materialization_specs(cursor)?),
        _ => return Err("invalid durable transaction materialization tag"),
    };
    let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
    let intent = DurableTransactionIntent::Exact {
        target_revision,
        encoded_target_revision: encoded_target_revision.clone(),
        materializations: materializations.clone(),
        semantic_modules,
    };
    let change = match cursor.u8()? {
        0 => DurableRevisionChange::RelationData {
            semantic_revision: SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            ),
            relation_mutations: decode_relation_mutations(cursor)?,
        },
        1 => {
            if materializations.is_some() {
                return Err("full revision record unexpectedly carries materializations");
            }
            DurableRevisionChange::FullRevision {
                encoded_target_revision,
            }
        }
        2 => DurableRevisionChange::FullRevisionAndMaterializations {
            encoded_target_revision,
            materializations: materializations
                .ok_or("combined revision record is missing materialization registry")?,
        },
        _ => return Err("unknown durable revision change tag"),
    };
    Ok((intent, change))
}

fn decode_current_prepare_payload(
    cursor: &mut Cursor<'_>,
    source_revision: RevisionId,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    match cursor.u8()? {
        5 => decode_relation_resolution_prepare(cursor, source_revision, target_revision),
        4 => decode_schema_migration_prepare(cursor, source_revision, target_revision),
        3 => {
            let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
            let semantic_revision = SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            );
            let relation_mutations = decode_relation_mutations(cursor)?;
            let rewrite_intents = decode_relation_rewrite_intents(cursor)?;
            if relation_mutations.len() != rewrite_intents.len()
                || relation_mutations
                    .iter()
                    .zip(&rewrite_intents)
                    .any(|(mutation, intent)| mutation.relation != intent.relation)
            {
                return Err("relation rewrite intents do not match relation mutations");
            }
            let intent = DurableTransactionIntent::RelationRewriteExact {
                source_revision,
                target_revision,
                semantic_revision,
                relation_mutations: relation_mutations.clone(),
                rewrite_intents,
                semantic_modules,
            };
            let change = DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            };
            Ok((intent, change))
        }
        2 => {
            let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
            let semantic_revision = SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            );
            let relation_mutations = decode_relation_mutations(cursor)?;
            let intent = DurableTransactionIntent::RelationDataExact {
                source_revision,
                target_revision,
                semantic_revision,
                relation_mutations: relation_mutations.clone(),
                semantic_modules,
            };
            let change = DurableRevisionChange::RelationData {
                semantic_revision,
                relation_mutations,
            };
            Ok((intent, change))
        }
        1 => {
            let encoded_target_revision = {
                let len = cursor.len()?;
                cursor.take(len)?.to_vec()
            };
            let materializations = match cursor.u8()? {
                0 => None,
                1 => Some(metadata::decode_materialization_specs(cursor)?),
                _ => return Err("invalid durable transaction materialization tag"),
            };
            let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
            let intent = DurableTransactionIntent::Exact {
                target_revision,
                encoded_target_revision: encoded_target_revision.clone(),
                materializations: materializations.clone(),
                semantic_modules,
            };
            let change = match cursor.u8()? {
                1 => {
                    if materializations.is_some() {
                        return Err("full revision record unexpectedly carries materializations");
                    }
                    DurableRevisionChange::FullRevision {
                        encoded_target_revision,
                    }
                }
                2 => DurableRevisionChange::FullRevisionAndMaterializations {
                    encoded_target_revision,
                    materializations: materializations
                        .ok_or("combined revision record is missing materialization registry")?,
                },
                _ => return Err("unknown durable revision change tag"),
            };
            Ok((intent, change))
        }
        _ => Err("new durable transaction intent tag is invalid"),
    }
}

fn decode_relation_resolution_prepare(
    cursor: &mut Cursor<'_>,
    source_revision: RevisionId,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
    let semantic_revision = SemanticRevision::new(
        kernel_types::SchemaRevisionId::new(cursor.u64()?),
        kernel_types::SemanticEnvId::new(cursor.u64()?),
    );
    let relation_mutations = decode_relation_mutations(cursor)?;
    let rewrite_intents = decode_relation_rewrite_intents(cursor)?;
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
    let mut causal_parents = Vec::with_capacity(cursor.bounded_capacity(count));
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
    Ok((
        DurableTransactionIntent::RelationResolutionExact {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations: relation_mutations.clone(),
            rewrite_intents,
            causal_parents,
            semantic_modules,
        },
        DurableRevisionChange::RelationData {
            semantic_revision,
            relation_mutations,
        },
    ))
}

fn decode_schema_migration_prepare(
    cursor: &mut Cursor<'_>,
    source_revision: RevisionId,
    target_revision: RevisionId,
) -> Result<(DurableTransactionIntent, DurableRevisionChange), &'static str> {
    let encoded_target_revision = {
        let len = cursor.len()?;
        cursor.take(len)?.to_vec()
    };
    let mut complements = metadata::decode_migration_complements(cursor)?;
    if complements.len() != 1 {
        return Err("schema migration prepare must carry exactly one complement");
    }
    let migration_complement = complements.remove(0);
    let semantic_modules = metadata::decode_semantic_module_specs(cursor)?;
    let intent = DurableTransactionIntent::SchemaMigrationExact {
        source_revision,
        target_revision,
        encoded_target_revision: encoded_target_revision.clone(),
        migration_complement,
        semantic_modules,
    };
    Ok((
        intent,
        DurableRevisionChange::FullRevision {
            encoded_target_revision,
        },
    ))
}
