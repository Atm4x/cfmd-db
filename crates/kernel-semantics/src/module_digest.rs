use kernel_schema::ModuleDigest;
use kernel_types::SemanticId;

pub(super) fn typed_entity_digest(tag: u8, entity_type: SemanticId) -> ModuleDigest {
    let raw = entity_type.raw().to_le_bytes();
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(&raw);
    bytes[16..].copy_from_slice(&raw);
    bytes[0] ^= tag;
    bytes[16] ^= tag.rotate_left(1);
    ModuleDigest(bytes)
}
