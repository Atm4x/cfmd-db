use std::collections::BTreeSet;

use crate::{AggregateSpec, RelExpr, RelQueryError};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RelReadFootprint {
    relations: BTreeSet<kernel_types::SemanticId>,
    columns: BTreeSet<(kernel_types::SemanticId, kernel_types::SemanticId)>,
}

impl RelReadFootprint {
    #[must_use]
    pub const fn relations(&self) -> &BTreeSet<kernel_types::SemanticId> {
        &self.relations
    }

    #[must_use]
    pub const fn columns(
        &self,
    ) -> &BTreeSet<(kernel_types::SemanticId, kernel_types::SemanticId)> {
        &self.columns
    }
}

pub(crate) fn read_footprint(
    expr: &RelExpr,
    context: &kernel_schema::SemanticContext,
    root_width: usize,
) -> Result<RelReadFootprint, RelQueryError> {
    let mut footprint = RelReadFootprint::default();
    let demanded = (0..root_width).collect();
    collect(expr, context, &demanded, &mut footprint)?;
    Ok(footprint)
}

fn collect(
    expr: &RelExpr,
    context: &kernel_schema::SemanticContext,
    demanded: &BTreeSet<usize>,
    footprint: &mut RelReadFootprint,
) -> Result<usize, RelQueryError> {
    match expr {
        RelExpr::Scan(relation) => {
            let definition = context
                .schema
                .relation(*relation)
                .ok_or(RelQueryError::UnknownRelation(*relation))?;
            let width = definition.columns.len();
            footprint.relations.insert(*relation);
            for column in demanded {
                if *column >= width {
                    return Err(RelQueryError::ColumnOutOfBounds);
                }
                let column_id = context
                    .schema
                    .relation_column_id(*relation, *column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                footprint.columns.insert((*relation, column_id));
            }
            Ok(width)
        }
        RelExpr::FilterEqConst { input, column, .. }
        | RelExpr::FilterOrderConst { input, column, .. }
        | RelExpr::TopKWithTies { input, column, .. } => {
            let mut input_demanded = demanded.clone();
            input_demanded.insert(*column);
            collect(input, context, &input_demanded, footprint)
        }
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            ..
        } => {
            let mut input_demanded = demanded.clone();
            input_demanded.insert(*left_column);
            input_demanded.insert(*right_column);
            collect(input, context, &input_demanded, footprint)
        }
        RelExpr::Project { input, columns } => {
            let mut input_demanded = BTreeSet::new();
            for output_column in demanded {
                let source_column = columns
                    .get(*output_column)
                    .copied()
                    .ok_or(RelQueryError::ColumnOutOfBounds)?;
                input_demanded.insert(source_column);
            }
            collect(input, context, &input_demanded, footprint)?;
            Ok(columns.len())
        }
        RelExpr::JoinEq {
            left,
            right,
            left_column,
            right_column,
            ..
        } => {
            let left_width = output_width(left, context)?;
            let right_width = output_width(right, context)?;
            let mut left_demanded = BTreeSet::from([*left_column]);
            let mut right_demanded = BTreeSet::from([*right_column]);
            for output_column in demanded {
                if *output_column < left_width {
                    left_demanded.insert(*output_column);
                } else {
                    let right_output = output_column - left_width;
                    if right_output >= right_width {
                        return Err(RelQueryError::ColumnOutOfBounds);
                    }
                    right_demanded.insert(right_output);
                }
            }
            collect(left, context, &left_demanded, footprint)?;
            collect(right, context, &right_demanded, footprint)?;
            Ok(left_width + right_width)
        }
        RelExpr::Difference { left, right } | RelExpr::Union { left, right } => {
            let width = output_width(left, context)?;
            if output_width(right, context)? != width {
                return Err(RelQueryError::TypeMismatch);
            }
            let equality_demand = (0..width).collect();
            collect(left, context, &equality_demand, footprint)?;
            collect(right, context, &equality_demand, footprint)?;
            Ok(width)
        }
        RelExpr::AntiJoin {
            left,
            right,
            left_column,
            right_column,
            ..
        } => {
            let left_width = output_width(left, context)?;
            let mut left_demanded = demanded.clone();
            left_demanded.insert(*left_column);
            collect(left, context, &left_demanded, footprint)?;
            collect(right, context, &BTreeSet::from([*right_column]), footprint)?;
            Ok(left_width)
        }
        RelExpr::Distinct { input, .. } => {
            let width = output_width(input, context)?;
            let equality_demand = (0..width).collect();
            collect(input, context, &equality_demand, footprint)?;
            Ok(width)
        }
        RelExpr::Group {
            input,
            group_columns,
            aggregate,
            ..
        } => {
            let mut input_demanded = group_columns.iter().copied().collect::<BTreeSet<_>>();
            if let AggregateSpec::ExactF64Sum { value_column, .. } = aggregate {
                input_demanded.insert(*value_column);
            }
            collect(input, context, &input_demanded, footprint)?;
            Ok(group_columns.len() + 1)
        }
        RelExpr::PromoteToBag(input) => collect(input, context, demanded, footprint),
    }
}

fn output_width(
    expr: &RelExpr,
    context: &kernel_schema::SemanticContext,
) -> Result<usize, RelQueryError> {
    match expr {
        RelExpr::Scan(relation) => context
            .schema
            .relation(*relation)
            .map(|relation| relation.columns.len())
            .ok_or(RelQueryError::UnknownRelation(*relation)),
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterOrderConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::Distinct { input, .. }
        | RelExpr::TopKWithTies { input, .. }
        | RelExpr::PromoteToBag(input) => output_width(input, context),
        RelExpr::Project { columns, .. } => Ok(columns.len()),
        RelExpr::JoinEq { left, right, .. } => Ok(output_width(left, context)? + output_width(right, context)?),
        RelExpr::Difference { left, .. } | RelExpr::Union { left, .. } | RelExpr::AntiJoin { left, .. } => {
            output_width(left, context)
        }
        RelExpr::Group { group_columns, .. } => Ok(group_columns.len() + 1),
    }
}
