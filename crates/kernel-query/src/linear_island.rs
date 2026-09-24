//! Compiled normal form for maximal linear maintained-delta islands.
//!
//! This is a physical lowering of existing Γ-DTC `Linear` nodes.  It does not
//! change relational semantics and deliberately excludes zero-crossing/stateful
//! barriers such as `Set` projection, `Distinct`, `Group`, `TopK` and `Join`.

use crate::{
    AdaptiveDelta, DeltaSink, DeltaView, PreparedRelGraph, RelExpr, RelQueryError, Row, Value,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinearIslandPredicate {
    EqConst {
        source_column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    EqColumns {
        left_source_column: usize,
        right_source_column: usize,
        equivalence: kernel_types::SemanticId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearIslandNormalForm {
    input_width: usize,
    predicates: Vec<LinearIslandPredicate>,
    projection: Vec<usize>,
}

impl LinearIslandNormalForm {
    #[must_use]
    pub const fn input_width(&self) -> usize {
        self.input_width
    }

    #[must_use]
    pub fn predicates(&self) -> &[LinearIslandPredicate] {
        &self.predicates
    }

    #[must_use]
    pub fn projection(&self) -> &[usize] {
        &self.projection
    }

    pub fn execute(
        &self,
        input: &impl DeltaView<Row>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<AdaptiveDelta<Row, 2>, RelQueryError> {
        let mut output = AdaptiveDelta::default();
        let mut error = None;
        input.visit(|weight, row| {
            if error.is_some() {
                return;
            }
            match self.project_if_accepted(row, context, registry) {
                Ok(Some(projected)) => output.push_weighted(weight, projected),
                Ok(None) => {}
                Err(cause) => error = Some(cause),
            }
        });
        error.map_or(Ok(output), Err)
    }

    fn project_if_accepted(
        &self,
        row: &Row,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Option<Row>, RelQueryError> {
        if row.len() != self.input_width {
            return Err(RelQueryError::TypeMismatch);
        }
        for predicate in &self.predicates {
            let passes = match predicate {
                LinearIslandPredicate::EqConst {
                    source_column,
                    value,
                    equivalence,
                } => {
                    let source = row
                        .get(*source_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?;
                    registry.equivalent(context, *equivalence, source, value)?
                }
                LinearIslandPredicate::EqColumns {
                    left_source_column,
                    right_source_column,
                    equivalence,
                } => {
                    let left = row
                        .get(*left_source_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?;
                    let right = row
                        .get(*right_source_column)
                        .ok_or(RelQueryError::ColumnOutOfBounds)?;
                    registry.equivalent(context, *equivalence, left, right)?
                }
            };
            if !passes {
                return Ok(None);
            }
        }
        let mut projected = Vec::with_capacity(self.projection.len());
        for source_column in &self.projection {
            projected.push(
                row.get(*source_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?
                    .clone(),
            );
        }
        Ok(Some(projected))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LinearStep {
    FilterEqConst {
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterEqColumns {
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    ProjectBag {
        columns: Vec<usize>,
    },
    PromoteToBag,
}

/// Physical companion compiled from one exact relational expression.
///
/// `linear_islands` lists every maximal DTC-Linear chain. Stateful/non-linear
/// barriers remain in the existing maintained plan and are ported separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledDeltaProgram {
    linear_islands: Vec<LinearIslandNormalForm>,
    barriers: Vec<BarrierKernelClass>,
    execution_graph: PreparedRelGraph,
}

/// Physical state-kernel classes required by non-linear maintained nodes.
///
/// This is deliberately the same partition as the semantic DTC taxonomy, but
/// contains no state implementation choice. Dense/radix/hash/ordered backends
/// are selected behind these barrier identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BarrierKernelClass {
    ZeroCrossing,
    Annotation,
    OrderedBoundary,
    BilinearPullback,
    BlockerZeroCrossing,
}

impl CompiledDeltaProgram {
    pub fn compile(
        query: &RelExpr,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let execution_graph = PreparedRelGraph::compile_typed(query, context, registry)?;
        let root = execution_graph.root();
        let mut linear_islands = Vec::new();
        collect_islands(query, root, &execution_graph, &mut linear_islands)?;
        let mut barriers = Vec::new();
        collect_barriers(query, root, &execution_graph, &mut barriers)?;
        Ok(Self {
            linear_islands,
            barriers,
            execution_graph,
        })
    }

    #[must_use]
    pub fn linear_islands(&self) -> &[LinearIslandNormalForm] {
        &self.linear_islands
    }

    #[must_use]
    pub fn barriers(&self) -> &[BarrierKernelClass] {
        &self.barriers
    }

    #[must_use]
    pub const fn execution_graph(&self) -> &PreparedRelGraph {
        &self.execution_graph
    }
}

fn collect_barriers(
    query: &RelExpr,
    node: crate::NodeId,
    graph: &PreparedRelGraph,
    out: &mut Vec<BarrierKernelClass>,
) -> Result<(), RelQueryError> {
    match query {
        RelExpr::Scan(_) => Ok(()),
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::PromoteToBag(input) => {
            let child = graph
                .unary_input(node)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            collect_barriers(input, child, graph, out)
        }
        RelExpr::Project { input, .. } => {
            let child = graph
                .unary_input(node)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            collect_barriers(input, child, graph, out)?;
            if matches!(
                graph
                    .result_type(child)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                    .semantics,
                kernel_schema::RelationSemantics::Set { .. }
            ) {
                out.push(BarrierKernelClass::ZeroCrossing);
            }
            Ok(())
        }
        RelExpr::Distinct { input, .. } => {
            collect_unary_barrier_child(input, node, graph, out)?;
            out.push(BarrierKernelClass::ZeroCrossing);
            Ok(())
        }
        RelExpr::Group { input, .. } => {
            collect_unary_barrier_child(input, node, graph, out)?;
            out.push(BarrierKernelClass::Annotation);
            Ok(())
        }
        RelExpr::TopKWithTies { input, .. } => {
            collect_unary_barrier_child(input, node, graph, out)?;
            out.push(BarrierKernelClass::OrderedBoundary);
            Ok(())
        }
        RelExpr::JoinEq { left, right, .. } => {
            collect_binary_barrier_children(left, right, node, graph, out)?;
            out.push(BarrierKernelClass::BilinearPullback);
            Ok(())
        }
        RelExpr::Difference { left, right } | RelExpr::AntiJoin { left, right, .. } => {
            collect_binary_barrier_children(left, right, node, graph, out)?;
            out.push(BarrierKernelClass::BlockerZeroCrossing);
            Ok(())
        }
    }
}

fn collect_unary_barrier_child(
    input: &RelExpr,
    node: crate::NodeId,
    graph: &PreparedRelGraph,
    out: &mut Vec<BarrierKernelClass>,
) -> Result<(), RelQueryError> {
    let child = graph
        .unary_input(node)
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
    collect_barriers(input, child, graph, out)
}

fn collect_binary_barrier_children(
    left: &RelExpr,
    right: &RelExpr,
    node: crate::NodeId,
    graph: &PreparedRelGraph,
    out: &mut Vec<BarrierKernelClass>,
) -> Result<(), RelQueryError> {
    let (left_id, right_id) = graph
        .binary_inputs(node)
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
    collect_barriers(left, left_id, graph, out)?;
    collect_barriers(right, right_id, graph, out)
}

fn collect_islands(
    query: &RelExpr,
    node: crate::NodeId,
    graph: &PreparedRelGraph,
    out: &mut Vec<LinearIslandNormalForm>,
) -> Result<(), RelQueryError> {
    let mut cursor = query;
    let mut cursor_id = node;
    let mut outer_steps = Vec::new();
    while let Some((input, input_id, step)) = linear_step(cursor, cursor_id, graph)? {
        outer_steps.push(step);
        cursor = input;
        cursor_id = input_id;
    }
    if !outer_steps.is_empty() {
        let input_width = graph
            .result_type(cursor_id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?
            .columns
            .len();
        outer_steps.reverse();
        out.push(compile_normal_form(input_width, &outer_steps)?);
    }
    collect_barrier_children(cursor, cursor_id, graph, out)
}

fn collect_barrier_children(
    query: &RelExpr,
    node: crate::NodeId,
    graph: &PreparedRelGraph,
    out: &mut Vec<LinearIslandNormalForm>,
) -> Result<(), RelQueryError> {
    match query {
        RelExpr::Scan(_) => Ok(()),
        RelExpr::FilterEqConst { .. }
        | RelExpr::FilterEqColumns { .. }
        | RelExpr::PromoteToBag(_) => collect_islands(query, node, graph, out),
        RelExpr::Project { input, .. }
        | RelExpr::Distinct { input, .. }
        | RelExpr::Group { input, .. }
        | RelExpr::TopKWithTies { input, .. } => {
            let child = graph
                .unary_input(node)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            collect_islands(input, child, graph, out)
        }
        RelExpr::JoinEq { left, right, .. }
        | RelExpr::Difference { left, right }
        | RelExpr::AntiJoin { left, right, .. } => {
            let (left_id, right_id) = graph
                .binary_inputs(node)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            collect_islands(left, left_id, graph, out)?;
            collect_islands(right, right_id, graph, out)
        }
    }
}

fn linear_step<'a>(
    query: &'a RelExpr,
    node: crate::NodeId,
    graph: &PreparedRelGraph,
) -> Result<Option<(&'a RelExpr, crate::NodeId, LinearStep)>, RelQueryError> {
    let unary = || {
        graph
            .unary_input(node)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    };
    let step = match query {
        RelExpr::FilterEqConst {
            input,
            column,
            value,
            equivalence,
        } => Some((
            input.as_ref(),
            unary()?,
            LinearStep::FilterEqConst {
                column: *column,
                value: value.clone(),
                equivalence: *equivalence,
            },
        )),
        RelExpr::FilterEqColumns {
            input,
            left_column,
            right_column,
            equivalence,
        } => Some((
            input.as_ref(),
            unary()?,
            LinearStep::FilterEqColumns {
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            },
        )),
        RelExpr::Project { input, columns } => {
            let input_id = unary()?;
            if matches!(
                graph
                    .result_type(input_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                    .semantics,
                kernel_schema::RelationSemantics::Bag { .. }
            ) {
                Some((
                    input.as_ref(),
                    input_id,
                    LinearStep::ProjectBag {
                        columns: columns.clone(),
                    },
                ))
            } else {
                None
            }
        }
        RelExpr::PromoteToBag(input) => Some((input.as_ref(), unary()?, LinearStep::PromoteToBag)),
        _ => None,
    };
    Ok(step)
}

fn compile_normal_form(
    input_width: usize,
    steps: &[LinearStep],
) -> Result<LinearIslandNormalForm, RelQueryError> {
    let mut mapping = (0..input_width).collect::<Vec<_>>();
    let mut predicates = Vec::new();
    for step in steps {
        match step {
            LinearStep::FilterEqConst {
                column,
                value,
                equivalence,
            } => predicates.push(LinearIslandPredicate::EqConst {
                source_column: *mapping
                    .get(*column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?,
                value: value.clone(),
                equivalence: *equivalence,
            }),
            LinearStep::FilterEqColumns {
                left_column,
                right_column,
                equivalence,
            } => predicates.push(LinearIslandPredicate::EqColumns {
                left_source_column: *mapping
                    .get(*left_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?,
                right_source_column: *mapping
                    .get(*right_column)
                    .ok_or(RelQueryError::ColumnOutOfBounds)?,
                equivalence: *equivalence,
            }),
            LinearStep::ProjectBag { columns } => {
                let mut next = Vec::with_capacity(columns.len());
                for column in columns {
                    next.push(
                        *mapping
                            .get(*column)
                            .ok_or(RelQueryError::ColumnOutOfBounds)?,
                    );
                }
                mapping = next;
            }
            LinearStep::PromoteToBag => {}
        }
    }
    Ok(LinearIslandNormalForm {
        input_width,
        predicates,
        projection: mapping,
    })
}
