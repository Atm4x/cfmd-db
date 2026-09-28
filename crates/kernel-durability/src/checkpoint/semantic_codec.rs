use std::collections::BTreeMap;

use kernel_schema::{StructuralEquivalenceDef, StructuralOrderingDef, SymbolKind};
use kernel_types::{EntityId, SemanticId};

use crate::binary_codec::{Cursor, push_len, push_u128};
use crate::runtime::{CodecError, DurabilityError};

fn corrupt(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

pub(super) fn encode_structural_equivalence(
    out: &mut Vec<u8>,
    definition: &StructuralEquivalenceDef,
) -> Result<(), CodecError> {
    match definition {
        StructuralEquivalenceDef::Mu { body } => {
            out.push(0);
            push_u128(out, body.raw());
        }
        StructuralEquivalenceDef::Var { binder } => {
            out.push(1);
            push_u128(out, binder.raw());
        }
        StructuralEquivalenceDef::Product { fields } => {
            out.push(2);
            encode_semantic_map(out, fields)?;
        }
        StructuralEquivalenceDef::Option { inner } => {
            out.push(3);
            push_u128(out, inner.raw());
        }
        StructuralEquivalenceDef::Sum { variants } => {
            out.push(4);
            encode_semantic_map(out, variants)?;
        }
        StructuralEquivalenceDef::Set { element } => {
            out.push(5);
            push_u128(out, element.raw());
        }
        StructuralEquivalenceDef::Bag { element } => {
            out.push(6);
            push_u128(out, element.raw());
        }
        StructuralEquivalenceDef::Seq { element } => {
            out.push(7);
            push_u128(out, element.raw());
        }
        StructuralEquivalenceDef::Map { key, value } => {
            out.push(8);
            push_u128(out, key.raw());
            push_u128(out, value.raw());
        }
    }
    Ok(())
}

pub(super) fn decode_structural_equivalence(
    cursor: &mut Cursor<'_>,
) -> Result<StructuralEquivalenceDef, DurabilityError> {
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(StructuralEquivalenceDef::Mu {
            body: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        1 => Ok(StructuralEquivalenceDef::Var {
            binder: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        2 => Ok(StructuralEquivalenceDef::Product {
            fields: decode_semantic_map(cursor)?,
        }),
        3 => Ok(StructuralEquivalenceDef::Option {
            inner: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        4 => Ok(StructuralEquivalenceDef::Sum {
            variants: decode_semantic_map(cursor)?,
        }),
        5 => Ok(StructuralEquivalenceDef::Set {
            element: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        6 => Ok(StructuralEquivalenceDef::Bag {
            element: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        7 => Ok(StructuralEquivalenceDef::Seq {
            element: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        8 => Ok(StructuralEquivalenceDef::Map {
            key: SemanticId::new(cursor.u128().map_err(corrupt)?),
            value: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        _ => Err(corrupt("unknown structural equivalence tag")),
    }
}

pub(super) fn encode_structural_ordering(
    out: &mut Vec<u8>,
    definition: &StructuralOrderingDef,
) -> Result<(), CodecError> {
    match definition {
        StructuralOrderingDef::Mu { body } => {
            out.push(0);
            push_u128(out, body.raw());
        }
        StructuralOrderingDef::Var { binder } => {
            out.push(1);
            push_u128(out, binder.raw());
        }
        StructuralOrderingDef::Product { fields } => {
            out.push(2);
            encode_semantic_pairs(out, fields)?;
        }
        StructuralOrderingDef::Option { inner, none_first } => {
            out.push(3);
            push_u128(out, inner.raw());
            out.push(u8::from(*none_first));
        }
        StructuralOrderingDef::Sum { variants } => {
            out.push(4);
            encode_semantic_pairs(out, variants)?;
        }
        StructuralOrderingDef::Set { element } => {
            out.push(5);
            push_u128(out, element.raw());
        }
        StructuralOrderingDef::Bag { element } => {
            out.push(6);
            push_u128(out, element.raw());
        }
        StructuralOrderingDef::Seq { element } => {
            out.push(7);
            push_u128(out, element.raw());
        }
        StructuralOrderingDef::Map { key, value } => {
            out.push(8);
            push_u128(out, key.raw());
            push_u128(out, value.raw());
        }
    }
    Ok(())
}

pub(super) fn decode_structural_ordering(
    cursor: &mut Cursor<'_>,
) -> Result<StructuralOrderingDef, DurabilityError> {
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(StructuralOrderingDef::Mu {
            body: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        1 => Ok(StructuralOrderingDef::Var {
            binder: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        2 => Ok(StructuralOrderingDef::Product {
            fields: decode_semantic_pairs(cursor)?,
        }),
        3 => {
            let inner = SemanticId::new(cursor.u128().map_err(corrupt)?);
            let none_first = match cursor.u8().map_err(corrupt)? {
                0 => false,
                1 => true,
                _ => return Err(corrupt("invalid structural option ordering flag")),
            };
            Ok(StructuralOrderingDef::Option { inner, none_first })
        }
        4 => Ok(StructuralOrderingDef::Sum {
            variants: decode_semantic_pairs(cursor)?,
        }),
        5 => Ok(StructuralOrderingDef::Set {
            element: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        6 => Ok(StructuralOrderingDef::Bag {
            element: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        7 => Ok(StructuralOrderingDef::Seq {
            element: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        8 => Ok(StructuralOrderingDef::Map {
            key: SemanticId::new(cursor.u128().map_err(corrupt)?),
            value: SemanticId::new(cursor.u128().map_err(corrupt)?),
        }),
        _ => Err(corrupt("unknown structural ordering tag")),
    }
}

fn encode_semantic_pairs(
    out: &mut Vec<u8>,
    pairs: &[(SemanticId, SemanticId)],
) -> Result<(), CodecError> {
    push_len(out, pairs.len())?;
    for (name, child) in pairs {
        push_u128(out, name.raw());
        push_u128(out, child.raw());
    }
    Ok(())
}

fn decode_semantic_pairs(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<(SemanticId, SemanticId)>, DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    (0..count)
        .map(|_| {
            Ok((
                SemanticId::new(cursor.u128().map_err(corrupt)?),
                SemanticId::new(cursor.u128().map_err(corrupt)?),
            ))
        })
        .collect()
}

fn encode_semantic_map(
    out: &mut Vec<u8>,
    map: &BTreeMap<SemanticId, SemanticId>,
) -> Result<(), CodecError> {
    push_len(out, map.len())?;
    for (key, value) in map {
        push_u128(out, key.raw());
        push_u128(out, value.raw());
    }
    Ok(())
}

fn decode_semantic_map(
    cursor: &mut Cursor<'_>,
) -> Result<BTreeMap<SemanticId, SemanticId>, DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut map = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        let key = ordered_semantic_id(cursor, &mut previous, "semantic map not strictly sorted")?;
        map.insert(key, SemanticId::new(cursor.u128().map_err(corrupt)?));
    }
    Ok(map)
}

pub(super) fn encode_semantic_ids(out: &mut Vec<u8>, ids: &[SemanticId]) -> Result<(), CodecError> {
    push_len(out, ids.len())?;
    for id in ids {
        push_u128(out, id.raw());
    }
    Ok(())
}

pub(super) fn decode_semantic_ids(
    cursor: &mut Cursor<'_>,
) -> Result<Vec<SemanticId>, DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut ids = Vec::with_capacity(cursor.bounded_capacity(count));
    for _ in 0..count {
        ids.push(SemanticId::new(cursor.u128().map_err(corrupt)?));
    }
    Ok(ids)
}

pub(super) fn ordered_semantic_id(
    cursor: &mut Cursor<'_>,
    previous: &mut Option<SemanticId>,
    reason: &'static str,
) -> Result<SemanticId, DurabilityError> {
    let id = SemanticId::new(cursor.u128().map_err(corrupt)?);
    if previous.is_some_and(|previous| previous >= id) {
        return Err(corrupt(reason));
    }
    *previous = Some(id);
    Ok(id)
}

pub(super) fn ordered_entity_id(
    cursor: &mut Cursor<'_>,
    previous: &mut Option<EntityId>,
    reason: &'static str,
) -> Result<EntityId, DurabilityError> {
    let id = EntityId::new(cursor.u128().map_err(corrupt)?);
    if previous.is_some_and(|previous| previous >= id) {
        return Err(corrupt(reason));
    }
    *previous = Some(id);
    Ok(id)
}

pub(super) const fn encode_symbol_kind(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Entity => 0,
        SymbolKind::Value => 1,
        SymbolKind::Field => 2,
        SymbolKind::Relation => 3,
        SymbolKind::Capability => 4,
        SymbolKind::Function => 5,
    }
}

pub(super) fn decode_symbol_kind(tag: u8) -> Result<SymbolKind, DurabilityError> {
    match tag {
        0 => Ok(SymbolKind::Entity),
        1 => Ok(SymbolKind::Value),
        2 => Ok(SymbolKind::Field),
        3 => Ok(SymbolKind::Relation),
        4 => Ok(SymbolKind::Capability),
        5 => Ok(SymbolKind::Function),
        _ => Err(corrupt("unknown symbol kind tag")),
    }
}
