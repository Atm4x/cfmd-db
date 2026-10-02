use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use kernel_model::Value;
use kernel_persistent::{PersistentOrdMap, PersistentVec};

use crate::{
    MaintainedDelta,
    blocker::{
        BlockerBuildSpec, BlockerDeltaPatch, MaintainedBlockerKind, MaterializedBlockerDeltaState,
    },
    delta_abi::{
        CompiledDeltaEdgeIdentity, ExactDeltaSink, ExactDeltaView, ValidatedTransitionFrame,
    },
    delta_kernels::{rel_delta_filter, rel_delta_filter_columns, rel_delta_filter_order_const},
    delta_materialization::{
        maintained_delta_from_relation_delta, materialize_exact_delta_view,
        materialize_exact_delta_view_uncounted,
    },
    differential_api::relation_deltas_semantically_equivalent,
    differential_program::{RelDifferentialProgram, RelDifferentialStateRequirement},
    execgraph::{NodeId, NodeInbox, UnifiedTransitionScratch},
    group::{GroupCommitPatch, MaterializedGroupDeltaState},
    join::{JoinDeltaPatch, MaterializedJoinDeltaState},
    maintained_delta_kernels::{
        OrderFilterSpec, filter_columns_delta_view, filter_delta_view, filter_order_delta_view,
        project_bag_delta_view, project_delta_view,
    },
    projection::project_rows,
    quotient::{
        CanonicalRowPositionIndex, canonical_row_position_index,
        canonical_row_position_index_from_keys,
    },
    rel_model::{
        RelExpr, RelQueryError, RelType, RelationValue, Row, relation_column_equivalences,
        relation_value_from_rows,
    },
    relation_oracles::relation_values_semantically_equivalent,
    relation_state::{
        MaintainedScanCommitPatch, MaterializedSetSupportState, RelationDelta,
        RelationScanOccurrenceSeed, SetSupportPatch, StorageResolvedRelationDelta,
        StorageResolvedScanPatch, commit_relation_mutation, plan_relation_mutation,
    },
    topk::{MaterializedTopKDeltaState, TopKDeltaPatch},
};

#[cfg(debug_assertions)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MaintainedRelPlanNode {
    Scan {
        relation: kernel_types::SemanticId,
        value: PersistentVec<Row>,
        handles: Option<MaintainedLeafHandles>,
        base_witness: Option<crate::RelationBaseWitness>,
        canonical_lookup: CanonicalRowPositionIndex,
    },
    Filter {
        input: Box<MaterializedRelPlanState>,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterOrder {
        input: Box<MaterializedRelPlanState>,
        column: usize,
        value: Value,
        ordering: kernel_types::SemanticId,
        comparison: crate::OrderComparison,
    },
    FilterColumns {
        input: Box<MaterializedRelPlanState>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    ProjectBag {
        input: Box<MaterializedRelPlanState>,
        columns: Vec<usize>,
    },
    ProjectSet {
        input: Box<MaterializedRelPlanState>,
        columns: Vec<usize>,
        supports: MaterializedSetSupportState,
    },
    Distinct {
        input: Box<MaterializedRelPlanState>,
        supports: MaterializedSetSupportState,
    },
    PromoteToBag {
        input: Box<MaterializedRelPlanState>,
    },
    UnionBag {
        left: Box<MaterializedRelPlanState>,
        right: Box<MaterializedRelPlanState>,
    },
    UnionSet {
        left: Box<MaterializedRelPlanState>,
        right: Box<MaterializedRelPlanState>,
        supports: MaterializedSetSupportState,
    },
    Blocker {
        left: Box<MaterializedRelPlanState>,
        right: Box<MaterializedRelPlanState>,
        state: MaterializedBlockerDeltaState,
    },
    Join {
        left: Box<MaterializedRelPlanState>,
        right: Box<MaterializedRelPlanState>,
        state: MaterializedJoinDeltaState,
    },
    Group {
        input: Box<MaterializedRelPlanState>,
        state: MaterializedGroupDeltaState,
    },
    TopK {
        input: Box<MaterializedRelPlanState>,
        state: MaterializedTopKDeltaState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FlatMaintainedRelPlanNode {
    result_type: RelType,
    kind: FlatMaintainedRelPlanNodeKind,
}

struct BuiltFlatMaintainedSubtree {
    id: NodeId,
    output: RelationValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FlatMaintainedRelPlanNodeKind {
    Scan {
        relation: kernel_types::SemanticId,
        value: PersistentVec<Row>,
        handles: Option<MaintainedLeafHandles>,
        base_witness: Option<crate::RelationBaseWitness>,
        canonical_lookup: CanonicalRowPositionIndex,
    },
    Filter {
        input: NodeId,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterOrder {
        input: NodeId,
        column: usize,
        value: Value,
        ordering: kernel_types::SemanticId,
        comparison: crate::OrderComparison,
    },
    FilterColumns {
        input: NodeId,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    ProjectBag {
        input: NodeId,
        columns: Vec<usize>,
    },
    ProjectSet {
        input: NodeId,
        columns: Vec<usize>,
        supports: MaterializedSetSupportState,
    },
    Distinct {
        input: NodeId,
        supports: MaterializedSetSupportState,
    },
    PromoteToBag {
        input: NodeId,
    },
    UnionBag {
        left: NodeId,
        right: NodeId,
    },
    UnionSet {
        left: NodeId,
        right: NodeId,
        supports: MaterializedSetSupportState,
    },
    Blocker {
        left: NodeId,
        right: NodeId,
        state: MaterializedBlockerDeltaState,
    },
    Join {
        left: NodeId,
        right: NodeId,
        state: MaterializedJoinDeltaState,
    },
    Group {
        input: NodeId,
        state: MaterializedGroupDeltaState,
    },
    TopK {
        input: NodeId,
        state: MaterializedTopKDeltaState,
    },
}

#[derive(Debug)]
#[cfg(debug_assertions)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations to this representation."
)]
enum MaintainedRelPlanPatch {
    Scan(Option<MaintainedScanCommitPatch>),
    Unary(Box<MaintainedRelPlanPatch>),
    SetUnary {
        input: Box<MaintainedRelPlanPatch>,
        patch: SetSupportPatch,
    },
    Binary {
        left: Box<MaintainedRelPlanPatch>,
        right: Box<MaintainedRelPlanPatch>,
    },
    SetBinary {
        left: Box<MaintainedRelPlanPatch>,
        right: Box<MaintainedRelPlanPatch>,
        patch: SetSupportPatch,
    },
    Blocker {
        left: Box<MaintainedRelPlanPatch>,
        right: Box<MaintainedRelPlanPatch>,
        patch: BlockerDeltaPatch,
    },
    Join {
        left: Box<MaintainedRelPlanPatch>,
        right: Box<MaintainedRelPlanPatch>,
        patch: JoinDeltaPatch,
    },
    Group {
        input: Box<MaintainedRelPlanPatch>,
        patch: GroupCommitPatch,
    },
    TopK {
        input: Box<MaintainedRelPlanPatch>,
        patch: TopKDeltaPatch,
    },
}

#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "Preserve inline state ownership without adding allocations to this representation."
)]
enum GraphNodePatch {
    Scan(MaintainedScanCommitPatch),
    SetSupport(SetSupportPatch),
    Blocker(BlockerDeltaPatch),
    Join(JoinDeltaPatch),
    Group(GroupCommitPatch),
    TopK(TopKDeltaPatch),
}

#[derive(Debug)]
struct GraphPatchSet {
    nodes: Vec<(NodeId, GraphNodePatch)>,
    root_effect: MaintainedDelta,
}

struct PlannedGraphNodeTransition {
    patch: Option<GraphNodePatch>,
    effect: MaintainedDelta,
}

#[derive(Debug)]
#[cfg(debug_assertions)]
struct PlannedMaintainedRelPlanTransition {
    patch: MaintainedRelPlanPatch,
    effect: MaintainedDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MaintainedLeafLink {
    previous: Option<kernel_types::StableRowHandle>,
    next: Option<kernel_types::StableRowHandle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaintainedLeafHandles {
    dense_ids: PersistentVec<kernel_types::StableRowHandle>,
    positions: PersistentOrdMap<kernel_types::StableRowHandle, usize>,
    links: PersistentOrdMap<kernel_types::StableRowHandle, MaintainedLeafLink>,
    logical_head: Option<kernel_types::StableRowHandle>,
    logical_tail: Option<kernel_types::StableRowHandle>,
}

impl MaintainedLeafHandles {
    fn new(ids: Vec<kernel_types::StableRowHandle>) -> Result<Self, RelQueryError> {
        let mut positions = PersistentOrdMap::default();
        let mut links = PersistentOrdMap::default();
        for (position, id) in ids.iter().copied().enumerate() {
            if positions.insert(id, position).is_some() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let link = MaintainedLeafLink {
                previous: position.checked_sub(1).map(|index| ids[index]),
                next: ids.get(position + 1).copied(),
            };
            links.insert(id, link);
        }
        Ok(Self {
            logical_head: ids.first().copied(),
            logical_tail: ids.last().copied(),
            dense_ids: ids.into(),
            positions,
            links,
        })
    }

    fn row_for_handle<'a>(
        &self,
        value: &'a PersistentVec<Row>,
        id: kernel_types::StableRowHandle,
    ) -> Result<&'a Row, RelQueryError> {
        let position = self
            .positions
            .get(&id)
            .copied()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        value
            .get(position)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    }

    fn ordered_rows(&self, value: &PersistentVec<Row>) -> Result<Vec<Row>, RelQueryError> {
        if value.len() != self.dense_ids.len()
            || self.positions.len() != self.dense_ids.len()
            || self.links.len() != self.dense_ids.len()
        {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }

        let mut rows = Vec::with_capacity(self.dense_ids.len());
        let mut current = self.logical_head;
        while let Some(id) = current {
            if rows.len() >= self.dense_ids.len() {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            rows.push(self.row_for_handle(value, id)?.clone());
            current = self
                .links
                .get(&id)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                .next;
        }
        if rows.len() != self.dense_ids.len() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        Ok(rows)
    }

    fn remove(
        &mut self,
        value: &mut PersistentVec<Row>,
        id: kernel_types::StableRowHandle,
    ) -> Result<(), RelQueryError> {
        let position = self
            .positions
            .remove(&id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let link = self
            .links
            .remove(&id)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;

        match link.previous {
            Some(previous) => {
                let mut previous_link = *self
                    .links
                    .get(&previous)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                previous_link.next = link.next;
                self.links.insert(previous, previous_link);
            }
            None => self.logical_head = link.next,
        }
        match link.next {
            Some(next) => {
                let mut next_link = *self
                    .links
                    .get(&next)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                next_link.previous = link.previous;
                self.links.insert(next, next_link);
            }
            None => self.logical_tail = link.previous,
        }

        value.swap_remove(position);
        let removed_id = self.dense_ids.swap_remove(position);
        if removed_id != id {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        if position < self.dense_ids.len() {
            self.positions.insert(self.dense_ids[position], position);
        }
        Ok(())
    }

    fn insert(
        &mut self,
        value: &mut PersistentVec<Row>,
        id: kernel_types::StableRowHandle,
        row: Row,
    ) -> Result<(), RelQueryError> {
        if self.positions.contains_key(&id) || self.links.contains_key(&id) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let position = value.len();
        value.push(row);
        self.dense_ids.push(id);
        self.positions.insert(id, position);
        self.links.insert(
            id,
            MaintainedLeafLink {
                previous: self.logical_tail,
                next: None,
            },
        );
        if let Some(previous) = self.logical_tail {
            let mut previous_link = *self
                .links
                .get(&previous)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            previous_link.next = Some(id);
            self.links.insert(previous, previous_link);
        } else {
            self.logical_head = Some(id);
        }
        self.logical_tail = Some(id);
        Ok(())
    }
}

fn combine_maintained_deltas(left: &MaintainedDelta, right: &MaintainedDelta) -> MaintainedDelta {
    let mut combined = MaintainedDelta::with_capacity(left.support_len() + right.support_len());
    left.visit_exact(|weight, row| combined.push_exact(weight.clone(), row.clone()));
    right.visit_exact(|weight, row| combined.push_exact(weight.clone(), row.clone()));
    combined
}

fn collect_flat_maintained_state_requirements(
    arena: &[Arc<FlatMaintainedRelPlanNode>],
    out: &mut BTreeSet<RelDifferentialStateRequirement>,
) {
    for node in arena {
        match node.kind {
            FlatMaintainedRelPlanNodeKind::Scan { .. }
            | FlatMaintainedRelPlanNodeKind::Filter { .. }
            | FlatMaintainedRelPlanNodeKind::FilterOrder { .. }
            | FlatMaintainedRelPlanNodeKind::FilterColumns { .. }
            | FlatMaintainedRelPlanNodeKind::ProjectBag { .. }
            | FlatMaintainedRelPlanNodeKind::PromoteToBag { .. }
            | FlatMaintainedRelPlanNodeKind::UnionBag { .. } => {}
            FlatMaintainedRelPlanNodeKind::ProjectSet { .. }
            | FlatMaintainedRelPlanNodeKind::Distinct { .. }
            | FlatMaintainedRelPlanNodeKind::UnionSet { .. } => {
                out.insert(RelDifferentialStateRequirement::SetSupport);
            }
            FlatMaintainedRelPlanNodeKind::Blocker { .. } => {
                out.insert(RelDifferentialStateRequirement::BlockerMass);
            }
            FlatMaintainedRelPlanNodeKind::Join { .. } => {
                out.insert(RelDifferentialStateRequirement::JoinFibers);
            }
            FlatMaintainedRelPlanNodeKind::Group { .. } => {
                out.insert(RelDifferentialStateRequirement::GroupAnnotations);
            }
            FlatMaintainedRelPlanNodeKind::TopK { .. } => {
                out.insert(RelDifferentialStateRequirement::OrderedCut);
            }
        }
    }
}

/// Flat `NodeId` owner for maintained relational execution state.
///
/// Construction may use a temporary bottom-up tree to reuse the operator builders, but
/// release states discard that tree after flattening. Runtime planning, validation,
/// output reconstruction, and commit address authoritative state only through the
/// compiled graph's stable postorder `NodeId` coordinates.
#[derive(Debug)]
pub struct MaterializedRelPlanState {
    query: RelExpr,
    semantic_context: kernel_schema::SemanticContext,
    differential: Arc<RelDifferentialProgram>,
    result_type: RelType,
    #[cfg(debug_assertions)]
    node: Option<Arc<MaintainedRelPlanNode>>,
    arena: PersistentVec<Arc<FlatMaintainedRelPlanNode>>,
    execgraph_scratch: UnifiedTransitionScratch<MaintainedDelta>,
    transition_epoch: u64,
    revision: Option<kernel_types::RevisionId>,
}

impl Clone for MaterializedRelPlanState {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            semantic_context: self.semantic_context.clone(),
            differential: Arc::clone(&self.differential),
            result_type: self.result_type.clone(),
            #[cfg(debug_assertions)]
            node: self.node.as_ref().map(Arc::clone),
            arena: self.arena.clone(),
            execgraph_scratch: UnifiedTransitionScratch::default(),
            transition_epoch: self.transition_epoch,
            revision: self.revision,
        }
    }
}

impl PartialEq for MaterializedRelPlanState {
    fn eq(&self, other: &Self) -> bool {
        self.query == other.query
            && self.semantic_context == other.semantic_context
            && self.differential == other.differential
            && self.result_type == other.result_type
            && self.arena == other.arena
            && self.transition_epoch == other.transition_epoch
            && self.revision == other.revision
    }
}

impl Eq for MaterializedRelPlanState {}

impl MaterializedRelPlanState {
    pub fn build(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        Self::build_inner(query, old, context, registry, None)
    }

    pub fn build_with_scan_seeds(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scan_seeds: &BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>,
    ) -> Result<Self, RelQueryError> {
        Self::build_inner(query, old, context, registry, Some(scan_seeds))
    }

    fn build_inner(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scan_seeds: Option<&BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>>,
    ) -> Result<Self, RelQueryError> {
        let differential = Arc::new(RelDifferentialProgram::compile(query, context, registry)?);
        let graph = differential.physical_program().execution_graph();
        if !graph.has_typed_metadata() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let mut arena = Vec::with_capacity(graph.node_count());
        let built = Self::build_flat_subtree(
            query,
            old,
            context,
            registry,
            &differential,
            scan_seeds,
            &mut arena,
        )?;
        if built.id != graph.root() || arena.len() != graph.node_count() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let result_type = graph
            .result_type(built.id)
            .cloned()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let mut bound_requirements = BTreeSet::new();
        collect_flat_maintained_state_requirements(&arena, &mut bound_requirements);
        if bound_requirements != differential.state_requirements() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        #[cfg(debug_assertions)]
        let node = Some(Arc::new(Self::rehydrate_debug_tree(
            query,
            built.id,
            &arena,
            context,
            &differential,
        )?));
        Ok(Self {
            query: query.clone(),
            semantic_context: context.clone(),
            differential,
            result_type,
            #[cfg(debug_assertions)]
            node,
            arena: arena.into(),
            execgraph_scratch: UnifiedTransitionScratch::default(),
            transition_epoch: 0,
            revision: None,
        })
    }

    #[allow(clippy::too_many_lines)]
    fn build_flat_subtree(
        query: &RelExpr,
        old: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        differential: &Arc<RelDifferentialProgram>,
        scan_seeds: Option<&BTreeMap<kernel_types::SemanticId, RelationScanOccurrenceSeed>>,
        out: &mut Vec<Arc<FlatMaintainedRelPlanNode>>,
    ) -> Result<BuiltFlatMaintainedSubtree, RelQueryError> {
        let graph = differential.physical_program().execution_graph();
        let (kind, output) = match query {
            RelExpr::Scan(relation) => {
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let value = relation_value_from_rows(
                    old.relations
                        .materialize_owned(relation)
                        .unwrap_or_default(),
                    result_type,
                );
                let canonical_lookup =
                    if let Some(seed) = scan_seeds.and_then(|seeds| seeds.get(relation)) {
                        if seed.relation() != *relation
                            || seed.result_type() != result_type
                            || seed.semantic_context() != context
                            || seed.row_count() != value.rows().len()
                        {
                            return Err(RelQueryError::StructuralRewriteBaseMismatch);
                        }
                        let evidence = seed.canonical_keys_by_row();
                        canonical_row_position_index_from_keys(evidence.iter())
                    } else {
                        canonical_row_position_index(
                            value.rows(),
                            relation_column_equivalences(result_type),
                            context,
                            registry,
                        )?
                    };
                (
                    FlatMaintainedRelPlanNodeKind::Scan {
                        relation: *relation,
                        value: value.rows().to_vec().into(),
                        handles: None,
                        base_witness: None,
                        canonical_lookup,
                    },
                    value,
                )
            }
            RelExpr::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let input_type = graph
                    .result_type(input.id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let filtered = rel_delta_filter(
                    RelationDelta {
                        inserted: input.output.into_rows(),
                        removed: Vec::new(),
                        result_type: input_type,
                    },
                    *column,
                    value,
                    *equivalence,
                    context,
                    registry,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                (
                    FlatMaintainedRelPlanNodeKind::Filter {
                        input: input.id,
                        column: *column,
                        value: value.clone(),
                        equivalence: *equivalence,
                    },
                    relation_value_from_rows(filtered.inserted, result_type),
                )
            }
            RelExpr::FilterOrderConst {
                input,
                column,
                value,
                ordering,
                comparison,
            } => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let input_type = graph
                    .result_type(input.id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let filtered = rel_delta_filter_order_const(
                    RelationDelta {
                        inserted: input.output.into_rows(),
                        removed: Vec::new(),
                        result_type: input_type,
                    },
                    *column,
                    value,
                    *ordering,
                    *comparison,
                    context,
                    registry,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                (
                    FlatMaintainedRelPlanNodeKind::FilterOrder {
                        input: input.id,
                        column: *column,
                        value: value.clone(),
                        ordering: *ordering,
                        comparison: *comparison,
                    },
                    relation_value_from_rows(filtered.inserted, result_type),
                )
            }
            RelExpr::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let input_type = graph
                    .result_type(input.id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let filtered = rel_delta_filter_columns(
                    RelationDelta {
                        inserted: input.output.into_rows(),
                        removed: Vec::new(),
                        result_type: input_type,
                    },
                    *left_column,
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                (
                    FlatMaintainedRelPlanNodeKind::FilterColumns {
                        input: input.id,
                        left_column: *left_column,
                        right_column: *right_column,
                        equivalence: *equivalence,
                    },
                    relation_value_from_rows(filtered.inserted, result_type),
                )
            }
            RelExpr::Project { input, columns } => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let rows = project_rows(input.output.into_rows(), columns)?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                if matches!(
                    graph
                        .result_type(input.id)
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?
                        .semantics,
                    kernel_schema::RelationSemantics::Set { .. }
                ) {
                    let supports =
                        MaterializedSetSupportState::build(&rows, result_type, context, registry)?;
                    let output = supports.output_value();
                    (
                        FlatMaintainedRelPlanNodeKind::ProjectSet {
                            input: input.id,
                            columns: columns.clone(),
                            supports,
                        },
                        output,
                    )
                } else {
                    let output = relation_value_from_rows(rows, &result_type);
                    (
                        FlatMaintainedRelPlanNodeKind::ProjectBag {
                            input: input.id,
                            columns: columns.clone(),
                        },
                        output,
                    )
                }
            }
            RelExpr::Union { left, right } => {
                let left = Self::build_flat_subtree(
                    left,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let right = Self::build_flat_subtree(
                    right,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                if matches!(
                    result_type.semantics,
                    kernel_schema::RelationSemantics::Set { .. }
                ) {
                    let mut rows = left.output.into_rows();
                    rows.extend(right.output.into_rows());
                    let supports =
                        MaterializedSetSupportState::build(&rows, result_type, context, registry)?;
                    let output = supports.output_value();
                    (
                        FlatMaintainedRelPlanNodeKind::UnionSet {
                            left: left.id,
                            right: right.id,
                            supports,
                        },
                        output,
                    )
                } else {
                    let mut rows = left.output.into_rows();
                    rows.extend(right.output.into_rows());
                    let output = relation_value_from_rows(rows, &result_type);
                    (
                        FlatMaintainedRelPlanNodeKind::UnionBag {
                            left: left.id,
                            right: right.id,
                        },
                        output,
                    )
                }
            }
            RelExpr::Difference { left, right } => {
                let left = Self::build_flat_subtree(
                    left,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let right = Self::build_flat_subtree(
                    right,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let state = MaterializedBlockerDeltaState::build(
                    &left.output,
                    &right.output,
                    BlockerBuildSpec {
                        kind: &MaintainedBlockerKind::Difference,
                        left_type: graph
                            .result_type(left.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        right_type: graph
                            .result_type(right.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        result_type,
                        context,
                        registry,
                    },
                )?;
                let output = state.output_value()?;
                (
                    FlatMaintainedRelPlanNodeKind::Blocker {
                        left: left.id,
                        right: right.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => {
                let left = Self::build_flat_subtree(
                    left,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let right = Self::build_flat_subtree(
                    right,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let id = out.len();
                let kind = MaintainedBlockerKind::AntiJoin {
                    left_column: *left_column,
                    right_column: *right_column,
                    equivalence: *equivalence,
                };
                let state = MaterializedBlockerDeltaState::build(
                    &left.output,
                    &right.output,
                    BlockerBuildSpec {
                        kind: &kind,
                        left_type: graph
                            .result_type(left.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        right_type: graph
                            .result_type(right.id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        result_type: graph
                            .result_type(id)
                            .cloned()
                            .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                        context,
                        registry,
                    },
                )?;
                let output = state.output_value()?;
                (
                    FlatMaintainedRelPlanNodeKind::Blocker {
                        left: left.id,
                        right: right.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::Distinct { input, .. } => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let id = out.len();
                let result_type = graph
                    .result_type(id)
                    .cloned()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let supports = MaterializedSetSupportState::build(
                    input.output.rows(),
                    result_type,
                    context,
                    registry,
                )?;
                let output = supports.output_value();
                (
                    FlatMaintainedRelPlanNodeKind::Distinct {
                        input: input.id,
                        supports,
                    },
                    output,
                )
            }
            RelExpr::PromoteToBag(input) => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let output = RelationValue::Bag(input.output.into_rows());
                (
                    FlatMaintainedRelPlanNodeKind::PromoteToBag { input: input.id },
                    output,
                )
            }
            RelExpr::JoinEq { left, right, .. } => {
                let left = Self::build_flat_subtree(
                    left,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let right = Self::build_flat_subtree(
                    right,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let state = MaterializedJoinDeltaState::build_from_input_values(
                    query,
                    &left.output,
                    &right.output,
                    context,
                    registry,
                )?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let output = state.output_value(context, registry)?;
                (
                    FlatMaintainedRelPlanNodeKind::Join {
                        left: left.id,
                        right: right.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::Group { input, .. } => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let state = MaterializedGroupDeltaState::build_from_input_value(
                    query,
                    input.output,
                    context,
                    registry,
                )?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let output = state.output_value()?;
                (
                    FlatMaintainedRelPlanNodeKind::Group {
                        input: input.id,
                        state,
                    },
                    output,
                )
            }
            RelExpr::TopKWithTies { input, .. } => {
                let input = Self::build_flat_subtree(
                    input,
                    old,
                    context,
                    registry,
                    differential,
                    scan_seeds,
                    out,
                )?;
                let state = MaterializedTopKDeltaState::build_from_input_value(
                    query,
                    input.output,
                    context,
                    registry,
                )?
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let output = state.output_value()?;
                (
                    FlatMaintainedRelPlanNodeKind::TopK {
                        input: input.id,
                        state,
                    },
                    output,
                )
            }
        };
        let id = out.len();
        let result_type = graph
            .result_type(id)
            .cloned()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        out.push(Arc::new(FlatMaintainedRelPlanNode { result_type, kind }));
        Ok(BuiltFlatMaintainedSubtree { id, output })
    }

    #[cfg(debug_assertions)]
    #[allow(clippy::too_many_lines)]
    fn rehydrate_debug_tree(
        query: &RelExpr,
        node_id: NodeId,
        arena: &[Arc<FlatMaintainedRelPlanNode>],
        context: &kernel_schema::SemanticContext,
        differential: &Arc<RelDifferentialProgram>,
    ) -> Result<MaintainedRelPlanNode, RelQueryError> {
        let flat = arena
            .get(node_id)
            .map(Arc::as_ref)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let child_state =
            |child_query: &RelExpr, child_id: NodeId| -> Result<Box<Self>, RelQueryError> {
                let child = arena
                    .get(child_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                Ok(Box::new(Self {
                    query: child_query.clone(),
                    semantic_context: context.clone(),
                    differential: Arc::clone(differential),
                    result_type: child.result_type.clone(),
                    node: Some(Arc::new(Self::rehydrate_debug_tree(
                        child_query,
                        child_id,
                        arena,
                        context,
                        differential,
                    )?)),
                    arena: PersistentVec::default(),
                    execgraph_scratch: UnifiedTransitionScratch::default(),
                    transition_epoch: 0,
                    revision: None,
                }))
            };
        Ok(match (query, &flat.kind) {
            (
                RelExpr::Scan(relation),
                FlatMaintainedRelPlanNodeKind::Scan {
                    relation: flat_relation,
                    value,
                    handles,
                    base_witness,
                    canonical_lookup,
                },
            ) if relation == flat_relation => MaintainedRelPlanNode::Scan {
                relation: *relation,
                value: value.clone(),
                handles: handles.clone(),
                base_witness: base_witness.clone(),
                canonical_lookup: canonical_lookup.clone(),
            },
            (
                RelExpr::FilterEqConst {
                    input,
                    column,
                    value,
                    equivalence,
                },
                FlatMaintainedRelPlanNodeKind::Filter { input: id, .. },
            ) => MaintainedRelPlanNode::Filter {
                input: child_state(input, *id)?,
                column: *column,
                value: value.clone(),
                equivalence: *equivalence,
            },
            (
                RelExpr::FilterOrderConst {
                    input,
                    column,
                    value,
                    ordering,
                    comparison,
                },
                FlatMaintainedRelPlanNodeKind::FilterOrder { input: id, .. },
            ) => MaintainedRelPlanNode::FilterOrder {
                input: child_state(input, *id)?,
                column: *column,
                value: value.clone(),
                ordering: *ordering,
                comparison: *comparison,
            },
            (
                RelExpr::FilterEqColumns {
                    input,
                    left_column,
                    right_column,
                    equivalence,
                },
                FlatMaintainedRelPlanNodeKind::FilterColumns { input: id, .. },
            ) => MaintainedRelPlanNode::FilterColumns {
                input: child_state(input, *id)?,
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            },
            (
                RelExpr::Project { input, columns },
                FlatMaintainedRelPlanNodeKind::ProjectBag { input: id, .. },
            ) => MaintainedRelPlanNode::ProjectBag {
                input: child_state(input, *id)?,
                columns: columns.clone(),
            },
            (
                RelExpr::Project { input, columns },
                FlatMaintainedRelPlanNodeKind::ProjectSet {
                    input: id,
                    supports,
                    ..
                },
            ) => MaintainedRelPlanNode::ProjectSet {
                input: child_state(input, *id)?,
                columns: columns.clone(),
                supports: supports.clone(),
            },
            (
                RelExpr::Distinct { input, .. },
                FlatMaintainedRelPlanNodeKind::Distinct {
                    input: id,
                    supports,
                },
            ) => MaintainedRelPlanNode::Distinct {
                input: child_state(input, *id)?,
                supports: supports.clone(),
            },
            (
                RelExpr::PromoteToBag(input),
                FlatMaintainedRelPlanNodeKind::PromoteToBag { input: id },
            ) => MaintainedRelPlanNode::PromoteToBag {
                input: child_state(input, *id)?,
            },
            (
                RelExpr::Union { left, right },
                FlatMaintainedRelPlanNodeKind::UnionBag {
                    left: left_id,
                    right: right_id,
                },
            ) => MaintainedRelPlanNode::UnionBag {
                left: child_state(left, *left_id)?,
                right: child_state(right, *right_id)?,
            },
            (
                RelExpr::Union { left, right },
                FlatMaintainedRelPlanNodeKind::UnionSet {
                    left: left_id,
                    right: right_id,
                    supports,
                },
            ) => MaintainedRelPlanNode::UnionSet {
                left: child_state(left, *left_id)?,
                right: child_state(right, *right_id)?,
                supports: supports.clone(),
            },
            (
                RelExpr::Difference { left, right } | RelExpr::AntiJoin { left, right, .. },
                FlatMaintainedRelPlanNodeKind::Blocker {
                    left: left_id,
                    right: right_id,
                    state,
                },
            ) => MaintainedRelPlanNode::Blocker {
                left: child_state(left, *left_id)?,
                right: child_state(right, *right_id)?,
                state: state.clone(),
            },
            (
                RelExpr::JoinEq { left, right, .. },
                FlatMaintainedRelPlanNodeKind::Join {
                    left: left_id,
                    right: right_id,
                    state,
                },
            ) => MaintainedRelPlanNode::Join {
                left: child_state(left, *left_id)?,
                right: child_state(right, *right_id)?,
                state: state.clone(),
            },
            (
                RelExpr::Group { input, .. },
                FlatMaintainedRelPlanNodeKind::Group { input: id, state },
            ) => MaintainedRelPlanNode::Group {
                input: child_state(input, *id)?,
                state: state.clone(),
            },
            (
                RelExpr::TopKWithTies { input, .. },
                FlatMaintainedRelPlanNodeKind::TopK { input: id, state },
            ) => MaintainedRelPlanNode::TopK {
                input: child_state(input, *id)?,
                state: state.clone(),
            },
            _ => return Err(RelQueryError::InconsistentIncrementalDelta),
        })
    }

    /// Exact Γ-DTC program whose state requirements are bound by this maintained tree.
    ///
    /// This is reconstructible execution metadata, not semantic authority. Construction
    /// rejects any drift between the compiled differential requirements and the concrete
    /// maintained-state capabilities owned by the tree.
    #[must_use]
    pub fn differential(&self) -> &RelDifferentialProgram {
        self.differential.as_ref()
    }

    #[must_use]
    pub fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub fn result_type(&self) -> &RelType {
        &self.result_type
    }

    #[must_use]
    pub fn scan_relations(&self) -> BTreeSet<kernel_types::SemanticId> {
        self.arena
            .iter()
            .filter_map(|node| match &node.kind {
                FlatMaintainedRelPlanNodeKind::Scan { relation, .. } => Some(*relation),
                _ => None,
            })
            .collect()
    }

    pub fn output_value(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        if self.arena.is_empty() {
            #[cfg(debug_assertions)]
            {
                return self.output_value_recursive(context, registry);
            }
            #[cfg(not(debug_assertions))]
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        let root = self
            .differential
            .physical_program()
            .execution_graph()
            .root();
        self.output_value_from_arena(root, context, registry)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn output_value_from_arena(
        &self,
        node_id: NodeId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        let node = self
            .arena
            .get(node_id)
            .map(Arc::as_ref)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        match &node.kind {
            FlatMaintainedRelPlanNodeKind::Scan { value, handles, .. } => {
                let rows = if let Some(handles) = handles {
                    handles.ordered_rows(value)?
                } else {
                    value.iter().cloned().collect()
                };
                Ok(relation_value_from_rows(rows, &node.result_type))
            }
            FlatMaintainedRelPlanNodeKind::Filter {
                input,
                column,
                value,
                equivalence,
            } => self.output_unary_filter_from_arena(
                *input,
                &node.result_type,
                context,
                registry,
                |input_delta| {
                    rel_delta_filter(input_delta, *column, value, *equivalence, context, registry)
                },
            ),
            FlatMaintainedRelPlanNodeKind::FilterOrder {
                input,
                column,
                value,
                ordering,
                comparison,
            } => self.output_unary_filter_from_arena(
                *input,
                &node.result_type,
                context,
                registry,
                |input_delta| {
                    rel_delta_filter_order_const(
                        input_delta,
                        *column,
                        value,
                        *ordering,
                        *comparison,
                        context,
                        registry,
                    )
                },
            ),
            FlatMaintainedRelPlanNodeKind::FilterColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => self.output_unary_filter_from_arena(
                *input,
                &node.result_type,
                context,
                registry,
                |input_delta| {
                    rel_delta_filter_columns(
                        input_delta,
                        *left_column,
                        *right_column,
                        *equivalence,
                        context,
                        registry,
                    )
                },
            ),
            FlatMaintainedRelPlanNodeKind::ProjectBag { input, columns } => {
                let rows = project_rows(
                    self.output_value_from_arena(*input, context, registry)?
                        .into_rows(),
                    columns,
                )?;
                Ok(relation_value_from_rows(rows, &node.result_type))
            }
            FlatMaintainedRelPlanNodeKind::ProjectSet { supports, .. }
            | FlatMaintainedRelPlanNodeKind::Distinct { supports, .. } => {
                Ok(supports.output_value())
            }
            FlatMaintainedRelPlanNodeKind::PromoteToBag { input } => Ok(RelationValue::Bag(
                self.output_value_from_arena(*input, context, registry)?
                    .into_rows(),
            )),
            FlatMaintainedRelPlanNodeKind::UnionBag { left, right } => {
                let mut rows = self
                    .output_value_from_arena(*left, context, registry)?
                    .into_rows();
                rows.extend(
                    self.output_value_from_arena(*right, context, registry)?
                        .into_rows(),
                );
                Ok(RelationValue::Bag(rows))
            }
            FlatMaintainedRelPlanNodeKind::UnionSet { supports, .. } => Ok(supports.output_value()),
            FlatMaintainedRelPlanNodeKind::Blocker { state, .. } => state.output_value(),
            FlatMaintainedRelPlanNodeKind::Join { state, .. } => {
                state.output_value(context, registry)
            }
            FlatMaintainedRelPlanNodeKind::Group { state, .. } => state.output_value(),
            FlatMaintainedRelPlanNodeKind::TopK { state, .. } => state.output_value(),
        }
    }

    fn arena_unary_input_delta(
        &self,
        input: NodeId,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let input_node = self
            .arena
            .get(input)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let input_value = self.output_value_from_arena(input, context, registry)?;
        Ok(RelationDelta {
            inserted: input_value.into_rows(),
            removed: Vec::new(),
            result_type: input_node.result_type.clone(),
        })
    }

    fn output_unary_filter_from_arena<F>(
        &self,
        input: NodeId,
        result_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        filter: F,
    ) -> Result<RelationValue, RelQueryError>
    where
        F: FnOnce(RelationDelta) -> Result<RelationDelta, RelQueryError>,
    {
        let filtered = filter(self.arena_unary_input_delta(input, context, registry)?)?;
        Ok(relation_value_from_rows(filtered.inserted, result_type))
    }

    #[cfg(debug_assertions)]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn output_value_recursive(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        let node = self
            .node
            .as_deref()
            .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        match node {
            MaintainedRelPlanNode::Scan { value, handles, .. } => {
                let rows = if let Some(handles) = handles {
                    handles.ordered_rows(value)?
                } else {
                    value.iter().cloned().collect()
                };
                Ok(relation_value_from_rows(rows, &self.result_type))
            }
            MaintainedRelPlanNode::Filter {
                input,
                column,
                value,
                equivalence,
            } => {
                let input_value = input.output_value_recursive(context, registry)?;
                let input_delta = RelationDelta {
                    inserted: input_value.into_rows(),
                    removed: Vec::new(),
                    result_type: input.result_type.clone(),
                };
                let filtered =
                    rel_delta_filter(input_delta, *column, value, *equivalence, context, registry)?;
                Ok(relation_value_from_rows(
                    filtered.inserted,
                    &self.result_type,
                ))
            }
            MaintainedRelPlanNode::FilterOrder {
                input,
                column,
                value,
                ordering,
                comparison,
            } => {
                let input_value = input.output_value_recursive(context, registry)?;
                let filtered = rel_delta_filter_order_const(
                    RelationDelta {
                        inserted: input_value.into_rows(),
                        removed: Vec::new(),
                        result_type: input.result_type.clone(),
                    },
                    *column,
                    value,
                    *ordering,
                    *comparison,
                    context,
                    registry,
                )?;
                Ok(relation_value_from_rows(
                    filtered.inserted,
                    &self.result_type,
                ))
            }
            MaintainedRelPlanNode::FilterColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => {
                let input_value = input.output_value_recursive(context, registry)?;
                let input_delta = RelationDelta {
                    inserted: input_value.into_rows(),
                    removed: Vec::new(),
                    result_type: input.result_type.clone(),
                };
                let filtered = rel_delta_filter_columns(
                    input_delta,
                    *left_column,
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?;
                Ok(relation_value_from_rows(
                    filtered.inserted,
                    &self.result_type,
                ))
            }
            MaintainedRelPlanNode::ProjectBag { input, columns } => {
                let rows = project_rows(
                    input.output_value_recursive(context, registry)?.into_rows(),
                    columns,
                )?;
                Ok(relation_value_from_rows(rows, &self.result_type))
            }
            MaintainedRelPlanNode::ProjectSet { supports, .. }
            | MaintainedRelPlanNode::Distinct { supports, .. }
            | MaintainedRelPlanNode::UnionSet { supports, .. } => Ok(supports.output_value()),
            MaintainedRelPlanNode::PromoteToBag { input } => Ok(RelationValue::Bag(
                input.output_value_recursive(context, registry)?.into_rows(),
            )),
            MaintainedRelPlanNode::UnionBag { left, right } => {
                let mut rows = left.output_value_recursive(context, registry)?.into_rows();
                rows.extend(right.output_value_recursive(context, registry)?.into_rows());
                Ok(RelationValue::Bag(rows))
            }
            MaintainedRelPlanNode::Blocker { state, .. } => state.output_value(),
            MaintainedRelPlanNode::Join { state, .. } => state.output_value(context, registry),
            MaintainedRelPlanNode::Group { state, .. } => state.output_value(),
            MaintainedRelPlanNode::TopK { state, .. } => state.output_value(),
        }
    }

    pub fn apply_relation_deltas(
        &mut self,
        deltas: &BTreeMap<kernel_types::SemanticId, RelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if self.revision.is_some() {
            return Err(RelQueryError::RevisionBoundMutationRequiresPreparedTransition);
        }
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let transition_program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        for relation in deltas.keys() {
            if !transition_program.contains_source(*relation) {
                return Err(RelQueryError::UnknownRelation(*relation));
            }
        }
        #[cfg(debug_assertions)]
        let recursive_oracle = self.recursive_oracle_from_frames(
            self.validate_leaf_deltas(deltas, context, registry)?,
            context,
            registry,
        )?;
        let mut validated_frames = self.validate_leaf_deltas(deltas, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        let planned =
            self.plan_relation_deltas_execgraph(&mut validated_frames, context, registry)?;
        let output = materialize_exact_delta_view(&planned.root_effect, self.result_type.clone())?;
        #[cfg(debug_assertions)]
        if !relation_deltas_semantically_equivalent(
            &output,
            &recursive_oracle.1,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        self.commit_graph_patch_set(planned);
        #[cfg(debug_assertions)]
        if !relation_values_semantically_equivalent(
            &self.output_value(context, registry)?,
            &recursive_oracle
                .0
                .output_value_recursive(context, registry)?,
            &self.result_type,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        #[cfg(debug_assertions)]
        {
            self.node = recursive_oracle.0.node.as_ref().map(Arc::clone);
        }
        self.transition_epoch = next_epoch;
        Ok(output)
    }

    /// Attaches authoritative storage row identities to every `Scan` of one
    /// relation. Each handle is paired with the exact row payload in logical
    /// scan order. The binding is rejected unless it exactly matches the
    /// maintained Scan snapshot; semantic bag equality alone is insufficient.
    pub fn attach_storage_rows(
        &mut self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
    ) -> Result<(), RelQueryError> {
        self.attach_storage_rows_inner(relation, rows, None)
    }

    /// Attaches storage identities and the already-built exact Γ occurrence
    /// witness owned by the current physical relation. The witness is cloned
    /// by persistent-root sharing; no position -> handle Γ index translation
    /// or row re-canonicalization is performed.
    pub fn attach_storage_rows_with_base_witness(
        &mut self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
        witness: &crate::RelationBaseWitness,
    ) -> Result<(), RelQueryError> {
        self.attach_storage_rows_inner(relation, rows, Some(witness))
    }

    fn attach_storage_rows_inner(
        &mut self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
        witness: Option<&crate::RelationBaseWitness>,
    ) -> Result<(), RelQueryError> {
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        let handles = self.validate_storage_rows_binding(relation, rows)?;
        if let Some(witness) = witness {
            if witness.relation() != relation
                || witness.semantic_context() != &self.semantic_context
                || !witness.certifies_identity_storage_handles(
                    &rows.iter().map(|(handle, _)| *handle).collect::<Vec<_>>(),
                )
            {
                return Err(RelQueryError::StructuralRewriteBaseMismatch);
            }
            let source_nodes = self
                .differential
                .physical_program()
                .execution_graph()
                .transition_program()
                .source_occurrence_directory(relation)
                .ok_or(RelQueryError::UnknownRelation(relation))?;
            if source_nodes.iter().any(|occurrence| {
                self.arena
                    .get(occurrence.node())
                    .is_none_or(|node| node.result_type != *witness.result_type())
            }) {
                return Err(RelQueryError::TypeMismatch);
            }
        }
        #[cfg(debug_assertions)]
        self.attach_storage_rows_recursive(relation, rows, witness)?;
        self.commit_storage_rows_binding(relation, &handles, witness);
        self.transition_epoch = next_epoch;
        Ok(())
    }

    /// Returns an O(1) clone of the shared current-relation Γ occurrence
    /// witness when every Scan occurrence is bound to the same authority.
    pub fn storage_relation_base_witness(
        &self,
        relation: kernel_types::SemanticId,
    ) -> Result<Option<crate::RelationBaseWitness>, RelQueryError> {
        let occurrences = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program()
            .source_occurrence_directory(relation)
            .ok_or(RelQueryError::UnknownRelation(relation))?;
        let mut shared: Option<crate::RelationBaseWitness> = None;
        for occurrence in occurrences {
            let node = self
                .arena
                .get(occurrence.node())
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let FlatMaintainedRelPlanNodeKind::Scan { base_witness, .. } = &node.kind else {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            };
            let Some(witness) = base_witness else {
                return Ok(None);
            };
            if let Some(current) = &shared {
                if !current.certifies_same_base(witness) {
                    return Err(RelQueryError::StructuralRewriteBaseMismatch);
                }
            } else {
                shared = Some(witness.clone());
            }
        }
        Ok(shared)
    }

    fn validate_storage_rows_binding(
        &self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
    ) -> Result<MaintainedLeafHandles, RelQueryError> {
        let program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        let occurrences = program
            .source_occurrence_directory(relation)
            .ok_or(RelQueryError::UnknownRelation(relation))?;
        for occurrence in occurrences {
            let node = self
                .arena
                .get(occurrence.node())
                .map(Arc::as_ref)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let FlatMaintainedRelPlanNodeKind::Scan {
                relation: current,
                value,
                ..
            } = &node.kind
            else {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            };
            if *current != relation
                || value.len() != rows.len()
                || value
                    .iter()
                    .zip(rows.iter().map(|(_, row)| row))
                    .any(|(logical, physical)| logical != physical)
            {
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
        }
        MaintainedLeafHandles::new(rows.iter().map(|(id, _)| *id).collect())
    }

    fn commit_storage_rows_binding(
        &mut self,
        relation: kernel_types::SemanticId,
        handles: &MaintainedLeafHandles,
        base_witness: Option<&crate::RelationBaseWitness>,
    ) {
        let source_nodes = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program()
            .source_occurrence_directory(relation)
            .expect("validated storage binding relation must remain compiled")
            .iter()
            .map(|occurrence| occurrence.node())
            .collect::<Vec<_>>();
        for node_id in source_nodes {
            let node = Arc::make_mut(
                self.arena
                    .get_mut(node_id)
                    .expect("compiled storage binding NodeId must remain valid"),
            );
            let FlatMaintainedRelPlanNodeKind::Scan {
                relation: current,
                handles: target,
                base_witness: target_witness,
                ..
            } = &mut node.kind
            else {
                unreachable!("compiled storage binding source must remain a Scan");
            };
            debug_assert_eq!(*current, relation);
            *target = Some(handles.clone());
            *target_witness = base_witness.cloned();
        }
    }

    #[cfg(debug_assertions)]
    fn attach_storage_rows_recursive(
        &mut self,
        relation: kernel_types::SemanticId,
        rows: &[(kernel_types::StableRowHandle, Row)],
        base_witness: Option<&crate::RelationBaseWitness>,
    ) -> Result<(), RelQueryError> {
        match Arc::make_mut(
            self.node
                .as_mut()
                .expect("debug construction tree must exist while binding storage rows"),
        ) {
            MaintainedRelPlanNode::Scan {
                relation: current,
                value,
                handles,
                base_witness: target_witness,
                ..
            } => {
                if *current != relation {
                    return Ok(());
                }
                if value.len() != rows.len()
                    || value
                        .iter()
                        .zip(rows.iter().map(|(_, row)| row))
                        .any(|(logical, physical)| logical != physical)
                {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                *handles = Some(MaintainedLeafHandles::new(
                    rows.iter().map(|(id, _)| *id).collect(),
                )?);
                *target_witness = base_witness.cloned();
                Ok(())
            }
            MaintainedRelPlanNode::Filter { input, .. }
            | MaintainedRelPlanNode::FilterOrder { input, .. }
            | MaintainedRelPlanNode::FilterColumns { input, .. }
            | MaintainedRelPlanNode::ProjectBag { input, .. }
            | MaintainedRelPlanNode::ProjectSet { input, .. }
            | MaintainedRelPlanNode::Distinct { input, .. }
            | MaintainedRelPlanNode::PromoteToBag { input }
            | MaintainedRelPlanNode::Group { input, .. }
            | MaintainedRelPlanNode::TopK { input, .. } => {
                input.attach_storage_rows_recursive(relation, rows, base_witness)
            }
            MaintainedRelPlanNode::UnionBag { left, right }
            | MaintainedRelPlanNode::UnionSet { left, right, .. }
            | MaintainedRelPlanNode::Blocker { left, right, .. }
            | MaintainedRelPlanNode::Join { left, right, .. } => {
                left.attach_storage_rows_recursive(relation, rows, base_witness)?;
                right.attach_storage_rows_recursive(relation, rows, base_witness)
            }
        }
    }

    /// Binds this maintained snapshot to its dependency-frontier revision anchor.
    ///
    /// The anchor advances only when a dependency of this materialization changes;
    /// a runtime may therefore publish unrelated global revisions without cloning
    /// this state solely to rewrite a revision tag. A bound state can advance only
    /// through a revision-aware candidate transition.
    pub fn bind_revision(
        &mut self,
        revision: kernel_types::RevisionId,
    ) -> Result<(), RelQueryError> {
        match self.revision {
            Some(current) if current == revision => return Ok(()),
            Some(_) => return Err(RelQueryError::RevisionBindingMismatch),
            None => {}
        }
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        self.revision = Some(revision);
        self.transition_epoch = next_epoch;
        Ok(())
    }

    #[must_use]
    pub const fn revision(&self) -> Option<kernel_types::RevisionId> {
        self.revision
    }

    #[must_use]
    pub const fn transition_epoch(&self) -> u64 {
        self.transition_epoch
    }

    #[cfg(test)]
    fn test_arena_shares_storage_with(&self, other: &Self) -> bool {
        self.arena.shares_storage_with(&other.arena)
    }

    #[cfg(test)]
    fn test_arena_len(&self) -> usize {
        self.arena.len()
    }

    #[cfg(test)]
    fn test_node_arc_shared_with(&self, other: &Self, node: NodeId) -> Option<bool> {
        Some(Arc::ptr_eq(self.arena.get(node)?, other.arena.get(node)?))
    }

    #[cfg(test)]
    fn test_scan_value_sharing_with(&self, other: &Self, node: NodeId) -> Option<bool> {
        let left = self.arena.get(node)?;
        let right = other.arena.get(node)?;
        match (&left.kind, &right.kind) {
            (
                FlatMaintainedRelPlanNodeKind::Scan { value: left, .. },
                FlatMaintainedRelPlanNodeKind::Scan { value: right, .. },
            ) => Some(left.shares_storage_with(right)),
            _ => None,
        }
    }

    #[cfg(test)]
    fn test_scan_value_page_sharing_with(
        &self,
        other: &Self,
        node: NodeId,
        index: usize,
    ) -> Option<bool> {
        let left = self.arena.get(node)?;
        let right = other.arena.get(node)?;
        match (&left.kind, &right.kind) {
            (
                FlatMaintainedRelPlanNodeKind::Scan { value: left, .. },
                FlatMaintainedRelPlanNodeKind::Scan { value: right, .. },
            ) => Some(left.shares_page_with(right, index)),
            _ => None,
        }
    }

    #[cfg(test)]
    fn test_scan_value_len(&self, node: NodeId) -> Option<usize> {
        match &self.arena.get(node)?.kind {
            FlatMaintainedRelPlanNodeKind::Scan { value, .. } => Some(value.len()),
            _ => None,
        }
    }

    #[cfg(test)]
    fn test_distinct_support_sharing_with(&self, other: &Self) -> Option<(bool, bool)> {
        let left = self.arena.iter().find_map(|node| match &node.kind {
            FlatMaintainedRelPlanNodeKind::Distinct { supports, .. } => Some(supports),
            _ => None,
        })?;
        let right = other.arena.iter().find_map(|node| match &node.kind {
            FlatMaintainedRelPlanNodeKind::Distinct { supports, .. } => Some(supports),
            _ => None,
        })?;
        Some(left.test_shares_storage_with(right))
    }

    #[cfg(test)]
    fn test_blocker_storage_sharing_with(&self, other: &Self) -> Option<bool> {
        let left = self.arena.iter().find_map(|node| match &node.kind {
            FlatMaintainedRelPlanNodeKind::Blocker { state, .. } => Some(state),
            _ => None,
        })?;
        let right = other.arena.iter().find_map(|node| match &node.kind {
            FlatMaintainedRelPlanNodeKind::Blocker { state, .. } => Some(state),
            _ => None,
        })?;
        left.test_shares_difference_storage_with(right)
    }

    #[cfg(test)]
    fn test_group_storage_sharing_with(&self, other: &Self) -> Option<(bool, bool)> {
        let left = self.arena.iter().find_map(|node| match &node.kind {
            FlatMaintainedRelPlanNodeKind::Group { state, .. } => Some(state),
            _ => None,
        })?;
        let right = other.arena.iter().find_map(|node| match &node.kind {
            FlatMaintainedRelPlanNodeKind::Group { state, .. } => Some(state),
            _ => None,
        })?;
        Some(left.test_storage_sharing_with(right))
    }

    #[cfg(test)]
    fn test_execgraph_sparse_patch_shape(
        &self,
        deltas: &BTreeMap<kernel_types::SemanticId, RelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(usize, Vec<NodeId>, usize), RelQueryError> {
        let mut frames = self.validate_leaf_deltas(deltas, context, registry)?;
        let mut scratch = UnifiedTransitionScratch::default();
        let patch = self.plan_relation_deltas_execgraph_with_scratch(
            &mut frames,
            context,
            registry,
            &mut scratch,
        )?;
        Ok((
            patch.nodes.len(),
            patch.nodes.iter().map(|(node_id, _)| *node_id).collect(),
            self.differential
                .physical_program()
                .execution_graph()
                .node_count(),
        ))
    }

    #[cfg(all(test, not(debug_assertions)))]
    fn test_compiled_graph_shape(&self) -> (usize, bool) {
        let graph = self.differential.physical_program().execution_graph();
        (graph.node_count(), graph.has_typed_metadata())
    }

    /// Builds a detached candidate for a revision-bound maintained plan from
    /// storage-resolved row identities. The current dependency-frontier anchor
    /// is authoritative and is advanced to `target_revision` only in the detached
    /// candidate. Publication remains owned by `kernel-plan`.
    pub fn candidate_from_storage_resolved_deltas_for_revision(
        &self,
        target_revision: kernel_types::RevisionId,
        deltas: &BTreeMap<kernel_types::SemanticId, StorageResolvedRelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(Self, RelationDelta), RelQueryError> {
        let source_revision = self
            .revision
            .ok_or(RelQueryError::RevisionBindingMismatch)?;
        if source_revision == target_revision {
            return Err(RelQueryError::InvalidRevisionTransition);
        }
        let mut candidate = self.clone();
        let output_delta =
            candidate.apply_storage_resolved_deltas_in_place(deltas, context, registry)?;
        candidate.revision = Some(target_revision);
        Ok((candidate, output_delta))
    }

    /// Builds the exact same compiled-edge frame map as the semantic-delta
    /// planner, but seals Scan publication to already-resolved stable handles.
    /// All operator state above Scan therefore goes through one immutable
    /// plan/commit semantics regardless of the source delta representation.
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn validate_resolved_leaf_frames(
        &self,
        deltas: &BTreeMap<kernel_types::SemanticId, StorageResolvedRelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<ValidatedLeafTransitionFrames, RelQueryError> {
        let mut frames = BTreeMap::new();
        let program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        for (relation, resolved) in deltas {
            let occurrences = program
                .source_occurrence_directory(*relation)
                .ok_or(RelQueryError::UnknownRelation(*relation))?;
            for occurrence in occurrences {
                let node = self
                    .arena
                    .get(occurrence.node())
                    .map(Arc::as_ref)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let FlatMaintainedRelPlanNodeKind::Scan {
                    relation: node_relation,
                    value,
                    handles,
                    base_witness,
                    ..
                } = &node.kind
                else {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                };
                if node_relation != relation || resolved.relation() != *relation {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                let edge = CompiledDeltaEdgeIdentity::new(occurrence.ordinal(), *relation);
                if resolved.delta().result_type != node.result_type
                    || resolved.removed_handles().len() != resolved.delta().removed.len()
                    || resolved.inserted_handles().len() != resolved.delta().inserted.len()
                {
                    return Err(RelQueryError::TypeMismatch);
                }
                MaterializedSetSupportState::validate_rows(
                    &resolved.delta().removed,
                    &node.result_type,
                    context,
                    registry,
                )?;
                MaterializedSetSupportState::validate_rows(
                    &resolved.delta().inserted,
                    &node.result_type,
                    context,
                    registry,
                )?;
                let handles = handles
                    .as_ref()
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let mut seen = BTreeSet::new();
                for (id, row) in resolved
                    .removed_handles()
                    .iter()
                    .copied()
                    .zip(&resolved.delta().removed)
                {
                    if !seen.insert(id) || handles.row_for_handle(value, id)? != row {
                        return Err(RelQueryError::InconsistentIncrementalDelta);
                    }
                }
                for id in resolved.inserted_handles() {
                    if handles.positions.contains_key(id) || !seen.insert(*id) {
                        return Err(RelQueryError::InconsistentIncrementalDelta);
                    }
                }
                let next_base_witness = match base_witness {
                    Some(witness) => witness.advance_with_expected_handles(
                        kernel_types::RevisionId::new(0),
                        resolved.delta(),
                        resolved.removed_handles(),
                        resolved.inserted_handles(),
                        registry,
                    )?,
                    None => None,
                };
                let patch = StorageResolvedScanPatch {
                    removed_handles: resolved.removed_handles().to_vec(),
                    inserted: resolved
                        .inserted_handles()
                        .iter()
                        .copied()
                        .zip(resolved.inserted_keys())
                        .zip(&resolved.delta().inserted)
                        .map(|((id, key), row)| (id, key.clone(), row.clone()))
                        .collect(),
                    next_base_witness,
                };
                if frames
                    .insert(
                        edge,
                        ValidatedTransitionFrame::new(
                            edge,
                            MaintainedScanCommitPatch::StorageResolved(patch),
                            resolved.delta().clone(),
                        ),
                    )
                    .is_some()
                {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
            }
        }
        Ok(frames)
    }

    fn commit_storage_resolved_scan_patch(
        value: &mut PersistentVec<Row>,
        handles: &mut Option<MaintainedLeafHandles>,
        base_witness: &mut Option<crate::RelationBaseWitness>,
        canonical_lookup: &mut CanonicalRowPositionIndex,
        patch: StorageResolvedScanPatch,
    ) {
        let handles = handles
            .as_mut()
            .expect("sealed storage-resolved Scan patch requires bound handles");
        let StorageResolvedScanPatch {
            removed_handles,
            inserted,
            next_base_witness,
        } = patch;
        for id in removed_handles {
            let position = *handles
                .positions
                .get(&id)
                .expect("validated storage-resolved Scan removal must have a position");
            canonical_lookup.remove_position(position);
            handles
                .remove(value, id)
                .expect("validated storage-resolved Scan removal must commit");
        }
        for (id, key, row) in inserted {
            handles
                .insert(value, id, row)
                .expect("validated storage-resolved Scan insertion must commit");
            canonical_lookup.push_key(key);
        }
        *base_witness = next_base_witness;
    }

    fn apply_storage_resolved_deltas_in_place(
        &mut self,
        deltas: &BTreeMap<kernel_types::SemanticId, StorageResolvedRelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        let transition_program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        for (relation, resolved) in deltas {
            if *relation != resolved.relation() || !transition_program.contains_source(*relation) {
                return Err(RelQueryError::UnknownRelation(*relation));
            }
        }
        #[cfg(debug_assertions)]
        let recursive_oracle = self.recursive_oracle_from_frames(
            self.validate_resolved_leaf_frames(deltas, context, registry)?,
            context,
            registry,
        )?;
        let mut validated_frames = self.validate_resolved_leaf_frames(deltas, context, registry)?;
        let next_epoch = self
            .transition_epoch
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        let planned =
            self.plan_relation_deltas_execgraph(&mut validated_frames, context, registry)?;
        let output = materialize_exact_delta_view(&planned.root_effect, self.result_type.clone())?;
        #[cfg(debug_assertions)]
        if !relation_deltas_semantically_equivalent(
            &output,
            &recursive_oracle.1,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        self.commit_graph_patch_set(planned);
        #[cfg(debug_assertions)]
        if !relation_values_semantically_equivalent(
            &self.output_value(context, registry)?,
            &recursive_oracle
                .0
                .output_value_recursive(context, registry)?,
            &self.result_type,
            context,
            registry,
        )? {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        #[cfg(debug_assertions)]
        {
            self.node = recursive_oracle.0.node.as_ref().map(Arc::clone);
        }
        self.transition_epoch = next_epoch;
        Ok(output)
    }

    fn validate_leaf_deltas(
        &self,
        deltas: &BTreeMap<kernel_types::SemanticId, RelationDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<ValidatedLeafTransitionFrames, RelQueryError> {
        let mut frames = BTreeMap::new();
        let program = self
            .differential
            .physical_program()
            .execution_graph()
            .transition_program();
        for (relation, delta) in deltas {
            let occurrences = program
                .source_occurrence_directory(*relation)
                .ok_or(RelQueryError::UnknownRelation(*relation))?;
            for occurrence in occurrences {
                let node = self
                    .arena
                    .get(occurrence.node())
                    .map(Arc::as_ref)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                let FlatMaintainedRelPlanNodeKind::Scan {
                    relation: node_relation,
                    value,
                    canonical_lookup,
                    ..
                } = &node.kind
                else {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                };
                if node_relation != relation {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
                let edge = CompiledDeltaEdgeIdentity::new(occurrence.ordinal(), *relation);
                if delta.result_type != node.result_type {
                    return Err(RelQueryError::TypeMismatch);
                }
                MaterializedSetSupportState::validate_rows(
                    &delta.removed,
                    &node.result_type,
                    context,
                    registry,
                )?;
                MaterializedSetSupportState::validate_rows(
                    &delta.inserted,
                    &node.result_type,
                    context,
                    registry,
                )?;
                let plan = plan_relation_mutation(
                    value,
                    delta,
                    &node.result_type,
                    canonical_lookup,
                    context,
                    registry,
                )?;
                if frames
                    .insert(
                        edge,
                        ValidatedTransitionFrame::new(
                            edge,
                            MaintainedScanCommitPatch::Semantic(plan),
                            delta.clone(),
                        ),
                    )
                    .is_some()
                {
                    return Err(RelQueryError::InconsistentIncrementalDelta);
                }
            }
        }
        Ok(frames)
    }

    fn plan_relation_deltas_execgraph(
        &mut self,
        validated_frames: &mut ValidatedLeafTransitionFrames,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<GraphPatchSet, RelQueryError> {
        let mut scratch = std::mem::take(&mut self.execgraph_scratch);
        let result = self.plan_relation_deltas_execgraph_with_scratch(
            validated_frames,
            context,
            registry,
            &mut scratch,
        );
        if result.is_err() {
            scratch.reset();
        }
        self.execgraph_scratch = scratch;
        result
    }

    fn plan_relation_deltas_execgraph_with_scratch(
        &self,
        validated_frames: &mut ValidatedLeafTransitionFrames,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        scratch: &mut UnifiedTransitionScratch<MaintainedDelta>,
    ) -> Result<GraphPatchSet, RelQueryError> {
        let graph = self.differential.physical_program().execution_graph();
        let program = graph.transition_program();
        if self.arena.len() != graph.node_count() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }

        let mut patches = Vec::new();
        scratch.ensure_nodes(graph.node_count());
        scratch.reset();
        let mut root_effect = MaintainedDelta::default();

        for (edge, frame) in std::mem::take(validated_frames) {
            if frame.edge() != edge {
                scratch.reset();
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let node_id = program
                .source_node_for_occurrence(edge.relation(), edge.ordinal())
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let state = self
                .arena
                .get(node_id)
                .map(Arc::as_ref)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let FlatMaintainedRelPlanNodeKind::Scan { relation, .. } = &state.kind else {
                scratch.reset();
                return Err(RelQueryError::InconsistentIncrementalDelta);
            };
            if *relation != edge.relation() {
                scratch.reset();
                return Err(RelQueryError::InconsistentIncrementalDelta);
            }
            let (patch, delta) = frame.into_parts();
            if delta.result_type != state.result_type {
                scratch.reset();
                return Err(RelQueryError::TypeMismatch);
            }
            patches.push((node_id, GraphNodePatch::Scan(patch)));
            let effect = maintained_delta_from_relation_delta(delta);
            if node_id == program.root() {
                root_effect = effect;
            } else if effect.support_len() != 0
                && let Err(error) = program.deliver_output(node_id, effect, scratch)
            {
                scratch.reset();
                return Err(error);
            }
        }

        while let Some((node_id, inbox)) = scratch.pop_next() {
            let state = self
                .arena
                .get(node_id)
                .map(Arc::as_ref)
                .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
            let planned = match Self::plan_execgraph_node(state, inbox, context, registry) {
                Ok(planned) => planned,
                Err(error) => {
                    scratch.reset();
                    return Err(error);
                }
            };
            if let Some(patch) = planned.patch {
                patches.push((node_id, patch));
            }
            if node_id == program.root() {
                root_effect = planned.effect;
            } else if planned.effect.support_len() != 0
                && let Err(error) = program.deliver_output(node_id, planned.effect, scratch)
            {
                scratch.reset();
                return Err(error);
            }
        }
        scratch.finish_success();
        patches.sort_unstable_by_key(|(node_id, _)| *node_id);
        if patches.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        Ok(GraphPatchSet {
            nodes: patches,
            root_effect,
        })
    }

    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn plan_execgraph_node(
        state: &FlatMaintainedRelPlanNode,
        mut inbox: NodeInbox<MaintainedDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedGraphNodeTransition, RelQueryError> {
        let empty = MaintainedDelta::default;
        match &state.kind {
            FlatMaintainedRelPlanNodeKind::Scan { .. } => {
                Err(RelQueryError::InconsistentIncrementalDelta)
            }
            FlatMaintainedRelPlanNodeKind::Filter {
                input: _,
                column,
                value,
                equivalence,
            } => Ok(PlannedGraphNodeTransition {
                patch: None,
                effect: filter_delta_view(
                    &inbox.take_unary().unwrap_or_else(empty),
                    &state.result_type,
                    *column,
                    value,
                    *equivalence,
                    context,
                    registry,
                )?,
            }),
            FlatMaintainedRelPlanNodeKind::FilterOrder {
                input: _,
                column,
                value,
                ordering,
                comparison,
            } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                Self::plan_execgraph_filter_order(
                    state,
                    &input,
                    OrderFilterSpec {
                        column: *column,
                        value,
                        ordering: *ordering,
                        comparison: *comparison,
                    },
                    context,
                    registry,
                )
            }
            FlatMaintainedRelPlanNodeKind::FilterColumns {
                input: _,
                left_column,
                right_column,
                equivalence,
            } => Ok(PlannedGraphNodeTransition {
                patch: None,
                effect: filter_columns_delta_view(
                    &inbox.take_unary().unwrap_or_else(empty),
                    &state.result_type,
                    *left_column,
                    *right_column,
                    *equivalence,
                    context,
                    registry,
                )?,
            }),
            FlatMaintainedRelPlanNodeKind::ProjectBag { columns, .. } => {
                Ok(PlannedGraphNodeTransition {
                    patch: None,
                    effect: project_bag_delta_view(
                        &inbox.take_unary().unwrap_or_else(empty),
                        columns,
                        &state.result_type,
                        context,
                        registry,
                    )?,
                })
            }
            FlatMaintainedRelPlanNodeKind::ProjectSet {
                columns, supports, ..
            } => Self::plan_execgraph_project_set(
                supports,
                columns,
                &inbox.take_unary().unwrap_or_else(empty),
                context,
                registry,
            ),
            FlatMaintainedRelPlanNodeKind::Distinct { supports, .. } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                let planned = supports.plan_delta_view(&input, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::SetSupport(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::PromoteToBag { .. } => Ok(PlannedGraphNodeTransition {
                patch: None,
                effect: inbox.take_unary().unwrap_or_else(empty),
            }),
            FlatMaintainedRelPlanNodeKind::UnionBag { .. } => {
                let left = inbox.take_left().unwrap_or_else(empty);
                let right = inbox.take_right().unwrap_or_else(empty);
                Ok(PlannedGraphNodeTransition {
                    patch: None,
                    effect: combine_maintained_deltas(&left, &right),
                })
            }
            FlatMaintainedRelPlanNodeKind::UnionSet { supports, .. } => {
                let left = inbox.take_left().unwrap_or_else(empty);
                let right = inbox.take_right().unwrap_or_else(empty);
                let combined = combine_maintained_deltas(&left, &right);
                let planned = supports.plan_delta_view(&combined, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::SetSupport(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::Blocker { .. }
            | FlatMaintainedRelPlanNodeKind::Join { .. }
            | FlatMaintainedRelPlanNodeKind::Group { .. }
            | FlatMaintainedRelPlanNodeKind::TopK { .. } => {
                Self::plan_execgraph_stateful_node(state, inbox, context, registry)
            }
        }
    }

    fn plan_execgraph_filter_order(
        state: &FlatMaintainedRelPlanNode,
        input: &MaintainedDelta,
        spec: OrderFilterSpec<'_>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedGraphNodeTransition, RelQueryError> {
        Ok(PlannedGraphNodeTransition {
            patch: None,
            effect: filter_order_delta_view(input, &state.result_type, spec, context, registry)?,
        })
    }

    fn plan_execgraph_project_set(
        supports: &MaterializedSetSupportState,
        columns: &[usize],
        input: &MaintainedDelta,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedGraphNodeTransition, RelQueryError> {
        let projected = project_delta_view(input, columns)?;
        let planned = supports.plan_delta_view(&projected, context, registry)?;
        Ok(PlannedGraphNodeTransition {
            patch: Some(GraphNodePatch::SetSupport(planned.patch)),
            effect: planned.effect,
        })
    }

    fn plan_execgraph_stateful_node(
        state: &FlatMaintainedRelPlanNode,
        mut inbox: NodeInbox<MaintainedDelta>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<PlannedGraphNodeTransition, RelQueryError> {
        let empty = MaintainedDelta::default;
        match &state.kind {
            FlatMaintainedRelPlanNodeKind::Blocker { state, .. } => {
                let left = inbox.take_left().unwrap_or_else(empty);
                let right = inbox.take_right().unwrap_or_else(empty);
                let planned = state.plan_exact_delta_views(&left, &right, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::Blocker(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::Join { state, .. } => {
                let left = inbox.take_left().unwrap_or_else(empty);
                let right = inbox.take_right().unwrap_or_else(empty);
                let planned = state.plan_exact_delta_views(&left, &right, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::Join(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::Group { state, .. } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                let planned = state.plan_sealed_exact_delta_view(&input, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::Group(planned.patch)),
                    effect: planned.effect,
                })
            }
            FlatMaintainedRelPlanNodeKind::TopK { state, .. } => {
                let input = inbox.take_unary().unwrap_or_else(empty);
                let planned = state.plan_exact_delta_view(&input, context, registry)?;
                Ok(PlannedGraphNodeTransition {
                    patch: Some(GraphNodePatch::TopK(planned.patch)),
                    effect: planned.effect,
                })
            }
            _ => Err(RelQueryError::InconsistentIncrementalDelta),
        }
    }

    fn commit_graph_patch_set(&mut self, patch_set: GraphPatchSet) {
        debug_assert!(patch_set.nodes.len() <= self.arena.len());
        for (node_id, patch) in patch_set.nodes {
            let node = Arc::make_mut(
                self.arena
                    .get_mut(node_id)
                    .expect("execgraph patch NodeId must exist in flat arena"),
            );
            match (&mut node.kind, patch) {
                (
                    FlatMaintainedRelPlanNodeKind::Scan {
                        value,
                        handles,
                        base_witness,
                        canonical_lookup,
                        ..
                    },
                    GraphNodePatch::Scan(patch),
                ) => match patch {
                    MaintainedScanCommitPatch::Semantic(plan) => {
                        commit_relation_mutation(value, canonical_lookup, plan);
                        *base_witness = None;
                    }
                    MaintainedScanCommitPatch::StorageResolved(plan) => {
                        Self::commit_storage_resolved_scan_patch(
                            value,
                            handles,
                            base_witness,
                            canonical_lookup,
                            plan,
                        );
                    }
                },
                (
                    FlatMaintainedRelPlanNodeKind::ProjectSet { supports, .. }
                    | FlatMaintainedRelPlanNodeKind::Distinct { supports, .. }
                    | FlatMaintainedRelPlanNodeKind::UnionSet { supports, .. },
                    GraphNodePatch::SetSupport(patch),
                ) => supports.commit_support_patch(patch),
                (
                    FlatMaintainedRelPlanNodeKind::Blocker { state, .. },
                    GraphNodePatch::Blocker(patch),
                ) => state.commit_patch(patch),
                (
                    FlatMaintainedRelPlanNodeKind::Join { state, .. },
                    GraphNodePatch::Join(patch),
                ) => state.commit_join_patch(patch),
                (
                    FlatMaintainedRelPlanNodeKind::Group { state, .. },
                    GraphNodePatch::Group(patch),
                ) => state.commit_sealed_patch(patch),
                (
                    FlatMaintainedRelPlanNodeKind::TopK { state, .. },
                    GraphNodePatch::TopK(patch),
                ) => state.commit_topk_patch(patch),
                _ => unreachable!("execgraph patch/flat-arena node mismatch"),
            }
        }
    }

    #[cfg(debug_assertions)]
    fn recursive_oracle_from_frames(
        &self,
        mut validated_frames: ValidatedLeafTransitionFrames,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(Self, RelationDelta), RelQueryError> {
        let mut candidate = self.clone();
        let mut plan = RelationDeltaPlanContext {
            validated_frames: &mut validated_frames,
            next_edge_ordinal: 0,
            context,
            registry,
        };
        let planned = candidate.plan_relation_deltas_inner(&mut plan)?;
        if !validated_frames.is_empty() {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let output =
            materialize_exact_delta_view_uncounted(&planned.effect, candidate.result_type.clone())?;
        candidate.commit_relation_plan(planned.patch);
        Ok((candidate, output))
    }

    #[cfg(debug_assertions)]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn plan_relation_deltas_inner(
        &self,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        match self
            .node
            .as_deref()
            .expect("debug recursive oracle tree must exist")
        {
            MaintainedRelPlanNode::Scan { relation, .. } => {
                plan.plan_scan(*relation, &self.result_type)
            }
            MaintainedRelPlanNode::Filter {
                input,
                column,
                value,
                equivalence,
            } => Self::plan_filter_transition(input, *column, value, *equivalence, plan),
            MaintainedRelPlanNode::FilterOrder {
                input,
                column,
                value,
                ordering,
                comparison,
            } => Self::plan_filter_order_transition(
                input,
                *column,
                value,
                *ordering,
                *comparison,
                plan,
            ),
            MaintainedRelPlanNode::FilterColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Self::plan_filter_columns_transition(
                input,
                *left_column,
                *right_column,
                *equivalence,
                plan,
            ),
            MaintainedRelPlanNode::ProjectBag { input, columns } => {
                let child = input.plan_relation_deltas_inner(plan)?;
                let effect = project_bag_delta_view(
                    &child.effect,
                    columns,
                    &self.result_type,
                    plan.context,
                    plan.registry,
                )?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
                    effect,
                })
            }
            MaintainedRelPlanNode::ProjectSet {
                input,
                columns,
                supports,
            } => Self::plan_project_set_transition(input, columns, supports, plan),
            MaintainedRelPlanNode::Distinct { input, supports } => {
                Self::plan_distinct_transition(input, supports, plan)
            }
            MaintainedRelPlanNode::PromoteToBag { input } => {
                let child = input.plan_relation_deltas_inner(plan)?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
                    effect: child.effect,
                })
            }
            MaintainedRelPlanNode::UnionBag { left, right } => {
                let left = left.plan_relation_deltas_inner(plan)?;
                let right = right.plan_relation_deltas_inner(plan)?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::Binary {
                        left: Box::new(left.patch),
                        right: Box::new(right.patch),
                    },
                    effect: combine_maintained_deltas(&left.effect, &right.effect),
                })
            }
            MaintainedRelPlanNode::UnionSet {
                left,
                right,
                supports,
            } => {
                let left = left.plan_relation_deltas_inner(plan)?;
                let right = right.plan_relation_deltas_inner(plan)?;
                let combined = combine_maintained_deltas(&left.effect, &right.effect);
                let planned = supports.plan_delta_view(&combined, plan.context, plan.registry)?;
                Ok(PlannedMaintainedRelPlanTransition {
                    patch: MaintainedRelPlanPatch::SetBinary {
                        left: Box::new(left.patch),
                        right: Box::new(right.patch),
                        patch: planned.patch,
                    },
                    effect: planned.effect,
                })
            }
            MaintainedRelPlanNode::Blocker { left, right, state } => {
                Self::plan_blocker_transition(left, right, state, plan)
            }
            MaintainedRelPlanNode::Join { left, right, state } => {
                Self::plan_join_transition(left, right, state, plan)
            }
            MaintainedRelPlanNode::Group { input, state } => {
                Self::plan_group_transition(input, state, plan)
            }
            MaintainedRelPlanNode::TopK { input, state } => {
                Self::plan_top_k_transition(input, state, plan)
            }
        }
    }

    #[cfg(debug_assertions)]
    fn plan_filter_transition(
        input: &MaterializedRelPlanState,
        column: usize,
        value: &Value,
        equivalence: kernel_types::SemanticId,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let effect = filter_delta_view(
            &child.effect,
            &input.result_type,
            column,
            value,
            equivalence,
            plan.context,
            plan.registry,
        )?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
            effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_filter_order_transition(
        input: &MaterializedRelPlanState,
        column: usize,
        value: &Value,
        ordering: kernel_types::SemanticId,
        comparison: crate::OrderComparison,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let effect = filter_order_delta_view(
            &child.effect,
            &input.result_type,
            OrderFilterSpec {
                column,
                value,
                ordering,
                comparison,
            },
            plan.context,
            plan.registry,
        )?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
            effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_filter_columns_transition(
        input: &MaterializedRelPlanState,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let effect = filter_columns_delta_view(
            &child.effect,
            &input.result_type,
            left_column,
            right_column,
            equivalence,
            plan.context,
            plan.registry,
        )?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Unary(Box::new(child.patch)),
            effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_project_set_transition(
        input: &MaterializedRelPlanState,
        columns: &[usize],
        supports: &MaterializedSetSupportState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let projected = project_delta_view(&child.effect, columns)?;
        let planned = supports.plan_delta_view(&projected, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::SetUnary {
                input: Box::new(child.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_distinct_transition(
        input: &MaterializedRelPlanState,
        supports: &MaterializedSetSupportState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let planned = supports.plan_delta_view(&child.effect, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::SetUnary {
                input: Box::new(child.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_blocker_transition(
        left: &MaterializedRelPlanState,
        right: &MaterializedRelPlanState,
        state: &MaterializedBlockerDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let left = left.plan_relation_deltas_inner(plan)?;
        let right = right.plan_relation_deltas_inner(plan)?;
        let planned = state.plan_exact_delta_views(
            &left.effect,
            &right.effect,
            plan.context,
            plan.registry,
        )?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Blocker {
                left: Box::new(left.patch),
                right: Box::new(right.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_join_transition(
        left: &MaterializedRelPlanState,
        right: &MaterializedRelPlanState,
        state: &MaterializedJoinDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let left = left.plan_relation_deltas_inner(plan)?;
        let right = right.plan_relation_deltas_inner(plan)?;
        let planned = state.plan_exact_delta_views(
            &left.effect,
            &right.effect,
            plan.context,
            plan.registry,
        )?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Join {
                left: Box::new(left.patch),
                right: Box::new(right.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_group_transition(
        input: &MaterializedRelPlanState,
        state: &MaterializedGroupDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let planned =
            state.plan_sealed_exact_delta_view(&child.effect, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Group {
                input: Box::new(child.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn plan_top_k_transition(
        input: &MaterializedRelPlanState,
        state: &MaterializedTopKDeltaState,
        plan: &mut RelationDeltaPlanContext<'_>,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let child = input.plan_relation_deltas_inner(plan)?;
        let planned = state.plan_exact_delta_view(&child.effect, plan.context, plan.registry)?;
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::TopK {
                input: Box::new(child.patch),
                patch: planned.patch,
            },
            effect: planned.effect,
        })
    }

    #[cfg(debug_assertions)]
    fn commit_debug_scan_patch(
        value: &mut PersistentVec<Row>,
        handles: &mut Option<MaintainedLeafHandles>,
        base_witness: &mut Option<crate::RelationBaseWitness>,
        canonical_lookup: &mut CanonicalRowPositionIndex,
        patch: MaintainedScanCommitPatch,
    ) {
        match patch {
            MaintainedScanCommitPatch::Semantic(plan) => {
                commit_relation_mutation(value, canonical_lookup, plan);
                *base_witness = None;
            }
            MaintainedScanCommitPatch::StorageResolved(plan) => {
                Self::commit_storage_resolved_scan_patch(
                    value,
                    handles,
                    base_witness,
                    canonical_lookup,
                    plan,
                );
            }
        }
    }

    #[cfg(debug_assertions)]
    #[allow(
        clippy::too_many_lines,
        reason = "Keep the complete operator or protocol case analysis together."
    )]
    fn commit_relation_plan(&mut self, patch: MaintainedRelPlanPatch) {
        match (
            Arc::make_mut(
                self.node
                    .as_mut()
                    .expect("debug recursive oracle tree must exist"),
            ),
            patch,
        ) {
            (
                MaintainedRelPlanNode::Scan {
                    value,
                    handles,
                    base_witness,
                    canonical_lookup,
                    ..
                },
                MaintainedRelPlanPatch::Scan(Some(plan)),
            ) => {
                Self::commit_debug_scan_patch(value, handles, base_witness, canonical_lookup, plan);
            }
            (MaintainedRelPlanNode::Scan { .. }, MaintainedRelPlanPatch::Scan(None)) => {}
            (
                MaintainedRelPlanNode::Filter { input, .. }
                | MaintainedRelPlanNode::FilterOrder { input, .. }
                | MaintainedRelPlanNode::FilterColumns { input, .. }
                | MaintainedRelPlanNode::ProjectBag { input, .. }
                | MaintainedRelPlanNode::PromoteToBag { input },
                MaintainedRelPlanPatch::Unary(child),
            ) => input.commit_relation_plan(*child),
            (
                MaintainedRelPlanNode::ProjectSet {
                    input, supports, ..
                }
                | MaintainedRelPlanNode::Distinct { input, supports },
                MaintainedRelPlanPatch::SetUnary {
                    input: child,
                    patch,
                },
            ) => {
                supports.commit_support_patch(patch);
                input.commit_relation_plan(*child);
            }
            (
                MaintainedRelPlanNode::UnionBag { left, right },
                MaintainedRelPlanPatch::Binary {
                    left: left_patch,
                    right: right_patch,
                },
            ) => {
                left.commit_relation_plan(*left_patch);
                right.commit_relation_plan(*right_patch);
            }
            (
                MaintainedRelPlanNode::UnionSet {
                    left,
                    right,
                    supports,
                },
                MaintainedRelPlanPatch::SetBinary {
                    left: left_patch,
                    right: right_patch,
                    patch,
                },
            ) => {
                supports.commit_support_patch(patch);
                left.commit_relation_plan(*left_patch);
                right.commit_relation_plan(*right_patch);
            }
            (
                MaintainedRelPlanNode::Blocker { left, right, state },
                MaintainedRelPlanPatch::Blocker {
                    left: left_patch,
                    right: right_patch,
                    patch,
                },
            ) => {
                state.commit_patch(patch);
                left.commit_relation_plan(*left_patch);
                right.commit_relation_plan(*right_patch);
            }
            (
                MaintainedRelPlanNode::Join { left, right, state },
                MaintainedRelPlanPatch::Join {
                    left: left_patch,
                    right: right_patch,
                    patch,
                },
            ) => {
                state.commit_join_patch(patch);
                left.commit_relation_plan(*left_patch);
                right.commit_relation_plan(*right_patch);
            }
            (
                MaintainedRelPlanNode::Group { input, state },
                MaintainedRelPlanPatch::Group {
                    input: child,
                    patch,
                },
            ) => {
                state.commit_sealed_patch(patch);
                input.commit_relation_plan(*child);
            }
            (
                MaintainedRelPlanNode::TopK { input, state },
                MaintainedRelPlanPatch::TopK {
                    input: child,
                    patch,
                },
            ) => {
                state.commit_topk_patch(patch);
                input.commit_relation_plan(*child);
            }
            _ => unreachable!("maintained plan patch/node mismatch"),
        }
    }
}

type ValidatedLeafTransitionFrames = BTreeMap<
    CompiledDeltaEdgeIdentity,
    ValidatedTransitionFrame<MaintainedScanCommitPatch, RelationDelta>,
>;

#[cfg(debug_assertions)]
struct RelationDeltaPlanContext<'a> {
    validated_frames: &'a mut ValidatedLeafTransitionFrames,
    next_edge_ordinal: u32,
    context: &'a kernel_schema::SemanticContext,
    registry: &'a kernel_semantics::SemanticRegistry,
}

#[cfg(debug_assertions)]
impl RelationDeltaPlanContext<'_> {
    fn next_edge(
        &mut self,
        relation: kernel_types::SemanticId,
    ) -> Result<CompiledDeltaEdgeIdentity, RelQueryError> {
        let edge = CompiledDeltaEdgeIdentity::new(self.next_edge_ordinal, relation);
        self.next_edge_ordinal = self
            .next_edge_ordinal
            .checked_add(1)
            .ok_or(RelQueryError::TransitionEpochExhausted)?;
        Ok(edge)
    }

    fn plan_scan(
        &mut self,
        relation: kernel_types::SemanticId,
        result_type: &RelType,
    ) -> Result<PlannedMaintainedRelPlanTransition, RelQueryError> {
        let edge = self.next_edge(relation)?;
        let Some(frame) = self.validated_frames.remove(&edge) else {
            return Ok(PlannedMaintainedRelPlanTransition {
                patch: MaintainedRelPlanPatch::Scan(None),
                effect: MaintainedDelta::default(),
            });
        };
        if frame.edge() != edge {
            return Err(RelQueryError::InconsistentIncrementalDelta);
        }
        let (patch, delta) = frame.into_parts();
        if delta.result_type != *result_type {
            return Err(RelQueryError::TypeMismatch);
        }
        Ok(PlannedMaintainedRelPlanTransition {
            patch: MaintainedRelPlanPatch::Scan(Some(patch)),
            effect: maintained_delta_from_relation_delta(delta),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rel_model::AggregateSpec;
    use kernel_model::FiniteModel;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

    fn setup() -> (
        SemanticContext,
        SemanticRegistry,
        SemanticId,
        SemanticId,
        SemanticId,
    ) {
        let text_eq = SemanticId::new(100);
        let i64_eq = SemanticId::new(101);
        let left = SemanticId::new(200);
        let right = SemanticId::new(201);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::Text),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![text_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        (context, registry, text_eq, left, right)
    }

    #[test]
    fn maintained_plan_clone_uses_cow_and_isolates_mutation() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Scan(relation);
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let mut candidate = state.clone();
        assert!(state.test_arena_shares_storage_with(&candidate));

        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Gamma".into()), Value::I64(3)]],
            removed: vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
            result_type: query.typecheck(&context, &registry).unwrap(),
        };
        candidate
            .apply_relation_deltas(&BTreeMap::from([(relation, delta)]), &context, &registry)
            .unwrap();

        assert!(!state.test_arena_shares_storage_with(&candidate));
        assert_eq!(
            state.output_value(&context, &registry).unwrap().rows(),
            &[
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ]
        );
        assert_eq!(
            candidate.output_value(&context, &registry).unwrap().rows(),
            &[
                vec![Value::Text("Beta".into()), Value::I64(2)],
                vec![Value::Text("Gamma".into()), Value::I64(3)],
            ]
        );
    }

    #[test]
    fn maintained_plan_arena_path_copy_preserves_untouched_node_arcs() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("ALPHA".into()),
            equivalence: text_eq,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        assert_eq!(state.test_arena_len(), 2);
        let mut candidate = state.clone();
        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Gamma".into()), Value::I64(3)]],
            removed: Vec::new(),
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        candidate
            .apply_relation_deltas(&BTreeMap::from([(relation, delta)]), &context, &registry)
            .unwrap();

        assert!(!state.test_arena_shares_storage_with(&candidate));
        assert_eq!(state.test_node_arc_shared_with(&candidate, 0), Some(false));
        assert_eq!(
            state.test_node_arc_shared_with(&candidate, 1),
            Some(true),
            "immutable filter node must stay physically shared after scan-only commit"
        );
    }

    #[test]
    fn execgraph_patch_transport_is_sparse_across_large_unaffected_branch() {
        fn balanced_difference(relation: SemanticId, depth: usize) -> RelExpr {
            if depth == 0 {
                return RelExpr::Scan(relation);
            }
            RelExpr::Difference {
                left: Box::new(balanced_difference(relation, depth - 1)),
                right: Box::new(balanced_difference(relation, depth - 1)),
            }
        }

        let (context, registry, _, left, right) = setup();
        let query = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(balanced_difference(right, 5)),
        };
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(left, vec![vec![Value::Text("Alpha".into()), Value::I64(1)]]);
        model.relations.insert(right, Vec::new());
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let deltas = BTreeMap::from([(
            left,
            RelationDelta {
                inserted: vec![vec![Value::Text("Beta".into()), Value::I64(2)]],
                removed: Vec::new(),
                result_type: RelExpr::Scan(left).typecheck(&context, &registry).unwrap(),
            },
        )]);

        let (patch_count, patch_nodes, graph_nodes) = state
            .test_execgraph_sparse_patch_shape(&deltas, &context, &registry)
            .unwrap();
        assert_eq!(graph_nodes, 65);
        assert_eq!(patch_count, 2);
        assert_eq!(patch_nodes.first().copied(), Some(0));
        assert_eq!(patch_nodes.last().copied(), Some(64));
    }

    #[test]
    fn scan_payload_path_copies_rows_under_reader_snapshot() {
        let (context, registry, _, relation, _) = setup();
        let rows = (0_i64..1_024)
            .map(|value| vec![Value::Text(format!("k{value}")), Value::I64(value)])
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(relation, rows);
        let query = RelExpr::Scan(relation);
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let mut next = state.clone();

        assert_eq!(state.test_scan_value_sharing_with(&next, 0), Some(true));

        next.apply_relation_deltas(
            &BTreeMap::from([(
                relation,
                RelationDelta {
                    inserted: vec![vec![Value::Text("replacement".into()), Value::I64(2_000)]],
                    removed: vec![vec![Value::Text("k0".into()), Value::I64(0)]],
                    result_type: query.typecheck(&context, &registry).unwrap(),
                },
            )]),
            &context,
            &registry,
        )
        .unwrap();

        assert_eq!(state.test_scan_value_sharing_with(&next, 0), Some(false));
        assert_eq!(
            state.test_scan_value_page_sharing_with(&next, 0, 512),
            Some(true),
            "untouched middle scan page must remain physically shared"
        );
        assert_eq!(state.test_scan_value_len(0), Some(1_024));
        assert_eq!(next.test_scan_value_len(0), Some(1_024));
        assert_eq!(
            state.output_value(&context, &registry).unwrap().rows()[0][1],
            Value::I64(0)
        );
        assert!(
            next.output_value(&context, &registry)
                .unwrap()
                .rows()
                .iter()
                .any(|row| row[1] == Value::I64(2_000))
        );
    }

    #[test]
    fn set_support_payload_path_copies_under_reader_snapshot() {
        let (context, registry, text_eq, left, _) = setup();
        let i64_eq = SemanticId::new(101);
        let rows = (0_i64..1_024)
            .map(|value| vec![Value::Text(format!("k{value}")), Value::I64(value)])
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(left, rows);
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(left)),
            column_equivalences: vec![text_eq, i64_eq],
        };
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let mut next = state.clone();
        assert_eq!(
            state.test_distinct_support_sharing_with(&next),
            Some((true, true))
        );
        next.apply_relation_deltas(
            &BTreeMap::from([(
                left,
                RelationDelta {
                    inserted: vec![vec![Value::Text("new".into()), Value::I64(9_999)]],
                    removed: Vec::new(),
                    result_type: RelExpr::Scan(left).typecheck(&context, &registry).unwrap(),
                },
            )]),
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(
            state.test_distinct_support_sharing_with(&next),
            Some((false, false))
        );
    }

    #[test]
    fn blocker_payload_path_copies_under_reader_snapshot() {
        let (context, registry, _, left, right) = setup();
        let rows = (0_i64..1_024)
            .map(|value| vec![Value::Text(format!("k{value}")), Value::I64(value)])
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(left, rows);
        model.relations.insert(right, Vec::new());
        let query = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        };
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let mut next = state.clone();
        assert_eq!(state.test_blocker_storage_sharing_with(&next), Some(true));
        next.apply_relation_deltas(
            &BTreeMap::from([(
                left,
                RelationDelta {
                    inserted: vec![vec![Value::Text("blocker-new".into()), Value::I64(10_000)]],
                    removed: Vec::new(),
                    result_type: RelExpr::Scan(left).typecheck(&context, &registry).unwrap(),
                },
            )]),
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(state.test_blocker_storage_sharing_with(&next), Some(false));
    }

    #[test]
    fn generic_group_payload_path_copies_under_reader_snapshot() {
        let (context, registry, text_eq, left, _) = setup();
        let i64_eq = SemanticId::new(101);
        let rows = (0_i64..1_024)
            .map(|value| vec![Value::Text(format!("k{value}")), Value::I64(value)])
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(left, rows);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(left)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let mut next = state.clone();
        assert_eq!(
            state.test_group_storage_sharing_with(&next),
            Some((true, true))
        );
        next.apply_relation_deltas(
            &BTreeMap::from([(
                left,
                RelationDelta {
                    inserted: vec![vec![Value::Text("group-new".into()), Value::I64(10_001)]],
                    removed: Vec::new(),
                    result_type: RelExpr::Scan(left).typecheck(&context, &registry).unwrap(),
                },
            )]),
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(
            state.test_group_storage_sharing_with(&next),
            Some((false, false))
        );
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn release_maintained_plan_is_direct_flat_arena() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
        );
        let state = MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let (arena_len, has_typed_metadata) = state.test_compiled_graph_shape();
        assert_eq!(state.test_arena_len(), arena_len);
        assert!(has_typed_metadata);
    }

    #[test]
    fn attach_storage_rows_validates_before_cow_commit_and_is_failure_atomic() {
        let (context, registry, text_eq, relation, _) = setup();
        let i64_eq = SemanticId::new(101);
        let alpha = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let beta = vec![Value::Text("Beta".into()), Value::I64(2)];
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![alpha.clone(), beta.clone()]);
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![text_eq, i64_eq],
        };
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let before = state.clone();
        let epoch = state.transition_epoch();
        let handles = [
            kernel_types::StableRowHandle {
                slot: 0,
                generation: 0,
            },
            kernel_types::StableRowHandle {
                slot: 1,
                generation: 0,
            },
        ];

        assert_eq!(
            state.attach_storage_rows(
                relation,
                &[(handles[0], beta.clone()), (handles[1], alpha.clone())],
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
        assert_eq!(state.transition_epoch(), epoch);
        assert!(state.test_arena_shares_storage_with(&before));
        assert_eq!(
            state.attach_storage_rows(SemanticId::new(999_999), &[]),
            Err(RelQueryError::UnknownRelation(SemanticId::new(999_999)))
        );
        assert_eq!(state.transition_epoch(), epoch);

        state
            .attach_storage_rows(relation, &[(handles[0], alpha), (handles[1], beta)])
            .unwrap();
        assert_eq!(state.transition_epoch(), epoch + 1);
        assert!(!state.test_arena_shares_storage_with(&before));
        assert_eq!(
            before.output_value(&context, &registry).unwrap(),
            state.output_value(&context, &registry).unwrap()
        );
    }
}
