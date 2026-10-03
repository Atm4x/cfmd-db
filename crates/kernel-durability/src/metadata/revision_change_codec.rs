use kernel_types::SemanticRevision;

use crate::binary_codec::{BinarySink, BinarySource, push_bytes, push_u64};
use crate::domain::DurableRevisionChange;
use crate::runtime::CodecError;

use super::{
    decode_materialization_specs, decode_model_delta, decode_relation_mutations,
    decode_schema_migration_program, encode_materialization_specs, encode_model_delta,
    encode_relation_mutations, encode_schema_migration_program,
};

pub(crate) fn encode_revision_change(
    out: &mut impl BinarySink,
    change: &DurableRevisionChange,
) -> Result<(), CodecError> {
    match change {
        DurableRevisionChange::RelationData {
            semantic_revision,
            relation_mutations,
        } => {
            out.push(1);
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
        }
        DurableRevisionChange::MixedRevision {
            semantic_revision,
            relation_mutations,
            model_delta,
            model_complement,
        } => {
            out.push(2);
            push_u64(out, semantic_revision.schema.raw());
            push_u64(out, semantic_revision.environment.raw());
            encode_relation_mutations(out, relation_mutations)?;
            encode_model_delta(out, model_delta)?;
            match model_complement {
                None => out.push(0),
                Some(complement) => {
                    out.push(1);
                    encode_model_delta(out, complement)?;
                }
            }
        }
        DurableRevisionChange::FullRevision {
            encoded_target_revision,
        } => {
            out.push(3);
            push_bytes(out, encoded_target_revision)?;
        }
        DurableRevisionChange::FullRevisionAndMaterializations {
            encoded_target_revision,
            materializations,
        } => {
            out.push(4);
            push_bytes(out, encoded_target_revision)?;
            encode_materialization_specs(out, materializations)?;
        }
        DurableRevisionChange::SchemaMigration { program } => {
            out.push(5);
            encode_schema_migration_program(out, program)?;
        }
    }
    Ok(())
}

pub(crate) fn decode_revision_change(
    cursor: &mut impl BinarySource,
) -> Result<DurableRevisionChange, &'static str> {
    match cursor.u8()? {
        1 => Ok(DurableRevisionChange::RelationData {
            semantic_revision: SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            ),
            relation_mutations: decode_relation_mutations(cursor)?,
        }),
        2 => {
            let semantic_revision = SemanticRevision::new(
                kernel_types::SchemaRevisionId::new(cursor.u64()?),
                kernel_types::SemanticEnvId::new(cursor.u64()?),
            );
            let relation_mutations = decode_relation_mutations(cursor)?;
            let model_delta = decode_model_delta(cursor)?;
            let model_complement = match cursor.u8()? {
                0 => None,
                1 => Some(Box::new(decode_model_delta(cursor)?)),
                _ => return Err("invalid realized model complement tag"),
            };
            Ok(DurableRevisionChange::MixedRevision {
                semantic_revision,
                relation_mutations,
                model_delta,
                model_complement,
            })
        }
        3 => {
            let len = cursor.len()?;
            Ok(DurableRevisionChange::FullRevision {
                encoded_target_revision: cursor.take_owned(len)?,
            })
        }
        4 => {
            let len = cursor.len()?;
            Ok(DurableRevisionChange::FullRevisionAndMaterializations {
                encoded_target_revision: cursor.take_owned(len)?,
                materializations: decode_materialization_specs(cursor)?,
            })
        }
        5 => Ok(DurableRevisionChange::SchemaMigration {
            program: decode_schema_migration_program(cursor)?,
        }),
        _ => Err("invalid durable revision change tag"),
    }
}
