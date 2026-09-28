use super::{
    BTreeMap, BTreeSet, BlockerBuildSpec, Change, CompiledDeltaProgram, Impact,
    MaintainedBlockerKind, MaterializedBlockerDeltaState, MaterializedGroupDeltaState,
    MaterializedJoinDeltaState, MaterializedTopKDeltaState, NodeId, PreparedRelGraph, RelExpr,
    RelQueryError, RelType, RelationDelta, Value, canonical_row_multiset_counts,
    collect_rel_source_relations, materialize_exact_quotient_delta_view, rel_delta_distinct,
    rel_delta_filter, rel_delta_filter_columns, rel_delta_project_bag, rel_delta_project_set,
    rel_delta_scan, rel_impact_by_recompute, relation_column_equivalences,
};

/// Exact differential class of a relational operator.
///
/// This is a semantic maintenance classification, not a physical-state
/// prescription. Physical implementations may lower the same class to SAMF
/// fibers/annotations, specialized native state, or a recomputation oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelDifferentialClass {
    Source,
    Linear,
    ZeroCrossing,
    BilinearPullback,
    Annotation,
    OrderedBoundary,
    BlockerZeroCrossing,
}

/// Reconstructible state capability required by an exact differential node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelDifferentialStateRequirement {
    SetSupport,
    JoinFibers,
    GroupAnnotations,
    OrderedCut,
    BlockerMass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RelDifferentialNode {
    Scan {
        relation: kernel_types::SemanticId,
    },
    FilterEqConst {
        input: Box<Self>,
        column: usize,
        value: Value,
        equivalence: kernel_types::SemanticId,
    },
    FilterEqColumns {
        input: Box<Self>,
        left_column: usize,
        right_column: usize,
        equivalence: kernel_types::SemanticId,
    },
    Project {
        input: Box<Self>,
        input_expr: RelExpr,
        columns: Vec<usize>,
        result_expr: RelExpr,
        set_semantics: bool,
    },
    JoinEq {
        left: Box<Self>,
        right: Box<Self>,
        left_expr: RelExpr,
        right_expr: RelExpr,
        result_expr: RelExpr,
    },
    Difference {
        left: Box<Self>,
        right: Box<Self>,
        result_expr: RelExpr,
    },
    AntiJoin {
        left: Box<Self>,
        right: Box<Self>,
        result_expr: RelExpr,
    },
    Distinct {
        input: Box<Self>,
        input_expr: RelExpr,
        result_expr: RelExpr,
    },
    Group {
        input: Box<Self>,
        input_expr: RelExpr,
        result_expr: RelExpr,
    },
    TopKWithTies {
        input: Box<Self>,
        input_expr: RelExpr,
        result_expr: RelExpr,
    },
    PromoteToBag {
        input: Box<Self>,
        result_type: RelType,
    },
}

impl RelDifferentialNode {
    #[allow(clippy::too_many_lines)]
    fn compile(
        expr: &RelExpr,
        node: NodeId,
        graph: &PreparedRelGraph,
    ) -> Result<Self, RelQueryError> {
        match expr {
            RelExpr::Scan(relation) => Ok(Self::Scan {
                relation: *relation,
            }),
            RelExpr::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => Ok(Self::FilterEqConst {
                input: Box::new(Self::compile(
                    input,
                    Self::unary_child(node, graph)?,
                    graph,
                )?),
                column: *column,
                value: value.clone(),
                equivalence: *equivalence,
            }),
            RelExpr::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Ok(Self::FilterEqColumns {
                input: Box::new(Self::compile(
                    input,
                    Self::unary_child(node, graph)?,
                    graph,
                )?),
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            }),
            RelExpr::Project { input, columns } => {
                let input_id = Self::unary_child(node, graph)?;
                let input_type = graph
                    .result_type(input_id)
                    .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
                Ok(Self::Project {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    columns: columns.clone(),
                    result_expr: expr.clone(),
                    set_semantics: matches!(
                        input_type.semantics,
                        kernel_schema::RelationSemantics::Set { .. }
                    ),
                })
            }
            RelExpr::JoinEq { left, right, .. } => {
                let (left_id, right_id) = Self::binary_children(node, graph)?;
                Ok(Self::JoinEq {
                    left: Box::new(Self::compile(left, left_id, graph)?),
                    right: Box::new(Self::compile(right, right_id, graph)?),
                    left_expr: left.as_ref().clone(),
                    right_expr: right.as_ref().clone(),
                    result_expr: expr.clone(),
                })
            }
            RelExpr::Difference { left, right } => {
                Self::compile_blocker(left, right, expr, node, graph, false)
            }
            RelExpr::AntiJoin { left, right, .. } => {
                Self::compile_blocker(left, right, expr, node, graph, true)
            }
            RelExpr::Distinct { input, .. } => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::Distinct {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    result_expr: expr.clone(),
                })
            }
            RelExpr::Group { input, .. } => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::Group {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    result_expr: expr.clone(),
                })
            }
            RelExpr::TopKWithTies { input, .. } => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::TopKWithTies {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    input_expr: input.as_ref().clone(),
                    result_expr: expr.clone(),
                })
            }
            RelExpr::PromoteToBag(input) => {
                let input_id = Self::unary_child(node, graph)?;
                Ok(Self::PromoteToBag {
                    input: Box::new(Self::compile(input, input_id, graph)?),
                    result_type: graph
                        .result_type(node)
                        .cloned()
                        .ok_or(RelQueryError::InconsistentIncrementalDelta)?,
                })
            }
        }
    }

    fn unary_child(node: NodeId, graph: &PreparedRelGraph) -> Result<NodeId, RelQueryError> {
        graph
            .unary_input(node)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    }

    fn binary_children(
        node: NodeId,
        graph: &PreparedRelGraph,
    ) -> Result<(NodeId, NodeId), RelQueryError> {
        graph
            .binary_inputs(node)
            .ok_or(RelQueryError::InconsistentIncrementalDelta)
    }

    fn compile_blocker(
        left: &RelExpr,
        right: &RelExpr,
        result_expr: &RelExpr,
        node: NodeId,
        graph: &PreparedRelGraph,
        anti_join: bool,
    ) -> Result<Self, RelQueryError> {
        let (left_id, right_id) = Self::binary_children(node, graph)?;
        let left = Box::new(Self::compile(left, left_id, graph)?);
        let right = Box::new(Self::compile(right, right_id, graph)?);
        Ok(if anti_join {
            Self::AntiJoin {
                left,
                right,
                result_expr: result_expr.clone(),
            }
        } else {
            Self::Difference {
                left,
                right,
                result_expr: result_expr.clone(),
            }
        })
    }

    const fn class(&self) -> RelDifferentialClass {
        match self {
            Self::Scan { .. } => RelDifferentialClass::Source,
            Self::FilterEqConst { .. }
            | Self::FilterEqColumns { .. }
            | Self::PromoteToBag { .. }
            | Self::Project {
                set_semantics: false,
                ..
            } => RelDifferentialClass::Linear,
            Self::Project {
                set_semantics: true,
                ..
            }
            | Self::Distinct { .. } => RelDifferentialClass::ZeroCrossing,
            Self::JoinEq { .. } => RelDifferentialClass::BilinearPullback,
            Self::Difference { .. } | Self::AntiJoin { .. } => {
                RelDifferentialClass::BlockerZeroCrossing
            }
            Self::Group { .. } => RelDifferentialClass::Annotation,
            Self::TopKWithTies { .. } => RelDifferentialClass::OrderedBoundary,
        }
    }

    fn collect_state_requirements(&self, out: &mut BTreeSet<RelDifferentialStateRequirement>) {
        match self {
            Self::Project {
                input,
                set_semantics,
                ..
            } => {
                input.collect_state_requirements(out);
                if *set_semantics {
                    out.insert(RelDifferentialStateRequirement::SetSupport);
                }
            }
            Self::Distinct { input, .. } => {
                input.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::SetSupport);
            }
            Self::JoinEq { left, right, .. } => {
                left.collect_state_requirements(out);
                right.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::JoinFibers);
            }
            Self::Difference { left, right, .. } | Self::AntiJoin { left, right, .. } => {
                left.collect_state_requirements(out);
                right.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::BlockerMass);
            }
            Self::Group { input, .. } => {
                input.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::GroupAnnotations);
            }
            Self::TopKWithTies { input, .. } => {
                input.collect_state_requirements(out);
                out.insert(RelDifferentialStateRequirement::OrderedCut);
            }
            Self::FilterEqConst { input, .. }
            | Self::FilterEqColumns { input, .. }
            | Self::PromoteToBag { input, .. } => input.collect_state_requirements(out),
            Self::Scan { .. } => {}
        }
    }

    fn apply(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        match self {
            Self::Scan { relation } => rel_delta_scan(*relation, old, change, context, registry),
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => rel_delta_filter(
                input.apply(old, change, context, registry)?,
                *column,
                value,
                *equivalence,
                context,
                registry,
            ),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => rel_delta_filter_columns(
                input.apply(old, change, context, registry)?,
                *left_column,
                *right_column,
                *equivalence,
                context,
                registry,
            ),
            Self::Project { .. } => self.apply_project(old, change, context, registry),
            Self::JoinEq { .. } => self.apply_join(old, change, context, registry),
            Self::Difference { .. } | Self::AntiJoin { .. } => {
                self.apply_blocker(old, change, context, registry)
            }
            Self::Distinct {
                input,
                input_expr,
                result_expr,
            } => rel_delta_distinct(
                input_expr,
                input.apply(old, change, context, registry)?,
                result_expr,
                old,
                context,
                registry,
            ),
            Self::Group { .. } => self.apply_group(old, change, context, registry),
            Self::TopKWithTies { .. } => self.apply_top_k(old, change, context, registry),
            Self::PromoteToBag { input, result_type } => {
                let input_delta = input.apply(old, change, context, registry)?;
                Ok(RelationDelta {
                    inserted: input_delta.inserted,
                    removed: input_delta.removed,
                    result_type: result_type.clone(),
                })
            }
        }
    }

    fn apply_project(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::Project {
            input,
            input_expr,
            columns,
            result_expr,
            set_semantics,
        } = self
        else {
            unreachable!("apply_project is only called for project nodes");
        };
        let input_delta = input.apply(old, change, context, registry)?;
        if *set_semantics {
            rel_delta_project_set(
                input_expr,
                input_delta,
                columns,
                result_expr,
                old,
                context,
                registry,
            )
        } else {
            rel_delta_project_bag(input_delta, columns, result_expr, context, registry)
        }
    }

    fn apply_blocker(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let (Self::Difference {
            left,
            right,
            result_expr,
        }
        | Self::AntiJoin {
            left,
            right,
            result_expr,
        }) = self
        else {
            unreachable!("apply_blocker is only called for blocker nodes");
        };
        let left_delta = left.apply(old, change, context, registry)?;
        let right_delta = right.apply(old, change, context, registry)?;
        let (left_expr, right_expr, kind) = match result_expr {
            RelExpr::Difference { left, right } => (
                left.as_ref(),
                right.as_ref(),
                MaintainedBlockerKind::Difference,
            ),
            RelExpr::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => (
                left.as_ref(),
                right.as_ref(),
                MaintainedBlockerKind::AntiJoin {
                    left_column: *left_column,
                    right_column: *right_column,
                    equivalence: *equivalence,
                },
            ),
            _ => unreachable!("blocker differential node/query mismatch"),
        };
        let left_value = left_expr.evaluate(old, context, registry)?;
        let right_value = right_expr.evaluate(old, context, registry)?;
        let state = MaterializedBlockerDeltaState::build(
            &left_value,
            &right_value,
            BlockerBuildSpec {
                kind: &kind,
                left_type: left_expr.typecheck(context, registry)?,
                right_type: right_expr.typecheck(context, registry)?,
                result_type: result_expr.typecheck(context, registry)?,
                context,
                registry,
            },
        )?;
        let planned = state.plan_exact_delta_views(
            &left_delta.as_delta_view(),
            &right_delta.as_delta_view(),
            context,
            registry,
        )?;
        materialize_exact_quotient_delta_view(
            &planned.effect,
            state.result_type().clone(),
            context,
            registry,
        )
    }

    fn apply_join(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::JoinEq {
            left,
            right,
            left_expr,
            right_expr,
            result_expr,
        } = self
        else {
            unreachable!("apply_join is only called for join nodes");
        };
        let left_delta = left.apply(old, change, context, registry)?;
        let right_delta = right.apply(old, change, context, registry)?;
        let left_value = left_expr.evaluate(old, context, registry)?;
        let right_value = right_expr.evaluate(old, context, registry)?;
        let state = MaterializedJoinDeltaState::build_from_input_values(
            result_expr,
            &left_value,
            &right_value,
            context,
            registry,
        )?
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let planned = state.plan_exact_delta_views(
            &left_delta.as_delta_view(),
            &right_delta.as_delta_view(),
            context,
            registry,
        )?;
        materialize_exact_quotient_delta_view(
            &planned.effect,
            state.result_type().clone(),
            context,
            registry,
        )
    }

    fn apply_group(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::Group {
            input,
            input_expr,
            result_expr,
        } = self
        else {
            unreachable!("apply_group is only called for group nodes");
        };
        let input_delta = input.apply(old, change, context, registry)?;
        let input_value = input_expr.evaluate(old, context, registry)?;
        let state = MaterializedGroupDeltaState::build_from_input_value(
            result_expr,
            input_value,
            context,
            registry,
        )?
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let effect = state.plan_exact_effect(&input_delta.as_delta_view(), context, registry)?;
        materialize_exact_quotient_delta_view(
            &effect,
            state.result_type().clone(),
            context,
            registry,
        )
    }

    fn apply_top_k(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        let Self::TopKWithTies {
            input,
            input_expr,
            result_expr,
        } = self
        else {
            unreachable!("apply_top_k is only called for TopK nodes");
        };
        let input_delta = input.apply(old, change, context, registry)?;
        let input_value = input_expr.evaluate(old, context, registry)?;
        let state = MaterializedTopKDeltaState::build_from_input_value(
            result_expr,
            input_value,
            context,
            registry,
        )?
        .ok_or(RelQueryError::InconsistentIncrementalDelta)?;
        let planned =
            state.plan_exact_delta_view(&input_delta.as_delta_view(), context, registry)?;
        materialize_exact_quotient_delta_view(
            &planned.effect,
            state.result_type().clone(),
            context,
            registry,
        )
    }
}

/// Pinned exact differential program compiled from one relational expression.
///
/// The program is reconstructible from `RelExpr + Γ`; it is never semantic
/// authority. It provides the stable production boundary for Γ-DTC while the
/// lower-level physical kernels migrate to shared SAMF overlays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelDifferentialProgram {
    semantic_context: kernel_schema::SemanticContext,
    root: RelDifferentialNode,
    physical: CompiledDeltaProgram,
}

impl RelDifferentialProgram {
    pub fn compile(
        query: &RelExpr,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let physical = CompiledDeltaProgram::compile(query, context, registry)?;
        let root = RelDifferentialNode::compile(
            query,
            physical.execution_graph().root(),
            physical.execution_graph(),
        )?;
        Ok(Self {
            semantic_context: context.clone(),
            root,
            physical,
        })
    }

    #[must_use]
    pub const fn root_class(&self) -> RelDifferentialClass {
        self.root.class()
    }

    #[must_use]
    pub fn state_requirements(&self) -> BTreeSet<RelDifferentialStateRequirement> {
        let mut requirements = BTreeSet::new();
        self.root.collect_state_requirements(&mut requirements);
        requirements
    }

    #[must_use]
    pub const fn physical_program(&self) -> &CompiledDeltaProgram {
        &self.physical
    }

    pub fn apply(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationDelta, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        self.root.apply(old, change, context, registry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelObservationKey {
    rows: BTreeMap<Vec<kernel_semantics::CanonicalEqKey>, usize>,
}

impl RelObservationKey {
    #[must_use]
    pub fn distinct_row_classes(&self) -> usize {
        self.rows.len()
    }

    #[must_use]
    pub fn row_multiplicity(&self) -> usize {
        self.rows.values().sum()
    }
}

/// Exact observation-fiber guard for one pinned relational observation.
///
/// The guard is derived/reconstructible state. Its normalized output key names
/// the current observation fiber, while the differential program provides the
/// exact impact test for candidate model deltas. `source_relations` is only a
/// sound routing envelope; it never replaces exact differential validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelObservationGuard {
    semantic_context: kernel_schema::SemanticContext,
    query: RelExpr,
    observed: RelObservationKey,
    source_relations: BTreeSet<kernel_types::SemanticId>,
    differential: RelDifferentialProgram,
}

impl RelObservationGuard {
    pub fn observe(
        query: &RelExpr,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, RelQueryError> {
        let prepared = query.prepare(context, registry)?;
        let value = prepared.evaluate(model, context, registry)?;
        let column_equivalences = relation_column_equivalences(prepared.result_type());
        let rows =
            canonical_row_multiset_counts(value.rows(), column_equivalences, context, registry)?;
        let mut source_relations = BTreeSet::new();
        collect_rel_source_relations(query, &mut source_relations);
        Ok(Self {
            semantic_context: context.clone(),
            query: query.clone(),
            observed: RelObservationKey { rows },
            source_relations,
            differential: RelDifferentialProgram::compile(query, context, registry)?,
        })
    }

    #[must_use]
    pub const fn observed_key(&self) -> &RelObservationKey {
        &self.observed
    }

    #[must_use]
    pub fn source_relations(&self) -> &BTreeSet<kernel_types::SemanticId> {
        &self.source_relations
    }

    #[must_use]
    pub const fn query(&self) -> &RelExpr {
        &self.query
    }

    #[must_use]
    pub const fn differential(&self) -> &RelDifferentialProgram {
        &self.differential
    }

    pub fn impact(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Impact, RelQueryError> {
        if context != &self.semantic_context {
            return Err(RelQueryError::SemanticRevisionMismatch);
        }
        if matches!(change, Change::NoChange) {
            return Ok(Impact::Unaffected);
        }
        let delta = self.differential.apply(old, change, context, registry)?;
        Ok(if delta.is_empty() {
            Impact::Unaffected
        } else {
            Impact::Changed
        })
    }

    /// Exact transition impact between two complete finite models under the
    /// pinned semantic context of this observation.
    ///
    /// This convenience boundary keeps callers outside `kernel-query` from
    /// depending on the current coarse `Change<FiniteModel>` representation.
    /// Γ-DTC remains the production impact engine; `impact_by_recompute_oracle`
    /// remains an independent parity oracle during rollout.
    pub fn impact_between(
        &self,
        old: &kernel_model::FiniteModel,
        new: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Impact, RelQueryError> {
        self.impact(old, &Change::Replace(new.clone()), context, registry)
    }

    /// Independent full-recompute transition oracle corresponding to
    /// `impact_between`.
    #[must_use]
    pub fn impact_between_by_recompute_oracle(
        &self,
        old: &kernel_model::FiniteModel,
        new: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Impact {
        self.impact_by_recompute_oracle(old, &Change::Replace(new.clone()), context, registry)
    }

    /// Exact parity oracle retained during OFC rollout.
    #[must_use]
    pub fn impact_by_recompute_oracle(
        &self,
        old: &kernel_model::FiniteModel,
        change: &Change<kernel_model::FiniteModel>,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Impact {
        rel_impact_by_recompute(&self.query, old, change, context, registry)
    }
}
