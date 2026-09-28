use std::collections::BTreeSet;

use kernel_change::RevisionEffectId;
use kernel_types::{ClientTransactionId, RevisionId};

use super::super::{
    DurableSequencerOrder, ReplicaId, ReplicatedEffectEnvelope, ReplicationBranchId,
    ReplicationMembership, ReplicationMembershipChange, ReplicationQuorumCertificate,
};
use crate::binary_codec::{Cursor, push_len, push_u64, push_u128};
use crate::domain::{DurableRevisionEffectRecord, IdempotencyEpoch};
use crate::metadata;
use crate::runtime::CodecError;

#[must_use]
pub const fn replicated_effect_id(origin: ReplicaId, sequence: u64) -> RevisionEffectId {
    RevisionEffectId(((origin.raw() as u128) << 64) | sequence as u128)
}

#[must_use]
pub const fn replicated_origin(id: RevisionEffectId) -> Option<ReplicaId> {
    let raw = (id.0 >> 64) as u64;
    if raw == 0 {
        None
    } else {
        Some(ReplicaId::new(raw))
    }
}

pub(in crate::replication) fn encode_ingest(
    envelope: &ReplicatedEffectEnvelope,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, envelope.origin.raw());
    push_u64(&mut out, envelope.origin_sequence);
    push_u128(&mut out, envelope.branch.raw());
    push_u64(&mut out, envelope.ordered_by.sequencer.raw());
    push_u64(&mut out, envelope.ordered_by.epoch);
    push_u64(&mut out, envelope.ordered_by.position);
    encode_effect(&mut out, &envelope.effect)?;
    Ok(out)
}

pub(in crate::replication) fn decode_ingest(
    bytes: &[u8],
) -> Result<ReplicatedEffectEnvelope, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let origin = ReplicaId::new(cursor.u64()?);
    let origin_sequence = cursor.u64()?;
    let branch = ReplicationBranchId::new(cursor.u128()?);
    let ordered_by = DurableSequencerOrder {
        sequencer: ReplicaId::new(cursor.u64()?),
        epoch: cursor.u64()?,
        position: cursor.u64()?,
    };
    let effect = decode_effect(&mut cursor)?;
    cursor.finish()?;
    Ok(ReplicatedEffectEnvelope {
        origin,
        origin_sequence,
        branch,
        effect,
        ordered_by,
    })
}

fn encode_effect(
    out: &mut Vec<u8>,
    effect: &DurableRevisionEffectRecord,
) -> Result<(), CodecError> {
    push_u128(out, effect.id.0);
    push_len(out, effect.prerequisites.len())?;
    for prerequisite in &effect.prerequisites {
        push_u128(out, prerequisite.0);
    }
    push_u64(out, effect.transaction_epoch.raw());
    push_u128(out, effect.transaction_id.raw());
    metadata::encode_transaction_intent(out, &effect.intent)?;
    push_u64(out, effect.source_revision.raw());
    push_u64(out, effect.target_revision.raw());
    Ok(())
}

fn decode_effect(cursor: &mut Cursor<'_>) -> Result<DurableRevisionEffectRecord, &'static str> {
    let id = RevisionEffectId(cursor.u128()?);
    let count = cursor.len()?;
    let mut prerequisites = BTreeSet::new();
    let mut previous = None;
    for _ in 0..count {
        let prerequisite = RevisionEffectId(cursor.u128()?);
        if previous.is_some_and(|prior| prior >= prerequisite) {
            return Err("replicated prerequisites are not strictly sorted and unique");
        }
        previous = Some(prerequisite);
        prerequisites.insert(prerequisite);
    }
    let transaction_epoch = IdempotencyEpoch::new(cursor.u64()?);
    let transaction_id = ClientTransactionId::new(cursor.u128()?);
    let intent = metadata::decode_transaction_intent(cursor)?;
    let source_revision = RevisionId::new(cursor.u64()?);
    let target_revision = RevisionId::new(cursor.u64()?);
    let effect = DurableRevisionEffectRecord {
        id,
        prerequisites,
        transaction_epoch,
        transaction_id,
        intent,
        source_revision,
        target_revision,
    };
    effect.validate_identity()?;
    Ok(effect)
}

pub(in crate::replication) fn decode_retire(
    bytes: &[u8],
) -> Result<(ReplicationBranchId, RevisionEffectId), &'static str> {
    let mut cursor = Cursor::new(bytes);
    let branch = ReplicationBranchId::new(cursor.u128()?);
    let effect = RevisionEffectId(cursor.u128()?);
    cursor.finish()?;
    Ok((branch, effect))
}

pub(in crate::replication) fn encode_membership_change(
    change: &ReplicationMembershipChange,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u64(&mut out, change.next.epoch);
    push_len(&mut out, change.next.members.len())?;
    for member in &change.next.members {
        push_u64(&mut out, member.raw());
    }
    push_len(&mut out, change.next.quorum_size)?;
    push_len(&mut out, change.acknowledged_by_previous.len())?;
    for member in &change.acknowledged_by_previous {
        push_u64(&mut out, member.raw());
    }
    Ok(out)
}

pub(in crate::replication) fn decode_membership_change(
    bytes: &[u8],
) -> Result<ReplicationMembershipChange, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let epoch = cursor.u64()?;
    let members = decode_replica_set(
        &mut cursor,
        "replication membership is not sorted and unique",
    )?;
    let quorum_size = cursor.len()?;
    let acknowledged_by_previous = decode_replica_set(
        &mut cursor,
        "replication membership acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationMembershipChange {
        next: ReplicationMembership {
            epoch,
            members,
            quorum_size,
        },
        acknowledged_by_previous,
    })
}

pub(in crate::replication) fn encode_quorum_certificate(
    certificate: &ReplicationQuorumCertificate,
) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u128(&mut out, certificate.effect.0);
    push_u64(&mut out, certificate.membership_epoch);
    push_len(&mut out, certificate.acknowledged_by.len())?;
    for member in &certificate.acknowledged_by {
        push_u64(&mut out, member.raw());
    }
    Ok(out)
}

pub(in crate::replication) fn decode_quorum_certificate(
    bytes: &[u8],
) -> Result<ReplicationQuorumCertificate, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let effect = RevisionEffectId(cursor.u128()?);
    let membership_epoch = cursor.u64()?;
    let acknowledged_by = decode_replica_set(
        &mut cursor,
        "replication quorum acknowledgements are not sorted and unique",
    )?;
    cursor.finish()?;
    Ok(ReplicationQuorumCertificate {
        effect,
        membership_epoch,
        acknowledged_by,
    })
}

pub(in crate::replication) fn decode_publish(
    bytes: &[u8],
) -> Result<RevisionEffectId, &'static str> {
    let mut cursor = Cursor::new(bytes);
    let effect = RevisionEffectId(cursor.u128()?);
    cursor.finish()?;
    Ok(effect)
}

pub(super) fn decode_replica_set(
    cursor: &mut Cursor<'_>,
    unsorted_reason: &'static str,
) -> Result<BTreeSet<ReplicaId>, &'static str> {
    let count = cursor.len()?;
    let mut replicas = BTreeSet::new();
    let mut previous = None;
    for _ in 0..count {
        let replica = ReplicaId::new(cursor.u64()?);
        if previous.is_some_and(|prior| prior >= replica) {
            return Err(unsorted_reason);
        }
        previous = Some(replica);
        replicas.insert(replica);
    }
    Ok(replicas)
}
