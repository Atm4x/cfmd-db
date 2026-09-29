use kernel_types::{EntityId, SemanticId};

use crate::binary_codec::{BinarySource, encode_value, push_len, push_u128};
use crate::domain::{
    DurableCarrierPatch, DurableFieldPatch, DurableKeepsAlivePatch, DurableModelDelta,
};
use crate::runtime::CodecError;

fn encode_entities(
    out: &mut impl crate::binary_codec::BinarySink,
    entities: &[EntityId],
) -> Result<(), CodecError> {
    push_len(out, entities.len())?;
    for entity in entities {
        push_u128(out, entity.raw());
    }
    Ok(())
}

fn decode_entities(cursor: &mut impl BinarySource) -> Result<Vec<EntityId>, &'static str> {
    let count = cursor.len()?;
    let mut entities = Vec::with_capacity(cursor.bounded_capacity(count));
    let mut previous = None;
    for _ in 0..count {
        let entity = EntityId::new(cursor.u128()?);
        if previous.is_some_and(|prior| prior >= entity) {
            return Err("entity ids are not strictly sorted");
        }
        previous = Some(entity);
        entities.push(entity);
    }
    Ok(entities)
}

fn encode_bool(out: &mut impl crate::binary_codec::BinarySink, value: bool) {
    out.push(u8::from(value));
}

fn decode_bool(cursor: &mut impl BinarySource) -> Result<bool, &'static str> {
    match cursor.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err("invalid bool encoding"),
    }
}

pub(crate) fn encode_model_delta(
    out: &mut impl crate::binary_codec::BinarySink,
    delta: &DurableModelDelta,
) -> Result<(), CodecError> {
    push_len(out, delta.carriers.len())?;
    for patch in &delta.carriers {
        push_u128(out, patch.carrier.raw());
        encode_bool(out, patch.target_present);
        encode_entities(out, &patch.inserted)?;
        encode_entities(out, &patch.removed)?;
    }

    push_len(out, delta.fields.len())?;
    for patch in &delta.fields {
        push_u128(out, patch.field.raw());
        push_u128(out, patch.owner.raw());
        match &patch.value {
            None => out.push(0),
            Some(value) => {
                out.push(1);
                encode_value(out, value, 0)?;
            }
        }
    }

    encode_entities(out, &delta.lifecycle_entities_inserted)?;
    encode_entities(out, &delta.lifecycle_entities_removed)?;
    encode_entities(out, &delta.lifecycle_roots_inserted)?;
    encode_entities(out, &delta.lifecycle_roots_removed)?;

    push_len(out, delta.lifecycle_keeps_alive.len())?;
    for patch in &delta.lifecycle_keeps_alive {
        push_u128(out, patch.parent.raw());
        encode_bool(out, patch.target_present);
        encode_entities(out, &patch.inserted)?;
        encode_entities(out, &patch.removed)?;
    }
    Ok(())
}

pub(crate) fn decode_model_delta(
    cursor: &mut impl BinarySource,
) -> Result<DurableModelDelta, &'static str> {
    let carrier_count = cursor.len()?;
    let mut carriers = Vec::with_capacity(cursor.bounded_capacity(carrier_count));
    let mut previous_carrier = None;
    for _ in 0..carrier_count {
        let carrier = SemanticId::new(cursor.u128()?);
        if previous_carrier.is_some_and(|prior| prior >= carrier) {
            return Err("carrier patches are not strictly sorted");
        }
        previous_carrier = Some(carrier);
        let target_present = decode_bool(cursor)?;
        let inserted = decode_entities(cursor)?;
        let removed = decode_entities(cursor)?;
        if inserted
            .iter()
            .any(|entity| removed.binary_search(entity).is_ok())
        {
            return Err("carrier patch inserts and removes the same entity");
        }
        carriers.push(DurableCarrierPatch {
            carrier,
            target_present,
            inserted,
            removed,
        });
    }

    let field_count = cursor.len()?;
    let mut fields = Vec::with_capacity(cursor.bounded_capacity(field_count));
    let mut previous_field = None;
    for _ in 0..field_count {
        let field = SemanticId::new(cursor.u128()?);
        let owner = EntityId::new(cursor.u128()?);
        let key = (field, owner);
        if previous_field.is_some_and(|prior| prior >= key) {
            return Err("field patches are not strictly sorted");
        }
        previous_field = Some(key);
        let value = match cursor.u8()? {
            0 => None,
            1 => Some(cursor.value(0)?),
            _ => return Err("invalid field patch value tag"),
        };
        fields.push(DurableFieldPatch {
            field,
            owner,
            value,
        });
    }

    let lifecycle_entities_inserted = decode_entities(cursor)?;
    let lifecycle_entities_removed = decode_entities(cursor)?;
    let lifecycle_roots_inserted = decode_entities(cursor)?;
    let lifecycle_roots_removed = decode_entities(cursor)?;
    if lifecycle_entities_inserted
        .iter()
        .any(|entity| lifecycle_entities_removed.binary_search(entity).is_ok())
        || lifecycle_roots_inserted
            .iter()
            .any(|entity| lifecycle_roots_removed.binary_search(entity).is_ok())
    {
        return Err("lifecycle patch inserts and removes the same entity");
    }

    let keeps_count = cursor.len()?;
    let mut lifecycle_keeps_alive = Vec::with_capacity(cursor.bounded_capacity(keeps_count));
    let mut previous_parent = None;
    for _ in 0..keeps_count {
        let parent = EntityId::new(cursor.u128()?);
        if previous_parent.is_some_and(|prior| prior >= parent) {
            return Err("keeps-alive patches are not strictly sorted");
        }
        previous_parent = Some(parent);
        let target_present = decode_bool(cursor)?;
        let inserted = decode_entities(cursor)?;
        let removed = decode_entities(cursor)?;
        if inserted
            .iter()
            .any(|entity| removed.binary_search(entity).is_ok())
        {
            return Err("keeps-alive patch inserts and removes the same entity");
        }
        lifecycle_keeps_alive.push(DurableKeepsAlivePatch {
            parent,
            target_present,
            inserted,
            removed,
        });
    }

    Ok(DurableModelDelta {
        carriers,
        fields,
        lifecycle_entities_inserted,
        lifecycle_entities_removed,
        lifecycle_roots_inserted,
        lifecycle_roots_removed,
        lifecycle_keeps_alive,
    })
}
