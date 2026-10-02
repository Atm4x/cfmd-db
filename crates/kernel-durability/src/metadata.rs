mod aggregate;
mod artifact_codec;
mod intent_codec;
mod migration_program_codec;
mod model_delta_codec;
mod query_codec;

#[cfg(test)]
pub(crate) use aggregate::encode;
pub(crate) use aggregate::{
    DurableCheckpointRealizationBinding, DurableStoreMetadata, decode, decode_from_reader,
    encoded_len, stream,
};
pub(crate) use artifact_codec::{
    decode_materialization_specs, decode_migration_complements, encode_materialization_specs,
    encode_migration_complements,
};
pub(crate) use intent_codec::{
    decode_relation_mutations, decode_relation_mutations_legacy, decode_relation_rewrite_intents, decode_semantic_module_specs,
    decode_transaction_intent, encode_relation_mutations, encode_relation_rewrite_intents,
    encode_semantic_module_specs, encode_transaction_intent,
};
pub(crate) use migration_program_codec::{
    decode_schema_migration_program, encode_schema_migration_program,
};
pub(crate) use model_delta_codec::{decode_model_delta, encode_model_delta};

#[cfg(test)]
mod tests;
