use kernel_change::RevisionEffectId;
use kernel_types::{ClientTransactionId, RevisionId};

use crate::binary_codec::{Cursor, push_u16, push_u64, push_u128, read_u32, read_u64};
use crate::descriptor::DurableRevisionDescriptor;
use crate::domain::IdempotencyEpoch;
use crate::metadata;
use crate::runtime::CodecError;

/// Current pre-release mutation payload discriminator.
///
/// CFMD has no released on-disk compatibility boundary yet. This tag identifies
/// the one maintained layout; older internal pass layouts fail closed instead of
/// creating compatibility branches in the active runtime.
pub const MUTATION_FORMAT_TAG: u16 = 0xC469;

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

fn encode_prepare_identity_prefix(out: &mut Vec<u8>, descriptor: &DurableRevisionDescriptor) {
    push_u16(out, MUTATION_FORMAT_TAG);
    push_u128(out, descriptor.transaction_id.raw());
    push_u64(out, descriptor.idempotency_epoch.raw());
    match descriptor.revision_effect_id {
        Some(id) => {
            out.push(1);
            push_u128(out, id.0);
        }
        None => out.push(0),
    }
    push_u64(out, descriptor.source_revision.raw());
}

/// PREPARE is deliberately just three orthogonal authorities:
/// transaction/provenance identity, canonical client intent, and realized change.
/// Direct vs residual/rebased/schema-transported is not a durable wire taxonomy.
pub(crate) fn encode_prepare_payload(
    descriptor: &DurableRevisionDescriptor,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    encode_prepare_identity_prefix(&mut out, descriptor);
    metadata::encode_transaction_intent(&mut out, &descriptor.intent)?;
    metadata::encode_revision_change(&mut out, &descriptor.change)?;
    Ok(out)
}

pub(crate) fn decode_prepare_payload(
    target_revision: RevisionId,
    payload: &[u8],
) -> Result<DurableRevisionDescriptor, &'static str> {
    let mut cursor = Cursor::new(payload);
    if cursor.u16()? != MUTATION_FORMAT_TAG {
        return Err("unsupported pre-release mutation payload format");
    }
    let transaction_id = ClientTransactionId::new(cursor.u128()?);
    let idempotency_epoch = IdempotencyEpoch::new(cursor.u64()?);
    let revision_effect_id = match cursor.u8()? {
        0 => None,
        1 => Some(RevisionEffectId(cursor.u128()?)),
        _ => return Err("invalid revision effect identity tag"),
    };
    let source_revision = RevisionId::new(cursor.u64()?);
    let intent = metadata::decode_transaction_intent(&mut cursor)?;
    let change = metadata::decode_revision_change(&mut cursor)?;
    cursor.finish()?;

    let descriptor = DurableRevisionDescriptor {
        idempotency_epoch,
        revision_effect_id,
        transaction_id,
        source_revision,
        target_revision,
        intent,
        change,
    };
    if descriptor.intent.target_revision() != target_revision {
        return Err("prepare intent target revision mismatch");
    }
    if descriptor
        .intent
        .source_revision()
        .is_some_and(|source| source != source_revision)
    {
        return Err("prepare intent source revision mismatch");
    }
    Ok(descriptor)
}
