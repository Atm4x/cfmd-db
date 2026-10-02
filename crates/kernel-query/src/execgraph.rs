use crate::{AggregateSpec, RelExpr, RelQueryError, RelType};
use std::collections::BTreeMap;

pub type NodeId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionInputSlot {
    Unary,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EdgeTarget {
    node: NodeId,
    slot: ExecutionInputSlot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparedNodeInputs {
    Source(kernel_types::SemanticId),
    Unary(NodeId),
    Binary { left: NodeId, right: NodeId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PreparedGraphNode {
    inputs: PreparedNodeInputs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceProgram {
    occurrences: Box<[SourceOccurrence]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceOccurrence {
    ordinal: u32,
    node: NodeId,
}

impl SourceOccurrence {
    #[must_use]
    pub(crate) const fn ordinal(self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub(crate) const fn node(self) -> NodeId {
        self.node
    }
}

/// One physical transition scheduler for every relational graph shape.
///
/// Unlike the retired V3 experiment, this program has no source-specific route
/// kind and no cost model. Chains remain cheap because runtime scheduling keeps
/// one ready node in the continuation register; real branching spills into the
/// same hierarchical ready set and paged inbox representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedTransitionProgram {
    sources: BTreeMap<kernel_types::SemanticId, SourceProgram>,
    out_edges: Box<[Box<[EdgeTarget]>]>,
    root: NodeId,
    node_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedRelGraph {
    nodes: Box<[PreparedGraphNode]>,
    result_types: Option<Box<[RelType]>>,
    users: Box<[Box<[NodeId]>]>,
    program: UnifiedTransitionProgram,
}

impl PreparedRelGraph {
    #[must_use]
    pub fn compile(query: &RelExpr) -> Self {
        let mut nodes = Vec::new();
        let root = compile_postorder(query, &mut nodes);
        Self::from_postorder(nodes, None, root)
    }

    pub fn compile_typed(
        query: &RelExpr,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        // Validate the complete expression exactly once. The second traversal below is
        // deliberately non-validating: it only materializes the already-proved local
        // result types in the same stable postorder used for NodeId assignment.
        query.typecheck(context, registry)?;
        let mut nodes = Vec::new();
        let mut result_types = Vec::new();
        let root = compile_postorder_typed(query, context, &mut nodes, &mut result_types)?;
        Ok(Self::from_postorder(
            nodes,
            Some(result_types.into_boxed_slice()),
            root,
        ))
    }

    fn from_postorder(
        nodes: Vec<PreparedGraphNode>,
        result_types: Option<Box<[RelType]>>,
        root: NodeId,
    ) -> Self {
        let mut users = vec![Vec::new(); nodes.len()];
        for (node_id, node) in nodes.iter().enumerate() {
            match node.inputs {
                PreparedNodeInputs::Source(_) => {}
                PreparedNodeInputs::Unary(input) => users[input].push(node_id),
                PreparedNodeInputs::Binary { left, right } => {
                    users[left].push(node_id);
                    users[right].push(node_id);
                }
            }
        }
        let users = users
            .into_iter()
            .map(Vec::into_boxed_slice)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let nodes = nodes.into_boxed_slice();
        let program = UnifiedTransitionProgram::compile(&nodes, &users, root);
        Self {
            nodes,
            result_types,
            users,
            program,
        }
    }

    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.program.root
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn source_count(&self) -> usize {
        self.program.source_count()
    }

    #[must_use]
    pub fn result_type(&self, node: NodeId) -> Option<&RelType> {
        self.result_types.as_deref()?.get(node)
    }

    pub(crate) fn unary_input(&self, node: NodeId) -> Option<NodeId> {
        match self.nodes.get(node)?.inputs {
            PreparedNodeInputs::Unary(input) => Some(input),
            PreparedNodeInputs::Source(_) | PreparedNodeInputs::Binary { .. } => None,
        }
    }

    pub(crate) fn binary_inputs(&self, node: NodeId) -> Option<(NodeId, NodeId)> {
        match self.nodes.get(node)?.inputs {
            PreparedNodeInputs::Binary { left, right } => Some((left, right)),
            PreparedNodeInputs::Source(_) | PreparedNodeInputs::Unary(_) => None,
        }
    }

    #[must_use]
    pub fn has_typed_metadata(&self) -> bool {
        self.result_types.is_some()
    }

    #[must_use]
    pub const fn transition_program(&self) -> &UnifiedTransitionProgram {
        &self.program
    }

    #[must_use]
    pub fn user_count(&self, node: NodeId) -> Option<usize> {
        self.users.get(node).map(|users| users.len())
    }
}

impl UnifiedTransitionProgram {
    fn compile(nodes: &[PreparedGraphNode], users: &[Box<[NodeId]>], root: NodeId) -> Self {
        let mut out_edges = vec![Vec::<EdgeTarget>::new(); nodes.len()];
        for (child, node_users) in users.iter().enumerate() {
            for &user in node_users {
                let slot = input_slot(nodes, child, user)
                    .expect("prepared relational graph edge must target its declared input");
                out_edges[child].push(EdgeTarget { node: user, slot });
            }
        }

        let mut occurrence_directory =
            BTreeMap::<kernel_types::SemanticId, Vec<SourceOccurrence>>::new();
        let mut next_ordinal = 0_u32;
        for (node, prepared) in nodes.iter().enumerate() {
            let PreparedNodeInputs::Source(relation) = prepared.inputs else {
                continue;
            };
            occurrence_directory
                .entry(relation)
                .or_default()
                .push(SourceOccurrence {
                    ordinal: next_ordinal,
                    node,
                });
            next_ordinal = next_ordinal
                .checked_add(1)
                .expect("compiled source occurrence count must fit u32");
        }

        let mut sources = BTreeMap::new();
        for (relation, compiled_occurrences) in occurrence_directory {
            sources.insert(
                relation,
                SourceProgram {
                    occurrences: compiled_occurrences.into_boxed_slice(),
                },
            );
        }

        Self {
            sources,
            out_edges: out_edges
                .into_iter()
                .map(Vec::into_boxed_slice)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            root,
            node_count: nodes.len(),
        }
    }

    #[must_use]
    pub const fn root(&self) -> NodeId {
        self.root
    }

    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.node_count
    }

    #[must_use]
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    #[must_use]
    pub fn contains_source(&self, relation: kernel_types::SemanticId) -> bool {
        self.sources.contains_key(&relation)
    }

    #[must_use]
    pub fn source_occurrences(&self, relation: kernel_types::SemanticId) -> Option<usize> {
        self.sources
            .get(&relation)
            .map(|source| source.occurrences.len())
    }

    pub(crate) fn source_occurrence_directory(
        &self,
        relation: kernel_types::SemanticId,
    ) -> Option<&[SourceOccurrence]> {
        self.sources
            .get(&relation)
            .map(|source| source.occurrences.as_ref())
    }

    pub(crate) fn source_node_for_occurrence(
        &self,
        relation: kernel_types::SemanticId,
        ordinal: u32,
    ) -> Option<NodeId> {
        let occurrences = self.source_occurrence_directory(relation)?;
        let index = occurrences
            .binary_search_by_key(&ordinal, |occurrence| occurrence.ordinal)
            .ok()?;
        Some(occurrences[index].node)
    }

    #[must_use]
    pub fn source_is_root(&self, relation: kernel_types::SemanticId) -> bool {
        self.sources.get(&relation).is_some_and(|source| {
            source
                .occurrences
                .iter()
                .any(|entry| entry.node == self.root)
        })
    }

    #[must_use]
    pub fn source_feed_count(&self, relation: kernel_types::SemanticId) -> Option<usize> {
        self.sources.get(&relation).map(|source| {
            source
                .occurrences
                .iter()
                .map(|entry| self.out_edges[entry.node].len())
                .sum()
        })
    }

    #[must_use]
    pub fn out_edge_count(&self, node: NodeId) -> Option<usize> {
        self.out_edges.get(node).map(|edges| edges.len())
    }

    pub(crate) fn deliver_output<D: Clone>(
        &self,
        node: NodeId,
        delta: D,
        scratch: &mut UnifiedTransitionScratch<D>,
    ) -> Result<(), RelQueryError> {
        let edges = self
            .out_edges
            .get(node)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let Some((last, prefix)) = edges.split_last() else {
            return Ok(());
        };
        for edge in prefix {
            scratch.deliver(edge.node, edge.slot, delta.clone())?;
        }
        scratch.deliver(last.node, last.slot, delta)
    }
}

fn input_slot(
    nodes: &[PreparedGraphNode],
    child: NodeId,
    user: NodeId,
) -> Option<ExecutionInputSlot> {
    match nodes.get(user)?.inputs {
        PreparedNodeInputs::Unary(input) if input == child => Some(ExecutionInputSlot::Unary),
        PreparedNodeInputs::Binary { left, .. } if left == child => Some(ExecutionInputSlot::Left),
        PreparedNodeInputs::Binary { right, .. } if right == child => {
            Some(ExecutionInputSlot::Right)
        }
        PreparedNodeInputs::Source(_)
        | PreparedNodeInputs::Unary(_)
        | PreparedNodeInputs::Binary { .. } => None,
    }
}

fn compile_postorder(query: &RelExpr, nodes: &mut Vec<PreparedGraphNode>) -> NodeId {
    let inputs = match query {
        RelExpr::Scan(relation) => PreparedNodeInputs::Source(*relation),
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterOrderConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::Project { input, .. }
        | RelExpr::Distinct { input, .. }
        | RelExpr::Group { input, .. }
        | RelExpr::TopKWithTies { input, .. }
        | RelExpr::PromoteToBag(input) => {
            PreparedNodeInputs::Unary(compile_postorder(input, nodes))
        }
        RelExpr::JoinEq { left, right, .. }
        | RelExpr::Difference { left, right }
        | RelExpr::Union { left, right }
        | RelExpr::AntiJoin { left, right, .. } => PreparedNodeInputs::Binary {
            left: compile_postorder(left, nodes),
            right: compile_postorder(right, nodes),
        },
    };
    let id = nodes.len();
    nodes.push(PreparedGraphNode { inputs });
    id
}

fn compile_postorder_typed(
    query: &RelExpr,
    context: &kernel_schema::SemanticContext,
    nodes: &mut Vec<PreparedGraphNode>,
    result_types: &mut Vec<RelType>,
) -> Result<NodeId, RelQueryError> {
    let (inputs, result_type) = match query {
        RelExpr::Scan(relation) => (
            PreparedNodeInputs::Source(*relation),
            RelExpr::typecheck_scan(*relation, context)?,
        ),
        RelExpr::FilterEqConst { input, .. }
        | RelExpr::FilterOrderConst { input, .. }
        | RelExpr::FilterEqColumns { input, .. }
        | RelExpr::TopKWithTies { input, .. } => {
            let input_id = compile_postorder_typed(input, context, nodes, result_types)?;
            (
                PreparedNodeInputs::Unary(input_id),
                result_types[input_id].clone(),
            )
        }
        RelExpr::Project { input, columns } => {
            let input_id = compile_postorder_typed(input, context, nodes, result_types)?;
            (
                PreparedNodeInputs::Unary(input_id),
                projected_type(&result_types[input_id], columns)?,
            )
        }
        RelExpr::Distinct {
            input,
            column_equivalences,
        } => {
            let input_id = compile_postorder_typed(input, context, nodes, result_types)?;
            (
                PreparedNodeInputs::Unary(input_id),
                RelType {
                    columns: result_types[input_id].columns.clone(),
                    semantics: kernel_schema::RelationSemantics::Set {
                        column_equivalences: column_equivalences.clone(),
                    },
                },
            )
        }
        RelExpr::PromoteToBag(input) => {
            let input_id = compile_postorder_typed(input, context, nodes, result_types)?;
            (
                PreparedNodeInputs::Unary(input_id),
                promoted_bag_type(&result_types[input_id]),
            )
        }
        RelExpr::Group {
            input,
            group_columns,
            group_equivalences,
            aggregate,
        } => {
            let input_id = compile_postorder_typed(input, context, nodes, result_types)?;
            (
                PreparedNodeInputs::Unary(input_id),
                grouped_type(
                    &result_types[input_id],
                    group_columns,
                    group_equivalences,
                    aggregate,
                )?,
            )
        }
        RelExpr::JoinEq { left, right, .. } => {
            let left_id = compile_postorder_typed(left, context, nodes, result_types)?;
            let right_id = compile_postorder_typed(right, context, nodes, result_types)?;
            (
                PreparedNodeInputs::Binary {
                    left: left_id,
                    right: right_id,
                },
                joined_type(&result_types[left_id], &result_types[right_id]),
            )
        }
        RelExpr::Difference { left, right }
        | RelExpr::Union { left, right }
        | RelExpr::AntiJoin { left, right, .. } => {
            let left_id = compile_postorder_typed(left, context, nodes, result_types)?;
            let right_id = compile_postorder_typed(right, context, nodes, result_types)?;
            (
                PreparedNodeInputs::Binary {
                    left: left_id,
                    right: right_id,
                },
                result_types[left_id].clone(),
            )
        }
    };
    let id = nodes.len();
    nodes.push(PreparedGraphNode { inputs });
    result_types.push(result_type);
    Ok(id)
}

fn projected_type(input: &RelType, columns: &[usize]) -> Result<RelType, RelQueryError> {
    let projected_columns = columns
        .iter()
        .map(|column| {
            input
                .columns
                .get(*column)
                .cloned()
                .ok_or(RelQueryError::ColumnOutOfBounds)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source_equivalences = column_equivalences(input);
    let projected_equivalences = columns
        .iter()
        .map(|column| {
            source_equivalences
                .get(*column)
                .copied()
                .ok_or(RelQueryError::ColumnOutOfBounds)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let semantics = if matches!(
        input.semantics,
        kernel_schema::RelationSemantics::Set { .. }
    ) {
        kernel_schema::RelationSemantics::Set {
            column_equivalences: projected_equivalences,
        }
    } else {
        kernel_schema::RelationSemantics::Bag {
            column_equivalences: projected_equivalences,
        }
    };
    Ok(RelType {
        columns: projected_columns,
        semantics,
    })
}

fn promoted_bag_type(input: &RelType) -> RelType {
    RelType {
        columns: input.columns.clone(),
        semantics: kernel_schema::RelationSemantics::Bag {
            column_equivalences: column_equivalences(input).to_vec(),
        },
    }
}

fn grouped_type(
    input: &RelType,
    group_columns: &[usize],
    group_equivalences: &[kernel_types::SemanticId],
    aggregate: &AggregateSpec,
) -> Result<RelType, RelQueryError> {
    let mut columns = group_columns
        .iter()
        .map(|column| {
            input
                .columns
                .get(*column)
                .cloned()
                .ok_or(RelQueryError::ColumnOutOfBounds)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (aggregate_type, result_equivalence) = match aggregate {
        AggregateSpec::Count { result_equivalence } => (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::I64),
            *result_equivalence,
        ),
        AggregateSpec::ExactF64Sum {
            result_equivalence, ..
        } => (
            kernel_schema::TypeExpr::Scalar(kernel_schema::ScalarType::F64),
            *result_equivalence,
        ),
    };
    columns.push(aggregate_type);
    let mut equivalences = group_equivalences.to_vec();
    equivalences.push(result_equivalence);
    Ok(RelType {
        columns,
        semantics: kernel_schema::RelationSemantics::Set {
            column_equivalences: equivalences,
        },
    })
}

fn joined_type(left: &RelType, right: &RelType) -> RelType {
    let equivalences = column_equivalences(left)
        .iter()
        .chain(column_equivalences(right))
        .copied()
        .collect();
    let semantics = if matches!(left.semantics, kernel_schema::RelationSemantics::Set { .. })
        && matches!(
            right.semantics,
            kernel_schema::RelationSemantics::Set { .. }
        ) {
        kernel_schema::RelationSemantics::Set {
            column_equivalences: equivalences,
        }
    } else {
        kernel_schema::RelationSemantics::Bag {
            column_equivalences: equivalences,
        }
    };
    RelType {
        columns: left.columns.iter().chain(&right.columns).cloned().collect(),
        semantics,
    }
}

fn column_equivalences(ty: &RelType) -> &[kernel_types::SemanticId] {
    match &ty.semantics {
        kernel_schema::RelationSemantics::Set {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        } => column_equivalences,
    }
}

#[derive(Debug)]
pub struct NodeInbox<D> {
    unary: Option<D>,
    left: Option<D>,
    right: Option<D>,
}

impl<D> Default for NodeInbox<D> {
    fn default() -> Self {
        Self {
            unary: None,
            left: None,
            right: None,
        }
    }
}

impl<D> NodeInbox<D> {
    pub fn take_unary(&mut self) -> Option<D> {
        self.unary.take()
    }

    pub fn take_left(&mut self) -> Option<D> {
        self.left.take()
    }

    pub fn take_right(&mut self) -> Option<D> {
        self.right.take()
    }
}

#[derive(Debug)]
struct ReadyNode<D> {
    node: NodeId,
    inbox: NodeInbox<D>,
}

/// Reusable sparse scheduling scratch.
///
/// A linear chain remains allocation-free in `continuation`. Real branching
/// spills only affected ready nodes into the ordered map, so candidate clones
/// never allocate capacity proportional to the complete compiled graph.
#[derive(Debug, Default)]
pub struct UnifiedTransitionScratch<D> {
    node_bound: usize,
    continuation: Option<ReadyNode<D>>,
    ready: BTreeMap<NodeId, NodeInbox<D>>,
}

impl<D> UnifiedTransitionScratch<D> {
    pub fn ensure_nodes(&mut self, nodes: usize) {
        debug_assert!(self.continuation.is_none());
        debug_assert!(self.ready.is_empty());
        self.node_bound = nodes;
    }

    pub fn deliver(
        &mut self,
        node: NodeId,
        slot: ExecutionInputSlot,
        delta: D,
    ) -> Result<(), RelQueryError> {
        if node >= self.node_bound {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        if let Some(continuation) = self.continuation.as_mut()
            && continuation.node == node
        {
            return put_side(&mut continuation.inbox, slot, delta);
        }
        if let Some(inbox) = self.ready.get_mut(&node) {
            return put_side(inbox, slot, delta);
        }

        let mut inbox = NodeInbox::default();
        put_side(&mut inbox, slot, delta)?;
        let ready = ReadyNode { node, inbox };
        match self.continuation.take() {
            None => {
                if self
                    .ready
                    .first_key_value()
                    .is_none_or(|(&queued, _)| ready.node < queued)
                {
                    self.continuation = Some(ready);
                } else {
                    self.spill(ready)?;
                }
            }
            Some(current) if ready.node < current.node => {
                self.spill(current)?;
                self.continuation = Some(ready);
            }
            Some(current) => {
                self.continuation = Some(current);
                self.spill(ready)?;
            }
        }
        Ok(())
    }

    fn spill(&mut self, ready: ReadyNode<D>) -> Result<(), RelQueryError> {
        if ready.node >= self.node_bound || self.ready.insert(ready.node, ready.inbox).is_some() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        Ok(())
    }

    pub fn pop_next(&mut self) -> Option<(NodeId, NodeInbox<D>)> {
        if let Some(ready) = self.continuation.take() {
            return Some((ready.node, ready.inbox));
        }
        self.ready.pop_first()
    }

    pub fn finish_success(&mut self) {
        debug_assert!(self.continuation.is_none());
        debug_assert!(self.ready.is_empty());
    }

    pub fn reset(&mut self) {
        self.continuation = None;
        self.ready.clear();
    }
}

fn put_side<D>(
    inbox: &mut NodeInbox<D>,
    slot: ExecutionInputSlot,
    delta: D,
) -> Result<(), RelQueryError> {
    let target = match slot {
        ExecutionInputSlot::Unary => &mut inbox.unary,
        ExecutionInputSlot::Left => &mut inbox.left,
        ExecutionInputSlot::Right => &mut inbox.right,
    };
    if target.is_some() {
        return Err(RelQueryError::InconsistentIncrementalDelta);
    }
    *target = Some(delta);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AggregateSpec, OrderDirection, Value};

    fn scan(id: u128) -> RelExpr {
        RelExpr::Scan(kernel_types::SemanticId::new(id))
    }

    #[test]
    fn compiles_every_rel_expr_shape_into_one_unified_program() {
        let eq = kernel_types::SemanticId::new(100);
        let ord = kernel_types::SemanticId::new(101);
        let aggregate_eq = kernel_types::SemanticId::new(102);
        let left = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(scan(1)),
                column: 0,
                value: Value::I64(7),
                equivalence: eq,
            }),
            columns: vec![0],
        };
        let right = RelExpr::Distinct {
            input: Box::new(RelExpr::FilterEqColumns {
                input: Box::new(scan(2)),
                left_column: 0,
                right_column: 0,
                equivalence: eq,
            }),
            column_equivalences: vec![eq],
        };
        let joined = RelExpr::JoinEq {
            left: Box::new(left),
            right: Box::new(right),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
        let blocked = RelExpr::AntiJoin {
            left: Box::new(joined),
            right: Box::new(RelExpr::Difference {
                left: Box::new(scan(3)),
                right: Box::new(scan(4)),
            }),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
        let grouped = RelExpr::Group {
            input: Box::new(RelExpr::PromoteToBag(Box::new(blocked))),
            group_columns: vec![0],
            group_equivalences: vec![eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: aggregate_eq,
            },
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(grouped),
            column: 1,
            ordering: ord,
            direction: OrderDirection::Descending,
            k: 3,
        };
        let graph = PreparedRelGraph::compile(&query);
        assert_eq!(graph.node_count(), 14);
        assert_eq!(graph.root(), 13);
        assert_eq!(graph.source_count(), 4);
        assert_eq!(graph.transition_program().node_count(), 14);
        for id in 1..=4 {
            assert!(
                graph
                    .transition_program()
                    .contains_source(kernel_types::SemanticId::new(id))
            );
        }
    }

    #[test]
    fn repeated_source_is_compiled_without_route_choice() {
        let relation = kernel_types::SemanticId::new(1);
        let query = RelExpr::Difference {
            left: Box::new(RelExpr::Difference {
                left: Box::new(RelExpr::Scan(relation)),
                right: Box::new(RelExpr::Scan(relation)),
            }),
            right: Box::new(scan(2)),
        };
        let graph = PreparedRelGraph::compile(&query);
        let program = graph.transition_program();
        assert_eq!(program.source_occurrences(relation), Some(2));
        assert_eq!(program.source_feed_count(relation), Some(2));
        assert!(program.contains_source(kernel_types::SemanticId::new(2)));
        let repeated = program.source_occurrence_directory(relation).unwrap();
        assert_eq!(repeated.len(), 2);
        assert_eq!(repeated[0].ordinal(), 0);
        assert_eq!(repeated[0].node(), 0);
        assert_eq!(repeated[1].ordinal(), 1);
        assert_eq!(repeated[1].node(), 1);
        let other = program
            .source_occurrence_directory(kernel_types::SemanticId::new(2))
            .unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].ordinal(), 2);
        assert_eq!(program.source_node_for_occurrence(relation, 1), Some(1));
    }

    #[test]
    fn sparse_ready_map_orders_only_affected_nodes_without_capacity_allocation() {
        let mut scratch = UnifiedTransitionScratch::<i32>::default();
        scratch.ensure_nodes(1_000_000);
        for node in [999_999, 2, 4097, 64, 63, 4096, 1] {
            scratch.deliver(node, ExecutionInputSlot::Unary, 1).unwrap();
        }
        let mut actual = Vec::new();
        while let Some((node, _)) = scratch.pop_next() {
            actual.push(node);
        }
        assert_eq!(actual, vec![1, 2, 63, 64, 4096, 4097, 999_999]);
        scratch.finish_success();
    }

    #[test]
    fn continuation_handles_chain_and_spills_branching_into_same_scheduler() {
        let mut scratch = UnifiedTransitionScratch::<i32>::default();
        scratch.ensure_nodes(256);
        scratch.deliver(100, ExecutionInputSlot::Unary, 1).unwrap();
        scratch.deliver(50, ExecutionInputSlot::Left, 2).unwrap();
        scratch.deliver(150, ExecutionInputSlot::Right, 3).unwrap();
        let (node, mut inbox) = scratch.pop_next().unwrap();
        assert_eq!(node, 50);
        assert_eq!(inbox.take_left(), Some(2));
        let (node, mut inbox) = scratch.pop_next().unwrap();
        assert_eq!(node, 100);
        assert_eq!(inbox.take_unary(), Some(1));
        let (node, mut inbox) = scratch.pop_next().unwrap();
        assert_eq!(node, 150);
        assert_eq!(inbox.take_right(), Some(3));
        assert!(scratch.pop_next().is_none());
        scratch.finish_success();
    }

    #[test]
    fn failed_schedule_reset_discards_sparse_ready_nodes() {
        let mut scratch = UnifiedTransitionScratch::<i32>::default();
        scratch.ensure_nodes(4097);
        scratch.deliver(4096, ExecutionInputSlot::Right, 9).unwrap();
        scratch.deliver(1, ExecutionInputSlot::Unary, 7).unwrap();
        scratch.reset();
        assert!(scratch.pop_next().is_none());
        scratch.deliver(4096, ExecutionInputSlot::Left, 11).unwrap();
        let (node, mut inbox) = scratch.pop_next().unwrap();
        assert_eq!(node, 4096);
        assert_eq!(inbox.take_left(), Some(11));
        assert_eq!(inbox.take_right(), None);
    }
}
