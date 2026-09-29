mod revision_codec;
mod semantic_codec;
mod state_codec;

pub(crate) use revision_codec::{
    decode_revision, decode_revision_from_reader, encode_revision, encoded_revision_len,
    stream_revision,
};

#[cfg(test)]
mod tests;
