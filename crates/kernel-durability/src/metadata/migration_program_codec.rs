use kernel_query::{ExactQuery, Expr};
use kernel_transport::{
    MigrationColumnRewrite, MigrationFieldRewrite, MigrationRelationRewrite, MigrationRowRewrite,
    RelationRewrite, SchemaMigrationProgram,
};

use crate::binary_codec::{BinarySink, BinarySource, encode_value, push_len, push_u128};
use crate::checkpoint;
use crate::runtime::CodecError;

const MAX_EXPR_DEPTH: usize = 128;

pub(crate) fn encode_schema_migration_program(
    out: &mut impl BinarySink,
    program: &SchemaMigrationProgram,
) -> Result<(), CodecError> {
    crate::binary_codec::push_u16(out, checkpoint::CHECKPOINT_CODEC_VERSION);
    checkpoint::encode_context(out, program.target())?;
    push_len(out, program.field_rewrites().len())?;
    for rewrite in program.field_rewrites() {
        push_len(out, rewrite.source_fields.len())?;
        for source in &rewrite.source_fields {
            push_u128(out, source.raw());
        }
        push_u128(out, rewrite.target_field.raw());
        encode_exact_query(out, &rewrite.transform, 0)?;
    }
    push_len(out, program.relation_rewrites().len())?;
    for rewrite in program.relation_rewrites() {
        match rewrite {
            MigrationRelationRewrite::Query(rewrite) => {
                out.push(0);
                push_u128(out, rewrite.target_relation.raw());
                super::query_codec::encode_rel_expr(out, &rewrite.transform, 0)?;
            }
            MigrationRelationRewrite::Rows(rewrite) => {
                out.push(1);
                push_u128(out, rewrite.source_relation.raw());
                push_u128(out, rewrite.target_relation.raw());
                push_len(out, rewrite.columns.len())?;
                for column in &rewrite.columns {
                    push_len(out, column.source_columns.len())?;
                    for source in &column.source_columns {
                        push_u128(out, source.raw());
                    }
                    push_u128(out, column.target_column.raw());
                    encode_exact_query(out, &column.transform, 0)?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn decode_schema_migration_program(
    cursor: &mut impl BinarySource,
) -> Result<SchemaMigrationProgram, &'static str> {
    let context_version = cursor.u16()?;
    if context_version != checkpoint::CHECKPOINT_CODEC_VERSION {
        return Err("unsupported migration semantic-context codec version");
    }
    let target = checkpoint::decode_context(cursor, context_version)
        .map_err(|_| "invalid migration target semantic context")?;
    let field_count = cursor.len()?;
    let mut fields = Vec::with_capacity(cursor.bounded_capacity(field_count));
    for _ in 0..field_count {
        let source_count = cursor.len()?;
        let mut source_fields = Vec::with_capacity(cursor.bounded_capacity(source_count));
        for _ in 0..source_count {
            source_fields.push(kernel_types::SemanticId::new(cursor.u128()?));
        }
        fields.push(MigrationFieldRewrite {
            source_fields,
            target_field: kernel_types::SemanticId::new(cursor.u128()?),
            transform: decode_exact_query(cursor, 0)?,
        });
    }
    let relation_count = cursor.len()?;
    let mut relations = Vec::with_capacity(cursor.bounded_capacity(relation_count));
    for _ in 0..relation_count {
        relations.push(match cursor.u8()? {
            0 => MigrationRelationRewrite::Query(RelationRewrite {
                target_relation: kernel_types::SemanticId::new(cursor.u128()?),
                transform: super::query_codec::decode_rel_expr(cursor, 0)?,
            }),
            1 => {
                let source_relation = kernel_types::SemanticId::new(cursor.u128()?);
                let target_relation = kernel_types::SemanticId::new(cursor.u128()?);
                let column_count = cursor.len()?;
                let mut columns = Vec::with_capacity(cursor.bounded_capacity(column_count));
                for _ in 0..column_count {
                    let source_count = cursor.len()?;
                    let mut source_columns =
                        Vec::with_capacity(cursor.bounded_capacity(source_count));
                    for _ in 0..source_count {
                        source_columns.push(kernel_types::SemanticId::new(cursor.u128()?));
                    }
                    columns.push(MigrationColumnRewrite {
                        source_columns,
                        target_column: kernel_types::SemanticId::new(cursor.u128()?),
                        transform: decode_exact_query(cursor, 0)?,
                    });
                }
                MigrationRelationRewrite::Rows(MigrationRowRewrite {
                    source_relation,
                    target_relation,
                    columns,
                })
            }
            _ => return Err("unknown migration relation rewrite tag"),
        });
    }
    let program = SchemaMigrationProgram::new(target, fields, relations);
    Ok(program)
}

fn encode_exact_query(
    out: &mut impl BinarySink,
    query: &ExactQuery,
    depth: usize,
) -> Result<(), CodecError> {
    encode_expr(out, query.root(), depth)
}

fn encode_expr(out: &mut impl BinarySink, expr: &Expr, depth: usize) -> Result<(), CodecError> {
    if depth > MAX_EXPR_DEPTH {
        return Err(CodecError::ValueNestingTooDeep);
    }
    match expr {
        Expr::Input => out.push(0),
        Expr::Const(value) => {
            out.push(1);
            encode_value(out, value, 0)?;
        }
        Expr::TypedConst { value, ty } => {
            out.push(2);
            encode_value(out, value, 0)?;
            checkpoint::encode_type_expr(out, ty, 0)?;
        }
        Expr::ProductField { input, field } => {
            out.push(3);
            encode_expr(out, input, depth + 1)?;
            push_u128(out, field.raw());
        }
        Expr::SeqLength(input) => {
            out.push(4);
            encode_expr(out, input, depth + 1)?;
        }
        Expr::SeqSumI64(input) => {
            out.push(5);
            encode_expr(out, input, depth + 1)?;
        }
        Expr::AddI64(left, right) => {
            out.push(6);
            encode_expr(out, left, depth + 1)?;
            encode_expr(out, right, depth + 1)?;
        }
        Expr::I64ToF64(input) => {
            out.push(7);
            encode_expr(out, input, depth + 1)?;
        }
        Expr::If {
            condition,
            when_true,
            when_false,
        } => {
            out.push(8);
            encode_expr(out, condition, depth + 1)?;
            encode_expr(out, when_true, depth + 1)?;
            encode_expr(out, when_false, depth + 1)?;
        }
        Expr::WidenSum {
            input,
            target_variants,
        } => {
            out.push(9);
            encode_expr(out, input, depth + 1)?;
            push_len(out, target_variants.len())?;
            for (tag, ty) in target_variants {
                push_u128(out, tag.raw());
                checkpoint::encode_type_expr(out, ty, 0)?;
            }
        }
    }
    Ok(())
}

fn decode_exact_query(
    cursor: &mut impl BinarySource,
    depth: usize,
) -> Result<ExactQuery, &'static str> {
    Ok(ExactQuery::new(decode_expr(cursor, depth)?))
}

fn decode_expr(cursor: &mut impl BinarySource, depth: usize) -> Result<Expr, &'static str> {
    if depth > MAX_EXPR_DEPTH {
        return Err("migration scalar expression nesting exceeds hard limit");
    }
    Ok(match cursor.u8()? {
        0 => Expr::Input,
        1 => Expr::Const(cursor.value(0)?),
        2 => Expr::TypedConst {
            value: cursor.value(0)?,
            ty: checkpoint::decode_type_expr(cursor, 0)
                .map_err(|_| "invalid migration constant type")?,
        },
        3 => Expr::ProductField {
            input: Box::new(decode_expr(cursor, depth + 1)?),
            field: kernel_types::SemanticId::new(cursor.u128()?),
        },
        4 => Expr::SeqLength(Box::new(decode_expr(cursor, depth + 1)?)),
        5 => Expr::SeqSumI64(Box::new(decode_expr(cursor, depth + 1)?)),
        6 => Expr::AddI64(
            Box::new(decode_expr(cursor, depth + 1)?),
            Box::new(decode_expr(cursor, depth + 1)?),
        ),
        7 => Expr::I64ToF64(Box::new(decode_expr(cursor, depth + 1)?)),
        8 => Expr::If {
            condition: Box::new(decode_expr(cursor, depth + 1)?),
            when_true: Box::new(decode_expr(cursor, depth + 1)?),
            when_false: Box::new(decode_expr(cursor, depth + 1)?),
        },
        9 => {
            let input = Box::new(decode_expr(cursor, depth + 1)?);
            let count = cursor.len()?;
            let mut target_variants = std::collections::BTreeMap::new();
            for _ in 0..count {
                let tag = kernel_types::SemanticId::new(cursor.u128()?);
                let ty = checkpoint::decode_type_expr(cursor, 0)
                    .map_err(|_| "invalid widened sum target type")?;
                if target_variants.insert(tag, ty).is_some() {
                    return Err("duplicate widened sum target variant");
                }
            }
            Expr::WidenSum {
                input,
                target_variants,
            }
        }
        _ => return Err("unknown migration scalar expression tag"),
    })
}
