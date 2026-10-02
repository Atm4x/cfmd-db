use kernel_query::{AggregateSpec, OrderComparison, OrderDirection, RelExpr};
use kernel_types::SemanticId;

use crate::binary_codec::{BinarySource, encode_value, push_len, push_u64, push_u128};
use crate::runtime::CodecError;

const MAX_QUERY_DEPTH: usize = 128;

pub(super) fn encode_rel_expr(
    out: &mut impl crate::binary_codec::BinarySink,
    expr: &RelExpr,
    depth: usize,
) -> Result<(), CodecError> {
    if depth > MAX_QUERY_DEPTH {
        return Err(CodecError::ValueNestingTooDeep);
    }
    match expr {
        RelExpr::Scan(relation) => {
            out.push(0);
            push_u128(out, relation.raw());
        }
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => encode_eq_filter(out, input, *column, value, *equivalence, depth)?,
        RelExpr::FilterOrderConst {
            input,
            column,
            value,
            ordering,
            comparison,
        } => encode_order_filter(out, input, *column, value, *ordering, *comparison, depth)?,
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => encode_filter_columns(out, input, *left_column, *right_column, *equivalence, depth)?,
        RelExpr::Project { input, columns } => {
            out.push(2);
            encode_rel_expr(out, input, depth + 1)?;
            push_len(out, columns.len())?;
            for column in columns {
                push_usize(out, *column)?;
            }
        }
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            out.push(3);
            encode_rel_expr(out, left, depth + 1)?;
            encode_rel_expr(out, right, depth + 1)?;
            push_usize(out, *left_column)?;
            push_usize(out, *right_column)?;
            push_u128(out, equivalence.raw());
        }
        RelExpr::Difference { left, right } => {
            out.push(9);
            encode_rel_expr(out, left, depth + 1)?;
            encode_rel_expr(out, right, depth + 1)?;
        }
        RelExpr::Union { left, right } => {
            out.push(12);
            encode_rel_expr(out, left, depth + 1)?;
            encode_rel_expr(out, right, depth + 1)?;
        }
        RelExpr::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            equivalence,
        } => {
            out.push(10);
            encode_rel_expr(out, left, depth + 1)?;
            encode_rel_expr(out, right, depth + 1)?;
            push_usize(out, *left_column)?;
            push_usize(out, *right_column)?;
            push_u128(out, equivalence.raw());
        }
        RelExpr::Distinct {
            input,
            column_equivalences,
        } => {
            out.push(4);
            encode_rel_expr(out, input, depth + 1)?;
            encode_semantic_ids(out, column_equivalences)?;
        }
        RelExpr::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } => encode_rel_group(
            out,
            input,
            group_columns,
            group_equivalences,
            aggregate,
            depth,
        )?,
        RelExpr::TopKWithTies {
            input,
            column,
            ordering,
            direction,
            k,
        } => encode_rel_top_k(out, input, *column, *ordering, *direction, *k, depth)?,
        RelExpr::PromoteToBag(input) => encode_promote_to_bag(out, input, depth)?,
    }
    Ok(())
}

fn encode_eq_filter(
    out: &mut impl crate::binary_codec::BinarySink,
    input: &RelExpr,
    column: usize,
    value: &kernel_model::Value,
    equivalence: SemanticId,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(1);
    encode_rel_expr(out, input, depth + 1)?;
    push_usize(out, column)?;
    encode_value(out, value, 0)?;
    push_u128(out, equivalence.raw());
    Ok(())
}

fn encode_order_filter(
    out: &mut impl crate::binary_codec::BinarySink,
    input: &RelExpr,
    column: usize,
    value: &kernel_model::Value,
    ordering: SemanticId,
    comparison: OrderComparison,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(11);
    encode_rel_expr(out, input, depth + 1)?;
    push_usize(out, column)?;
    encode_value(out, value, 0)?;
    push_u128(out, ordering.raw());
    out.push(match comparison {
        OrderComparison::Less => 0,
        OrderComparison::LessOrEqual => 1,
        OrderComparison::Greater => 2,
        OrderComparison::GreaterOrEqual => 3,
    });
    Ok(())
}

fn encode_promote_to_bag(
    out: &mut impl crate::binary_codec::BinarySink,
    input: &RelExpr,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(7);
    encode_rel_expr(out, input, depth + 1)
}

fn encode_filter_columns(
    out: &mut impl crate::binary_codec::BinarySink,
    input: &RelExpr,
    left_column: usize,
    right_column: usize,
    equivalence: SemanticId,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(8);
    encode_rel_expr(out, input, depth + 1)?;
    push_usize(out, left_column)?;
    push_usize(out, right_column)?;
    push_u128(out, equivalence.raw());
    Ok(())
}

fn encode_rel_group(
    out: &mut impl crate::binary_codec::BinarySink,
    input: &RelExpr,
    group_columns: &[usize],
    group_equivalences: &[SemanticId],
    aggregate: &AggregateSpec,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(5);
    encode_rel_expr(out, input, depth + 1)?;
    push_len(out, group_columns.len())?;
    for column in group_columns {
        push_usize(out, *column)?;
    }
    encode_semantic_ids(out, group_equivalences)?;
    encode_aggregate(out, aggregate)
}

fn encode_rel_top_k(
    out: &mut impl crate::binary_codec::BinarySink,
    input: &RelExpr,
    column: usize,
    ordering: SemanticId,
    direction: OrderDirection,
    k: usize,
    depth: usize,
) -> Result<(), CodecError> {
    out.push(6);
    encode_rel_expr(out, input, depth + 1)?;
    push_usize(out, column)?;
    push_u128(out, ordering.raw());
    out.push(match direction {
        OrderDirection::Ascending => 0,
        OrderDirection::Descending => 1,
    });
    push_usize(out, k)
}

pub(super) fn decode_rel_expr(
    cursor: &mut impl BinarySource,
    depth: usize,
) -> Result<RelExpr, &'static str> {
    if depth > MAX_QUERY_DEPTH {
        return Err("query nesting exceeds hard limit");
    }
    match cursor.u8()? {
        0 => Ok(RelExpr::Scan(SemanticId::new(cursor.u128()?))),
        1 => Ok(RelExpr::FilterEqConst {
            input: Box::new(decode_rel_expr(cursor, depth + 1)?),
            column: decode_usize(cursor)?,
            value: cursor.value(0)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        2 => {
            let input = Box::new(decode_rel_expr(cursor, depth + 1)?);
            let count = cursor.len()?;
            let mut columns = Vec::with_capacity(cursor.bounded_capacity(count));
            for _ in 0..count {
                columns.push(decode_usize(cursor)?);
            }
            Ok(RelExpr::Project { input, columns })
        }
        3 => Ok(RelExpr::JoinEq {
            left: Box::new(decode_rel_expr(cursor, depth + 1)?),
            right: Box::new(decode_rel_expr(cursor, depth + 1)?),
            left_column: decode_usize(cursor)?,
            right_column: decode_usize(cursor)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        4 => Ok(RelExpr::Distinct {
            input: Box::new(decode_rel_expr(cursor, depth + 1)?),
            column_equivalences: decode_semantic_ids(cursor)?,
        }),
        5 => {
            let input = Box::new(decode_rel_expr(cursor, depth + 1)?);
            let count = cursor.len()?;
            let mut group_columns = Vec::with_capacity(cursor.bounded_capacity(count));
            for _ in 0..count {
                group_columns.push(decode_usize(cursor)?);
            }
            Ok(RelExpr::Group {
                input,
                group_columns,
                group_equivalences: decode_semantic_ids(cursor)?,
                aggregate: decode_aggregate(cursor)?,
            })
        }
        6 => {
            let input = Box::new(decode_rel_expr(cursor, depth + 1)?);
            let column = decode_usize(cursor)?;
            let ordering = SemanticId::new(cursor.u128()?);
            let direction = match cursor.u8()? {
                0 => OrderDirection::Ascending,
                1 => OrderDirection::Descending,
                _ => return Err("invalid order direction"),
            };
            let k = decode_usize(cursor)?;
            Ok(RelExpr::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            })
        }
        7 => Ok(RelExpr::PromoteToBag(Box::new(decode_rel_expr(
            cursor,
            depth + 1,
        )?))),
        8 => Ok(RelExpr::FilterEqColumns {
            input: Box::new(decode_rel_expr(cursor, depth + 1)?),
            left_column: decode_usize(cursor)?,
            right_column: decode_usize(cursor)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        9 => Ok(RelExpr::Difference {
            left: Box::new(decode_rel_expr(cursor, depth + 1)?),
            right: Box::new(decode_rel_expr(cursor, depth + 1)?),
        }),
        10 => Ok(RelExpr::AntiJoin {
            left: Box::new(decode_rel_expr(cursor, depth + 1)?),
            right: Box::new(decode_rel_expr(cursor, depth + 1)?),
            left_column: decode_usize(cursor)?,
            right_column: decode_usize(cursor)?,
            equivalence: SemanticId::new(cursor.u128()?),
        }),
        11 => decode_order_filter(cursor, depth),
        12 => Ok(RelExpr::Union {
            left: Box::new(decode_rel_expr(cursor, depth + 1)?),
            right: Box::new(decode_rel_expr(cursor, depth + 1)?),
        }),
        _ => Err("unknown durable relation expression tag"),
    }
}

fn decode_order_filter(
    cursor: &mut impl BinarySource,
    depth: usize,
) -> Result<RelExpr, &'static str> {
    let input = Box::new(decode_rel_expr(cursor, depth + 1)?);
    let column = decode_usize(cursor)?;
    let value = cursor.value(0)?;
    let ordering = SemanticId::new(cursor.u128()?);
    let comparison = match cursor.u8()? {
        0 => OrderComparison::Less,
        1 => OrderComparison::LessOrEqual,
        2 => OrderComparison::Greater,
        3 => OrderComparison::GreaterOrEqual,
        _ => return Err("invalid order comparison"),
    };
    Ok(RelExpr::FilterOrderConst {
        input,
        column,
        value,
        ordering,
        comparison,
    })
}

fn encode_aggregate(
    out: &mut impl crate::binary_codec::BinarySink,
    aggregate: &AggregateSpec,
) -> Result<(), CodecError> {
    match aggregate {
        AggregateSpec::Count { result_equivalence } => {
            out.push(0);
            push_u128(out, result_equivalence.raw());
        }
        AggregateSpec::ExactF64Sum {
            value_column,
            result_equivalence,
        } => {
            out.push(1);
            push_usize(out, *value_column)?;
            push_u128(out, result_equivalence.raw());
        }
    }
    Ok(())
}

fn decode_aggregate(cursor: &mut impl BinarySource) -> Result<AggregateSpec, &'static str> {
    match cursor.u8()? {
        0 => Ok(AggregateSpec::Count {
            result_equivalence: SemanticId::new(cursor.u128()?),
        }),
        1 => Ok(AggregateSpec::ExactF64Sum {
            value_column: decode_usize(cursor)?,
            result_equivalence: SemanticId::new(cursor.u128()?),
        }),
        _ => Err("unknown durable aggregate tag"),
    }
}

fn encode_semantic_ids(
    out: &mut impl crate::binary_codec::BinarySink,
    ids: &[SemanticId],
) -> Result<(), CodecError> {
    push_len(out, ids.len())?;
    for id in ids {
        push_u128(out, id.raw());
    }
    Ok(())
}

fn decode_semantic_ids(cursor: &mut impl BinarySource) -> Result<Vec<SemanticId>, &'static str> {
    let count = cursor.len()?;
    let mut ids = Vec::with_capacity(cursor.bounded_capacity(count));
    for _ in 0..count {
        ids.push(SemanticId::new(cursor.u128()?));
    }
    Ok(ids)
}

fn push_usize(
    out: &mut impl crate::binary_codec::BinarySink,
    value: usize,
) -> Result<(), CodecError> {
    let value = u64::try_from(value).map_err(|_| CodecError::LengthOverflow)?;
    push_u64(out, value);
    Ok(())
}

fn decode_usize(cursor: &mut impl BinarySource) -> Result<usize, &'static str> {
    usize::try_from(cursor.u64()?).map_err(|_| "usize value overflow")
}
