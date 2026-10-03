impl Plan {
    #[must_use]
    pub fn lower_baseline(expr: &RelExpr) -> Self {
        Self::lower_with_catalog(expr, &PhysicalCatalog::default())
    }

    #[must_use]
    #[allow(clippy::too_many_lines, reason = "Keep the complete operator or protocol case analysis together.")]
    pub fn lower_with_catalog(expr: &RelExpr, catalog: &PhysicalCatalog) -> Self {
        match expr {
            RelExpr::Scan(relation) => Self::Scan {
                relation: *relation,
                layout: catalog.relation_layout(*relation),
            },
            RelExpr::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => Self::lower_eq_filter(input, *column, value, *equivalence, catalog),
            RelExpr::FilterOrderConst { .. } => Self::lower_order_filter(expr, catalog),
            RelExpr::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => Self::lower_column_filter(
                input,
                *left_column,
                *right_column,
                *equivalence,
                catalog,
            ),
            RelExpr::Project { input, columns } => Self::Project {
                input: Box::new(Self::lower_with_catalog(input, catalog)),
                columns: columns.clone(),
            },
            RelExpr::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::lower_join(
                left,
                right,
                *left_column,
                *right_column,
                *equivalence,
                catalog,
            ),
            RelExpr::Difference { left, right } => Self::Difference {
                left: Box::new(Self::lower_with_catalog(left, catalog)),
                right: Box::new(Self::lower_with_catalog(right, catalog)),
            },
            RelExpr::Union { left, right } => Self::Union {
                left: Box::new(Self::lower_with_catalog(left, catalog)),
                right: Box::new(Self::lower_with_catalog(right, catalog)),
            },
            RelExpr::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => Self::lower_anti_join(
                left,
                right,
                *left_column,
                *right_column,
                *equivalence,
                catalog,
            ),
            RelExpr::Distinct {
                input,
                column_equivalences,
            } => Self::Distinct {
                input: Box::new(Self::lower_with_catalog(input, catalog)),
                column_equivalences: column_equivalences.clone(),
            },
            RelExpr::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
            } => Self::lower_group(
                input,
                group_columns,
                group_equivalences,
                aggregate,
                catalog,
            ),
            RelExpr::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            } => Self::lower_top_k(
                input,
                *column,
                *ordering,
                *direction,
                *k,
                catalog,
            ),
            RelExpr::PromoteToBag(input) => {
                Self::PromoteToBag(Box::new(Self::lower_with_catalog(input, catalog)))
            }
        }
    }

    fn lower_join(
        left: &RelExpr,
        right: &RelExpr,
        left_column: usize,
        right_column: usize,
        equivalence: SemanticId,
        catalog: &PhysicalCatalog,
    ) -> Self {
        let left = Self::lower_with_catalog(left, catalog);
        let right = Self::lower_with_catalog(right, catalog);
        Self::JoinEq {
            left: Box::new(left),
            right: Box::new(right),
            left_column,
            right_column,
            equivalence,
        }
    }

    fn lower_eq_filter(
        input: &RelExpr,
        column: usize,
        value: &Value,
        equivalence: SemanticId,
        catalog: &PhysicalCatalog,
    ) -> Self {
        Self::FilterEqConst {
            input: Box::new(Self::lower_with_catalog(input, catalog)),
            column,
            value: value.clone(),
            equivalence,
        }
    }

    fn lower_column_filter(
        input: &RelExpr,
        left_column: usize,
        right_column: usize,
        equivalence: SemanticId,
        catalog: &PhysicalCatalog,
    ) -> Self {
        Self::FilterEqColumns {
            input: Box::new(Self::lower_with_catalog(input, catalog)),
            left_column,
            right_column,
            equivalence,
        }
    }

    fn lower_anti_join(
        left: &RelExpr,
        right: &RelExpr,
        left_column: usize,
        right_column: usize,
        equivalence: SemanticId,
        catalog: &PhysicalCatalog,
    ) -> Self {
        Self::AntiJoin {
            left: Box::new(Self::lower_with_catalog(left, catalog)),
            right: Box::new(Self::lower_with_catalog(right, catalog)),
            left_column,
            right_column,
            equivalence,
        }
    }

    fn lower_top_k(
        input: &RelExpr,
        column: usize,
        ordering: SemanticId,
        direction: OrderDirection,
        k: usize,
        catalog: &PhysicalCatalog,
    ) -> Self {
        Self::TopKWithTies {
            input: Box::new(Self::lower_with_catalog(input, catalog)),
            column,
            ordering,
            direction,
            k,
        }
    }

    fn lower_order_filter(expr: &RelExpr, catalog: &PhysicalCatalog) -> Self {
        let RelExpr::FilterOrderConst {
            input,
            column,
            value,
            ordering,
            comparison,
        } = expr
        else {
            unreachable!("lower_order_filter requires FilterOrderConst");
        };
        Self::FilterOrderConst {
            input: Box::new(Self::lower_with_catalog(input, catalog)),
            column: *column,
            value: value.clone(),
            ordering: *ordering,
            comparison: *comparison,
        }
    }

    fn lower_group(
        input: &RelExpr,
        group_columns: &[usize],
        group_equivalences: &[SemanticId],
        aggregate: &AggregateSpec,
        catalog: &PhysicalCatalog,
    ) -> Self {
        Self::Group {
            input: Box::new(Self::lower_with_catalog(input, catalog)),
            group_columns: group_columns.to_vec(),
            group_equivalences: group_equivalences.to_vec(),
            aggregate: aggregate.clone(),
        }
    }

    #[must_use]
    pub fn to_logical_expr(&self) -> RelExpr {
        match self {
            Self::Scan { relation, .. } => RelExpr::Scan(*relation),
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => RelExpr::FilterEqConst {
                input: Box::new(input.to_logical_expr()),
                column: *column,
                value: value.clone(),
                equivalence: *equivalence,
            },
            Self::FilterOrderConst {
                input,
                column,
                value,
                ordering,
                comparison,
            } => Self::order_filter_to_logical(input, *column, value, *ordering, *comparison),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => RelExpr::FilterEqColumns {
                input: Box::new(input.to_logical_expr()),
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            },
            Self::Project { input, columns } => RelExpr::Project {
                input: Box::new(input.to_logical_expr()),
                columns: columns.clone(),
            },
            Self::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
                ..
            } => RelExpr::JoinEq {
                left: Box::new(left.to_logical_expr()),
                right: Box::new(right.to_logical_expr()),
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            },
            Self::Difference { left, right } => RelExpr::Difference {
                left: Box::new(left.to_logical_expr()),
                right: Box::new(right.to_logical_expr()),
            },
            Self::Union { left, right } => RelExpr::Union {
                left: Box::new(left.to_logical_expr()),
                right: Box::new(right.to_logical_expr()),
            },
            Self::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => RelExpr::AntiJoin {
                left: Box::new(left.to_logical_expr()),
                right: Box::new(right.to_logical_expr()),
                left_column: *left_column,
                right_column: *right_column,
                equivalence: *equivalence,
            },
            Self::Distinct {
                input,
                column_equivalences,
            } => RelExpr::Distinct {
                input: Box::new(input.to_logical_expr()),
                column_equivalences: column_equivalences.clone(),
            },
            Self::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
                ..
            } => Self::group_to_logical(input, group_columns, group_equivalences, aggregate),
            Self::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
                ..
            } => RelExpr::TopKWithTies {
                input: Box::new(input.to_logical_expr()),
                column: *column,
                ordering: *ordering,
                direction: *direction,
                k: *k,
            },
            Self::PromoteToBag(input) => RelExpr::PromoteToBag(Box::new(input.to_logical_expr())),
        }
    }

    fn order_filter_to_logical(
        input: &Self,
        column: usize,
        value: &Value,
        ordering: SemanticId,
        comparison: OrderComparison,
    ) -> RelExpr {
        RelExpr::FilterOrderConst {
            input: Box::new(input.to_logical_expr()),
            column,
            value: value.clone(),
            ordering,
            comparison,
        }
    }

    fn group_to_logical(
        input: &Self,
        group_columns: &[usize],
        group_equivalences: &[SemanticId],
        aggregate: &AggregateSpec,
    ) -> RelExpr {
        RelExpr::Group {
            input: Box::new(input.to_logical_expr()),
            group_columns: group_columns.to_vec(),
            group_equivalences: group_equivalences.to_vec(),
            aggregate: aggregate.clone(),
        }
    }

    #[must_use]
    pub fn shape(&self) -> PlanShape {
        let mut shape = PlanShape::default();
        self.accumulate_shape(&mut shape);
        shape
    }

    fn accumulate_shape(&self, shape: &mut PlanShape) {
        shape.nodes += 1;
        match self {
            Self::Scan { .. } => shape.scans += 1,
            Self::FilterEqConst { input, .. } | Self::FilterOrderConst { input, .. } | Self::FilterEqColumns { input, .. } => {
                shape.filters += 1;
                input.accumulate_shape(shape);
            }
            Self::Project { input, .. } => {
                shape.projects += 1;
                input.accumulate_shape(shape);
            }
            Self::JoinEq { left, right, .. } => {
                shape.joins += 1;
                left.accumulate_shape(shape);
                right.accumulate_shape(shape);
            }
            Self::Difference { left, right } => {
                shape.differences += 1;
                left.accumulate_shape(shape);
                right.accumulate_shape(shape);
            }
            Self::Union { left, right } => {
                shape.unions += 1;
                left.accumulate_shape(shape);
                right.accumulate_shape(shape);
            }
            Self::AntiJoin { left, right, .. } => {
                shape.anti_joins += 1;
                left.accumulate_shape(shape);
                right.accumulate_shape(shape);
            }
            Self::Distinct { input, .. } => {
                shape.distincts += 1;
                input.accumulate_shape(shape);
            }
            Self::Group { input, .. } => {
                shape.groups += 1;
                input.accumulate_shape(shape);
            }
            Self::TopKWithTies { input, .. } => {
                shape.top_k += 1;
                input.accumulate_shape(shape);
            }
            Self::PromoteToBag(input) => {
                shape.bag_promotions += 1;
                input.accumulate_shape(shape);
            }
        }
    }

    /// Reference-only semantic execution used to falsify lowering. This is not
    /// the physical runtime and therefore carries no performance claim.
    pub fn reference_execute(
        &self,
        model: &kernel_model::FiniteModel,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<RelationValue, RelQueryError> {
        self.to_logical_expr()
            .prepare(context, registry)?
            .evaluate(model, context, registry)
    }

    fn guarantees_set_uniqueness(&self) -> bool {
        match self {
            Self::Distinct { .. } | Self::Group { .. } => true,
            Self::FilterEqConst { input, .. }
            | Self::FilterOrderConst { input, .. }
            | Self::FilterEqColumns { input, .. }
            | Self::TopKWithTies { input, .. } => input.guarantees_set_uniqueness(),
            Self::Scan { .. }
            | Self::Project { .. }
            | Self::JoinEq { .. }
            | Self::Difference { .. }
            | Self::Union { .. }
            | Self::AntiJoin { .. }
            | Self::PromoteToBag(_) => false,
        }
    }

    pub fn execute_native(
        &self,
        store: &PhysicalStore,
        result_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<(RelationValue, ExecutionStats), PhysicalExecutionError> {
        self.execute_native_with_prepared_programs(
            store,
            result_type,
            context,
            registry,
            None,
            None,
        )
    }

    pub(super) fn execute_native_with_prepared_programs(
        &self,
        store: &PhysicalStore,
        result_type: &RelType,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        anchor_pullback_program: Option<&PreparedAnchorPullbackProgram>,
        prepared_program: Option<&PreparedSemanticQuotientProgram>,
    ) -> Result<(RelationValue, ExecutionStats), PhysicalExecutionError> {
        let mut stats = ExecutionStats::default();
        let rows = if let Some(program) = anchor_pullback_program {
            let accelerated_quotient = prepared_program
                .map(|quotient| {
                    prepared_quotient_acceleration_available(
                        self, quotient, store, context, registry,
                    )
                })
                .transpose()?
                .unwrap_or(false);
            if accelerated_quotient {
                if let Some(quotient) = prepared_program
                    && let Some(rows) = try_execute_nway_order_preserving_join(
                        self,
                        Some(quotient),
                        store,
                        context,
                        registry,
                        &mut stats,
                    )?
                {
                    stats.multiway_join_reorders = stats.multiway_join_reorders.saturating_add(1);
                    stats.multiway_join_order_preserving_enumerations = stats
                        .multiway_join_order_preserving_enumerations
                        .saturating_add(1);
                    stats.multiway_join_prepared_quotient_hits =
                        stats.multiway_join_prepared_quotient_hits.saturating_add(1);
                    rows
                } else if let Some(rows) = try_execute_anchor_pullback_join(
                    self, program, store, context, registry, &mut stats,
                )? {
                    rows
                } else {
                    self.execute_native_rows(store, context, registry, &mut stats)?
                }
            } else if let Some(rows) = try_execute_anchor_pullback_join(
                self, program, store, context, registry, &mut stats,
            )? {
                rows
            } else if let Some(quotient) = prepared_program {
                if let Some(rows) = try_execute_nway_order_preserving_join(
                    self,
                    Some(quotient),
                    store,
                    context,
                    registry,
                    &mut stats,
                )? {
                    stats.multiway_join_reorders = stats.multiway_join_reorders.saturating_add(1);
                    stats.multiway_join_order_preserving_enumerations = stats
                        .multiway_join_order_preserving_enumerations
                        .saturating_add(1);
                    stats.multiway_join_prepared_quotient_hits =
                        stats.multiway_join_prepared_quotient_hits.saturating_add(1);
                    rows
                } else {
                    self.execute_native_rows(store, context, registry, &mut stats)?
                }
            } else {
                self.execute_native_rows(store, context, registry, &mut stats)?
            }
        } else if let Some(program) = prepared_program {
            if let Some(rows) = try_execute_nway_order_preserving_join(
                self,
                Some(program),
                store,
                context,
                registry,
                &mut stats,
            )? {
                stats.multiway_join_reorders = stats.multiway_join_reorders.saturating_add(1);
                stats.multiway_join_order_preserving_enumerations = stats
                    .multiway_join_order_preserving_enumerations
                    .saturating_add(1);
                stats.multiway_join_prepared_quotient_hits =
                    stats.multiway_join_prepared_quotient_hits.saturating_add(1);
                rows
            } else {
                self.execute_native_rows(store, context, registry, &mut stats)?
            }
        } else {
            self.execute_native_rows(store, context, registry, &mut stats)?
        };
        let value = match &result_type.semantics {
            kernel_schema::RelationSemantics::Set {
                column_equivalences,
            } if self.guarantees_set_uniqueness() => RelationValue::Set {
                rows,
                column_equivalences: column_equivalences.clone(),
            },
            _ => relation_value_from_rows(rows, result_type, context, registry)?,
        };
        stats.output_rows = value.rows().len();
        Ok((value, stats))
    }

    pub(super) fn execute_native_rows(
        &self,
        store: &PhysicalStore,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        stats: &mut ExecutionStats,
    ) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
        if let Some(rows) = try_execute_multiway_join(self, store, context, registry, stats)? {
            return Ok(rows);
        }
        if let Some(rows) = try_execute_native_fast_path(self, store, context, registry, stats)? {
            return Ok(rows);
        }
        match self {
            Self::Project { input, columns } => {
                execute_project_plan(input, columns, store, context, registry, stats)
            }
            Self::FilterEqConst {
                input,
                column,
                value,
                equivalence,
            } => execute_filter_const(
                input,
                *column,
                value,
                *equivalence,
                store,
                (context, registry),
                stats,
            ),
            Self::FilterOrderConst {
                input,
                column,
                value,
                ordering,
                comparison,
            } => execute_filter_order(
                input,
                OrderFilterSpec {
                    column: *column,
                    value,
                    ordering: *ordering,
                    comparison: *comparison,
                },
                store,
                (context, registry),
                stats,
            ),
            Self::FilterEqColumns {
                input,
                left_column,
                right_column,
                equivalence,
            } => execute_filter_columns(
                input,
                *left_column,
                *right_column,
                *equivalence,
                store,
                (context, registry),
                stats,
            ),
            Self::Scan { relation, layout } => scan_rows(store, *relation, *layout, context, stats),
            Self::PromoteToBag(input) => input.execute_native_rows(store, context, registry, stats),
            _ => self.execute_native_rows_complex(store, context, registry, stats),
        }
    }

    fn execute_native_rows_complex(
        &self,
        store: &PhysicalStore,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
        stats: &mut ExecutionStats,
    ) -> Result<Vec<kernel_query::Row>, PhysicalExecutionError> {
        match self {
            Self::JoinEq {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => execute_join_plan(
                left,
                right,
                *left_column,
                *right_column,
                *equivalence,
                store,
                context,
                registry,
                stats,
            ),
            Self::Difference { left, right } => {
                execute_difference_plan(left, right, store, context, registry, stats)
            }
            Self::Union { left, right } => {
                execute_union_plan(left, right, store, context, registry, stats)
            }
            Self::AntiJoin {
                left,
                right,
                left_column,
                right_column,
                equivalence,
            } => execute_anti_join_plan(
                left,
                right,
                (*left_column, *right_column),
                *equivalence,
                store,
                (context, registry),
                stats,
            ),
            Self::Distinct {
                input,
                column_equivalences,
            } => execute_distinct_plan(input, column_equivalences, store, context, registry, stats),
            Self::Group {
                input,
                group_columns,
                group_equivalences,
                aggregate,
            } => execute_group_plan(
                input,
                group_columns,
                group_equivalences,
                aggregate,
                store,
                context,
                registry,
                stats,
            ),
            Self::TopKWithTies {
                input,
                column,
                ordering,
                direction,
                k,
            } => execute_top_k_plan(
                input, *column, *ordering, *direction, *k, store, context, registry, stats,
            ),
            Self::Project { .. }
            | Self::FilterEqConst { .. }
            | Self::FilterOrderConst { .. }
            | Self::FilterEqColumns { .. }
            | Self::Scan { .. }
            | Self::PromoteToBag(_) => unreachable!("simple plan delegated to complex executor"),
        }
    }
}

