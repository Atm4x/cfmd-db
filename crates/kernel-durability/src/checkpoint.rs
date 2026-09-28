mod revision_codec;
mod semantic_codec;
mod state_codec;

pub(crate) use revision_codec::{decode_revision, encode_revision};

#[cfg(test)]
mod tests;
