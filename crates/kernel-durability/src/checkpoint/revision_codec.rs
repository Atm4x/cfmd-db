use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use kernel_revision::Revision;
use kernel_schema::{
    CapabilityDef, FieldDef, FieldRule, FiniteF64, ModelRuleExpr, ModuleDigest, RelationDef,
    RelationSemantics, RuleValueExpr, Schema, SemanticContext, SemanticEnvironment,
    SemanticRuleExpr, Symbol, TextPattern,
};
use kernel_semantics::SemanticRegistry;
use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

use crate::binary_codec::{
    BinarySink, BinarySource, CountingBinarySink, Cursor, ReadBinarySource, StreamingBinarySink,
    push_bytes, push_len, push_u16, push_u64, push_u128,
};
use crate::runtime::{CodecError, DurabilityError};

use super::semantic_codec::{
    decode_semantic_ids, decode_structural_equivalence, decode_structural_ordering,
    decode_symbol_kind, encode_semantic_ids, encode_structural_equivalence,
    encode_structural_ordering, encode_symbol_kind, ordered_semantic_id,
};
use super::state_codec::{decode_state, decode_type_expr, encode_state, encode_type_expr};

pub(crate) const CHECKPOINT_CODEC_VERSION: u16 = 5;

pub(crate) fn encode_revision(revision: &Revision) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    encode_revision_into(&mut out, revision)?;
    Ok(out)
}

pub(crate) fn encoded_revision_len(revision: &Revision) -> Result<u64, CodecError> {
    let mut sink = CountingBinarySink::default();
    encode_revision_into(&mut sink, revision)?;
    sink.len()
}

pub(crate) fn stream_revision(
    revision: &Revision,
    emit: &mut dyn FnMut(&[u8]) -> Result<(), DurabilityError>,
) -> Result<(), DurabilityError> {
    let mut sink = StreamingBinarySink::new(emit);
    encode_revision_into(&mut sink, revision)?;
    sink.finish()
}

pub(crate) fn encode_revision_into(
    out: &mut impl BinarySink,
    revision: &Revision,
) -> Result<(), CodecError> {
    push_u16(out, CHECKPOINT_CODEC_VERSION);
    push_u64(out, revision.id().raw());
    encode_context(out, revision.semantic_context())?;
    encode_state(out, revision.state())?;
    Ok(())
}

pub(crate) fn decode_revision(
    bytes: &[u8],
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    let mut cursor = Cursor::new(bytes);
    decode_revision_from_cursor(&mut cursor, registry)
}

pub(crate) fn decode_revision_from_reader(
    reader: &mut dyn Read,
    len: u64,
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    let mut cursor = ReadBinarySource::new(reader, len);
    decode_revision_from_cursor(&mut cursor, registry)
}

fn decode_revision_from_cursor(
    cursor: &mut impl BinarySource,
    registry: &SemanticRegistry,
) -> Result<Revision, DurabilityError> {
    let version = cursor.u16().map_err(corrupt)?;
    if !(1..=CHECKPOINT_CODEC_VERSION).contains(&version) {
        return Err(corrupt("unsupported checkpoint codec version"));
    }
    let revision_id = RevisionId::new(cursor.u64().map_err(corrupt)?);
    let context = decode_context(cursor, version)?;
    let state = decode_state(cursor)?;
    cursor.finish().map_err(corrupt)?;
    Revision::build(revision_id, &context, registry, state)
        .map_err(|_| corrupt("checkpoint revision validation failed"))
}

fn corrupt(reason: &'static str) -> DurabilityError {
    DurabilityError::Corruption { offset: 0, reason }
}

pub(crate) fn encode_context(
    out: &mut impl crate::binary_codec::BinarySink,
    context: &SemanticContext,
) -> Result<(), CodecError> {
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

    let field_rules: Vec<_> = schema.all_field_rules().collect();
    push_len(out, field_rules.len())?;
    for (field, rule) in field_rules {
        push_u128(out, field.raw());
        encode_field_rule(out, rule)?;
    }

    let entity_rules: Vec<_> = schema.all_entity_rules().collect();
    push_len(out, entity_rules.len())?;
    for (owner, rule) in entity_rules {
        push_u128(out, owner.raw());
        encode_semantic_rule_expr(out, rule, 0)?;
    }

    let relations: Vec<_> = schema.relations().collect();
    push_len(out, relations.len())?;
    for relation in relations {
        push_u128(out, relation.id.raw());
        let column_ids = schema
            .relation_column_ids(relation.id)
            .ok_or(CodecError::LengthOverflow)?;
        push_len(out, relation.columns.len())?;
        for (column_id, column) in column_ids.iter().zip(&relation.columns) {
            push_u128(out, column_id.raw());
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

    push_len(out, schema.model_rules().len())?;
    for rule in schema.model_rules() {
        match rule {
            ModelRuleExpr::RelationCardinality { relation, min, max } => {
                out.push(0);
                push_u128(out, relation.raw());
                push_u64(out, *min);
                match max {
                    Some(max) => {
                        out.push(1);
                        push_u64(out, *max);
                    }
                    None => out.push(0),
                }
            }
            ModelRuleExpr::RelationExists {
                relation,
                predicate,
            } => {
                out.push(1);
                push_u128(out, relation.raw());
                encode_semantic_rule_expr(out, predicate, 0)?;
            }
            ModelRuleExpr::RelationAll {
                relation,
                predicate,
            } => {
                out.push(2);
                push_u128(out, relation.raw());
                encode_semantic_rule_expr(out, predicate, 0)?;
            }
            ModelRuleExpr::RelationExactF64SumRange {
                relation,
                column,
                min,
                max,
            } => {
                out.push(3);
                push_u128(out, relation.raw());
                push_u128(out, column.raw());
                match min {
                    Some(value) => {
                        out.push(1);
                        push_u64(out, value.bits());
                    }
                    None => out.push(0),
                }
                match max {
                    Some(value) => {
                        out.push(1);
                        push_u64(out, value.bits());
                    }
                    None => out.push(0),
                }
            }
        }
    }

    let relation_column_rules: Vec<_> = schema.all_relation_column_rules().collect();
    push_len(out, relation_column_rules.len())?;
    for ((relation, column), rule) in relation_column_rules {
        push_u128(out, relation.raw());
        push_u128(out, column.raw());
        encode_field_rule(out, rule)?;
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

pub(crate) fn decode_context(
    cursor: &mut impl BinarySource,
    version: u16,
) -> Result<SemanticContext, DurabilityError> {
    let mut schema = Schema::new(SchemaRevisionId::new(cursor.u64().map_err(corrupt)?));
    decode_symbols(cursor, &mut schema)?;
    decode_types(cursor, &mut schema)?;
    decode_capabilities(cursor, &mut schema)?;
    decode_fields(cursor, &mut schema)?;
    if version >= 3 {
        decode_field_rules(cursor, &mut schema)?;
    }
    if version >= 4 {
        decode_entity_rules(cursor, &mut schema)?;
    }
    decode_relations(cursor, &mut schema)?;
    if version >= 5 {
        decode_model_rules(cursor, &mut schema)?;
    }
    if version >= 3 {
        decode_relation_column_rules(cursor, &mut schema)?;
    }
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

fn decode_model_rules(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    for _ in 0..count {
        let rule = match cursor.u8().map_err(corrupt)? {
            0 => {
                let relation = SemanticId::new(cursor.u128().map_err(corrupt)?);
                let min = cursor.u64().map_err(corrupt)?;
                let max = match cursor.u8().map_err(corrupt)? {
                    0 => None,
                    1 => Some(cursor.u64().map_err(corrupt)?),
                    _ => return Err(corrupt("invalid model cardinality max tag")),
                };
                ModelRuleExpr::RelationCardinality { relation, min, max }
            }
            1 => ModelRuleExpr::RelationExists {
                relation: SemanticId::new(cursor.u128().map_err(corrupt)?),
                predicate: decode_semantic_rule_expr(cursor, 0)?,
            },
            2 => ModelRuleExpr::RelationAll {
                relation: SemanticId::new(cursor.u128().map_err(corrupt)?),
                predicate: decode_semantic_rule_expr(cursor, 0)?,
            },
            3 => {
                let relation = SemanticId::new(cursor.u128().map_err(corrupt)?);
                let column = SemanticId::new(cursor.u128().map_err(corrupt)?);
                let min = match cursor.u8().map_err(corrupt)? {
                    0 => None,
                    1 => Some(
                        FiniteF64::from_bits(cursor.u64().map_err(corrupt)?)
                            .ok_or_else(|| corrupt("non-finite model sum min"))?,
                    ),
                    _ => return Err(corrupt("invalid model sum min tag")),
                };
                let max = match cursor.u8().map_err(corrupt)? {
                    0 => None,
                    1 => Some(
                        FiniteF64::from_bits(cursor.u64().map_err(corrupt)?)
                            .ok_or_else(|| corrupt("non-finite model sum max"))?,
                    ),
                    _ => return Err(corrupt("invalid model sum max tag")),
                };
                ModelRuleExpr::RelationExactF64SumRange {
                    relation,
                    column,
                    min,
                    max,
                }
            }
            _ => return Err(corrupt("unknown model rule tag")),
        };
        schema
            .add_model_rule(rule)
            .map_err(|_| corrupt("invalid checkpoint model rule"))?;
    }
    Ok(())
}

fn decode_symbols(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
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

fn decode_types(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
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
    cursor: &mut impl BinarySource,
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

fn encode_field_rule(out: &mut impl BinarySink, rule: &FieldRule) -> Result<(), CodecError> {
    match rule {
        FieldRule::I64Range { min, max } => {
            out.push(0);
            encode_optional_i64(out, *min);
            encode_optional_i64(out, *max);
        }
        FieldRule::TextLength { min, max } => {
            out.push(1);
            push_u64(
                out,
                u64::try_from(*min).map_err(|_| CodecError::LengthOverflow)?,
            );
            match max {
                Some(max) => {
                    out.push(1);
                    push_u64(
                        out,
                        u64::try_from(*max).map_err(|_| CodecError::LengthOverflow)?,
                    );
                }
                None => out.push(0),
            }
        }
        FieldRule::TextOneOf(values) => {
            out.push(2);
            push_len(out, values.len())?;
            for value in values {
                push_bytes(out, value.as_bytes())?;
            }
        }
        FieldRule::TextMatches(pattern) => {
            out.push(3);
            encode_text_pattern(out, pattern)?;
        }
        FieldRule::Expr(expression) => {
            out.push(4);
            encode_semantic_rule_expr(out, expression, 0)?;
        }
    }
    Ok(())
}

const MAX_TEXT_PATTERN_DEPTH: usize = 128;

fn encode_text_pattern(out: &mut impl BinarySink, pattern: &TextPattern) -> Result<(), CodecError> {
    match pattern {
        TextPattern::Never => out.push(0),
        TextPattern::Empty => out.push(1),
        TextPattern::Literal(value) => {
            out.push(2);
            push_bytes(out, value.as_bytes())?;
        }
        TextPattern::AnyScalar => out.push(3),
        TextPattern::Concat(parts) => {
            out.push(4);
            push_len(out, parts.len())?;
            for part in parts {
                encode_text_pattern(out, part)?;
            }
        }
        TextPattern::Alternate(parts) => {
            out.push(5);
            push_len(out, parts.len())?;
            for part in parts {
                encode_text_pattern(out, part)?;
            }
        }
        TextPattern::ZeroOrMore(inner) => {
            out.push(6);
            encode_text_pattern(out, inner)?;
        }
    }
    Ok(())
}

fn decode_text_pattern(
    cursor: &mut impl BinarySource,
    depth: usize,
) -> Result<TextPattern, DurabilityError> {
    if depth > MAX_TEXT_PATTERN_DEPTH {
        return Err(corrupt("text pattern nesting exceeds codec limit"));
    }
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(TextPattern::Never),
        1 => Ok(TextPattern::Empty),
        2 => Ok(TextPattern::Literal(cursor.string().map_err(corrupt)?)),
        3 => Ok(TextPattern::AnyScalar),
        tag @ (4 | 5) => {
            let count = cursor.len().map_err(corrupt)?;
            let mut parts = Vec::with_capacity(count);
            for _ in 0..count {
                parts.push(decode_text_pattern(cursor, depth + 1)?);
            }
            if tag == 4 {
                Ok(TextPattern::Concat(parts))
            } else {
                Ok(TextPattern::Alternate(parts))
            }
        }
        6 => Ok(TextPattern::ZeroOrMore(Box::new(decode_text_pattern(
            cursor,
            depth + 1,
        )?))),
        _ => Err(corrupt("unknown text pattern tag")),
    }
}

fn encode_optional_i64(out: &mut impl BinarySink, value: Option<i64>) {
    match value {
        Some(value) => {
            out.push(1);
            push_u64(out, value as u64);
        }
        None => out.push(0),
    }
}

fn decode_optional_i64(cursor: &mut impl BinarySource) -> Result<Option<i64>, DurabilityError> {
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(None),
        1 => Ok(Some(cursor.u64().map_err(corrupt)? as i64)),
        _ => Err(corrupt("invalid optional i64 rule tag")),
    }
}

fn decode_field_rule(cursor: &mut impl BinarySource) -> Result<FieldRule, DurabilityError> {
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(FieldRule::I64Range {
            min: decode_optional_i64(cursor)?,
            max: decode_optional_i64(cursor)?,
        }),
        1 => {
            let min = usize::try_from(cursor.u64().map_err(corrupt)?)
                .map_err(|_| corrupt("field rule length overflow"))?;
            let max = match cursor.u8().map_err(corrupt)? {
                0 => None,
                1 => Some(
                    usize::try_from(cursor.u64().map_err(corrupt)?)
                        .map_err(|_| corrupt("field rule length overflow"))?,
                ),
                _ => return Err(corrupt("invalid optional length rule tag")),
            };
            Ok(FieldRule::TextLength { min, max })
        }
        2 => {
            let count = cursor.len().map_err(corrupt)?;
            let mut values = BTreeSet::new();
            for _ in 0..count {
                if !values.insert(cursor.string().map_err(corrupt)?) {
                    return Err(corrupt("duplicate text membership rule value"));
                }
            }
            Ok(FieldRule::TextOneOf(values))
        }
        3 => Ok(FieldRule::TextMatches(decode_text_pattern(cursor, 0)?)),
        4 => Ok(FieldRule::Expr(decode_semantic_rule_expr(cursor, 0)?)),
        _ => Err(corrupt("unknown field rule tag")),
    }
}

const MAX_SEMANTIC_RULE_DEPTH: usize = 128;

fn encode_rule_value_expr(out: &mut impl BinarySink, value: &RuleValueExpr) {
    match value {
        RuleValueExpr::Input => out.push(0),
        RuleValueExpr::Field(field) => {
            out.push(1);
            push_u128(out, field.raw());
        }
    }
}

fn decode_rule_value_expr(
    cursor: &mut impl BinarySource,
) -> Result<RuleValueExpr, DurabilityError> {
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(RuleValueExpr::Input),
        1 => Ok(RuleValueExpr::Field(SemanticId::new(
            cursor.u128().map_err(corrupt)?,
        ))),
        _ => Err(corrupt("unknown semantic rule value tag")),
    }
}

fn encode_semantic_rule_expr(
    out: &mut impl BinarySink,
    rule: &SemanticRuleExpr,
    depth: usize,
) -> Result<(), CodecError> {
    if depth > MAX_SEMANTIC_RULE_DEPTH {
        return Err(CodecError::LengthOverflow);
    }
    match rule {
        SemanticRuleExpr::True => out.push(0),
        SemanticRuleExpr::False => out.push(1),
        SemanticRuleExpr::And(rules) | SemanticRuleExpr::Or(rules) => {
            out.push(if matches!(rule, SemanticRuleExpr::And(_)) {
                2
            } else {
                3
            });
            push_len(out, rules.len())?;
            for rule in rules {
                encode_semantic_rule_expr(out, rule, depth + 1)?;
            }
        }
        SemanticRuleExpr::Not(rule) => {
            out.push(4);
            encode_semantic_rule_expr(out, rule, depth + 1)?;
        }
        SemanticRuleExpr::I64Range { value, min, max } => {
            out.push(5);
            encode_rule_value_expr(out, value);
            encode_optional_i64(out, *min);
            encode_optional_i64(out, *max);
        }
        SemanticRuleExpr::TextLength { value, min, max } => {
            out.push(6);
            encode_rule_value_expr(out, value);
            push_u64(
                out,
                u64::try_from(*min).map_err(|_| CodecError::LengthOverflow)?,
            );
            match max {
                Some(max) => {
                    out.push(1);
                    push_u64(
                        out,
                        u64::try_from(*max).map_err(|_| CodecError::LengthOverflow)?,
                    );
                }
                None => out.push(0),
            }
        }
        SemanticRuleExpr::TextOneOf { value, allowed } => {
            out.push(7);
            encode_rule_value_expr(out, value);
            push_len(out, allowed.len())?;
            for item in allowed {
                push_bytes(out, item.as_bytes())?;
            }
        }
        SemanticRuleExpr::TextMatches { value, pattern } => {
            out.push(8);
            encode_rule_value_expr(out, value);
            encode_text_pattern(out, pattern)?;
        }
    }
    Ok(())
}

fn decode_semantic_rule_expr(
    cursor: &mut impl BinarySource,
    depth: usize,
) -> Result<SemanticRuleExpr, DurabilityError> {
    if depth > MAX_SEMANTIC_RULE_DEPTH {
        return Err(corrupt("semantic rule nesting exceeds codec limit"));
    }
    match cursor.u8().map_err(corrupt)? {
        0 => Ok(SemanticRuleExpr::True),
        1 => Ok(SemanticRuleExpr::False),
        tag @ (2 | 3) => {
            let count = cursor.len().map_err(corrupt)?;
            let mut rules = Vec::with_capacity(count);
            for _ in 0..count {
                rules.push(decode_semantic_rule_expr(cursor, depth + 1)?);
            }
            if tag == 2 {
                Ok(SemanticRuleExpr::And(rules))
            } else {
                Ok(SemanticRuleExpr::Or(rules))
            }
        }
        4 => Ok(SemanticRuleExpr::Not(Box::new(decode_semantic_rule_expr(
            cursor,
            depth + 1,
        )?))),
        5 => Ok(SemanticRuleExpr::I64Range {
            value: decode_rule_value_expr(cursor)?,
            min: decode_optional_i64(cursor)?,
            max: decode_optional_i64(cursor)?,
        }),
        6 => {
            let value = decode_rule_value_expr(cursor)?;
            let min = usize::try_from(cursor.u64().map_err(corrupt)?)
                .map_err(|_| corrupt("semantic rule length overflow"))?;
            let max = match cursor.u8().map_err(corrupt)? {
                0 => None,
                1 => Some(
                    usize::try_from(cursor.u64().map_err(corrupt)?)
                        .map_err(|_| corrupt("semantic rule length overflow"))?,
                ),
                _ => return Err(corrupt("invalid semantic rule optional length")),
            };
            Ok(SemanticRuleExpr::TextLength { value, min, max })
        }
        7 => {
            let value = decode_rule_value_expr(cursor)?;
            let count = cursor.len().map_err(corrupt)?;
            let mut allowed = BTreeSet::new();
            for _ in 0..count {
                if !allowed.insert(cursor.string().map_err(corrupt)?) {
                    return Err(corrupt("duplicate semantic membership value"));
                }
            }
            Ok(SemanticRuleExpr::TextOneOf { value, allowed })
        }
        8 => Ok(SemanticRuleExpr::TextMatches {
            value: decode_rule_value_expr(cursor)?,
            pattern: decode_text_pattern(cursor, 0)?,
        }),
        _ => Err(corrupt("unknown semantic rule expression tag")),
    }
}

fn decode_entity_rules(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    for _ in 0..count {
        let owner = SemanticId::new(cursor.u128().map_err(corrupt)?);
        let rule = decode_semantic_rule_expr(cursor, 0)?;
        schema
            .add_entity_rule(owner, rule)
            .map_err(|_| corrupt("invalid checkpoint entity rule"))?;
    }
    Ok(())
}

fn decode_field_rules(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    for _ in 0..count {
        let field = SemanticId::new(cursor.u128().map_err(corrupt)?);
        let rule = decode_field_rule(cursor)?;
        schema
            .add_field_rule(field, rule)
            .map_err(|_| corrupt("invalid checkpoint field rule"))?;
    }
    Ok(())
}

fn decode_relation_column_rules(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    for _ in 0..count {
        let relation = SemanticId::new(cursor.u128().map_err(corrupt)?);
        let column = SemanticId::new(cursor.u128().map_err(corrupt)?);
        let rule = decode_field_rule(cursor)?;
        schema
            .add_relation_column_rule(relation, column, rule)
            .map_err(|_| corrupt("invalid checkpoint relation column rule"))?;
    }
    Ok(())
}

fn decode_fields(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
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

fn decode_relations(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "relations not strictly sorted")?;
        let column_count = cursor.len().map_err(corrupt)?;
        let mut columns = Vec::with_capacity(cursor.bounded_capacity(column_count));
        let mut column_ids = Vec::with_capacity(cursor.bounded_capacity(column_count));
        for _ in 0..column_count {
            column_ids.push(SemanticId::new(cursor.u128().map_err(corrupt)?));
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
            .define_relation_with_column_ids(
                RelationDef {
                    id,
                    columns,
                    semantics,
                },
                column_ids,
            )
            .map_err(|_| corrupt("invalid checkpoint relation"))?;
    }
    Ok(())
}

fn decode_structural_equivalences(
    cursor: &mut impl BinarySource,
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
    cursor: &mut impl BinarySource,
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

fn decode_inclusions(
    cursor: &mut impl BinarySource,
    schema: &mut Schema,
) -> Result<(), DurabilityError> {
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

fn decode_environment(
    cursor: &mut impl BinarySource,
) -> Result<SemanticEnvironment, DurabilityError> {
    let mut environment =
        SemanticEnvironment::new(SemanticEnvId::new(cursor.u64().map_err(corrupt)?));
    let count = cursor.len().map_err(corrupt)?;
    let mut previous = None;
    for _ in 0..count {
        let id = ordered_semantic_id(cursor, &mut previous, "modules not strictly sorted")?;
        let digest: [u8; 32] = cursor
            .take_owned(32)
            .map_err(corrupt)?
            .try_into()
            .map_err(|_| corrupt("module digest length"))?;
        environment.pin_module(id, ModuleDigest(digest));
    }
    Ok(environment)
}
