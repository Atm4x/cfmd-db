mod revision_codec;
mod semantic_codec;
mod state_codec;

pub(crate) use state_codec::{decode_state, decode_type_expr, encode_state, encode_type_expr};

pub(crate) use revision_codec::{
    CHECKPOINT_CODEC_VERSION, decode_context, decode_revision, decode_revision_from_reader,
    encode_context, encode_revision, encoded_revision_len, stream_revision,
};

#[cfg(test)]
mod tests;
