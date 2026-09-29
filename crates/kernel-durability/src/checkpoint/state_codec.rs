use std::collections::{BTreeMap, BTreeSet};

use kernel_model::DatabaseState;
use kernel_schema::{ScalarType, TypeExpr, TypeVar};
use kernel_types::{EntityId, SemanticId};

use crate::binary_codec::{
    BinarySource, MAX_VALUE_DEPTH, encode_rows, encode_value, push_len, push_u32, push_u128,
};
use crate::runtime::{CodecError, DurabilityError};

use super::semantic_codec::{ordered_entity_id, ordered_semantic_id};

fn corrupt(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

pub(super) fn encode_state(
    out: &mut impl crate::binary_codec::BinarySink,
    state: &DatabaseState,
) -> Result<(), CodecError> {
    push_len(out, state.lifecycle.entities.len())?;
    for entity in &state.lifecycle.entities {
        push_u128(out, entity.raw());
    }
    push_len(out, state.lifecycle.roots.len())?;
    for entity in &state.lifecycle.roots {
        push_u128(out, entity.raw());
    }
    push_len(out, state.lifecycle.keeps_alive.len())?;
    for (parent, children) in &state.lifecycle.keeps_alive {
        push_u128(out, parent.raw());
        push_len(out, children.len())?;
        for child in children {
            push_u128(out, child.raw());
        }
    }

    push_len(out, state.model.carriers.len())?;
    for (entity_type, entities) in &state.model.carriers {
        push_u128(out, entity_type.raw());
        push_len(out, entities.len())?;
        for entity in entities {
            push_u128(out, entity.raw());
        }
    }

    push_len(out, state.model.fields.len())?;
    for ((field, owner), value) in &state.model.fields {
        push_u128(out, field.raw());
        push_u128(out, owner.raw());
        encode_value(out, value, 0)?;
    }

    push_len(out, state.model.relations.len())?;
    for (relation, rows) in &state.model.relations {
        push_u128(out, relation.raw());
        encode_rows(out, rows)?;
    }
    Ok(())
}

pub(super) fn decode_state(
    cursor: &mut impl BinarySource,
) -> Result<DatabaseState, DurabilityError> {
    let mut state = DatabaseState::default();

    let count = cursor.len().map_err(corrupt)?;
    let mut previous_entity = None;
    for _ in 0..count {
        let entity =
            ordered_entity_id(cursor, &mut previous_entity, "entities not strictly sorted")?;
        state.lifecycle.entities.insert(entity);
    }
    let count = cursor.len().map_err(corrupt)?;
    previous_entity = None;
    for _ in 0..count {
        let entity = ordered_entity_id(cursor, &mut previous_entity, "roots not strictly sorted")?;
        state.lifecycle.roots.insert(entity);
    }
    let count = cursor.len().map_err(corrupt)?;
    previous_entity = None;
    for _ in 0..count {
        let parent = ordered_entity_id(
            cursor,
            &mut previous_entity,
            "keeps-alive parents not strictly sorted",
        )?;
        let child_count = cursor.len().map_err(corrupt)?;
        let mut children = BTreeSet::new();
        let mut previous_child = None;
        for _ in 0..child_count {
            children.insert(ordered_entity_id(
                cursor,
                &mut previous_child,
                "keeps-alive children not strictly sorted",
            )?);
        }
        state.lifecycle.keeps_alive.insert(parent, children);
    }

    let count = cursor.len().map_err(corrupt)?;
    let mut previous_semantic = None;
    for _ in 0..count {
        let entity_type = ordered_semantic_id(
            cursor,
            &mut previous_semantic,
            "carriers not strictly sorted",
        )?;
        let entity_count = cursor.len().map_err(corrupt)?;
        let mut entities = BTreeSet::new();
        let mut previous = None;
        for _ in 0..entity_count {
            entities.insert(ordered_entity_id(
                cursor,
                &mut previous,
                "carrier entities not strictly sorted",
            )?);
        }
        state.model.carriers.insert(entity_type, entities);
    }

    let count = cursor.len().map_err(corrupt)?;
    let mut previous_field = None;
    for _ in 0..count {
        let key = (
            SemanticId::new(cursor.u128().map_err(corrupt)?),
            EntityId::new(cursor.u128().map_err(corrupt)?),
        );
        if previous_field.is_some_and(|previous| previous >= key) {
            return Err(corrupt("model fields not strictly sorted"));
        }
        previous_field = Some(key);
        state
            .model
            .fields
            .insert(key, cursor.value(0).map_err(corrupt)?);
    }

    let count = cursor.len().map_err(corrupt)?;
    previous_semantic = None;
    for _ in 0..count {
        let relation = ordered_semantic_id(
            cursor,
            &mut previous_semantic,
            "model relations not strictly sorted",
        )?;
        state
            .model
            .relations
            .insert(relation, cursor.rows(0).map_err(corrupt)?);
    }
    Ok(state)
}

pub(super) fn encode_type_expr(
    out: &mut impl crate::binary_codec::BinarySink,
    ty: &TypeExpr,
    depth: usize,
) -> Result<(), CodecError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(CodecError::ValueNestingTooDeep);
    }
    match ty {
        TypeExpr::Scalar(scalar) => {
            out.push(0);
            encode_scalar(out, scalar);
        }
        TypeExpr::Product(fields) => {
            out.push(1);
            encode_type_map(out, fields, depth + 1)?;
        }
        TypeExpr::Sum(variants) => {
            out.push(2);
            encode_type_map(out, variants, depth + 1)?;
        }
        TypeExpr::Option(inner) => {
            out.push(3);
            encode_type_expr(out, inner, depth + 1)?;
        }
        TypeExpr::Set {
            element,
            equivalence,
        } => {
            out.push(4);
            push_u128(out, equivalence.raw());
            encode_type_expr(out, element, depth + 1)?;
        }
        TypeExpr::Bag {
            element,
            equivalence,
        } => {
            out.push(5);
            push_u128(out, equivalence.raw());
            encode_type_expr(out, element, depth + 1)?;
        }
        TypeExpr::Seq(inner) => {
            out.push(6);
            encode_type_expr(out, inner, depth + 1)?;
        }
        TypeExpr::Map {
            key,
            value,
            key_equivalence,
        } => {
            out.push(7);
            push_u128(out, key_equivalence.raw());
            encode_type_expr(out, key, depth + 1)?;
            encode_type_expr(out, value, depth + 1)?;
        }
        TypeExpr::Var(var) => {
            out.push(8);
            push_u32(out, var.0);
        }
        TypeExpr::Mu { binder, body } => {
            out.push(9);
            push_u32(out, binder.0);
            encode_type_expr(out, body, depth + 1)?;
        }
    }
    Ok(())
}

pub(super) fn decode_type_expr(
    cursor: &mut impl BinarySource,
    depth: usize,
) -> Result<TypeExpr, DurabilityError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(corrupt("type nesting exceeds hard limit"));
    }
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(TypeExpr::Scalar(decode_scalar(cursor)?)),
        1 => Ok(TypeExpr::Product(decode_type_map(cursor, depth + 1)?)),
        2 => Ok(TypeExpr::Sum(decode_type_map(cursor, depth + 1)?)),
        3 => Ok(TypeExpr::Option(Box::new(decode_type_expr(
            cursor,
            depth + 1,
        )?))),
        4 => Ok(TypeExpr::Set {
            equivalence: SemanticId::new(cursor.u128().map_err(corrupt)?),
            element: Box::new(decode_type_expr(cursor, depth + 1)?),
        }),
        5 => Ok(TypeExpr::Bag {
            equivalence: SemanticId::new(cursor.u128().map_err(corrupt)?),
            element: Box::new(decode_type_expr(cursor, depth + 1)?),
        }),
        6 => Ok(TypeExpr::Seq(Box::new(decode_type_expr(
            cursor,
            depth + 1,
        )?))),
        7 => Ok(TypeExpr::Map {
            key_equivalence: SemanticId::new(cursor.u128().map_err(corrupt)?),
            key: Box::new(decode_type_expr(cursor, depth + 1)?),
            value: Box::new(decode_type_expr(cursor, depth + 1)?),
        }),
        8 => Ok(TypeExpr::Var(TypeVar(cursor.u32().map_err(corrupt)?))),
        9 => Ok(TypeExpr::Mu {
            binder: TypeVar(cursor.u32().map_err(corrupt)?),
            body: Box::new(decode_type_expr(cursor, depth + 1)?),
        }),
        _ => Err(corrupt("unknown type expression tag")),
    }
}

fn encode_type_map(
    out: &mut impl crate::binary_codec::BinarySink,
    fields: &BTreeMap<SemanticId, TypeExpr>,
    depth: usize,
) -> Result<(), CodecError> {
    push_len(out, fields.len())?;
    for (id, ty) in fields {
        push_u128(out, id.raw());
        encode_type_expr(out, ty, depth)?;
    }
    Ok(())
}

fn decode_type_map(
    cursor: &mut impl BinarySource,
    depth: usize,
) -> Result<BTreeMap<SemanticId, TypeExpr>, DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut fields = BTreeMap::new();
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "type map not strictly sorted")?;
        fields.insert(id, decode_type_expr(cursor, depth)?);
    }
    Ok(fields)
}

fn encode_scalar(out: &mut impl crate::binary_codec::BinarySink, scalar: &ScalarType) {
    match scalar {
        ScalarType::Unit => out.push(0),
        ScalarType::Bool => out.push(1),
        ScalarType::I64 => out.push(2),
        ScalarType::F64 => out.push(3),
        ScalarType::Text => out.push(4),
        ScalarType::LiveEntityRef(entity_type) => {
            out.push(5);
            push_u128(out, entity_type.raw());
        }
        ScalarType::HistoricalEntityId(entity_type) => {
            out.push(6);
            push_u128(out, entity_type.raw());
        }
    }
}

fn decode_scalar(cursor: &mut impl BinarySource) -> Result<ScalarType, DurabilityError> {
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(ScalarType::Unit),
        1 => Ok(ScalarType::Bool),
        2 => Ok(ScalarType::I64),
        3 => Ok(ScalarType::F64),
        4 => Ok(ScalarType::Text),
        5 => Ok(ScalarType::LiveEntityRef(SemanticId::new(
            cursor.u128().map_err(corrupt)?,
        ))),
        6 => Ok(ScalarType::HistoricalEntityId(SemanticId::new(
            cursor.u128().map_err(corrupt)?,
        ))),
        _ => Err(corrupt("unknown scalar type tag")),
    }
}
