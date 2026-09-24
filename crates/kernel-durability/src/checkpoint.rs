use std::collections::{BTreeMap, BTreeSet};

use kernel_model::DatabaseState;
use kernel_revision::Revision;
use kernel_schema::{
    CapabilityDef, FieldDef, ModuleDigest, RelationDef, RelationSemantics, ScalarType, Schema,
    SemanticContext, SemanticEnvironment, StructuralEquivalenceDef, StructuralOrderingDef, Symbol,
    SymbolKind, TypeExpr, TypeVar,
};
use kernel_semantics::SemanticRegistry;
use kernel_types::{EntityId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

use super::{
    CodecError, Cursor, DurabilityError, MAX_VALUE_DEPTH, encode_value, push_bytes, push_len,
    push_u16, push_u64, push_u128,
};

pub(crate) const CHECKPOINT_CODEC_VERSION: u16 = 2;

pub(crate) fn encode_revision(revision: &Revision) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    push_u16(&mut out, CHECKPOINT_CODEC_VERSION);
    push_u64(&mut out, revision.id().raw());
    encode_context(&mut out, revision.semantic_context())?;
    encode_state(&mut out, revision.state())?;
    Ok(out)
}

pub(crate) fn decode_revision(
    bytes: &[u8],
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    let version = cursor.u16().map_err(corrupt)?;
    if !matches!(version, 1 | CHECKPOINT_CODEC_VERSION) {
        return Err(corrupt("unsupported checkpoint codec version"));
    }
    let revision_id = RevisionId::new(cursor.u64().map_err(corrupt)?);
    let context = decode_context(&mut cursor, version)?;
    let state = decode_state(&mut cursor)?;
    cursor.finish().map_err(corrupt)?;
    Revision::build(revision_id, &context, registry, state)
        .map_err(|_| corrupt("checkpoint revision validation failed"))
}

fn corrupt(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

fn encode_context(out: &mut Vec<u8>, context: &SemanticContext) -> Result<(), CodecError> {
    let schema = &context.schema;
    push_u64(out, schema.revision.raw());

    let symbols: Vec<_> = schema.symbols().collect();
    push_len(out, symbols.len())?;
    for symbol in symbols {
        push_u128(out, symbol.id.raw());
        out.push(encode_symbol_kind(symbol.kind));
        push_bytes(out, symbol.presentation_name.as_bytes())?;
    }

    let types: Vec<_> = schema.type_definitions().collect();
    push_len(out, types.len())?;
    for (id, definition) in types {
        push_u128(out, id.raw());
        encode_type_expr(out, definition, 0)?;
    }

    let capabilities: Vec<_> = schema.capabilities().collect();
    push_len(out, capabilities.len())?;
    for capability in capabilities {
        push_u128(out, capability.id.raw());
        push_len(out, capability.required_fields.len())?;
        for (field, ty) in &capability.required_fields {
            push_u128(out, field.raw());
            encode_type_expr(out, ty, 0)?;
        }
    }

    let fields: Vec<_> = schema.fields().collect();
    push_len(out, fields.len())?;
    for field in fields {
        push_u128(out, field.id.raw());
        push_u128(out, field.owner.raw());
        encode_type_expr(out, &field.value, 0)?;
    }

    let relations: Vec<_> = schema.relations().collect();
    push_len(out, relations.len())?;
    for relation in relations {
        push_u128(out, relation.id.raw());
        push_len(out, relation.columns.len())?;
        for column in &relation.columns {
            encode_type_expr(out, column, 0)?;
        }
        match &relation.semantics {
            RelationSemantics::Set {
                column_equivalences,
            } => {
                out.push(0);
                encode_semantic_ids(out, column_equivalences)?;
            }
            RelationSemantics::Bag {
                column_equivalences,
            } => {
                out.push(1);
                encode_semantic_ids(out, column_equivalences)?;
            }
        }
    }

    let structural: Vec<_> = schema.structural_equivalences().collect();
    push_len(out, structural.len())?;
    for (id, definition) in structural {
        push_u128(out, id.raw());
        encode_structural_equivalence(out, definition)?;
    }

    let orderings: Vec<_> = schema.structural_orderings().collect();
    push_len(out, orderings.len())?;
    for (id, definition) in orderings {
        push_u128(out, id.raw());
        encode_structural_ordering(out, definition)?;
    }

    let inclusions: Vec<_> = schema.inclusions().collect();
    push_len(out, inclusions.len())?;
    for (subtype, supertype) in inclusions {
        push_u128(out, subtype.raw());
        push_u128(out, supertype.raw());
    }

    push_u64(out, context.environment.revision.raw());
    let modules: Vec<_> = context.environment.modules().collect();
    push_len(out, modules.len())?;
    for (id, digest) in modules {
        push_u128(out, id.raw());
        out.extend_from_slice(&digest.0);
    }
    Ok(())
}

fn decode_context(
    cursor: &mut Cursor<'_>,
    version: u16,
) -> Result<SemanticContext, DurabilityError> {
    let mut schema = Schema::new(SchemaRevisionId::new(cursor.u64().map_err(corrupt)?));
    decode_symbols(cursor, &mut schema)?;
    decode_types(cursor, &mut schema)?;
    decode_capabilities(cursor, &mut schema)?;
    decode_fields(cursor, &mut schema)?;
    decode_relations(cursor, &mut schema)?;
    decode_structural_equivalences(cursor, &mut schema)?;
    if version >= 2 {
        decode_structural_orderings(cursor, &mut schema)?;
    }
    decode_inclusions(cursor, &mut schema)?;
    let environment = decode_environment(cursor)?;
    Ok(SemanticContext {
        schema,
        environment,
    })
}

fn decode_symbols(cursor: &mut Cursor<'_>, schema: &mut Schema) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "symbols not strictly sorted")?;
        let kind = decode_symbol_kind(cursor.u8().map_err(corrupt)?)?;
        let presentation_name = cursor.string().map_err(corrupt)?;
        schema
            .define(Symbol {
                id,
                kind,
                presentation_name,
            })
            .map_err(|_| corrupt("invalid checkpoint symbol table"))?;
    }
    Ok(())
}

fn decode_types(cursor: &mut Cursor<'_>, schema: &mut Schema) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "types not strictly sorted")?;
        schema
            .define_type(id, decode_type_expr(cursor, 0)?)
            .map_err(|_| corrupt("invalid checkpoint type definition"))?;
    }
    Ok(())
}

fn decode_capabilities(
    cursor: &mut Cursor<'_>,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "capabilities not strictly sorted")?;
        let field_count = cursor.len().map_err(corrupt)?;
        let mut required_fields = BTreeMap::new();
        let mut previous_field = None;
        for _ in 0..field_count {
            let field = ordered_semantic_id(
                cursor,
                &mut previous_field,
                "capability fields not strictly sorted",
            )?;
            required_fields.insert(field, decode_type_expr(cursor, 0)?);
        }
        schema
            .define_capability(CapabilityDef {
                id,
                required_fields,
            })
            .map_err(|_| corrupt("invalid checkpoint capability"))?;
    }
    Ok(())
}

fn decode_fields(cursor: &mut Cursor<'_>, schema: &mut Schema) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "fields not strictly sorted")?;
        let owner = SemanticId::new(cursor.u128().map_err(corrupt)?);
        let value = decode_type_expr(cursor, 0)?;
        schema
            .define_field(FieldDef { id, owner, value })
            .map_err(|_| corrupt("invalid checkpoint field"))?;
    }
    Ok(())
}

fn decode_relations(cursor: &mut Cursor<'_>, schema: &mut Schema) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "relations not strictly sorted")?;
        let column_count = cursor.len().map_err(corrupt)?;
        let mut columns = Vec::with_capacity(column_count);
        for _ in 0..column_count {
            columns.push(decode_type_expr(cursor, 0)?);
        }
        let semantics_tag = cursor.u8().map_err(corrupt)?;
        let equivalences = decode_semantic_ids(cursor)?;
        let semantics = match semantics_tag {
            0 => RelationSemantics::Set {
                column_equivalences: equivalences,
            },
            1 => RelationSemantics::Bag {
                column_equivalences: equivalences,
            },
            _ => return Err(corrupt("unknown relation semantics tag")),
        };
        schema
            .define_relation(RelationDef {
                id,
                columns,
                semantics,
            })
            .map_err(|_| corrupt("invalid checkpoint relation"))?;
    }
    Ok(())
}

fn decode_structural_equivalences(
    cursor: &mut Cursor<'_>,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(
            cursor,
            &mut previous,
            "structural equivalences not strictly sorted",
        )?;
        schema
            .define_structural_equivalence(id, decode_structural_equivalence(cursor)?)
            .map_err(|_| corrupt("invalid checkpoint structural equivalence"))?;
    }
    Ok(())
}

fn decode_structural_orderings(
    cursor: &mut Cursor<'_>,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(
            cursor,
            &mut previous,
            "structural orderings not strictly sorted",
        )?;
        schema
            .define_structural_ordering(id, decode_structural_ordering(cursor)?)
            .map_err(|_| corrupt("invalid checkpoint structural ordering"))?;
    }
    Ok(())
}

fn decode_inclusions(cursor: &mut Cursor<'_>, schema: &mut Schema) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let inclusion = (
            SemanticId::new(cursor.u128().map_err(corrupt)?),
            SemanticId::new(cursor.u128().map_err(corrupt)?),
        );
        if previous.is_some_and(|previous| previous >= inclusion) {
            return Err(corrupt("schema inclusions not strictly sorted"));
        }
        previous = Some(inclusion);
        schema
            .include(inclusion.0, inclusion.1)
            .map_err(|_| corrupt("invalid checkpoint schema inclusion"))?;
    }
    Ok(())
}

fn decode_environment(cursor: &mut Cursor<'_>) -> Result<SemanticEnvironment, DurabilityError> {
    let mut environment =
        SemanticEnvironment::new(SemanticEnvId::new(cursor.u64().map_err(corrupt)?));
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "modules not strictly sorted")?;
        let digest: [u8; 32] = cursor
            .take(32)
            .map_err(corrupt)?
            .try_into()
            .map_err(|_| corrupt("module digest length"))?;
        environment.pin_module(id, ModuleDigest(digest));
    }
    Ok(environment)
}

fn encode_state(out: &mut Vec<u8>, state: &DatabaseState) -> Result<(), CodecError> {
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
        super::encode_rows(out, rows)?;
    }
    Ok(())
}

fn decode_state(cursor: &mut Cursor<'_>) -> Result<DatabaseState, DurabilityError> {
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

fn encode_type_expr(out: &mut Vec<u8>, ty: &TypeExpr, depth: usize) -> Result<(), CodecError> {
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
            super::push_u32(out, var.0);
        }
        TypeExpr::Mu { binder, body } => {
            out.push(9);
            super::push_u32(out, binder.0);
            encode_type_expr(out, body, depth + 1)?;
        }
    }
    Ok(())
}

fn decode_type_expr(cursor: &mut Cursor<'_>, depth: usize) -> Result<TypeExpr, DurabilityError> {
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
    out: &mut Vec<u8>,
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
    cursor: &mut Cursor<'_>,
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

fn encode_scalar(out: &mut Vec<u8>, scalar: &ScalarType) {
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

fn decode_scalar(cursor: &mut Cursor<'_>) -> Result<ScalarType, DurabilityError> {
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

fn encode_structural_equivalence(
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

fn decode_structural_equivalence(
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

fn encode_structural_ordering(
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

fn decode_structural_ordering(
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

fn encode_semantic_ids(out: &mut Vec<u8>, ids: &[SemanticId]) -> Result<(), CodecError> {
    push_len(out, ids.len())?;
    for id in ids {
        push_u128(out, id.raw());
    }
    Ok(())
}

fn decode_semantic_ids(cursor: &mut Cursor<'_>) -> Result<Vec<SemanticId>, DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(SemanticId::new(cursor.u128().map_err(corrupt)?));
    }
    Ok(ids)
}

fn ordered_semantic_id(
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

fn ordered_entity_id(
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

const fn encode_symbol_kind(kind: SymbolKind) -> u8 {
    match kind {
        SymbolKind::Entity => 0,
        SymbolKind::Value => 1,
        SymbolKind::Field => 2,
        SymbolKind::Relation => 3,
        SymbolKind::Capability => 4,
        SymbolKind::Function => 5,
    }
}

fn decode_symbol_kind(tag: u8) -> Result<SymbolKind, DurabilityError> {
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

#[cfg(test)]
mod tests {
    use kernel_model::{DatabaseState, Value};
    use kernel_schema::{
        CapabilityDef, FieldDef, RelationDef, RelationSemantics, ScalarType, Schema,
        SemanticContext, SemanticEnvironment, StructuralEquivalenceDef, StructuralOrderingDef,
        Symbol, SymbolKind, TypeExpr, TypeVar,
    };
    use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
    use kernel_types::{EntityId, RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    fn sid(raw: u128) -> SemanticId {
        SemanticId::new(raw)
    }

    fn complex_revision() -> (Revision, SemanticRegistry) {
        let entity_type = sid(1);
        let subtype = sid(2);
        let field = sid(3);
        let relation = sid(4);
        let capability = sid(5);
        let recursive_type = sid(6);
        let structural = sid(7);
        let structural_order = sid(8);
        let eq_i64 = sid(20);
        let eq_text = sid(21);
        let order_i64 = sid(22);

        let mut registry = SemanticRegistry::default();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9));
        environment.pin_module(
            eq_i64,
            registry.install_equivalence(EquivalenceModule::I64Exact),
        );
        environment.pin_module(
            eq_text,
            registry.install_equivalence(EquivalenceModule::TextExact),
        );
        environment.pin_module(
            order_i64,
            registry.install_ordering(OrderingModule::I64Ascending),
        );

        let mut schema = Schema::new(SchemaRevisionId::new(8));
        for (id, kind, name) in [
            (entity_type, SymbolKind::Entity, "entity"),
            (subtype, SymbolKind::Entity, "subtype"),
            (field, SymbolKind::Field, "name"),
            (relation, SymbolKind::Relation, "events"),
            (capability, SymbolKind::Capability, "readable"),
            (recursive_type, SymbolKind::Value, "recursive"),
        ] {
            schema
                .define(Symbol {
                    id,
                    kind,
                    presentation_name: name.into(),
                })
                .unwrap();
        }
        schema
            .define_type(
                recursive_type,
                TypeExpr::Mu {
                    binder: TypeVar(1),
                    body: Box::new(TypeExpr::Seq(Box::new(TypeExpr::Var(TypeVar(1))))),
                },
            )
            .unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        schema
            .define_capability(CapabilityDef {
                id: capability,
                required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq_i64, eq_text],
                },
            })
            .unwrap();
        schema
            .define_structural_equivalence(
                structural,
                StructuralEquivalenceDef::Seq { element: eq_i64 },
            )
            .unwrap();
        schema
            .define_structural_ordering(
                structural_order,
                StructuralOrderingDef::Seq { element: order_i64 },
            )
            .unwrap();
        schema.include(subtype, entity_type).unwrap();

        let context = SemanticContext {
            schema,
            environment,
        };
        let state = complex_state(entity_type, field, relation);
        (
            Revision::build(RevisionId::new(44), &context, &registry, state).unwrap(),
            registry,
        )
    }

    fn complex_state(
        entity_type: SemanticId,
        field: SemanticId,
        relation: SemanticId,
    ) -> DatabaseState {
        let entity = EntityId::new(100);
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
        state
            .model
            .carriers
            .insert(entity_type, BTreeSet::from([entity]));
        state
            .model
            .fields
            .insert((field, entity), Value::Text("alpha".into()));
        state.model.relations.insert(
            relation,
            vec![vec![Value::I64(7), Value::Text("payload".into())]],
        );
        state
    }

    #[test]
    fn full_revision_checkpoint_codec_roundtrips_schema_semantics_lifecycle_and_model() {
        let (revision, registry) = complex_revision();
        let bytes = encode_revision(&revision).unwrap();
        let decoded = decode_revision(&bytes, &registry).unwrap();
        assert_eq!(decoded, revision);
    }
}
