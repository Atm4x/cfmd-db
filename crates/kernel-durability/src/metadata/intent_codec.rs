use kernel_semantics::{
    BuiltinSemanticModuleSpec, EquivalenceModule, OrderingModule, TokenizerModule,
};
use kernel_types::{RevisionId, SemanticId, SemanticRevision};

use crate::binary_codec::{BinarySource, encode_rows, push_bytes, push_len, push_u64, push_u128};
use crate::domain::{
    ClientIntentGuardDigest, DurableClientIntent, DurableCommittedTransaction,
    DurableRelationMutation, DurableRelationRewriteIntent, DurableTransactionIntent,
};
use crate::runtime::CodecError;

use super::artifact_codec::{
    decode_materialization_specs, decode_migration_complements, encode_materialization_specs,
    encode_migration_complements,
};

pub(crate) fn encode_committed_transaction(
    out: &mut impl crate::binary_codec::BinarySink,
    committed: &DurableCommittedTransaction,
) -> Result<(), CodecError> {
    push_u64(out, committed.target_revision.raw());
    match &committed.intent {
        DurableClientIntent::RelationData {
            semantic_revision,
            relation_mutations,
            guard_digest,
        } => {
            out.push(1);
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            encode_optional_guard_digest(out, *guard_digest);
        }
        DurableClientIntent::MixedRevision {
            semantic_revision,
            relation_mutations,
            model_delta,
            guard_digest,
        } => {
            out.push(2);
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            super::model_delta_codec::encode_model_delta(out, model_delta)?;
            encode_optional_guard_digest(out, *guard_digest);
        }
        DurableClientIntent::RelationRewrite {
            semantic_revision,
            relation_mutations,
            rewrite_intents,
        } => {
            out.push(3);
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            encode_relation_rewrite_intents(out, rewrite_intents)?;
        }
        DurableClientIntent::RelationResolution {
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            causal_parents,
        } => {
            out.push(4);
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            encode_relation_rewrite_intents(out, rewrite_intents)?;
            push_len(out, causal_parents.len())?;
            for parent in causal_parents {
                push_u64(out, parent.raw());
            }
        }
        DurableClientIntent::FullRevision {
            encoded_target_revision,
            materializations,
        } => {
            out.push(5);
            push_bytes(out, encoded_target_revision)?;
            match materializations {
                None => out.push(0),
                Some(specs) => {
                    out.push(1);
                    encode_materialization_specs(out, specs)?;
                }
            }
        }
        DurableClientIntent::SchemaMigration { program } => {
            out.push(6);
            super::migration_program_codec::encode_schema_migration_program(out, program)?;
        }
    }
    Ok(())
}

pub(crate) fn decode_committed_transaction(
    cursor: &mut impl BinarySource,
) -> Result<DurableCommittedTransaction, &'static str> {
    let target_revision = RevisionId::new(cursor.u64()?);
    let intent = match cursor.u8()? {
        1 => DurableClientIntent::RelationData {
            semantic_revision: decode_semantic_revision(cursor)?,
            relation_mutations: decode_relation_mutations(cursor)?,
            guard_digest: decode_optional_guard_digest(cursor)?,
        },
        2 => DurableClientIntent::MixedRevision {
            semantic_revision: decode_semantic_revision(cursor)?,
            relation_mutations: decode_relation_mutations(cursor)?,
            model_delta: super::model_delta_codec::decode_model_delta(cursor)?,
            guard_digest: decode_optional_guard_digest(cursor)?,
        },
        3 => DurableClientIntent::RelationRewrite {
            semantic_revision: decode_semantic_revision(cursor)?,
            relation_mutations: decode_relation_mutations(cursor)?,
            rewrite_intents: decode_relation_rewrite_intents(cursor)?,
        },
        4 => {
            let semantic_revision = decode_semantic_revision(cursor)?;
            let relation_mutations = decode_relation_mutations(cursor)?;
            let rewrite_intents = decode_relation_rewrite_intents(cursor)?;
            let count = cursor.len()?;
            let mut causal_parents = Vec::with_capacity(cursor.bounded_capacity(count));
            let mut previous = None;
            for _ in 0..count {
                let parent = RevisionId::new(cursor.u64()?);
                if previous.is_some_and(|prior| prior >= parent) {
                    return Err("committed resolution parents are not strictly sorted");
                }
                previous = Some(parent);
                causal_parents.push(parent);
            }
            DurableClientIntent::RelationResolution {
                semantic_revision,
                relation_mutations,
                rewrite_intents,
                causal_parents,
            }
        }
        5 => {
            let len = cursor.len()?;
            let encoded_target_revision = cursor.take_owned(len)?;
            let materializations = match cursor.u8()? {
                0 => None,
                1 => Some(decode_materialization_specs(cursor)?),
                _ => return Err("invalid committed full-revision materialization tag"),
            };
            DurableClientIntent::FullRevision {
                encoded_target_revision,
                materializations,
            }
        }
        6 => DurableClientIntent::SchemaMigration {
            program: super::migration_program_codec::decode_schema_migration_program(cursor)?,
        },
        _ => return Err("invalid committed client intent tag"),
    };
    Ok(DurableCommittedTransaction {
        target_revision,
        intent,
    })
}

fn decode_semantic_revision(
    cursor: &mut impl BinarySource,
) -> Result<SemanticRevision, &'static str> {
    Ok(SemanticRevision::new(
        kernel_types::SchemaRevisionId::new(cursor.u64()?),
        kernel_types::SemanticEnvId::new(cursor.u64()?),
    ))
}

fn encode_optional_guard_digest(
    out: &mut impl crate::binary_codec::BinarySink,
    digest: Option<ClientIntentGuardDigest>,
) {
    match digest {
        Some(digest) => {
            out.push(1);
            out.extend_from_slice(&digest.0);
        }
        None => out.push(0),
    }
}

fn decode_optional_guard_digest(
    cursor: &mut impl BinarySource,
) -> Result<Option<ClientIntentGuardDigest>, &'static str> {
    match cursor.u8()? {
        0 => Ok(None),
        1 => Ok(Some(decode_guard_digest(cursor)?)),
        _ => Err("invalid committed guard digest tag"),
    }
}

#[allow(clippy::too_many_lines)]
pub(crate) fn encode_transaction_intent(
    out: &mut impl crate::binary_codec::BinarySink,
    intent: &DurableTransactionIntent,
) -> Result<(), CodecError> {
    match intent {
        DurableTransactionIntent::RelationData {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            client_guard_digest,
            semantic_modules,
        } => {
            out.push(1);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            encode_optional_guard_digest(out, *client_guard_digest);
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::RelationRewrite {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            semantic_modules,
        } => {
            out.push(2);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            encode_relation_rewrite_intents(out, rewrite_intents)?;
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::RelationResolution {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            rewrite_intents,
            causal_parents,
            semantic_modules,
        } => {
            out.push(3);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            encode_relation_rewrite_intents(out, rewrite_intents)?;
            push_len(out, causal_parents.len())?;
            for parent in causal_parents {
                push_u64(out, parent.raw());
            }
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::MixedRevision {
            source_revision,
            target_revision,
            semantic_revision,
            relation_mutations,
            model_delta,
            client_guard_digest,
            semantic_modules,
        } => {
            out.push(4);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            super::model_delta_codec::encode_model_delta(out, model_delta)?;
            encode_optional_guard_digest(out, *client_guard_digest);
            encode_semantic_module_specs(out, semantic_modules)?;
        }
        DurableTransactionIntent::FullRevision {
            target_revision,
            encoded_target_revision,
            materializations,
            semantic_modules,
        } => {
            out.push(5);
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
        DurableTransactionIntent::SchemaMigration {
            source_revision,
            target_revision,
            program,
            migration_complement,
            semantic_modules,
        } => {
            out.push(6);
            push_u64(out, source_revision.raw());
            push_u64(out, target_revision.raw());
            super::migration_program_codec::encode_schema_migration_program(out, program)?;
            encode_migration_complements(out, std::slice::from_ref(migration_complement))?;
            encode_semantic_module_specs(out, semantic_modules)?;
        }
    }
    Ok(())
}

pub(crate) fn decode_transaction_intent(
    cursor: &mut impl BinarySource,
) -> Result<DurableTransactionIntent, &'static str> {
    let tag = cursor.u8()?;
    match tag {
        1 => Ok(DurableTransactionIntent::RelationData {
            source_revision: RevisionId::new(cursor.u64()?),
            target_revision: RevisionId::new(cursor.u64()?),
            semantic_revision: SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            ),
            relation_mutations: decode_relation_mutations(cursor)?,
            client_guard_digest: decode_optional_guard_digest(cursor)?,
            semantic_modules: decode_semantic_module_specs(cursor)?,
        }),
        2 => {
            let source_revision = RevisionId::new(cursor.u64()?);
            let target_revision = RevisionId::new(cursor.u64()?);
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
                    .any(|(m, i)| m.relation != i.relation)
            {
                return Err("relation rewrite intents do not match relation mutations");
            }
            Ok(DurableTransactionIntent::RelationRewrite {
                source_revision,
                target_revision,
                semantic_revision,
                relation_mutations,
                rewrite_intents,
                semantic_modules: decode_semantic_module_specs(cursor)?,
            })
        }
        3 => decode_relation_resolution_intent(cursor),
        4 => Ok(DurableTransactionIntent::MixedRevision {
            source_revision: RevisionId::new(cursor.u64()?),
            target_revision: RevisionId::new(cursor.u64()?),
            semantic_revision: SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            ),
            relation_mutations: decode_relation_mutations(cursor)?,
            model_delta: super::model_delta_codec::decode_model_delta(cursor)?,
            client_guard_digest: decode_optional_guard_digest(cursor)?,
            semantic_modules: decode_semantic_module_specs(cursor)?,
        }),
        5 => {
            let target_revision = RevisionId::new(cursor.u64()?);
            let len = cursor.len()?;
            let encoded_target_revision = cursor.take_owned(len)?;
            let materializations = match cursor.u8()? {
                0 => None,
                1 => Some(decode_materialization_specs(cursor)?),
                _ => return Err("invalid transaction intent materialization tag"),
            };
            Ok(DurableTransactionIntent::FullRevision {
                target_revision,
                encoded_target_revision,
                materializations,
                semantic_modules: decode_semantic_module_specs(cursor)?,
            })
        }
        6 => {
            let source_revision = RevisionId::new(cursor.u64()?);
            let target_revision = RevisionId::new(cursor.u64()?);
            let program = super::migration_program_codec::decode_schema_migration_program(cursor)?;
            let mut complements = decode_migration_complements(cursor)?;
            if complements.len() != 1 {
                return Err("schema migration intent must carry exactly one complement");
            }
            Ok(DurableTransactionIntent::SchemaMigration {
                source_revision,
                target_revision,
                program,
                migration_complement: complements.remove(0),
                semantic_modules: decode_semantic_module_specs(cursor)?,
            })
        }
        _ => Err("unsupported pre-release transaction intent tag"),
    }
}

fn decode_guard_digest(
    cursor: &mut impl BinarySource,
) -> Result<ClientIntentGuardDigest, &'static str> {
    let mut bytes = [0_u8; 32];
    cursor.read_exact_into(&mut bytes)?;
    Ok(ClientIntentGuardDigest(bytes))
}

fn decode_relation_resolution_intent(
    cursor: &mut impl BinarySource,
) -> Result<DurableTransactionIntent, &'static str> {
    let source_revision = RevisionId::new(cursor.u64()?);
    let target_revision = RevisionId::new(cursor.u64()?);
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
    Ok(DurableTransactionIntent::RelationResolution {
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
    out: &mut impl crate::binary_codec::BinarySink,
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
    cursor: &mut impl BinarySource,
) -> Result<Vec<BuiltinSemanticModuleSpec>, &'static str> {
    let count = cursor.len()?;
    let mut modules = Vec::with_capacity(cursor.bounded_capacity(count));
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

pub(super) fn encode_semantic_module_spec(
    out: &mut impl crate::binary_codec::BinarySink,
    spec: BuiltinSemanticModuleSpec,
) {
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

pub(super) fn decode_semantic_module_spec(
    cursor: &mut impl BinarySource,
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

fn encode_ordering_module(out: &mut impl crate::binary_codec::BinarySink, module: OrderingModule) {
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

fn decode_ordering_module(cursor: &mut impl BinarySource) -> Result<OrderingModule, &'static str> {
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

fn encode_equivalence_module(
    out: &mut impl crate::binary_codec::BinarySink,
    module: EquivalenceModule,
) {
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

fn decode_equivalence_module(
    cursor: &mut impl BinarySource,
) -> Result<EquivalenceModule, &'static str> {
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

pub(crate) fn encode_relation_mutations(
    out: &mut impl crate::binary_codec::BinarySink,
    relation_mutations: &[DurableRelationMutation],
) -> Result<(), CodecError> {
    push_len(out, relation_mutations.len())?;
    let mut previous = None;
    for mutation in relation_mutations {
        if previous.is_some_and(|id: SemanticId| id >= mutation.relation) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous = Some(mutation.relation);
        push_u128(out, mutation.relation.raw());
        encode_rows(out, &mutation.inserted)?;
        encode_rows(out, &mutation.removed)?;
        push_len(out, mutation.object_field_writes.len())?;
        let mut previous_field = None;
        for write in &mutation.object_field_writes {
            let key = (write.owner, write.field);
            if previous_field.is_some_and(|previous| previous >= key) {
                return Err(CodecError::CollectionTooLarge);
            }
            previous_field = Some(key);
            push_u128(out, write.owner.raw());
            push_u128(out, write.field.raw());
            encode_rows(out, &[vec![write.value.clone()]])?;
        }
        let authorization = mutation.authorization;
        out.push(
            u8::from(authorization.relation_write)
                | (u8::from(authorization.object_create) << 1)
                | (u8::from(authorization.object_delete) << 2)
                | (u8::from(authorization.relationship_attach) << 3)
                | (u8::from(authorization.relationship_detach) << 4)
                | (u8::from(authorization.relationship_move) << 5),
        );
    }
    Ok(())
}

pub(crate) fn encode_relation_rewrite_intents(
    out: &mut impl crate::binary_codec::BinarySink,
    rewrite_intents: &[DurableRelationRewriteIntent],
) -> Result<(), CodecError> {
    push_len(out, rewrite_intents.len())?;
    let mut previous = None;
    for intent in rewrite_intents {
        if previous.is_some_and(|relation: SemanticId| relation >= intent.relation) {
            return Err(CodecError::CollectionTooLarge);
        }
        previous = Some(intent.relation);
        push_u128(out, intent.relation.raw());
        push_u128(out, intent.rewrite_spec.raw());
        push_u128(out, intent.law_set.raw());
    }
    Ok(())
}

pub(crate) fn decode_relation_rewrite_intents(
    cursor: &mut impl BinarySource,
) -> Result<Vec<DurableRelationRewriteIntent>, &'static str> {
    let count = cursor.len()?;
    let mut rewrite_intents = Vec::with_capacity(cursor.bounded_capacity(count));
    let mut previous = None;
    for _ in 0..count {
        let relation = SemanticId::new(cursor.u128()?);
        if previous.is_some_and(|id: SemanticId| id >= relation) {
            return Err("relation rewrite intents are not strictly sorted and unique");
        }
        previous = Some(relation);
        rewrite_intents.push(DurableRelationRewriteIntent {
            relation,
            rewrite_spec: SemanticId::new(cursor.u128()?),
            law_set: SemanticId::new(cursor.u128()?),
        });
    }
    Ok(rewrite_intents)
}

pub(crate) fn decode_relation_mutations(
    cursor: &mut impl BinarySource,
) -> Result<Vec<DurableRelationMutation>, &'static str> {
    let count = cursor.len()?;
    let mut relation_mutations = Vec::with_capacity(cursor.bounded_capacity(count));
    let mut previous = None;
    for _ in 0..count {
        let relation = SemanticId::new(cursor.u128()?);
        if previous.is_some_and(|id: SemanticId| id >= relation) {
            return Err("relation mutations are not strictly sorted and unique");
        }
        previous = Some(relation);
        let inserted = cursor.rows(0)?;
        let removed = cursor.rows(0)?;
        let field_count = cursor.len()?;
        let mut object_field_writes = Vec::with_capacity(cursor.bounded_capacity(field_count));
        let mut previous_field = None;
        for _ in 0..field_count {
            let owner = kernel_types::EntityId::new(cursor.u128()?);
            let field = SemanticId::new(cursor.u128()?);
            let rows = cursor.rows(1)?;
            if rows.len() != 1 || rows[0].len() != 1 {
                return Err("invalid object field write value");
            }
            let value = rows[0][0].clone();
            let key = (owner, field);
            if previous_field.is_some_and(|previous| previous >= key) {
                return Err("object field writes are not strictly sorted and unique");
            }
            previous_field = Some(key);
            object_field_writes.push(crate::DurableObjectFieldWrite {
                owner,
                field,
                value,
            });
        }
        let authorization_bits = cursor.u8()?;
        if authorization_bits & !0b00_111111 != 0 {
            return Err("invalid relation authorization bits");
        }
        let authorization = crate::DurableRelationAuthorization {
            relation_write: authorization_bits & (1 << 0) != 0,
            object_create: authorization_bits & (1 << 1) != 0,
            object_delete: authorization_bits & (1 << 2) != 0,
            relationship_attach: authorization_bits & (1 << 3) != 0,
            relationship_detach: authorization_bits & (1 << 4) != 0,
            relationship_move: authorization_bits & (1 << 5) != 0,
        };
        relation_mutations.push(DurableRelationMutation {
            relation,
            inserted,
            removed,
            object_field_writes,
            authorization,
        });
    }
    Ok(relation_mutations)
}
