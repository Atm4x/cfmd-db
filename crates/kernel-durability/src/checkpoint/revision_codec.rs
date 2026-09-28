use std::collections::BTreeMap;

use kernel_revision::Revision;
use kernel_schema::{
    CapabilityDef, FieldDef, ModuleDigest, RelationDef, RelationSemantics, Schema, SemanticContext,
    SemanticEnvironment, Symbol,
};
use kernel_semantics::SemanticRegistry;
use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

use crate::binary_codec::{Cursor, push_bytes, push_len, push_u16, push_u64, push_u128};
use crate::runtime::{CodecError, DurabilityError};

use super::semantic_codec::{
    decode_semantic_ids, decode_structural_equivalence, decode_structural_ordering,
    decode_symbol_kind, encode_semantic_ids, encode_structural_equivalence,
    encode_structural_ordering, encode_symbol_kind, ordered_semantic_id,
};
use super::state_codec::{decode_state, decode_type_expr, encode_state, encode_type_expr};

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
        let mut columns = Vec::with_capacity(cursor.bounded_capacity(column_count));
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
