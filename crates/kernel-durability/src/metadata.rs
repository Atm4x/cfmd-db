mod aggregate;
mod artifact_codec;
mod intent_codec;
mod migration_program_codec;
mod model_delta_codec;
pub(crate) mod query_codec;
mod revision_change_codec;

#[cfg(test)]
pub(crate) use aggregate::encode;
pub(crate) use aggregate::{
    DurableCheckpointRealizationBinding, DurableStoreMetadata, decode, decode_from_reader,
    encoded_len, stream,
};
pub(crate) use artifact_codec::{decode_materialization_specs, encode_materialization_specs};
#[cfg(test)]
pub(crate) use intent_codec::encode_semantic_module_specs;
pub(crate) use intent_codec::{
    decode_relation_mutations, decode_transaction_intent, encode_relation_mutations,
    encode_transaction_intent,
};
pub(crate) use migration_program_codec::{
    decode_schema_migration_program, encode_schema_migration_program,
};
pub(crate) use model_delta_codec::{decode_model_delta, encode_model_delta};
pub(crate) use revision_change_codec::{decode_revision_change, encode_revision_change};

#[cfg(test)]
mod tests;

pub(crate) use intent_codec::canonical_relation_mutations_identity;
