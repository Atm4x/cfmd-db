#[cfg(test)]
mod relational_tests {
    use kernel_model::FiniteModel;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
    use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

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
    fn compiled_linear_island_matches_maintained_chain_and_uses_source_coordinates() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::Text("alpha".into()),
                    equivalence: text_eq,
                }),
                columns: vec![1, 0],
            }),
            column: 1,
            value: Value::Text("ALPHA".into()),
            equivalence: text_eq,
        };
        let differential = RelDifferentialProgram::compile(&query, &context, &registry).unwrap();
        let islands = differential.physical_program().linear_islands();
        assert_eq!(islands.len(), 1);
        assert_eq!(islands[0].input_width(), 2);
        assert_eq!(islands[0].projection(), &[1, 0]);
        assert_eq!(islands[0].predicates().len(), 2);
        assert!(islands[0].predicates().iter().all(|predicate| matches!(
            predicate,
            LinearIslandPredicate::EqConst {
                source_column: 0,
                ..
            }
        )));

        let source_delta = RelationDelta {
            inserted: vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("beta".into()), Value::I64(2)],
                vec![Value::Text("ALPHA".into()), Value::I64(3)],
            ],
            removed: Vec::new(),
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        let fused = islands[0]
            .execute(&source_delta.as_delta_view(), &context, &registry)
            .unwrap();
        let mut fused_rows = Vec::new();
        fused.visit(|weight, row| fused_rows.push((weight, row.clone())));

        let model = FiniteModel::default();
        let mut maintained =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let output = maintained
            .apply_relation_deltas(
                &BTreeMap::from([(relation, source_delta)]),
                &context,
                &registry,
            )
            .unwrap();
        let expected = output
            .inserted
            .into_iter()
            .map(|row| (1, row))
            .chain(output.removed.into_iter().map(|row| (-1, row)))
            .collect::<Vec<_>>();
        assert_eq!(fused_rows, expected);
    }

    #[test]
    fn compiled_delta_program_records_every_non_linear_barrier_class() {
        let (mut context, mut registry, text_eq, left, right) = setup();
        let i64_eq = SemanticId::new(101);
        let i64_order = SemanticId::new(102);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        context.environment.pin_module(i64_order, order_digest);

        let unary = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Group {
                input: Box::new(RelExpr::Project {
                    input: Box::new(RelExpr::Distinct {
                        input: Box::new(RelExpr::Scan(left)),
                        column_equivalences: vec![text_eq, i64_eq],
                    }),
                    columns: vec![0],
                }),
                group_columns: vec![0],
                group_equivalences: vec![text_eq],
                aggregate: AggregateSpec::Count {
                    result_equivalence: i64_eq,
                },
            }),
            column: 1,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 3,
        };
        let unary_program = RelDifferentialProgram::compile(&unary, &context, &registry).unwrap();
        assert_eq!(
            unary_program.physical_program().barriers(),
            &[
                BarrierKernelClass::ZeroCrossing,
                BarrierKernelClass::ZeroCrossing,
                BarrierKernelClass::Annotation,
                BarrierKernelClass::OrderedBoundary,
            ]
        );

        let join = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        assert_eq!(
            RelDifferentialProgram::compile(&join, &context, &registry)
                .unwrap()
                .physical_program()
                .barriers(),
            &[BarrierKernelClass::BilinearPullback]
        );

        let blocker = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        };
        assert_eq!(
            RelDifferentialProgram::compile(&blocker, &context, &registry)
                .unwrap()
                .physical_program()
                .barriers(),
            &[BarrierKernelClass::BlockerZeroCrossing]
        );
    }

    #[test]
    fn blocker_difference_plans_weighted_zero_crossing_without_mutation() {
        let (context, registry, _, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let result_type = left_type.clone();
        let alpha = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let left = RelationValue::Bag(vec![alpha.clone(), alpha.clone(), alpha.clone()]);
        let right = RelationValue::Bag(vec![alpha.clone()]);
        let kind = MaintainedBlockerKind::Difference;
        let mut state = MaterializedBlockerDeltaState::build(
            &left,
            &right,
            BlockerBuildSpec {
                kind: &kind,
                left_type,
                right_type,
                result_type,
                context: &context,
                registry: &registry,
            },
        )
        .unwrap();

        let before = state.clone();
        let left_delta = AdaptiveDelta::<Row, 4>::default();
        let mut right_delta = AdaptiveDelta::<Row, 4>::default();
        right_delta.push_weighted(2, alpha.clone());
        let planned = state
            .plan_delta_views(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        let mut effect = Vec::new();
        planned
            .effect
            .visit(|weight, row| effect.push((weight, row.clone())));
        assert_eq!(effect, vec![(-2, alpha.clone())]);
        assert_eq!(state, before);
        state.commit_patch(planned.patch);
        assert!(state.output_value().unwrap().rows().is_empty());

        let before_invalid = state.clone();
        let mut invalid = AdaptiveDelta::<Row, 4>::default();
        invalid.push_weighted(-4, alpha);
        assert!(matches!(
            state.plan_delta_views(
                &AdaptiveDelta::<Row, 4>::default(),
                &invalid,
                &context,
                &registry,
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        ));
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn blocker_difference_keeps_max_weight_compact() {
        let (context, registry, _, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let alpha = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let kind = MaintainedBlockerKind::Difference;
        let state = MaterializedBlockerDeltaState::build(
            &RelationValue::Bag(Vec::new()),
            &RelationValue::Bag(Vec::new()),
            BlockerBuildSpec {
                kind: &kind,
                left_type: left_type.clone(),
                right_type,
                result_type: left_type,
                context: &context,
                registry: &registry,
            },
        )
        .unwrap();
        let magnitude = kernel_exact::ExactNatural::from_u128(u128::from(i64::MAX as u64) + 7);
        let mut left_delta = ExactDelta::<Row>::default();
        left_delta.push_exact(
            kernel_exact::ExactInteger::from_parts(false, magnitude.clone()),
            alpha.clone(),
        );
        let planned = state
            .plan_exact_delta_views(
                &left_delta,
                &ExactDelta::<Row>::default(),
                &context,
                &registry,
            )
            .unwrap();
        let mut effect = Vec::new();
        planned
            .effect
            .visit_exact(|weight, row| effect.push((weight.clone(), row.clone())));
        assert_eq!(effect.len(), 1);
        assert_eq!(effect[0].0.magnitude(), &magnitude);
        assert_eq!(effect[0].1, alpha);
        let (left_count, right_count) = planned
            .patch
            .test_first_difference_counts()
            .expect("expected Difference patch");
        assert_eq!(left_count, magnitude);
        assert!(right_count.is_zero());
    }

    #[test]
    fn blocker_antijoin_preserves_same_key_left_replacement_and_zero_crossing() {
        let (context, registry, text_eq, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let first = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let second = vec![Value::Text("alpha".into()), Value::I64(2)];
        let replacement = vec![Value::Text("ALPHA".into()), Value::I64(3)];
        let blocker = vec![Value::Text("aLpHa".into()), Value::I64(99)];
        let kind = MaintainedBlockerKind::AntiJoin {
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut state = MaterializedBlockerDeltaState::build(
            &RelationValue::Bag(vec![first.clone(), second.clone()]),
            &RelationValue::Bag(Vec::new()),
            BlockerBuildSpec {
                kind: &kind,
                left_type,
                right_type,
                result_type: RelExpr::Scan(left_id)
                    .typecheck(&context, &registry)
                    .unwrap(),
                context: &context,
                registry: &registry,
            },
        )
        .unwrap();

        let mut left_delta = AdaptiveDelta::<Row, 4>::default();
        left_delta.push_weighted(-1, first.clone());
        left_delta.push_weighted(1, replacement.clone());
        let planned = state
            .plan_delta_views(
                &left_delta,
                &AdaptiveDelta::<Row, 4>::default(),
                &context,
                &registry,
            )
            .unwrap();
        let mut replacement_effect = Vec::new();
        planned
            .effect
            .visit(|weight, row| replacement_effect.push((weight, row.clone())));
        assert_eq!(
            replacement_effect,
            vec![(-1, first), (1, replacement.clone())]
        );
        state.commit_patch(planned.patch);

        let before_crossing = state.clone();
        let mut right_delta = AdaptiveDelta::<Row, 4>::default();
        right_delta.push_weighted(2, blocker.clone());
        let crossing = state
            .plan_delta_views(
                &AdaptiveDelta::<Row, 4>::default(),
                &right_delta,
                &context,
                &registry,
            )
            .unwrap();
        let mut crossing_effect = Vec::new();
        crossing
            .effect
            .visit(|weight, row| crossing_effect.push((weight, row.clone())));
        assert_eq!(crossing_effect.len(), 2);
        assert!(crossing_effect.iter().all(|(weight, _)| *weight == -1));
        assert!(crossing_effect.iter().any(|(_, row)| row == &second));
        assert!(crossing_effect.iter().any(|(_, row)| row == &replacement));
        assert_eq!(state, before_crossing);
        state.commit_patch(crossing.patch);
        assert!(state.output_value().unwrap().rows().is_empty());

        let before_invalid = state.clone();
        let mut invalid = AdaptiveDelta::<Row, 4>::default();
        invalid.push_weighted(-3, blocker);
        assert!(matches!(
            state.plan_delta_views(
                &AdaptiveDelta::<Row, 4>::default(),
                &invalid,
                &context,
                &registry,
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        ));
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn recursive_difference_matches_recompute_for_two_sided_transition() {
        let (context, registry, _, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let alpha1 = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let alpha1_alt = vec![Value::Text("ALPHA".into()), Value::I64(1)];
        let beta2 = vec![Value::Text("Beta".into()), Value::I64(2)];
        let gamma3 = vec![Value::Text("Gamma".into()), Value::I64(3)];
        let query = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left_id)),
            right: Box::new(RelExpr::Scan(right_id)),
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left_id, vec![alpha1.clone(), alpha1.clone(), beta2.clone()]);
        old.relations.insert(right_id, vec![alpha1_alt.clone()]);
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![gamma3.clone()],
            removed: vec![beta2],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: vec![alpha1_alt.clone()],
            removed: Vec::new(),
            result_type: right_type,
        };
        let mut next = old.clone();
        next.relations
            .insert(left_id, vec![alpha1.clone(), alpha1, gamma3]);
        next.relations
            .insert(right_id, vec![alpha1_alt.clone(), alpha1_alt]);
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let actual = state
            .apply_relation_deltas(
                &BTreeMap::from([(left_id, left_delta), (right_id, right_delta)]),
                &context,
                &registry,
            )
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&actual, &oracle, &context, &registry).unwrap()
        );
        assert_eq!(
            state.output_value(&context, &registry).unwrap(),
            query.evaluate(&next, &context, &registry).unwrap()
        );
    }

    #[test]
    fn recursive_antijoin_matches_recompute_and_invalid_blocker_is_atomic() {
        let (context, registry, text_eq, left_id, right_id) = setup();
        let left_type = RelExpr::Scan(left_id)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_id)
            .typecheck(&context, &registry)
            .unwrap();
        let alpha1 = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let alpha3 = vec![Value::Text("alpha".into()), Value::I64(3)];
        let beta2 = vec![Value::Text("Beta".into()), Value::I64(2)];
        let blocker = vec![Value::Text("aLpHa".into()), Value::I64(99)];
        let query = RelExpr::AntiJoin {
            left: Box::new(RelExpr::Scan(left_id)),
            right: Box::new(RelExpr::Scan(right_id)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left_id, vec![alpha1.clone(), beta2.clone()]);
        old.relations.insert(right_id, vec![blocker.clone()]);
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![alpha3.clone()],
            removed: vec![beta2],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![blocker.clone()],
            result_type: right_type.clone(),
        };
        let mut next = old.clone();
        next.relations.insert(left_id, vec![alpha1, alpha3]);
        next.relations.insert(right_id, Vec::new());
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let actual = state
            .apply_relation_deltas(
                &BTreeMap::from([(left_id, left_delta), (right_id, right_delta)]),
                &context,
                &registry,
            )
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&actual, &oracle, &context, &registry).unwrap()
        );
        assert_eq!(
            state.output_value(&context, &registry).unwrap(),
            query.evaluate(&next, &context, &registry).unwrap()
        );
        let before_invalid = state.clone();
        let invalid = RelationDelta {
            inserted: Vec::new(),
            removed: vec![blocker],
            result_type: right_type,
        };
        assert_eq!(
            state.apply_relation_deltas(
                &BTreeMap::from([(right_id, invalid)]),
                &context,
                &registry,
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn validated_frames_keep_same_relation_self_join_leaves_disjoint() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(relation)),
            right: Box::new(RelExpr::Scan(relation)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let first = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let second = vec![Value::Text("ALPHA".into()), Value::I64(2)];
        let mut old = FiniteModel::default();
        old.relations.insert(relation, vec![first.clone()]);
        let mut maintained =
            MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let scan_type = RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap();
        maintained
            .apply_relation_deltas(
                &BTreeMap::from([(
                    relation,
                    RelationDelta {
                        inserted: vec![second.clone()],
                        removed: Vec::new(),
                        result_type: scan_type,
                    },
                )]),
                &context,
                &registry,
            )
            .unwrap();

        let mut new = old;
        new.relations.get_mut(&relation).unwrap().push(second);
        assert_eq!(
            maintained.output_value(&context, &registry).unwrap(),
            query.evaluate(&new, &context, &registry).unwrap()
        );
    }

    #[test]
    fn relation_delta_prepares_intent_bearing_gamma_rewrite_from_exact_endpoint() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Scan(relation);
        let old = RelationValue::Bag(vec![vec![Value::Text("Alpha".into()), Value::I64(1)]]);
        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Beta".into()), Value::I64(2)]],
            removed: vec![vec![Value::Text("alpha".into()), Value::I64(1)]],
            result_type: query.typecheck(&context, &registry).unwrap(),
        };
        let spec = RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(7000)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(7001)),
            footprint: delta
                .rewrite_footprint(relation, &context, &registry)
                .unwrap(),
        };
        let prepared = delta
            .prepare_relation_rewrite(
                relation,
                &old,
                &context,
                &registry,
                &spec,
                vec![SemanticId::new(7002)],
            )
            .unwrap();
        assert_eq!(prepared.rewrite().spec(), spec.id);
        assert_eq!(prepared.rewrite().law_set(), spec.law_set);
        assert_eq!(
            prepared.rewrite().explicit_inputs(),
            vec![SemanticId::new(7002)]
        );
        assert_eq!(prepared.delta(), &delta);
        assert_eq!(
            prepared.apply_structural(&old, &registry).unwrap(),
            RelationValue::Bag(vec![vec![Value::Text("Beta".into()), Value::I64(2)]])
        );
    }

    #[test]
    fn structural_relation_rewrite_binds_gamma_base_not_host_representation() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Scan(relation);
        let old = RelationValue::Bag(vec![vec![Value::Text("Alpha".into()), Value::I64(1)]]);
        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Beta".into()), Value::I64(2)]],
            removed: vec![vec![Value::Text("alpha".into()), Value::I64(1)]],
            result_type: query.typecheck(&context, &registry).unwrap(),
        };
        let spec = RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(7005)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(7006)),
            footprint: delta
                .rewrite_footprint(relation, &context, &registry)
                .unwrap(),
        };
        let prepared = delta
            .prepare_relation_rewrite(
                relation,
                &old,
                &context,
                &registry,
                &spec,
                Vec::<SemanticId>::new(),
            )
            .unwrap();

        let equivalent_base =
            RelationValue::Bag(vec![vec![Value::Text("ALPHA".into()), Value::I64(1)]]);
        assert_eq!(
            prepared
                .apply_structural(&equivalent_base, &registry)
                .unwrap(),
            RelationValue::Bag(vec![vec![Value::Text("Beta".into()), Value::I64(2)]])
        );

        let unrelated_base =
            RelationValue::Bag(vec![vec![Value::Text("Gamma".into()), Value::I64(1)]]);
        assert_eq!(
            prepared.apply_structural(&unrelated_base, &registry),
            Err(RelQueryError::StructuralRewriteBaseMismatch)
        );
    }

    #[test]
    fn relation_delta_preparation_rejects_underdeclared_dynamic_footprint() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Scan(relation);
        let old = RelationValue::Bag(Vec::new());
        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
            removed: Vec::new(),
            result_type: query.typecheck(&context, &registry).unwrap(),
        };
        let spec = RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(7010)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(7011)),
            footprint: kernel_change::RewriteFootprint::default(),
        };
        assert_eq!(
            delta.prepare_relation_rewrite(
                relation,
                &old,
                &context,
                &registry,
                &spec,
                Vec::<SemanticId>::new(),
            ),
            Err(RelQueryError::RewriteFootprintMismatch)
        );
    }

    #[test]
    fn gamma_equal_relation_classes_require_generic_coordination() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Scan(relation);
        let old = RelationValue::Bag(Vec::new());
        let result_type = query.typecheck(&context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
            removed: Vec::new(),
            result_type: result_type.clone(),
        };
        let right_delta = RelationDelta {
            inserted: vec![vec![Value::Text("alpha".into()), Value::I64(1)]],
            removed: Vec::new(),
            result_type,
        };
        let left_spec = RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(7020)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(7021)),
            footprint: left_delta
                .rewrite_footprint(relation, &context, &registry)
                .unwrap(),
        };
        let right_spec = RewriteSpec {
            id: kernel_change::RewriteSpecId(SemanticId::new(7022)),
            law_set: kernel_change::RewriteLawSetId(SemanticId::new(7023)),
            footprint: right_delta
                .rewrite_footprint(relation, &context, &registry)
                .unwrap(),
        };
        assert_eq!(left_spec.footprint, right_spec.footprint);
        let left = left_delta
            .prepare_relation_rewrite(
                relation,
                &old,
                &context,
                &registry,
                &left_spec,
                Vec::<SemanticId>::new(),
            )
            .unwrap();
        let right = right_delta
            .prepare_relation_rewrite(
                relation,
                &old,
                &context,
                &registry,
                &right_spec,
                Vec::<SemanticId>::new(),
            )
            .unwrap();
        let mut coordination = kernel_change::RewriteCoordinationRegistry::default();
        coordination.register(&left_spec).unwrap();
        coordination.register(&right_spec).unwrap();
        let graph = coordination
            .compile([(0_u8, left.rewrite()), (1_u8, right.rewrite())])
            .unwrap();
        assert_eq!(
            graph.decision(0, 1),
            kernel_change::PairCoordinationDecision::RequiresCoordination
        );
    }

    #[test]
    fn filter_and_distinct_use_semantic_equality_not_rust_equality() {
        let (context, registry, text_eq, relation, _) = setup();
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("alpha".into()), Value::I64(2)],
                vec![Value::Text("Beta".into()), Value::I64(3)],
            ],
        );
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::Text("ALPHA".into()),
                    equivalence: text_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        assert_eq!(
            query.evaluate(&model, &context, &registry),
            Ok(RelationValue::Set {
                rows: vec![vec![Value::Text("Alpha".into())]],
                column_equivalences: vec![text_eq],
            })
        );
    }

    #[test]
    fn differential_program_classifies_state_and_matches_recompute_oracle() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::Text("ALPHA".into()),
                    equivalence: text_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let program = RelDifferentialProgram::compile(&query, &context, &registry).unwrap();
        assert_eq!(program.root_class(), RelDifferentialClass::ZeroCrossing);
        assert_eq!(
            program.state_requirements(),
            BTreeSet::from([RelDifferentialStateRequirement::SetSupport])
        );

        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        let mut next = old.clone();
        next.relations.get_mut(&relation).unwrap().extend([
            vec![Value::Text("alpha".into()), Value::I64(3)],
            vec![Value::Text("Gamma".into()), Value::I64(4)],
        ]);
        let change = Change::Replace(next);
        let exact = program.apply(&old, &change, &context, &registry).unwrap();
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        assert_eq!(exact, oracle);
    }

    #[test]
    fn differential_program_rejects_semantic_context_drift() {
        let (context, registry, _, relation, _) = setup();
        let program =
            RelDifferentialProgram::compile(&RelExpr::Scan(relation), &context, &registry).unwrap();
        let mut drifted = context.clone();
        drifted.environment = SemanticEnvironment::new(SemanticEnvId::new(999));
        assert_eq!(
            program.apply(
                &FiniteModel::default(),
                &Change::NoChange,
                &drifted,
                &registry
            ),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }

    #[test]
    fn observation_guard_uses_dtc_for_exact_fiber_impact_and_keeps_oracle_parity() {
        let (context, registry, text_eq, left, right) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(left)),
                    column: 0,
                    value: Value::Text("alpha".into()),
                    equivalence: text_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left, vec![vec![Value::Text("Alpha".into()), Value::I64(1)]]);
        old.relations.insert(
            right,
            vec![vec![Value::Text("Other".into()), Value::I64(9)]],
        );
        let guard = RelObservationGuard::observe(&query, &old, &context, &registry).unwrap();
        assert_eq!(guard.source_relations(), &BTreeSet::from([left]));
        assert_eq!(guard.observed_key().distinct_row_classes(), 1);
        assert_eq!(guard.observed_key().row_multiplicity(), 1);

        let mut unrelated = old.clone();
        unrelated
            .relations
            .get_mut(&right)
            .unwrap()
            .push(vec![Value::Text("Else".into()), Value::I64(10)]);
        let change = Change::Replace(unrelated);
        assert_eq!(
            guard.impact(&old, &change, &context, &registry).unwrap(),
            Impact::Unaffected
        );
        assert_eq!(
            guard.impact_by_recompute_oracle(&old, &change, &context, &registry),
            Impact::Unaffected
        );

        // The source relation changes, but the Distinct observation remains in
        // the same semantic fiber because "Alpha" ==Γ "alpha".
        let mut duplicate = old.clone();
        duplicate
            .relations
            .get_mut(&left)
            .unwrap()
            .push(vec![Value::Text("alpha".into()), Value::I64(2)]);
        let change = Change::Replace(duplicate);
        assert_eq!(
            guard.impact(&old, &change, &context, &registry).unwrap(),
            Impact::Unaffected
        );
        assert_eq!(
            guard.impact_by_recompute_oracle(&old, &change, &context, &registry),
            Impact::Unaffected
        );
    }

    #[test]
    fn canonical_relation_multiset_matches_reference_for_structural_composite_rows() {
        let text_eq = SemanticId::new(98_300);
        let set_eq = SemanticId::new(98_301);
        let i64_eq = SemanticId::new(98_302);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_300));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(98_300));
        schema
            .define_structural_equivalence(
                set_eq,
                kernel_schema::StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let set = |values: &[&str]| Value::Set {
            equivalence: text_eq,
            elements: values
                .iter()
                .map(|value| Value::Text((*value).into()))
                .collect(),
        };
        let equivalences = [set_eq, i64_eq];
        let source = vec![
            vec![set(&["A", "B"]), Value::I64(1)],
            vec![set(&["b", "a"]), Value::I64(1)],
            vec![set(&["C"]), Value::I64(2)],
            vec![set(&["D"]), Value::I64(3)],
        ];
        let target = vec![
            vec![set(&["B", "A"]), Value::I64(1)],
            vec![set(&["c"]), Value::I64(2)],
            vec![set(&["X"]), Value::I64(9)],
        ];

        let canonical =
            unmatched_semantic_rows(&source, &target, &equivalences, &context, &registry).unwrap();
        let reference = unmatched_semantic_rows_by_matching(
            &source,
            &target,
            &equivalences,
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(canonical, reference);
        assert_eq!(
            canonical,
            vec![
                vec![set(&["b", "a"]), Value::I64(1)],
                vec![set(&["D"]), Value::I64(3)],
            ]
        );

        let equivalent = vec![
            vec![set(&["d"]), Value::I64(3)],
            vec![set(&["C"]), Value::I64(2)],
            vec![set(&["A", "B"]), Value::I64(1)],
            vec![set(&["B", "a"]), Value::I64(1)],
        ];
        assert_eq!(
            rows_as_multisets_equivalent(&source, &equivalent, &equivalences, &context, &registry),
            rows_as_multisets_equivalent_by_matching(
                &source,
                &equivalent,
                &equivalences,
                &context,
                &registry,
            )
        );
        assert!(
            rows_as_multisets_equivalent(&source, &equivalent, &equivalences, &context, &registry,)
                .unwrap()
        );

        let fewer = equivalent[..3].to_vec();
        assert!(
            !rows_as_multisets_equivalent(&source, &fewer, &equivalences, &context, &registry,)
                .unwrap()
        );
    }

    #[test]
    #[ignore = "diagnostic benchmark; run explicitly in release mode"]
    fn pass55_canonical_relation_multiset_benchmark() {
        use std::time::Instant;

        let (context, registry, text_eq, _, _) = setup();
        let equivalences = [text_eq];
        let source = (0..4_000)
            .map(|value| vec![Value::Text(format!("Key-{value:05}"))])
            .collect::<Vec<_>>();
        let mut target = source.clone();
        target.reverse();

        let baseline_start = Instant::now();
        let baseline = unmatched_semantic_rows_by_matching(
            &source,
            &target,
            &equivalences,
            &context,
            &registry,
        )
        .unwrap();
        let baseline_elapsed = baseline_start.elapsed();

        let canonical_start = Instant::now();
        let canonical =
            unmatched_semantic_rows(&source, &target, &equivalences, &context, &registry).unwrap();
        let canonical_elapsed = canonical_start.elapsed();

        assert_eq!(canonical, baseline);
        println!(
            "PASS55_RELATION_MULTISET baseline_ns={} canonical_ns={} ratio_milli={}",
            baseline_elapsed.as_nanos(),
            canonical_elapsed.as_nanos(),
            baseline_elapsed
                .as_nanos()
                .saturating_mul(1_000)
                .checked_div(canonical_elapsed.as_nanos())
                .unwrap_or(u128::MAX)
        );
    }

    #[test]
    fn join_preserves_bag_multiplicity_and_uses_pinned_equality() {
        let (context, registry, text_eq, left, right) = setup();
        let mut model = FiniteModel::default();
        model.relations.insert(
            left,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(2)],
            ],
        );
        model
            .relations
            .insert(right, vec![vec![Value::Text("a".into()), Value::I64(10)]]);
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows().len(), 2);
        assert_eq!(result.rows()[0][1], Value::I64(1));
        assert_eq!(result.rows()[1][1], Value::I64(2));
    }

    #[test]
    fn top_k_with_ties_is_exact_without_observing_physical_row_order() {
        let relation = SemanticId::new(220);
        let i64_eq = SemanticId::new(221);
        let i64_order = SemanticId::new(222);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(220));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(220));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::I64(2)],
                vec![Value::I64(3)],
                vec![Value::I64(1)],
                vec![Value::I64(2)],
            ],
        );

        let ascending = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 2,
        };
        assert_eq!(
            ascending.evaluate(&model, &context, &registry),
            Ok(RelationValue::Bag(vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(2)],
            ]))
        );

        let descending = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 2,
        };
        assert_eq!(
            descending.evaluate(&model, &context, &registry),
            Ok(RelationValue::Bag(vec![
                vec![Value::I64(3)],
                vec![Value::I64(2)],
                vec![Value::I64(2)],
            ]))
        );
    }

    #[test]
    fn prepared_top_k_keeps_bound_ordering_execution_after_registry_replacement() {
        let relation = SemanticId::new(225);
        let i64_eq = SemanticId::new(226);
        let i64_order = SemanticId::new(227);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(225));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(225));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let prepared = query.prepare(&context, &registry).unwrap();
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::I64(3)],
                vec![Value::I64(1)],
                vec![Value::I64(1)],
            ],
        );

        let replacement_registry = SemanticRegistry::default();
        assert_eq!(
            prepared.evaluate(&model, &context, &replacement_registry),
            Ok(RelationValue::Bag(vec![
                vec![Value::I64(1)],
                vec![Value::I64(1)],
            ]))
        );
    }

    #[test]
    fn top_k_rejects_ordering_with_wrong_semantic_domain_before_execution() {
        let relation = SemanticId::new(230);
        let i64_eq = SemanticId::new(231);
        let text_order = SemanticId::new(232);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::TextBinary);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(230));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(text_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(230));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: text_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }

    #[test]
    fn top_k_requires_ordering_congruent_with_relation_equality() {
        let relation = SemanticId::new(240);
        let text_eq = SemanticId::new(241);
        let binary_order = SemanticId::new(242);
        let ci_order = SemanticId::new(243);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let binary_digest = registry.install_ordering(OrderingModule::TextBinary);
        let ci_digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(240));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(binary_order, binary_digest);
        environment.pin_module(ci_order, ci_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(240));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };

        let incompatible = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: binary_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        assert_eq!(
            incompatible.prepare(&context, &registry),
            Err(RelQueryError::OrderingNotCongruentWithEquality)
        );

        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("b".into())],
                vec![Value::Text("A".into())],
                vec![Value::Text("a".into())],
            ],
        );
        let compatible = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: ci_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        assert_eq!(
            compatible.evaluate(&model, &context, &registry),
            Ok(RelationValue::Bag(vec![
                vec![Value::Text("A".into())],
                vec![Value::Text("a".into())],
            ]))
        );
    }

    #[test]
    fn filter_rejects_equality_that_can_observe_relation_representatives() {
        let relation = SemanticId::new(245);
        let relation_eq = SemanticId::new(246);
        let exact_eq = SemanticId::new(247);
        let mut registry = SemanticRegistry::default();
        let relation_digest =
            registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(245));
        environment.pin_module(relation_eq, relation_digest);
        environment.pin_module(exact_eq, exact_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(245));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![relation_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };

        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("A".into()),
            equivalence: exact_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );
    }

    #[test]
    fn coarser_query_equality_is_safe_over_finer_relation_equality() {
        let relation = SemanticId::new(248);
        let exact_eq = SemanticId::new(249);
        let ci_eq = SemanticId::new(2500);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(248));
        environment.pin_module(exact_eq, exact_digest);
        environment.pin_module(ci_eq, ci_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(248));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![exact_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("A".into()),
            equivalence: ci_eq,
        };
        assert!(query.prepare(&context, &registry).is_ok());
    }

    #[test]
    fn equality_operators_reject_representative_observing_refinements() {
        let left = SemanticId::new(2510);
        let right = SemanticId::new(2511);
        let ci_eq = SemanticId::new(2512);
        let exact_eq = SemanticId::new(2513);
        let i64_eq = SemanticId::new(2514);
        let mut registry = SemanticRegistry::default();
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2510));
        environment.pin_module(ci_eq, ci_digest);
        environment.pin_module(exact_eq, exact_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2510));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::Text),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![ci_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };

        let join = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: exact_eq,
        };
        assert_eq!(
            join.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );

        let distinct = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(left)),
            column_equivalences: vec![exact_eq, i64_eq],
        };
        assert_eq!(
            distinct.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );

        let group = RelExpr::Group {
            input: Box::new(RelExpr::Scan(left)),
            group_columns: vec![0],
            group_equivalences: vec![exact_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        assert_eq!(
            group.prepare(&context, &registry),
            Err(RelQueryError::EquivalenceNotCongruentWithInputEquality)
        );
    }

    #[test]
    fn relational_impact_uses_bag_semantics_not_rust_row_representation() {
        let relation = SemanticId::new(250);
        let text_eq = SemanticId::new(251);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(250));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(250));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into())]]);
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("a".into())]]);
        assert_eq!(
            rel_impact_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            ),
            Impact::Unaffected
        );
        assert_eq!(
            rel_derivative_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace({
                    let mut changed = FiniteModel::default();
                    changed
                        .relations
                        .insert(relation, vec![vec![Value::Text("a".into())]]);
                    changed
                }),
                &context,
                &registry,
            ),
            Change::NoChange
        );
        let delta = rel_delta_by_recompute(
            &RelExpr::Scan(relation),
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        assert!(delta.is_empty());
        assert_eq!(
            delta.result_type,
            RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap()
        );
    }
    #[test]
    fn relational_bag_impact_ignores_physical_row_order() {
        let (context, registry, _, relation, _) = setup();
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("B".into()), Value::I64(2)],
            ],
        );
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![
                vec![Value::Text("B".into()), Value::I64(2)],
                vec![Value::Text("A".into()), Value::I64(1)],
            ],
        );
        assert_eq!(
            rel_impact_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            ),
            Impact::Unaffected
        );
    }

    #[test]
    fn relational_bag_impact_detects_multiplicity_change() {
        let (context, registry, _, relation, _) = setup();
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(1)],
            ],
        );
        assert_eq!(
            rel_impact_by_recompute(
                &RelExpr::Scan(relation),
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            ),
            Impact::Changed
        );
        let delta = rel_delta_by_recompute(
            &RelExpr::Scan(relation),
            &old,
            &Change::Replace(next),
            &context,
            &registry,
        )
        .unwrap();
        assert!(delta.removed.is_empty());
        assert_eq!(delta.inserted.len(), 1);
        assert!(matches!(&delta.inserted[0][0], Value::Text(_)));
    }

    #[test]
    fn optimized_scan_filter_project_delta_matches_recompute_oracle() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::Text("ALPHA".into()),
                equivalence: text_eq,
            }),
            columns: vec![0],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        let mut next = old.clone();
        next.relations
            .get_mut(&relation)
            .unwrap()
            .push(vec![Value::Text("alpha".into()), Value::I64(3)]);
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("scan/filter/bag-project path is supported");
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
        assert_eq!(optimized.inserted.len(), 1);
        assert!(optimized.removed.is_empty());
    }

    #[test]
    fn optimized_bag_project_cancels_projected_replacement_pairs() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::Text("ALPHA".into()),
                equivalence: text_eq,
            }),
            columns: vec![0],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![vec![Value::Text("Alpha".into()), Value::I64(1)]],
        );
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![vec![Value::Text("alpha".into()), Value::I64(2)]],
        );
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("scan/filter/bag-project path is supported");
        assert!(oracle.is_empty());
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
    }

    #[test]
    fn optimized_delta_matches_oracle_over_small_hostile_bag_state_space() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::Text("A".into()),
                equivalence: text_eq,
            }),
            columns: vec![0],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let model_for = |mask: u8| {
                    let mut model = FiniteModel::default();
                    model.relations.insert(
                        relation,
                        rows.iter()
                            .enumerate()
                            .filter(|(index, _)| mask & (1 << index) != 0)
                            .map(|(_, row)| row.clone())
                            .collect(),
                    );
                    model
                };
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("scan/filter/bag-project path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn materialized_set_support_state_updates_without_rebuilding_old_rows() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
        };
        let row_upper = vec![Value::Text("A".into())];
        let row_lower = vec![Value::Text("a".into())];
        let mut state = MaterializedSetSupportState::build(
            &[row_upper.clone(), row_lower.clone()],
            result_type,
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(
            state
                .support_count(&row_upper, &context, &registry)
                .unwrap(),
            2
        );

        let first = state
            .apply_rows_delta(Vec::new(), vec![row_upper.clone()], &context, &registry)
            .unwrap();
        assert!(first.is_empty());
        assert_eq!(
            state
                .support_count(&row_lower, &context, &registry)
                .unwrap(),
            1
        );

        let second = state
            .apply_rows_delta(Vec::new(), vec![row_lower.clone()], &context, &registry)
            .unwrap();
        assert!(second.inserted.is_empty());
        assert_eq!(second.removed.len(), 1);
        assert_eq!(
            state
                .support_count(&row_upper, &context, &registry)
                .unwrap(),
            0
        );

        let third = state
            .apply_rows_delta(vec![row_lower.clone()], Vec::new(), &context, &registry)
            .unwrap();
        assert_eq!(third.inserted.len(), 1);
        assert!(third.removed.is_empty());
        assert_eq!(
            state
                .support_count(&row_upper, &context, &registry)
                .unwrap(),
            1
        );
    }

    #[test]
    fn materialized_set_support_state_is_context_bound_and_atomic_on_error() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
        };
        let row = vec![Value::Text("A".into())];
        let mut state = MaterializedSetSupportState::build(
            std::slice::from_ref(&row),
            result_type,
            &context,
            &registry,
        )
        .unwrap();
        let before = state.clone();
        assert_eq!(
            state.apply_rows_delta(vec![vec![Value::I64(7)]], Vec::new(), &context, &registry,),
            Err(RelQueryError::TypeMismatch)
        );
        assert_eq!(state, before);
        assert_eq!(
            state.apply_rows_delta(Vec::new(), vec![row.clone(), row], &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);

        let mut other_context = context.clone();
        other_context.environment = SemanticEnvironment::new(SemanticEnvId::new(9999));
        assert_eq!(
            state.support_count(&vec![Value::Text("A".into())], &other_context, &registry),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }

    #[test]
    fn zero_crossing_kernel_plans_without_mutating_and_commits_once() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
        };
        let upper = vec![Value::Text("A".into())];
        let lower = vec![Value::Text("a".into())];
        let mut state = MaterializedSetSupportState::build(
            &[upper.clone(), lower.clone()],
            result_type,
            &context,
            &registry,
        )
        .unwrap();

        let remove_both = CompactDelta::Two(
            Weighted {
                weight: -1,
                row: upper.clone(),
            },
            Weighted {
                weight: -1,
                row: lower.clone(),
            },
        );
        let planned = state
            .plan_delta_view(&remove_both, &context, &registry)
            .unwrap();
        assert_eq!(state.support_count(&upper, &context, &registry).unwrap(), 2);
        assert_eq!(planned.effect.support_len(), 1);
        state.commit_support_patch(planned.patch);
        assert_eq!(state.support_count(&upper, &context, &registry).unwrap(), 0);

        let invalid = CompactDelta::one(-1, upper.clone());
        let before = state.clone();
        assert_eq!(
            state.plan_delta_view(&invalid, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn project_bag_keeps_max_weight_compact_and_cancels_semantically() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let result_type = query.typecheck(&context, &registry).unwrap();
        let positive = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let negative = vec![Value::Text("alpha".into()), Value::I64(2)];
        let cancel = CompactDelta::Two(
            Weighted {
                weight: i64::MAX,
                row: positive.clone(),
            },
            Weighted {
                weight: -i64::MAX,
                row: negative,
            },
        );
        let cancelled =
            project_bag_delta_view(&cancel, &[0], &result_type, &context, &registry).unwrap();
        assert_eq!(cancelled.support_len(), 0);

        let one = CompactDelta::one(i64::MAX, positive);
        let projected =
            project_bag_delta_view(&one, &[0], &result_type, &context, &registry).unwrap();
        let mut effect = Vec::new();
        projected.visit_exact(|weight, row| {
            effect.push((
                MaterializedJoinDeltaState::exact_integer_to_i64(weight).unwrap(),
                row.clone(),
            ));
        });
        assert_eq!(effect, vec![(i64::MAX, vec![Value::Text("Alpha".into())])]);
    }

    #[test]
    fn project_bag_keeps_exact_coefficient_beyond_i64_as_one_atom() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let result_type = query.typecheck(&context, &registry).unwrap();
        let row = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let coefficient = kernel_exact::ExactInteger::from_i64(i64::MAX)
            .scale_by_natural(&kernel_exact::ExactNatural::from_u64(2));
        let mut input = ExactDelta::default();
        input.push_exact(coefficient.clone(), row);

        let projected =
            project_bag_delta_view(&input, &[0], &result_type, &context, &registry).unwrap();
        assert_eq!(projected.support_len(), 1);
        projected.visit_exact(|weight, row| {
            assert_eq!(weight, &coefficient);
            assert_eq!(row, &vec![Value::Text("Alpha".into())]);
        });
    }

    #[test]
    fn set_support_tracks_exact_multiplicity_beyond_i64_without_expansion() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            semantics: RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
            columns: vec![kernel_schema::TypeExpr::Scalar(
                kernel_schema::ScalarType::Text,
            )],
        };
        let row = vec![Value::Text("Alpha".into())];
        let mut state =
            MaterializedSetSupportState::build(&[], result_type, &context, &registry).unwrap();
        let magnitude = kernel_exact::ExactNatural::from_u128(u128::from(i64::MAX as u64) + 7);
        let mut inserted = ExactDelta::default();
        inserted.push_exact(
            kernel_exact::ExactInteger::from_parts(false, magnitude.clone()),
            row.clone(),
        );
        let planned = state
            .plan_delta_view(&inserted, &context, &registry)
            .unwrap();
        assert_eq!(planned.effect.support_len(), 1);
        state.commit_support_patch(planned.patch);
        assert_eq!(state.test_support_count_exact_at(0), Some(&magnitude));

        let mut removed = ExactDelta::default();
        removed.push_exact(kernel_exact::ExactInteger::from_parts(true, magnitude), row);
        let planned = state
            .plan_delta_view(&removed, &context, &registry)
            .unwrap();
        assert_eq!(planned.effect.support_len(), 1);
        state.commit_support_patch(planned.patch);
        assert!(state.test_support_count_exact_at(0).is_some_and(kernel_exact::ExactNatural::is_zero));
    }

    #[test]
    fn exact_quotient_materialization_cancels_beyond_i64_before_observation() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![text_eq],
            },
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
        };
        let magnitude = kernel_exact::ExactNatural::from_u128(u128::from(i64::MAX as u64) + 17);
        let mut delta = ExactDelta::default();
        delta.push_exact(
            kernel_exact::ExactInteger::from_parts(false, magnitude.clone()),
            vec![Value::Text("Alpha".into())],
        );
        delta.push_exact(
            kernel_exact::ExactInteger::from_parts(true, magnitude),
            vec![Value::Text("alpha".into())],
        );
        delta.push_exact(
            kernel_exact::ExactInteger::from_i64(1),
            vec![Value::Text("ALPHA".into())],
        );

        let materialized = materialize_exact_quotient_delta_view(
            &delta,
            result_type.clone(),
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(materialized.result_type, result_type);
        assert_eq!(materialized.removed, Vec::<Row>::new());
        assert_eq!(materialized.inserted.len(), 1);
        assert!(registry
            .equivalent(
                &context,
                text_eq,
                &materialized.inserted[0][0],
                &Value::Text("alpha".into()),
            )
            .unwrap());
    }

    #[test]
    fn relation_delta_indexed_apply_preserves_first_representative_and_row_order() {
        let (context, registry, text_eq, _, _) = setup();
        let result_type = RelType {
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![text_eq],
            },
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
        };
        let old = RelationValue::Bag(vec![
            vec![Value::Text("Alpha".into())],
            vec![Value::Text("aLPHa".into())],
            vec![Value::Text("Beta".into())],
        ]);
        let delta = RelationDelta {
            removed: vec![vec![Value::Text("ALPHA".into())]],
            inserted: vec![vec![Value::Text("Gamma".into())]],
            result_type,
        };

        assert_eq!(
            delta.apply_to_value(old, &context, &registry).unwrap(),
            RelationValue::Bag(vec![
                vec![Value::Text("aLPHa".into())],
                vec![Value::Text("Beta".into())],
                vec![Value::Text("Gamma".into())],
            ])
        );
    }

    #[test]
    fn materialized_group_applies_max_weight_without_expanding_multiplicity() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: SemanticId::new(101),
            },
        };
        let mut model = FiniteModel::default();
        model.relations.insert(relation, Vec::new());
        let state = MaterializedGroupDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let row = vec![Value::Text("A".into()), Value::I64(1)];
        let delta = CompactDelta::one(i64::MAX, row);
        let planned_effect = state
            .test_plan_legacy_effect(&delta, &context, &registry)
            .unwrap();
        let mut effect = Vec::new();
        planned_effect.visit(|weight, row| effect.push((weight, row.clone())));
        assert_eq!(
            effect,
            vec![(1, vec![Value::Text("A".into()), Value::I64(i64::MAX)],)]
        );
    }

    #[test]
    fn materialized_group_count_state_matches_recompute_across_sequential_changes() {
        let (context, registry, text_eq, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };
        let sequence = [0_u8, 3, 2, 6, 7, 4, 5, 1, 0];
        let mut old = model_for(sequence[0]);
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("group state is supported");
        assert!(state.test_has_semantic_lookup());
        assert!(state.test_group_encoder_count().is_some());
        for &mask in &sequence[1..] {
            let next = model_for(mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next mask {mask:03b}: maintained={maintained:?} oracle={oracle:?}"
            );
            old = next;
        }
        assert_eq!(state.group_count(), 0);
    }

    #[test]
    fn materialized_exact_f64_group_state_matches_recompute_with_deletions() {
        let relation = SemanticId::new(2580);
        let text_eq = SemanticId::new(2581);
        let f64_eq = SemanticId::new(2582);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2580));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2580));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::F64Bits(0.1_f64.to_bits())],
            vec![Value::Text("a".into()), Value::F64Bits(0.2_f64.to_bits())],
            vec![
                Value::Text("B".into()),
                Value::F64Bits((-3.5_f64).to_bits()),
            ],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };
        let sequence = [0_u8, 7, 6, 2, 3, 1, 5, 4, 0];
        let mut old = model_for(sequence[0]);
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("exact group state is supported");
        for &mask in &sequence[1..] {
            let next = model_for(mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next mask {mask:03b}: maintained={maintained:?} oracle={oracle:?}"
            );
            old = next;
        }
        assert_eq!(state.group_count(), 0);
    }

    #[test]
    fn materialized_global_group_count_preserves_empty_identity_row() {
        let (context, registry, _, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![],
            group_equivalences: vec![],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let mut empty = FiniteModel::default();
        empty.relations.insert(relation, Vec::new());
        let mut one = FiniteModel::default();
        one.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        let mut state = MaterializedGroupDeltaState::build(&query, &empty, &context, &registry)
            .unwrap()
            .unwrap();
        for (old, next) in [
            (&empty, &one),
            (&one, &empty),
            (&empty, &one),
            (&one, &empty),
        ] {
            let change = Change::Replace(next.clone());
            let oracle = rel_delta_by_recompute(&query, old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
        }
        assert_eq!(state.group_count(), 1);
    }

    #[test]
    fn materialized_group_input_delta_rejects_malformed_or_underflowing_change_atomically() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: SemanticId::new(101),
            },
        };
        let mut model = FiniteModel::default();
        let row = vec![Value::Text("A".into()), Value::I64(1)];
        model.relations.insert(relation, vec![row.clone()]);
        let mut state = MaterializedGroupDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let before = state.clone();
        let input_type = RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap();
        let malformed = RelationDelta {
            inserted: vec![vec![Value::Text("B".into()), Value::Text("wrong".into())]],
            removed: Vec::new(),
            result_type: input_type.clone(),
        };
        assert_eq!(
            state.apply_input_delta(&malformed, &context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
        assert_eq!(state, before);

        let underflow = RelationDelta {
            inserted: Vec::new(),
            removed: vec![row.clone(), row],
            result_type: input_type,
        };
        assert_eq!(
            state.apply_input_delta(&underflow, &context, &registry),
            Err(RelQueryError::Aggregate(
                kernel_aggregate::AggregateError::CountUnderflow
            ))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn materialized_i64_count_fast_path_matches_recompute_sequentially() {
        let relation = SemanticId::new(2590);
        let i64_eq = SemanticId::new(2591);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2590));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2590));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let model_for = |values: &[i64]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                values
                    .iter()
                    .map(|value| vec![Value::I64(*value)])
                    .collect(),
            );
            model
        };
        let states = [
            vec![1, 1, 2],
            vec![1, 2, 3],
            vec![3, 3],
            vec![],
            vec![2, 2, 2],
        ];
        let mut old = model_for(&states[0]);
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        for values in &states[1..] {
            let next = model_for(values);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn dense_i64_group_plans_without_mutating_and_matches_recompute() {
        let relation = SemanticId::new(2592);
        let i64_eq = SemanticId::new(2593);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2592));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2592));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::I64(0)],
                vec![Value::I64(1)],
                vec![Value::I64(2)],
            ],
        );
        let mut next = old.clone();
        next.relations.insert(
            relation,
            vec![
                vec![Value::I64(1)],
                vec![Value::I64(2)],
                vec![Value::I64(3)],
            ],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_dense_i64_count_enabled());

        let carrier = CompactDelta::replace(vec![Value::I64(0)], vec![Value::I64(3)]);
        let observation = state.test_plan_commit_i64_count(&carrier).unwrap();
        assert!(observation.retain_dense);
        assert_eq!(observation.dense_move, Some((0, 3)));
        assert!(observation.unchanged_before_commit);

        let change = Change::Replace(next);
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = materialize_delta_view(&observation.effect, state.result_type().clone()).unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
        assert!(state.test_dense_i64_count_enabled());
    }

    #[test]
    fn dense_i64_group_outlier_falls_back_before_commit_without_semantic_error() {
        let relation = SemanticId::new(2594);
        let i64_eq = SemanticId::new(2595);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2594));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2594));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::I64(0)], vec![Value::I64(1)]]);
        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![vec![Value::I64(1)], vec![Value::I64(10_000)]],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_dense_i64_count_enabled());

        let carrier = CompactDelta::replace(vec![Value::I64(0)], vec![Value::I64(10_000)]);
        let observation = state.test_plan_commit_i64_count(&carrier).unwrap();
        assert!(!observation.retain_dense);
        assert!(observation.unchanged_before_commit);
        assert!(!state.test_dense_i64_count_enabled());

        let change = Change::Replace(next);
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = materialize_delta_view(&observation.effect, state.result_type().clone()).unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
    }

    #[test]
    fn optimized_set_project_uses_support_counts_when_projection_hides_a_removal() {
        let relation = SemanticId::new(2520);
        let text_eq = SemanticId::new(2521);
        let i64_eq = SemanticId::new(2522);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2520));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2520));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("A".into()), Value::I64(2)],
            ],
        );
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(2)]]);
        let change = Change::Replace(next);

        assert!(
            rel_delta_by_recompute(&query, &old, &change, &context, &registry)
                .unwrap()
                .is_empty()
        );
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("set projection support-count path is supported");
        assert!(optimized.is_empty());
    }

    #[test]
    fn materialized_support_state_matches_set_projection_oracle_over_all_small_transitions() {
        let relation = SemanticId::new(2525);
        let text_eq = SemanticId::new(2526);
        let i64_eq = SemanticId::new(2527);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2525));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2525));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let result_type = query.typecheck(&context, &registry).unwrap();
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("A".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(1)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let old_projected = project_rows(
                    old.relations.materialize_owned(&relation).unwrap_or_default(),
                    &[0],
                )
                .unwrap();
                let mut state = MaterializedSetSupportState::build(
                    &old_projected,
                    result_type.clone(),
                    &context,
                    &registry,
                )
                .unwrap();
                let scan_delta = rel_delta_optimized(
                    &RelExpr::Scan(relation),
                    &old,
                    &change,
                    &context,
                    &registry,
                )
                .unwrap()
                .expect("scan delta is supported");
                let maintained = state
                    .apply_rows_delta(
                        project_rows(scan_delta.inserted, &[0]).unwrap(),
                        project_rows(scan_delta.removed, &[0]).unwrap(),
                        &context,
                        &registry,
                    )
                    .unwrap();
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                assert!(
                    relation_deltas_semantically_equivalent(
                        &maintained,
                        &oracle,
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn long_lived_materialized_project_state_survives_sequential_model_changes() {
        let relation = SemanticId::new(2528);
        let text_eq = SemanticId::new(2529);
        let i64_eq = SemanticId::new(2539);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2528));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2528));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("A".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(1)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        let mut old = model_for(0);
        let mut state = MaterializedRelDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("set projection has materialized state");
        for next_mask in [3_u8, 2, 6, 0, 5, 1, 7, 4] {
            let next = model_for(next_mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
            );
            old = next;
        }
    }

    #[test]
    fn long_lived_materialized_distinct_state_survives_sequential_model_changes() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        let mut old = model_for(0);
        let mut state = MaterializedRelDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .expect("distinct has materialized state");
        for next_mask in [3_u8, 2, 6, 0, 5, 1, 7, 4] {
            let next = model_for(next_mask);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap(),
                "next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
            );
            old = next;
        }
    }

    #[test]
    fn optimized_set_project_matches_oracle_over_small_support_state_space() {
        let relation = SemanticId::new(2530);
        let text_eq = SemanticId::new(2531);
        let i64_eq = SemanticId::new(2532);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2530));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2530));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("A".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(1)],
        ];

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let model_for = |mask: u8| {
                    let mut model = FiniteModel::default();
                    model.relations.insert(
                        relation,
                        rows.iter()
                            .enumerate()
                            .filter(|(index, _)| mask & (1 << index) != 0)
                            .map(|(_, row)| row.clone())
                            .collect(),
                    );
                    model
                };
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("set projection support-count path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn materialized_support_state_matches_distinct_oracle_over_all_small_transitions() {
        let (context, registry, text_eq, relation, _) = setup();
        let child = RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        };
        let query = RelExpr::Distinct {
            input: Box::new(child.clone()),
            column_equivalences: vec![text_eq],
        };
        let result_type = query.typecheck(&context, &registry).unwrap();
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let next = model_for(next_mask);
                let change = Change::Replace(next);
                let old_child = child
                    .evaluate(&old, &context, &registry)
                    .unwrap()
                    .into_rows();
                let mut state = MaterializedSetSupportState::build(
                    &old_child,
                    result_type.clone(),
                    &context,
                    &registry,
                )
                .unwrap();
                let child_delta = rel_delta_optimized(&child, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("bag project child delta is supported");
                let maintained = state
                    .apply_rows_delta(
                        child_delta.inserted,
                        child_delta.removed,
                        &context,
                        &registry,
                    )
                    .unwrap();
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                assert!(
                    relation_deltas_semantically_equivalent(
                        &maintained,
                        &oracle,
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, maintained={maintained:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_distinct_support_counts_survive_duplicate_removal() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(2)],
            ],
        );
        let mut next = FiniteModel::default();
        next.relations
            .insert(relation, vec![vec![Value::Text("a".into()), Value::I64(2)]]);
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("distinct support-count path is supported");
        assert!(oracle.is_empty());
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
    }

    #[test]
    fn optimized_distinct_matches_oracle_over_small_hostile_state_space() {
        let (context, registry, text_eq, relation, _) = setup();
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let model_for = |mask: u8| {
                    let mut model = FiniteModel::default();
                    model.relations.insert(
                        relation,
                        rows.iter()
                            .enumerate()
                            .filter(|(index, _)| mask & (1 << index) != 0)
                            .map(|(_, row)| row.clone())
                            .collect(),
                    );
                    model
                };
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("bag project + distinct support-count path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_promote_to_bag_transports_input_delta() {
        let (context, registry, _, relation, _) = setup();
        let query = RelExpr::PromoteToBag(Box::new(RelExpr::Scan(relation)));
        let mut old = FiniteModel::default();
        old.relations
            .insert(relation, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        let mut next = old.clone();
        next.relations
            .get_mut(&relation)
            .unwrap()
            .push(vec![Value::Text("B".into()), Value::I64(2)]);
        let change = Change::Replace(next);

        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
            .unwrap()
            .expect("promote-to-bag path is supported");
        assert!(
            relation_deltas_semantically_equivalent(&optimized, &oracle, &context, &registry)
                .unwrap()
        );
    }

    #[test]
    fn optimized_join_local_replay_matches_oracle_over_two_sided_state_space() {
        let (context, registry, text_eq, left, right) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let left_rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("B".into()), Value::I64(2)],
        ];
        let right_rows = [
            vec![Value::Text("a".into()), Value::I64(10)],
            vec![Value::Text("B".into()), Value::I64(20)],
        ];

        let model_for = |state: u8| {
            let left_mask = state & 0b0011;
            let right_mask = (state >> 2) & 0b0011;
            let mut model = FiniteModel::default();
            model.relations.insert(
                left,
                left_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| left_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model.relations.insert(
                right,
                right_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| right_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_state in 0_u8..16 {
            for next_state in 0_u8..16 {
                let old = model_for(old_state);
                let change = Change::Replace(model_for(next_state));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("join local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_state:04b}, next={next_state:04b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_top_k_local_replay_matches_oracle_across_threshold_changes() {
        let relation = SemanticId::new(2540);
        let i64_eq = SemanticId::new(2541);
        let i64_order = SemanticId::new(2542);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2540));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2540));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let rows = [
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(2)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("top-k local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_group_count_local_replay_matches_oracle_across_group_birth_and_death() {
        let (context, registry, text_eq, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("B".into()), Value::I64(3)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("group local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_global_group_count_handles_empty_identity_transitions() {
        let (context, registry, _, relation, _) = setup();
        let count_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![],
            group_equivalences: vec![],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("B".into()), Value::I64(2)],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..4 {
            for next_mask in 0_u8..4 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("global group local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:02b}, next={next_mask:02b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_exact_f64_group_local_replay_matches_oracle() {
        let relation = SemanticId::new(2550);
        let text_eq = SemanticId::new(2551);
        let f64_eq = SemanticId::new(2552);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2550));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(2550));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        let rows = [
            vec![Value::Text("A".into()), Value::F64Bits(1.0_f64.to_bits())],
            vec![Value::Text("a".into()), Value::F64Bits(2.0_f64.to_bits())],
            vec![Value::Text("B".into()), Value::F64Bits(4.0_f64.to_bits())],
        ];
        let model_for = |mask: u8| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                rows.iter()
                    .enumerate()
                    .filter(|(index, _)| mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_mask in 0_u8..8 {
            for next_mask in 0_u8..8 {
                let old = model_for(old_mask);
                let change = Change::Replace(model_for(next_mask));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("exact-sum group local-replay path is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_mask:03b}, next={next_mask:03b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn optimized_composed_set_distinct_join_promote_pipeline_matches_oracle() {
        let (context, registry, text_eq, left, right) = setup();
        let distinct_text = |relation| RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Scan(relation)),
                columns: vec![0],
            }),
            column_equivalences: vec![text_eq],
        };
        let query = RelExpr::PromoteToBag(Box::new(RelExpr::JoinEq {
            left: Box::new(distinct_text(left)),
            right: Box::new(distinct_text(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        }));
        let left_rows = [
            vec![Value::Text("A".into()), Value::I64(1)],
            vec![Value::Text("a".into()), Value::I64(2)],
        ];
        let right_rows = [
            vec![Value::Text("A".into()), Value::I64(10)],
            vec![Value::Text("B".into()), Value::I64(20)],
        ];
        let model_for = |state: u8| {
            let left_mask = state & 0b0011;
            let right_mask = (state >> 2) & 0b0011;
            let mut model = FiniteModel::default();
            model.relations.insert(
                left,
                left_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| left_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model.relations.insert(
                right,
                right_rows
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| right_mask & (1 << index) != 0)
                    .map(|(_, row)| row.clone())
                    .collect(),
            );
            model
        };

        for old_state in 0_u8..16 {
            for next_state in 0_u8..16 {
                let old = model_for(old_state);
                let change = Change::Replace(model_for(next_state));
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let optimized = rel_delta_optimized(&query, &old, &change, &context, &registry)
                    .unwrap()
                    .expect("composed incremental pipeline is supported");
                assert!(
                    relation_deltas_semantically_equivalent(
                        &optimized, &oracle, &context, &registry,
                    )
                    .unwrap(),
                    "old={old_state:04b}, next={next_state:04b}, optimized={optimized:?}, oracle={oracle:?}"
                );
            }
        }
    }

    #[test]
    fn scan_preserves_set_semantics_from_schema() {
        let relation = SemanticId::new(300);
        let text_eq = SemanticId::new(301);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![vec![Value::Text("x".into())]]);
        assert_eq!(
            RelExpr::Scan(relation).evaluate(&model, &context, &registry),
            Ok(RelationValue::Set {
                rows: vec![vec![Value::Text("x".into())]],
                column_equivalences: vec![text_eq],
            })
        );
    }
    #[test]
    fn typecheck_rejects_wrong_equality_even_for_empty_relation() {
        let relation = SemanticId::new(400);
        let wrong_eq = SemanticId::new(401);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(wrong_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![wrong_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![wrong_eq],
        };
        assert!(matches!(
            query.prepare(&context, &registry),
            Err(RelQueryError::Semantic(
                kernel_semantics::SemanticError::EquivalenceDomainMismatch { .. }
            ))
        ));
    }
    #[test]
    fn typecheck_rejects_wrong_constant_type_on_empty_relation() {
        let relation = SemanticId::new(500);
        let text_eq = SemanticId::new(501);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(7),
            equivalence: text_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }

    #[test]
    fn join_rejects_unrelated_entity_reference_types() {
        let left_relation = SemanticId::new(510);
        let right_relation = SemanticId::new(511);
        let person = SemanticId::new(512);
        let order = SemanticId::new(513);
        let entity_eq = SemanticId::new(514);
        let order_eq = SemanticId::new(515);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let order_digest =
            registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(order));
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(entity_eq, digest);
        environment.pin_module(order_eq, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: left_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(person))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![entity_eq],
                },
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: right_relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(order))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![order_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left_relation)),
            right: Box::new(RelExpr::Scan(right_relation)),
            left_column: 0,
            right_column: 0,
            equivalence: entity_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }
    #[test]
    fn prepared_query_pins_full_semantic_context_not_only_revision_numbers() {
        let relation = SemanticId::new(600);
        let text_eq = SemanticId::new(601);
        let i64_eq = SemanticId::new(602);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut base_environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        base_environment.pin_module(text_eq, text_digest);
        base_environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: base_environment.clone(),
        };
        let prepared = RelExpr::Scan(relation)
            .prepare(&context, &registry)
            .unwrap();

        let mut changed_schema = Schema::new(SchemaRevisionId::new(1));
        changed_schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let changed_context = SemanticContext {
            schema: changed_schema,
            environment: base_environment,
        };
        assert_eq!(
            prepared.evaluate(&FiniteModel::default(), &changed_context, &registry),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }

    #[test]
    fn prepared_semantic_query_can_rebind_across_same_contract_implementation_upgrade() {
        let relation = SemanticId::new(650);
        let text_eq = SemanticId::new(651);
        let mut registry = SemanticRegistry::default();
        let old = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let new = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
        let changed = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let make_context = |revision, digest| {
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
            environment.pin_module(text_eq, digest);
            SemanticContext {
                schema: schema.clone(),
                environment,
            }
        };
        let source = make_context(1, old);
        let target = make_context(2, new);
        let changed_law = make_context(3, changed);
        let prepared = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("x".into()),
            equivalence: text_eq,
        }
        .prepare(&source, &registry)
        .unwrap();

        let rebound = prepared
            .rebind_preserving_semantics(&target, &registry)
            .unwrap();
        assert_eq!(rebound.result_type(), prepared.result_type());
        assert_eq!(
            prepared.rebind_preserving_semantics(&changed_law, &registry),
            Err(RelQueryError::SemanticRevisionMismatch)
        );
    }
    #[test]
    fn exact_f64_group_sum_is_reproducible_and_semantically_typed() {
        let relation = SemanticId::new(800);
        let text_eq = SemanticId::new(801);
        let f64_eq = SemanticId::new(802);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("x".into()), Value::F64Bits(1e16_f64.to_bits())],
                vec![Value::Text("x".into()), Value::F64Bits(1.0_f64.to_bits())],
                vec![
                    Value::Text("x".into()),
                    Value::F64Bits((-1e16_f64).to_bits()),
                ],
                vec![Value::Text("y".into()), Value::F64Bits(2.0_f64.to_bits())],
            ],
        );
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        let prepared = query.prepare(&context, &registry).unwrap();
        assert_eq!(
            prepared.result_type(),
            &RelType {
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            }
        );
        assert_eq!(
            prepared.evaluate(&model, &context, &registry).unwrap(),
            RelationValue::Set {
                rows: vec![
                    vec![Value::Text("x".into()), Value::F64Bits(1.0_f64.to_bits())],
                    vec![Value::Text("y".into()), Value::F64Bits(2.0_f64.to_bits())],
                ],
                column_equivalences: vec![text_eq, f64_eq],
            }
        );
    }

    #[test]
    fn exact_f64_group_sum_rejects_non_finite_input_explicitly() {
        let relation = SemanticId::new(810);
        let text_eq = SemanticId::new(811);
        let f64_eq = SemanticId::new(812);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(f64_eq, f64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![
                Value::Text("x".into()),
                Value::F64Bits(f64::NAN.to_bits()),
            ]],
        );
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::ExactF64Sum {
                value_column: 1,
                result_equivalence: f64_eq,
            },
        };
        assert_eq!(
            query.evaluate(&model, &context, &registry),
            Err(RelQueryError::Aggregate(
                kernel_aggregate::AggregateError::NonFiniteInput
            ))
        );
    }
    #[test]
    fn filter_rejects_reference_literal_with_wrong_nominal_type_before_execution() {
        let relation = SemanticId::new(820);
        let person = SemanticId::new(821);
        let order = SemanticId::new(822);
        let entity_eq = SemanticId::new(823);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(entity_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(person))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![entity_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::LiveEntityRef {
                entity_type: order,
                id: kernel_types::EntityId::new(1),
            },
            equivalence: entity_eq,
        };
        assert_eq!(
            query.prepare(&context, &registry),
            Err(RelQueryError::TypeMismatch)
        );
    }
    #[test]
    fn distinct_supports_schema_derived_structural_product_equality() {
        let relation = SemanticId::new(830);
        let product_eq = SemanticId::new(831);
        let text_eq = SemanticId::new(832);
        let i64_eq = SemanticId::new(833);
        let name = SemanticId::new(834);
        let age = SemanticId::new(835);
        let product_type = TypeExpr::Product(std::collections::BTreeMap::from([
            (name, TypeExpr::Scalar(ScalarType::Text)),
            (age, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: std::collections::BTreeMap::from([(name, text_eq), (age, i64_eq)]),
                },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![product_type],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![product_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let row = |label: &str| {
            vec![Value::Product(std::collections::BTreeMap::from([
                (name, Value::Text(label.into())),
                (age, Value::I64(30)),
            ]))]
        };
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![row("ALICE"), row("alice")]);
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![product_eq],
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows().len(), 1);

        let state = MaterializedSetSupportState::build(
            &[row("ALICE"), row("alice")],
            RelType {
                columns: vec![TypeExpr::Product(std::collections::BTreeMap::from([
                    (name, TypeExpr::Scalar(ScalarType::Text)),
                    (age, TypeExpr::Scalar(ScalarType::I64)),
                ]))],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![product_eq],
                },
            },
            &context,
            &registry,
        )
        .unwrap();
        assert_eq!(state.test_support_class_count(), 1);
        assert_eq!(
            state.support_count(&row("aLiCe"), &context, &registry),
            Ok(2)
        );
    }
    #[test]
    fn filter_accepts_structural_option_literal_with_derived_equivalence() {
        let relation = SemanticId::new(900);
        let text_eq = SemanticId::new(901);
        let option_eq = SemanticId::new(902);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                option_eq,
                kernel_schema::StructuralEquivalenceDef::Option { inner: text_eq },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Option(Box::new(TypeExpr::Scalar(
                    ScalarType::Text,
                )))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![option_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Option(Some(Box::new(Value::Text("ALPHA".into()))))],
                vec![Value::Option(Some(Box::new(Value::Text("beta".into()))))],
                vec![Value::Option(None)],
            ],
        );
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Option(Some(Box::new(Value::Text("alpha".into())))),
            equivalence: option_eq,
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows().len(), 1);
        assert_eq!(
            result.rows()[0][0],
            Value::Option(Some(Box::new(Value::Text("ALPHA".into()))))
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn filter_accepts_guarded_recursive_literal_with_recursive_equivalence() {
        let relation = SemanticId::new(905);
        let root_eq = SemanticId::new(906);
        let sum_eq = SemanticId::new(907);
        let product_eq = SemanticId::new(908);
        let var_eq = SemanticId::new(909);
        let unit_eq = SemanticId::new(910);
        let text_eq = SemanticId::new(911);
        let nil_tag = SemanticId::new(912);
        let cons_tag = SemanticId::new(913);
        let head_field = SemanticId::new(914);
        let tail_field = SemanticId::new(915);

        let mut registry = SemanticRegistry::default();
        let unit_digest = registry.install_equivalence(EquivalenceModule::UnitExact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(905));
        environment.pin_module(unit_eq, unit_digest);
        environment.pin_module(text_eq, text_digest);
        let x = kernel_schema::TypeVar(0);
        let recursive_type = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Sum(std::collections::BTreeMap::from([
                (nil_tag, TypeExpr::Scalar(ScalarType::Unit)),
                (
                    cons_tag,
                    TypeExpr::Product(std::collections::BTreeMap::from([
                        (head_field, TypeExpr::Scalar(ScalarType::Text)),
                        (tail_field, TypeExpr::Var(x)),
                    ])),
                ),
            ]))),
        };
        let mut schema = Schema::new(SchemaRevisionId::new(905));
        schema
            .define_structural_equivalence(
                root_eq,
                kernel_schema::StructuralEquivalenceDef::Mu { body: sum_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                sum_eq,
                kernel_schema::StructuralEquivalenceDef::Sum {
                    variants: std::collections::BTreeMap::from([
                        (nil_tag, unit_eq),
                        (cons_tag, product_eq),
                    ]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: std::collections::BTreeMap::from([
                        (head_field, text_eq),
                        (tail_field, var_eq),
                    ]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                var_eq,
                kernel_schema::StructuralEquivalenceDef::Var { binder: root_eq },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![recursive_type],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![root_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();

        let nil = || Value::Variant {
            tag: nil_tag,
            value: Box::new(Value::Unit),
        };
        let cons = |head: &str, tail: Value| Value::Variant {
            tag: cons_tag,
            value: Box::new(Value::Product(std::collections::BTreeMap::from([
                (head_field, Value::Text(head.into())),
                (tail_field, tail),
            ]))),
        };
        let stored = cons("ALPHA", cons("Beta", nil()));
        let literal = cons("alpha", cons("beta", nil()));
        let other = cons("gamma", nil());
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(relation, vec![vec![stored.clone()], vec![other]]);
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: literal,
            equivalence: root_eq,
        };
        let result = query.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(result.rows(), &[vec![stored]]);
    }

    #[test]
    fn exact_query_typecheck_rejects_ambiguous_untyped_constant_but_accepts_annotated_one() {
        let option_type = TypeExpr::Option(Box::new(TypeExpr::Scalar(ScalarType::Text)));
        let untyped = ExactQuery::new(Expr::Const(Value::Option(None)));
        assert_eq!(
            untyped.typecheck(&TypeExpr::Scalar(ScalarType::Unit)),
            Err(QueryTypeError::AmbiguousConstantType)
        );
        let typed = ExactQuery::new(Expr::TypedConst {
            value: Value::Option(None),
            ty: option_type.clone(),
        });
        assert_eq!(
            typed.typecheck(&TypeExpr::Scalar(ScalarType::Unit)),
            Ok(option_type)
        );
    }
    #[test]
    fn generic_group_count_uses_monoid_identity_for_empty_global_group() {
        let relation = SemanticId::new(910);
        let i64_eq = SemanticId::new(911);
        let text_eq = SemanticId::new(912);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(i64_eq, digest);
        environment.pin_module(text_eq, text_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![],
            group_equivalences: vec![],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        assert_eq!(
            query
                .evaluate(&FiniteModel::default(), &context, &registry)
                .unwrap(),
            RelationValue::Set {
                rows: vec![vec![Value::I64(0)]],
                column_equivalences: vec![i64_eq],
            }
        );
    }

    #[test]
    fn generic_group_count_respects_semantic_group_equality() {
        let relation = SemanticId::new(920);
        let text_eq = SemanticId::new(921);
        let count_eq = SemanticId::new(922);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let count_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(count_eq, count_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into())],
                vec![Value::Text("a".into())],
                vec![Value::Text("B".into())],
            ],
        );
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_eq,
            },
        };
        assert_eq!(
            query.evaluate(&model, &context, &registry).unwrap(),
            RelationValue::Set {
                rows: vec![
                    vec![Value::Text("A".into()), Value::I64(2)],
                    vec![Value::Text("B".into()), Value::I64(1)],
                ],
                column_equivalences: vec![text_eq, count_eq],
            }
        );
    }

    #[test]
    fn materialized_i64_top_k_matches_recompute_across_threshold_and_tie_changes() {
        let relation = SemanticId::new(9600);
        let i64_eq = SemanticId::new(9601);
        let i64_order = SemanticId::new(9602);
        let mut registry = kernel_semantics::SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
        let order_digest =
            registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9600));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9600));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let model_for = |values: &[i64]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                values
                    .iter()
                    .map(|value| vec![Value::I64(*value)])
                    .collect(),
            );
            model
        };
        let states = [
            vec![1, 2, 2, 3, 4],
            vec![0, 2, 2, 3, 4],
            vec![0, 1, 2, 3, 4],
            vec![3, 3, 3, 4],
            vec![5, 5, 1, 1, 1],
            vec![],
            vec![7, 7, 7, 6],
        ];
        for direction in [OrderDirection::Ascending, OrderDirection::Descending] {
            let query = RelExpr::TopKWithTies {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                ordering: i64_order,
                direction,
                k: 2,
            };
            let mut old = model_for(&states[0]);
            let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
                .unwrap()
                .expect("top-k state is supported");
            assert!(state.test_storage_is_counted());
            for values in &states[1..] {
                let next = model_for(values);
                let change = Change::Replace(next.clone());
                let oracle =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let maintained = state
                    .apply_model_change(&old, &change, &context, &registry)
                    .unwrap();
                assert!(
                    relation_deltas_semantically_equivalent(
                        &maintained,
                        &oracle,
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "direction={direction:?} values={values:?}: maintained={maintained:?} oracle={oracle:?}"
                );
                assert_eq!(state.row_count(), values.len());
                old = next;
            }
        }
    }

    #[test]
    fn materialized_top_k_cached_boundary_matches_recompute_over_hostile_transitions() {
        let relation = SemanticId::new(9605);
        let i64_eq = SemanticId::new(9606);
        let i64_order = SemanticId::new(9607);
        let mut registry = kernel_semantics::SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
        let order_digest =
            registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9605));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9605));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        let mut make_model = || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let len = usize::try_from(seed % 19).unwrap();
            let mut rows = Vec::with_capacity(len);
            for _ in 0..len {
                seed = seed
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let value = i64::try_from((seed >> 32) % 9).unwrap() - 4;
                rows.push(vec![Value::I64(value)]);
            }
            let mut model = FiniteModel::default();
            model.relations.insert(relation, rows);
            model
        };

        for direction in [OrderDirection::Ascending, OrderDirection::Descending] {
            let query = RelExpr::TopKWithTies {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                ordering: i64_order,
                direction,
                k: 5,
            };
            let mut old = make_model();
            let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
                .unwrap()
                .unwrap();
            state.test_assert_internal_consistency();
            for _ in 0..160 {
                let next = make_model();
                let change = Change::Replace(next.clone());
                let oracle_delta =
                    rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
                let maintained = state
                    .apply_model_change(&old, &change, &context, &registry)
                    .unwrap();
                assert!(
                    relation_deltas_semantically_equivalent(
                        &maintained,
                        &oracle_delta,
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "direction={direction:?} old={old:?} next={next:?}"
                );
                let maintained_output = state.output_value().unwrap();
                let oracle_output = query.evaluate(&next, &context, &registry).unwrap();
                assert!(
                    relation_values_semantically_equivalent(
                        &maintained_output,
                        &oracle_output,
                        state.result_type(),
                        &context,
                        &registry,
                    )
                    .unwrap(),
                    "direction={direction:?} next={next:?}"
                );
                state.test_assert_internal_consistency();
                old = next;
            }
        }
    }

    #[test]
    fn materialized_top_k_generic_text_ordering_preserves_semantic_ties() {
        let relation = SemanticId::new(9610);
        let text_eq = SemanticId::new(9611);
        let text_order = SemanticId::new(9612);
        let mut registry = kernel_semantics::SemanticRegistry::default();
        let eq_digest = registry
            .install_equivalence(kernel_semantics::EquivalenceModule::TextAsciiCaseInsensitive);
        let order_digest =
            registry.install_ordering(kernel_semantics::OrderingModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9610));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(text_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9610));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: text_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let model_for = |values: &[&str]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                values
                    .iter()
                    .map(|value| vec![Value::Text((*value).into())])
                    .collect(),
            );
            model
        };
        let states = [
            vec!["A", "a", "B"],
            vec!["a", "B", "c"],
            vec!["Z", "z", "a"],
        ];
        let mut old = model_for(&states[0]);
        let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_storage_is_counted());
        for values in &states[1..] {
            let next = model_for(values);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn materialized_top_k_semantic_duplicate_order_bucket_resolves_stable_row_id() {
        let relation = SemanticId::new(9_645);
        let order_eq = SemanticId::new(9_646);
        let payload_eq = SemanticId::new(9_647);
        let text_order = SemanticId::new(9_648);
        let mut registry = SemanticRegistry::default();
        let order_eq_digest =
            registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let payload_eq_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let order_digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9_645));
        environment.pin_module(order_eq, order_eq_digest);
        environment.pin_module(payload_eq, payload_eq_digest);
        environment.pin_module(text_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9_645));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![order_eq, payload_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: text_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let rows = (0..2_048)
            .map(|index| {
                vec![
                    Value::Text("same".into()),
                    Value::Text(format!("payload-{index}")),
                ]
            })
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(relation, rows.clone());
        let mut state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let snapshot = state.clone();
        assert!(state.test_shares_storage_with(&snapshot));
        let (total_rows, _, first_bucket_len) = state.test_stats();
        assert_eq!(total_rows, kernel_exact::ExactNatural::from_u64(2_048));
        assert_eq!(first_bucket_len, 2_048);

        let delta = RelationDelta {
            inserted: vec![],
            removed: vec![rows[2_047].clone()],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        let planned = state
            .plan_delta_view(&delta.as_delta_view(), &context, &registry)
            .unwrap();
        let (planned_total, _, planned_first_bucket_len) = planned.patch.test_stats();
        assert_eq!(planned_total, kernel_exact::ExactNatural::from_u64(2_047));
        assert_eq!(planned_first_bucket_len, 2_047);

        state
            .apply_input_delta(&delta, &context, &registry)
            .unwrap();
        assert!(!state.test_shares_storage_with(&snapshot));
        let (snapshot_total, _, _) = snapshot.test_stats();
        let (after_total, _, _) = state.test_stats();
        assert_eq!(snapshot_total, kernel_exact::ExactNatural::from_u64(2_048));
        assert_eq!(after_total, kernel_exact::ExactNatural::from_u64(2_047));
    }

    #[test]
    fn top_k_exact_ordered_measure_keeps_weight_beyond_i64_compact() {
        let relation = SemanticId::new(97_210);
        let i64_eq = SemanticId::new(97_211);
        let i64_order = SemanticId::new(97_212);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(97_210));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(97_210));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(relation, Vec::new());
        let state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let magnitude = kernel_exact::ExactNatural::from_u128(u128::from(i64::MAX as u64) + 7);
        let row = vec![Value::I64(1)];
        let mut delta = ExactDelta::<Row>::default();
        delta.push_exact(
            kernel_exact::ExactInteger::from_parts(false, magnitude.clone()),
            row.clone(),
        );
        let planned = state
            .plan_exact_delta_view(&delta, &context, &registry)
            .unwrap();
        assert_eq!(planned.effect.support_len(), 1);
        planned.effect.visit_exact(|weight, actual| {
            assert_eq!(actual, &row);
            assert_eq!(weight.magnitude(), &magnitude);
            assert!(!weight.is_negative());
        });
        let (next_total, next_buckets, next_first_bucket_len) = planned.patch.test_stats();
        assert_eq!(next_total, magnitude);
        assert_eq!(next_buckets, 1);
        assert_eq!(next_first_bucket_len, 1);
    }

    #[test]
    fn materialized_top_k_i64_rows_uses_universal_plan_commit_and_is_atomic() {
        let relation = SemanticId::new(9615);
        let i64_eq = SemanticId::new(9616);
        let text_eq = SemanticId::new(9617);
        let i64_order = SemanticId::new(9618);
        let mut registry = SemanticRegistry::default();
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9615));
        environment.pin_module(i64_eq, i64_digest);
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9615));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq, text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 2,
        };
        let row = |key, text: &str| vec![Value::I64(key), Value::Text(text.into())];
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![row(1, "a"), row(2, "b"), row(2, "c"), row(4, "d")],
        );
        let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_storage_is_counted());

        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![row(0, "z"), row(2, "c"), row(3, "x"), row(4, "d")],
        );
        let change = Change::Replace(next.clone());
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = state
            .apply_model_change(&old, &change, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
        old = next;

        let before = state.clone();
        let malformed = RelationDelta {
            inserted: vec![],
            removed: vec![row(99, "missing")],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        assert_eq!(
            state.apply_input_delta(&malformed, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
        assert_eq!(state.row_count(), old.relations[&relation].len());
    }

    #[test]
    fn materialized_top_k_i64_duplicate_bucket_resolves_stable_row_id() {
        let relation = SemanticId::new(9_635);
        let i64_eq = SemanticId::new(9_636);
        let text_eq = SemanticId::new(9_637);
        let i64_order = SemanticId::new(9_638);
        let mut registry = SemanticRegistry::default();
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9_635));
        environment.pin_module(i64_eq, i64_digest);
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9_635));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq, text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let rows = (0..2_048)
            .map(|index| vec![Value::I64(7), Value::Text(format!("row-{index}"))])
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(relation, rows.clone());
        let state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let (total_rows, _, first_bucket_len) = state.test_stats();
        assert_eq!(total_rows, kernel_exact::ExactNatural::from_u64(2_048));
        assert_eq!(first_bucket_len, 2_048);

        let delta = RelationDelta {
            inserted: vec![],
            removed: vec![rows[2_047].clone()],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        let planned = state
            .plan_delta_view(&delta.as_delta_view(), &context, &registry)
            .unwrap();
        let (planned_total, _, planned_first_bucket_len) = planned.patch.test_stats();
        assert_eq!(planned_total, kernel_exact::ExactNatural::from_u64(2_047));
        assert_eq!(planned_first_bucket_len, 2_047);
        let mut next = state.clone();
        next.commit_topk_patch(planned.patch);
        let (_, _, before_first_bucket_len) = state.test_stats();
        let (_, _, after_first_bucket_len) = next.test_stats();
        assert_eq!(before_first_bucket_len, 2_048);
        assert_eq!(after_first_bucket_len, 2_047);
        assert!(!state.test_shares_storage_with(&next));
    }

    #[test]
    fn materialized_top_k_semantic_ordered_plan_failure_is_atomic() {
        let relation = SemanticId::new(9619);
        let text_eq = SemanticId::new(9623);
        let text_order = SemanticId::new(9624);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let order_digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9619));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(text_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9619));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: text_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            relation,
            vec![vec![Value::Text("A".into())], vec![Value::Text("b".into())]],
        );
        let mut state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_storage_is_counted());
        let before = state.clone();
        let malformed = RelationDelta {
            inserted: vec![vec![Value::Text("c".into())]],
            removed: vec![vec![Value::Text("missing".into())]],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        assert_eq!(
            state.apply_input_delta(&malformed, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    fn materialized_top_k_rejects_missing_removal_atomically() {
        let relation = SemanticId::new(9620);
        let i64_eq = SemanticId::new(9621);
        let i64_order = SemanticId::new(9622);
        let mut registry = kernel_semantics::SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
        let order_digest =
            registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9620));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9620));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: i64_order,
            direction: OrderDirection::Ascending,
            k: 1,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(relation, vec![vec![Value::I64(1)]]);
        let mut state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let before = state.clone();
        let delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::I64(2)]],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        assert_eq!(
            state.apply_input_delta(&delta, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn materialized_join_i64_two_sided_delta_matches_recompute_and_is_atomic() {
        let left = SemanticId::new(9700);
        let right = SemanticId::new(9701);
        let i64_eq = SemanticId::new(9702);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9700));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9700));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            left,
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(2), Value::I64(20)],
            ],
        );
        old.relations.insert(
            right,
            vec![
                vec![Value::I64(1), Value::I64(100)],
                vec![Value::I64(2), Value::I64(200)],
            ],
        );
        let mut state = MaterializedJoinDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![vec![Value::I64(1), Value::I64(11)]],
            removed: vec![vec![Value::I64(2), Value::I64(20)]],
            result_type: left_type.clone(),
        };
        let right_delta = RelationDelta {
            inserted: vec![vec![Value::I64(1), Value::I64(101)]],
            removed: Vec::new(),
            result_type: right_type.clone(),
        };
        let mut next = old.clone();
        next.relations.insert(
            left,
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(1), Value::I64(11)],
            ],
        );
        next.relations.insert(
            right,
            vec![
                vec![Value::I64(1), Value::I64(100)],
                vec![Value::I64(2), Value::I64(200)],
                vec![Value::I64(1), Value::I64(101)],
            ],
        );
        let oracle =
            rel_delta_by_recompute(&query, &old, &Change::Replace(next), &context, &registry)
                .unwrap();
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );

        let before = state.clone();
        let missing = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::I64(9), Value::I64(9)]],
            result_type: left_type,
        };
        let empty_right = RelationDelta {
            inserted: Vec::new(),
            removed: Vec::new(),
            result_type: right_type,
        };
        assert_eq!(
            state.apply_input_deltas(&missing, &empty_right, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(state, before);
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn materialized_join_i64_delta_view_preserves_weighted_bilinear_cross_term() {
        let left = SemanticId::new(97_100);
        let right = SemanticId::new(97_101);
        let i64_eq = SemanticId::new(97_102);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(97_100));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(97_100));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left, vec![vec![Value::I64(1), Value::I64(10)]]);
        old.relations
            .insert(right, vec![vec![Value::I64(1), Value::I64(100)]]);
        let mut state = MaterializedJoinDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        let before_state = state.clone();

        let mut left_delta = AdaptiveDelta::<Row, 2>::default();
        left_delta.push_weighted(2, vec![Value::I64(1), Value::I64(11)]);
        let mut right_delta = AdaptiveDelta::<Row, 2>::default();
        right_delta.push_weighted(1, vec![Value::I64(1), Value::I64(101)]);

        let planned = state
            .plan_delta_views(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert_eq!(state, before_state);
        let maintained =
            materialize_delta_view(&planned.effect, state.result_type().clone()).unwrap();

        let mut next = old.clone();
        next.relations
            .get_mut(&left)
            .unwrap()
            .extend(std::iter::repeat_n(vec![Value::I64(1), Value::I64(11)], 2));
        next.relations
            .get_mut(&right)
            .unwrap()
            .push(vec![Value::I64(1), Value::I64(101)]);
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );

        state.commit_join_patch(planned.patch);
        let maintained_output = state.output_value(&context, &registry).unwrap();
        let oracle_output = query.evaluate(&next, &context, &registry).unwrap();
        assert!(
            relation_values_semantically_equivalent(
                &maintained_output,
                &oracle_output,
                state.result_type(),
                &context,
                &registry,
            )
            .unwrap()
        );

        let before_invalid = state.clone();
        let mut valid_left = AdaptiveDelta::<Row, 2>::default();
        valid_left.push_weighted(1, vec![Value::I64(1), Value::I64(12)]);
        let mut invalid_right = AdaptiveDelta::<Row, 2>::default();
        invalid_right.push_weighted(-1, vec![Value::I64(9), Value::I64(999)]);
        assert!(matches!(
            state.plan_delta_views(&valid_left, &invalid_right, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        ));
        assert_eq!(state, before_invalid);
    }

    #[test]
    fn counted_join_exact_effect_does_not_expand_large_bilinear_coefficient() {
        let left = SemanticId::new(97_140);
        let right = SemanticId::new(97_141);
        let i64_eq = SemanticId::new(97_142);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(97_140));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(97_140));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(left, Vec::new());
        model.relations.insert(
            right,
            vec![
                vec![Value::I64(1), Value::I64(100)],
                vec![Value::I64(1), Value::I64(100)],
            ],
        );
        let state = MaterializedJoinDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        assert_eq!(state.test_first_right_multiplicity_u64(), Some(2));

        let mut left_delta = AdaptiveDelta::<Row, 1>::default();
        left_delta.push_weighted(i64::MAX, vec![Value::I64(1), Value::I64(7)]);
        let right_delta = AdaptiveDelta::<Row, 1>::default();
        let planned = state
            .plan_exact_delta_views(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert_eq!(planned.effect.support_len(), 1);
        planned.effect.visit_exact(|weight, row| {
            assert_eq!(
                row,
                &vec![Value::I64(1), Value::I64(7), Value::I64(1), Value::I64(100)]
            );
            assert_eq!(
                weight.magnitude().to_decimal_string(),
                "18446744073709551614"
            );
            assert!(!weight.is_negative());
            assert!(weight.magnitude() > &kernel_exact::ExactNatural::from_u64(i64::MAX as u64));
        });
        assert!(matches!(
            state.plan_delta_views(&left_delta, &right_delta, &context, &registry),
            Err(RelQueryError::DerivedIdentityExhausted)
        ));
    }

    #[test]
    fn materialized_join_i64_duplicate_bucket_uses_stable_row_class_ids() {
        let left = SemanticId::new(97_150);
        let right = SemanticId::new(97_151);
        let i64_eq = SemanticId::new(97_152);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(97_150));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(97_150));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let left_rows = (0..2_048)
            .map(|payload| vec![Value::I64(1), Value::I64(payload)])
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(left, left_rows);
        model
            .relations
            .insert(right, vec![vec![Value::I64(1), Value::I64(10_000)]]);

        let state = MaterializedJoinDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        assert_eq!(state.test_left_bucket_stats(), (1, 2_048));

        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::I64(1), Value::I64(2_047)]],
            result_type: left_type.clone(),
        };
        let (delta_len, first_negative, next_first_bucket_len) = state
            .test_plan_left_mutation_stats(
                &delta,
                0,
                i64_eq,
                &left_type,
                &context,
                &registry,
            )
            .unwrap();
        assert_eq!(delta_len, 1);
        assert!(first_negative);
        assert_eq!(next_first_bucket_len, 2_047);
    }

    #[test]
    fn materialized_join_generic_preserves_ascii_ci_semantics() {
        let (context, registry, text_eq, left, right) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut old = FiniteModel::default();
        old.relations
            .insert(left, vec![vec![Value::Text("A".into()), Value::I64(1)]]);
        old.relations
            .insert(right, vec![vec![Value::Text("a".into()), Value::I64(10)]]);
        let mut state = MaterializedJoinDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        let snapshot = state.clone();
        assert_eq!(snapshot.test_shares_storage_with(&state), (true, true));
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![vec![Value::Text("ALPHA".into()), Value::I64(2)]],
            removed: Vec::new(),
            result_type: left_type.clone(),
        };
        let right_delta = RelationDelta {
            inserted: vec![vec![Value::Text("alpha".into()), Value::I64(20)]],
            removed: Vec::new(),
            result_type: right_type.clone(),
        };
        let mut next = old.clone();
        next.relations
            .get_mut(&left)
            .unwrap()
            .push(vec![Value::Text("ALPHA".into()), Value::I64(2)]);
        next.relations
            .get_mut(&right)
            .unwrap()
            .push(vec![Value::Text("alpha".into()), Value::I64(20)]);
        let oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert_eq!(snapshot.test_shares_storage_with(&state), (false, false));
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );

        old = next;
        let left_delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::Text("alpha".into()), Value::I64(2)]],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::Text("A".into()), Value::I64(10)]],
            result_type: right_type,
        };
        let mut next = old.clone();
        next.relations.get_mut(&left).unwrap().remove(1);
        next.relations.get_mut(&right).unwrap().remove(0);
        let oracle =
            rel_delta_by_recompute(&query, &old, &Change::Replace(next), &context, &registry)
                .unwrap();
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                .unwrap()
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn owned_join_group_top_k_tree_matches_recompute_across_leaf_deltas() {
        let left = SemanticId::new(9720);
        let right = SemanticId::new(9721);
        let i64_eq = SemanticId::new(9722);
        let i64_order = SemanticId::new(9723);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9720));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9720));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let join = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let group = RelExpr::Group {
            input: Box::new(join),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(group),
            column: 1,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 1,
        };
        let model_for = |left_rows: Vec<Row>, right_rows: Vec<Row>| {
            let mut model = FiniteModel::default();
            model.relations.insert(left, left_rows);
            model.relations.insert(right, right_rows);
            model
        };
        let mut old = model_for(
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(2), Value::I64(20)],
            ],
            vec![
                vec![Value::I64(1), Value::I64(100)],
                vec![Value::I64(2), Value::I64(200)],
            ],
        );
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let mut state = MaterializedJoinGroupTopKState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        let group_query = match &query {
            RelExpr::TopKWithTies { input, .. } => input.as_ref(),
            _ => unreachable!(),
        };
        let join_query = match group_query {
            RelExpr::Group { input, .. } => input.as_ref(),
            _ => unreachable!(),
        };
        let (join_state, group_state, top_k_state) = state.test_states();
        assert_eq!(
            join_state,
            &MaterializedJoinDeltaState::build(join_query, &old, &context, &registry)
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            group_state,
            &MaterializedGroupDeltaState::build(group_query, &old, &context, &registry)
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            top_k_state,
            &MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
                .unwrap()
                .unwrap()
        );
        let steps = [
            (
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(11)]],
                    removed: Vec::new(),
                    result_type: left_type.clone(),
                },
                RelationDelta {
                    inserted: Vec::new(),
                    removed: Vec::new(),
                    result_type: right_type.clone(),
                },
            ),
            (
                RelationDelta {
                    inserted: Vec::new(),
                    removed: Vec::new(),
                    result_type: left_type.clone(),
                },
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(101)]],
                    removed: vec![vec![Value::I64(2), Value::I64(200)]],
                    result_type: right_type.clone(),
                },
            ),
            (
                RelationDelta {
                    inserted: Vec::new(),
                    removed: vec![vec![Value::I64(1), Value::I64(10)]],
                    result_type: left_type.clone(),
                },
                RelationDelta {
                    inserted: vec![vec![Value::I64(2), Value::I64(201)]],
                    removed: Vec::new(),
                    result_type: right_type.clone(),
                },
            ),
        ];
        for (left_delta, right_delta) in steps {
            let old_left =
                RelationValue::Bag(old.relations.materialize_owned(&left).unwrap_or_default());
            let old_right =
                RelationValue::Bag(old.relations.materialize_owned(&right).unwrap_or_default());
            let next_left =
                apply_relation_delta_to_value(old_left, &left_delta, &context, &registry).unwrap();
            let next_right =
                apply_relation_delta_to_value(old_right, &right_delta, &context, &registry)
                    .unwrap();
            let next = model_for(next_left.into_rows(), next_right.into_rows());
            let oracle = rel_delta_by_recompute(
                &query,
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            )
            .unwrap();
            let maintained = state
                .apply_join_input_deltas(&left_delta, &right_delta, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn recursive_maintained_plan_owns_nontrivial_join_group_top_k_tree() {
        let left = SemanticId::new(9730);
        let right = SemanticId::new(9731);
        let i64_eq = SemanticId::new(9732);
        let i64_order = SemanticId::new(9733);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9730));
        environment.pin_module(i64_eq, eq_digest);
        environment.pin_module(i64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9730));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let left_filtered = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(left)),
            column: 1,
            value: Value::I64(1),
            equivalence: i64_eq,
        };
        let right_projected = RelExpr::Project {
            input: Box::new(RelExpr::Scan(right)),
            columns: vec![0],
        };
        let joined = RelExpr::JoinEq {
            left: Box::new(left_filtered),
            right: Box::new(right_projected),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        };
        let projected = RelExpr::Project {
            input: Box::new(joined),
            columns: vec![0],
        };
        let grouped = RelExpr::Group {
            input: Box::new(projected),
            group_columns: vec![0],
            group_equivalences: vec![i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(grouped),
            column: 1,
            ordering: i64_order,
            direction: OrderDirection::Descending,
            k: 1,
        };
        let model_for = |left_rows: Vec<Row>, right_rows: Vec<Row>| {
            let mut model = FiniteModel::default();
            model.relations.insert(left, left_rows);
            model.relations.insert(right, right_rows);
            model
        };
        let mut old = model_for(
            vec![
                vec![Value::I64(1), Value::I64(1)],
                vec![Value::I64(2), Value::I64(1)],
                vec![Value::I64(3), Value::I64(0)],
            ],
            vec![
                vec![Value::I64(1), Value::I64(10)],
                vec![Value::I64(2), Value::I64(20)],
            ],
        );
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        assert_eq!(
            state.output_value(&context, &registry).unwrap(),
            query.evaluate(&old, &context, &registry).unwrap()
        );
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();

        let mut resolved_state = state.clone();
        let left_bindings = old.relations[&left]
            .iter()
            .cloned()
            .enumerate()
            .map(|(slot, row)| {
                (
                    kernel_types::StableRowHandle {
                        slot,
                        generation: 0,
                    },
                    row,
                )
            })
            .collect::<Vec<_>>();
        resolved_state
            .attach_storage_rows(left, &left_bindings)
            .unwrap();
        let right_bindings = old.relations[&right]
            .iter()
            .cloned()
            .enumerate()
            .map(|(slot, row)| {
                (
                    kernel_types::StableRowHandle {
                        slot,
                        generation: 0,
                    },
                    row,
                )
            })
            .collect::<Vec<_>>();
        resolved_state
            .attach_storage_rows(right, &right_bindings)
            .unwrap();
        let resolved_delta = RelationDelta {
            inserted: vec![vec![Value::I64(1), Value::I64(1)]],
            removed: vec![vec![Value::I64(2), Value::I64(1)]],
            result_type: left_type.clone(),
        };
        let resolved = StorageResolvedRelationDelta::from_parts(
            left,
            resolved_delta.clone(),
            vec![kernel_types::StableRowHandle {
                slot: 1,
                generation: 0,
            }],
            vec![kernel_types::StableRowHandle {
                slot: 1,
                generation: 1,
            }],
            &context,
            &registry,
        )
        .unwrap();
        let mut resolved_next = old.clone();
        resolved_next.relations.get_mut(&left).unwrap().remove(1);
        resolved_next
            .relations
            .get_mut(&left)
            .unwrap()
            .push(vec![Value::I64(1), Value::I64(1)]);
        let resolved_oracle = rel_delta_by_recompute(
            &query,
            &old,
            &Change::Replace(resolved_next.clone()),
            &context,
            &registry,
        )
        .unwrap();
        let mut resolved_map = BTreeMap::new();
        resolved_map.insert(left, resolved);
        resolved_state.bind_revision(RevisionId::new(9_001)).unwrap();
        reset_relation_delta_materialization_count();
        let (next_resolved_state, resolved_output) = resolved_state
            .candidate_from_storage_resolved_deltas_for_revision(
                RevisionId::new(9_002),
                &resolved_map,
                &context,
                &registry,
            )
            .unwrap();
        resolved_state = next_resolved_state;
        assert_eq!(relation_delta_materialization_count(), 1);
        assert!(
            relation_deltas_semantically_equivalent(
                &resolved_output,
                &resolved_oracle,
                &context,
                &registry,
            )
            .unwrap()
        );
        assert_eq!(
            resolved_state.output_value(&context, &registry).unwrap(),
            query.evaluate(&resolved_next, &context, &registry).unwrap()
        );

        let steps = [
            (
                left,
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(1)]],
                    removed: Vec::new(),
                    result_type: left_type.clone(),
                },
            ),
            (
                right,
                RelationDelta {
                    inserted: vec![vec![Value::I64(1), Value::I64(11)]],
                    removed: Vec::new(),
                    result_type: right_type.clone(),
                },
            ),
            (
                left,
                RelationDelta {
                    inserted: Vec::new(),
                    removed: vec![vec![Value::I64(2), Value::I64(1)]],
                    result_type: left_type.clone(),
                },
            ),
        ];
        for (relation, delta) in steps {
            let mut deltas = BTreeMap::new();
            deltas.insert(relation, delta.clone());
            let mut next = old.clone();
            let old_value =
                RelationValue::Bag(next.relations.materialize_owned(&relation).unwrap_or_default());
            next.relations.insert(
                relation,
                apply_relation_delta_to_value(old_value, &delta, &context, &registry)
                    .unwrap()
                    .into_rows(),
            );
            let oracle = rel_delta_by_recompute(
                &query,
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            )
            .unwrap();
            reset_relation_delta_materialization_count();
            let maintained = state
                .apply_relation_deltas(&deltas, &context, &registry)
                .unwrap();
            assert_eq!(relation_delta_materialization_count(), 1);
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            assert_eq!(
                state.output_value(&context, &registry).unwrap(),
                query.evaluate(&next, &context, &registry).unwrap()
            );
            old = next;
        }

        let before = state.clone();
        let mut invalid = BTreeMap::new();
        invalid.insert(
            left,
            RelationDelta {
                inserted: Vec::new(),
                removed: vec![vec![Value::I64(999), Value::I64(1)]],
                result_type: left_type,
            },
        );
        reset_relation_delta_materialization_count();
        assert_eq!(
            state.apply_relation_deltas(&invalid, &context, &registry),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
        assert_eq!(relation_delta_materialization_count(), 0);
        assert_eq!(state, before);
    }

    #[test]
    fn recursive_maintained_plan_supports_distinct_and_promote_to_bag() {
        let relation = SemanticId::new(9740);
        let i64_eq = SemanticId::new(9741);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9740));
        environment.pin_module(i64_eq, eq_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(9740));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![i64_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::PromoteToBag(Box::new(RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 1,
                    value: Value::I64(1),
                    equivalence: i64_eq,
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![i64_eq],
        }));
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::I64(1), Value::I64(1)],
                vec![Value::I64(1), Value::I64(1)],
                vec![Value::I64(2), Value::I64(0)],
            ],
        );
        let relation_type = RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap();
        let mut state = MaterializedRelPlanState::build(&query, &old, &context, &registry).unwrap();
        let deltas = [
            RelationDelta {
                inserted: vec![vec![Value::I64(1), Value::I64(1)]],
                removed: Vec::new(),
                result_type: relation_type.clone(),
            },
            RelationDelta {
                inserted: Vec::new(),
                removed: vec![vec![Value::I64(1), Value::I64(1)]],
                result_type: relation_type.clone(),
            },
            RelationDelta {
                inserted: Vec::new(),
                removed: vec![vec![Value::I64(1), Value::I64(1)]],
                result_type: relation_type,
            },
        ];
        for delta in deltas {
            let mut leaf = BTreeMap::new();
            leaf.insert(relation, delta.clone());
            let old_value =
                RelationValue::Bag(old.relations.materialize_owned(&relation).unwrap_or_default());
            let next_value =
                apply_relation_delta_to_value(old_value, &delta, &context, &registry).unwrap();
            let mut next = old.clone();
            next.relations.insert(relation, next_value.into_rows());
            let oracle = rel_delta_by_recompute(
                &query,
                &old,
                &Change::Replace(next.clone()),
                &context,
                &registry,
            )
            .unwrap();
            let maintained = state
                .apply_relation_deltas(&leaf, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry,)
                    .unwrap()
            );
            assert_eq!(
                state.output_value(&context, &registry).unwrap(),
                query.evaluate(&next, &context, &registry).unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn maintained_scan_shares_factorized_relation_occurrence_witness_without_recanonicalization() {
        let (context, registry, _, relation, _) = setup();
        let rows = vec![
            vec![Value::Text("Alpha".into()), Value::I64(1)],
            vec![Value::Text("Beta".into()), Value::I64(2)],
            vec![Value::Text("Gamma".into()), Value::I64(3)],
        ];
        let mut model = FiniteModel::default();
        model.relations.insert(relation, rows.clone());
        let query = RelExpr::Scan(relation);
        let result_type = query.typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0),
            relation,
            &rows,
            result_type,
            &context,
            &registry,
        )
        .unwrap();
        let storage_rows = rows
            .iter()
            .cloned()
            .enumerate()
            .map(|(slot, row)| {
                (
                    kernel_types::StableRowHandle {
                        slot,
                        generation: 0,
                    },
                    row,
                )
            })
            .collect::<Vec<_>>();
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        state
            .attach_storage_rows_with_base_witness(relation, &storage_rows, &witness)
            .unwrap();
        let shared = state
            .storage_relation_base_witness(relation)
            .unwrap()
            .unwrap();
        assert!(witness.certifies_same_base(&shared));
        assert!(witness.shares_occurrence_root_with(&shared));

        let delta = RelationDelta {
            inserted: vec![vec![Value::Text("Delta".into()), Value::I64(4)]],
            removed: vec![rows[1].clone()],
            result_type: query.typecheck(&context, &registry).unwrap(),
        };
        let resolved = StorageResolvedRelationDelta::from_parts(
            relation,
            delta.clone(),
            vec![kernel_types::StableRowHandle {
                slot: 1,
                generation: 0,
            }],
            vec![kernel_types::StableRowHandle {
                slot: 3,
                generation: 0,
            }],
            &context,
            &registry,
        )
        .unwrap();
        state.bind_revision(RevisionId::new(90)).unwrap();
        let (next_state, _) = state
            .candidate_from_storage_resolved_deltas_for_revision(
                RevisionId::new(91),
                &BTreeMap::from([(relation, resolved)]),
                &context,
                &registry,
            )
            .unwrap();
        let shared_after = next_state
            .storage_relation_base_witness(relation)
            .unwrap()
            .unwrap();
        let expected_after = witness
            .advance(RevisionId::new(0), &delta, &registry)
            .unwrap();
        assert!(expected_after.certifies_same_base(&shared_after));
        assert!(!witness.shares_occurrence_root_with(&shared_after));

        let forged_handles = storage_rows
            .iter()
            .enumerate()
            .map(|(slot, (_, row))| {
                (
                    kernel_types::StableRowHandle {
                        slot: slot + 10,
                        generation: 7,
                    },
                    row.clone(),
                )
            })
            .collect::<Vec<_>>();
        let mut rejected =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        assert_eq!(
            rejected.attach_storage_rows_with_base_witness(
                relation,
                &forged_handles,
                &witness,
            ),
            Err(RelQueryError::StructuralRewriteBaseMismatch)
        );
    }

    #[test]
    fn storage_resolved_removal_rejects_handle_payload_mismatch() {
        let (context, registry, text_eq, relation, _) = setup();
        let i64_eq = SemanticId::new(101);
        let mut model = FiniteModel::default();
        let alpha = vec![Value::Text("Alpha".into()), Value::I64(1)];
        let beta = vec![Value::Text("Beta".into()), Value::I64(2)];
        model
            .relations
            .insert(relation, vec![alpha.clone(), beta.clone()]);

        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![text_eq, i64_eq],
        };
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let alpha_handle = kernel_types::StableRowHandle {
            slot: 0,
            generation: 0,
        };
        let beta_handle = kernel_types::StableRowHandle {
            slot: 1,
            generation: 0,
        };
        state
            .attach_storage_rows(
                relation,
                &[(alpha_handle, alpha.clone()), (beta_handle, beta.clone())],
            )
            .unwrap();

        let delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![beta],
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        let forged = StorageResolvedRelationDelta::from_parts(
            relation,
            delta,
            vec![alpha_handle],
            Vec::new(),
            &context,
            &registry,
        )
        .unwrap();
        let map = BTreeMap::from([(relation, forged)]);
        state.bind_revision(RevisionId::new(9_101)).unwrap();

        assert_eq!(
            state.candidate_from_storage_resolved_deltas_for_revision(
                RevisionId::new(9_102),
                &map,
                &context,
                &registry,
            ),
            Err(RelQueryError::InconsistentIncrementalDelta)
        );
    }

    #[test]
    fn composite_primitive_group_uses_semantic_index_and_matches_recompute() {
        let (context, registry, text_eq, relation, _) = setup();
        let i64_eq = SemanticId::new(101);
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0, 1],
            group_equivalences: vec![text_eq, i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("a".into()), Value::I64(1)],
                vec![Value::Text("B".into()), Value::I64(2)],
            ],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_has_semantic_lookup());
        assert_eq!(state.test_group_encoder_count(), Some(2));
        assert_eq!(state.group_count(), 2);

        let mut next = old.clone();
        next.relations.insert(
            relation,
            vec![
                vec![Value::Text("A".into()), Value::I64(1)],
                vec![Value::Text("b".into()), Value::I64(2)],
                vec![Value::Text("C".into()), Value::I64(3)],
            ],
        );
        let change = Change::Replace(next.clone());
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = state
            .apply_model_change(&old, &change, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );
        assert_eq!(state.group_count(), 3);
    }

    #[test]
    fn structural_and_primitive_group_uses_canonical_lookup_and_matches_recompute() {
        let relation = SemanticId::new(98_150);
        let text_eq = SemanticId::new(98_151);
        let set_eq = SemanticId::new(98_152);
        let i64_eq = SemanticId::new(98_153);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_150));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(98_150));
        schema
            .define_structural_equivalence(
                set_eq,
                kernel_schema::StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Set {
                        element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                        equivalence: text_eq,
                    },
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![set_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let set = |values: &[&str]| Value::Set {
            equivalence: text_eq,
            elements: values
                .iter()
                .map(|value| Value::Text((*value).into()))
                .collect(),
        };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0, 1],
            group_equivalences: vec![set_eq, i64_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        };
        let mut old = FiniteModel::default();
        old.relations.insert(
            relation,
            vec![
                vec![set(&["A", "B"]), Value::I64(1)],
                vec![set(&["b", "a"]), Value::I64(1)],
                vec![set(&["C"]), Value::I64(3)],
            ],
        );
        let mut state = MaterializedGroupDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_has_semantic_lookup());
        assert_eq!(state.test_group_encoder_count(), None);
        assert!(state.test_has_semantic_lookup());
        assert_eq!(state.group_count(), 2);

        let mut next = FiniteModel::default();
        next.relations.insert(
            relation,
            vec![
                vec![set(&["B", "A"]), Value::I64(1)],
                vec![set(&["c"]), Value::I64(3)],
                vec![set(&["D", "E"]), Value::I64(4)],
            ],
        );
        let change = Change::Replace(next.clone());
        let oracle = rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
        let maintained = state
            .apply_model_change(&old, &change, &context, &registry)
            .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );
        assert_eq!(state.group_count(), 3);
    }

    #[test]
    fn materialized_f64_total_top_k_uses_order_index_across_hostile_values() {
        let relation = SemanticId::new(98_100);
        let f64_eq = SemanticId::new(98_101);
        let f64_order = SemanticId::new(98_102);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let order_digest = registry.install_ordering(OrderingModule::F64Total);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_100));
        environment.pin_module(f64_eq, eq_digest);
        environment.pin_module(f64_order, order_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(98_100));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::F64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![f64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let query = RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering: f64_order,
            direction: OrderDirection::Ascending,
            k: 3,
        };
        let model_for = |bits: &[u64]| {
            let mut model = FiniteModel::default();
            model.relations.insert(
                relation,
                bits.iter()
                    .copied()
                    .map(|bits| vec![Value::F64Bits(bits)])
                    .collect(),
            );
            model
        };
        let states = [
            vec![
                f64::NEG_INFINITY.to_bits(),
                (-0.0_f64).to_bits(),
                0.0_f64.to_bits(),
                1.0_f64.to_bits(),
                0x7ff8_0000_0000_0001,
                0xfff8_0000_0000_0001,
            ],
            vec![
                0xfff8_0000_0000_0002,
                (-1.0_f64).to_bits(),
                (-0.0_f64).to_bits(),
                0.0_f64.to_bits(),
                f64::INFINITY.to_bits(),
                0x7ff8_0000_0000_0002,
            ],
            vec![
                f64::NEG_INFINITY.to_bits(),
                f64::INFINITY.to_bits(),
                5e-324_f64.to_bits(),
                (-5e-324_f64).to_bits(),
            ],
        ];
        let mut old = model_for(&states[0]);
        let mut state = MaterializedTopKDeltaState::build(&query, &old, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_storage_is_counted());
        for values in &states[1..] {
            let next = model_for(values);
            let change = Change::Replace(next.clone());
            let oracle =
                rel_delta_by_recompute(&query, &old, &change, &context, &registry).unwrap();
            let maintained = state
                .apply_model_change(&old, &change, &context, &registry)
                .unwrap();
            assert!(
                relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                    .unwrap()
            );
            old = next;
        }
    }

    #[test]
    fn semantic_indexed_join_accepts_same_call_remove_insert_replacement() {
        let (context, registry, text_eq, left_relation, right_relation) = setup();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left_relation)),
            right: Box::new(RelExpr::Scan(right_relation)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let mut model = FiniteModel::default();
        model.relations.insert(
            left_relation,
            vec![vec![Value::Text("key-0".into()), Value::I64(0)]],
        );
        model.relations.insert(
            right_relation,
            (0..1000)
                .map(|i| vec![Value::Text(format!("KEY-{i}")), Value::I64(i)])
                .collect(),
        );
        let mut state = MaterializedJoinDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let left_type = RelExpr::Scan(left_relation)
            .typecheck(&context, &registry)
            .unwrap();
        let right_type = RelExpr::Scan(right_relation)
            .typecheck(&context, &registry)
            .unwrap();
        let left_delta = RelationDelta {
            removed: vec![vec![Value::Text("key-0".into()), Value::I64(0)]],
            inserted: vec![vec![Value::Text("KEY-0".into()), Value::I64(1)]],
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            removed: Vec::new(),
            inserted: Vec::new(),
            result_type: right_type,
        };
        assert!(
            state
                .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
                .is_ok()
        );
    }

    #[test]
    fn structural_join_equivalence_uses_canonical_index_and_delta_matches_oracle() {
        let left = SemanticId::new(98_200);
        let right = SemanticId::new(98_201);
        let product_eq = SemanticId::new(98_202);
        let text_eq = SemanticId::new(98_203);
        let i64_eq = SemanticId::new(98_204);
        let field = SemanticId::new(98_205);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(98_200));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let product_type = TypeExpr::Product(BTreeMap::from([(
            field,
            TypeExpr::Scalar(ScalarType::Text),
        )]));
        let mut schema = Schema::new(SchemaRevisionId::new(98_200));
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, text_eq)]),
                },
            )
            .unwrap();
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![product_type.clone(), TypeExpr::Scalar(ScalarType::I64)],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![product_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext {
            schema,
            environment,
        };
        let product =
            |label: &str| Value::Product(BTreeMap::from([(field, Value::Text(label.into()))]));
        let mut model = FiniteModel::default();
        model
            .relations
            .insert(left, vec![vec![product("Alpha"), Value::I64(1)]]);
        model
            .relations
            .insert(right, vec![vec![product("alpha"), Value::I64(2)]]);
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
            left_column: 0,
            right_column: 0,
            equivalence: product_eq,
        };
        let state = MaterializedJoinDeltaState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        assert!(state.test_storage_is_counted());
        let mut state = state;
        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let left_delta = RelationDelta {
            inserted: vec![vec![product("BETA"), Value::I64(3)]],
            removed: Vec::new(),
            result_type: left_type,
        };
        let right_delta = RelationDelta {
            inserted: vec![
                vec![product("beta"), Value::I64(4)],
                vec![product("ALPHA"), Value::I64(5)],
            ],
            removed: Vec::new(),
            result_type: right_type,
        };
        let maintained = state
            .apply_input_deltas(&left_delta, &right_delta, &context, &registry)
            .unwrap();

        let mut next = model.clone();
        next.relations
            .get_mut(&left)
            .unwrap()
            .push(vec![product("BETA"), Value::I64(3)]);
        next.relations.get_mut(&right).unwrap().extend([
            vec![product("beta"), Value::I64(4)],
            vec![product("ALPHA"), Value::I64(5)],
        ]);
        let oracle =
            rel_delta_by_recompute(&query, &model, &Change::Replace(next), &context, &registry)
                .unwrap();
        assert!(
            relation_deltas_semantically_equivalent(&maintained, &oracle, &context, &registry)
                .unwrap()
        );
    }
}

#[cfg(test)]
mod positive_recursive_query_tests {
    use super::*;
    use kernel_model::FiniteModel;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        TypeExpr,
    };
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};

    fn bag_type() -> RelType {
        RelType {
            columns: vec![kernel_schema::TypeExpr::Scalar(
                kernel_schema::ScalarType::I64,
            )],
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences: vec![],
            },
        }
    }

    fn empty_context() -> (
        kernel_schema::SemanticContext,
        kernel_semantics::SemanticRegistry,
    ) {
        (
            kernel_schema::SemanticContext {
                schema: kernel_schema::Schema::new(kernel_types::SchemaRevisionId::new(1)),
                environment: kernel_schema::SemanticEnvironment::new(
                    kernel_types::SemanticEnvId::new(1),
                ),
            },
            kernel_semantics::SemanticRegistry::default(),
        )
    }

    #[test]
    fn fixpoint_call_returns_compact_exact_finite_bag_weights() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(1)],
                    seed_multiplicity: 2,
                },
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(2)],
                    seed_multiplicity: 0,
                },
            ],
            rules: vec![PositiveRecursiveRowRule {
                body: vec![0, 0],
                head: 1,
                coefficient: 3,
            }],
        };
        let (context, registry) = empty_context();
        let result = call.evaluate_compact(&context, &registry).unwrap();
        assert_eq!(result.entries().len(), 2);
        assert_eq!(
            result.entries()[1].1,
            kernel_fixpoint::NaturalInfinity::finite_u64(12)
        );
        assert!(result.require_finite().is_ok());
    }

    #[test]
    fn fixpoint_call_reports_nonfinite_without_expanding_rows() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![PositiveRecursiveRowAtom {
                row: vec![Value::I64(1)],
                seed_multiplicity: 1,
            }],
            rules: vec![PositiveRecursiveRowRule {
                body: vec![0],
                head: 0,
                coefficient: 1,
            }],
        };
        let (context, registry) = empty_context();
        let result = call.evaluate_compact(&context, &registry).unwrap();
        assert_eq!(result.entries().len(), 1);
        assert!(result.entries()[0].1.is_infinite());
        assert_eq!(
            result.require_finite(),
            Err(RelQueryError::NonFiniteRecursiveMultiplicity)
        );
    }

    #[test]
    fn hostile_fixpoint_call_rejects_atom_outside_finite_carrier() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![PositiveRecursiveRowAtom {
                row: vec![Value::I64(1)],
                seed_multiplicity: 1,
            }],
            rules: vec![PositiveRecursiveRowRule {
                body: vec![7],
                head: 0,
                coefficient: 1,
            }],
        };
        let (context, registry) = empty_context();
        assert_eq!(
            call.evaluate_compact(&context, &registry),
            Err(RelQueryError::RecursiveAtomOutsideCarrier)
        );
    }

    #[test]
    fn set_union_and_difference_execution_certificates_reuse_gamma_support_as_relation_witness() {
        let eq = SemanticId::new(91_000);
        let left = SemanticId::new(91_001);
        let right = SemanticId::new(91_002);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(910));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(910));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                    semantics: RelationSemantics::Set {
                        column_equivalences: vec![eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext { schema, environment };
        let mut model = FiniteModel::default();
        model.relations.insert(
            left,
            vec![vec![Value::Text("Alpha".into())], vec![Value::Text("beta".into())]],
        );
        model.relations.insert(
            right,
            vec![vec![Value::Text("ALPHA".into())], vec![Value::Text("Gamma".into())]],
        );
        let query = RelExpr::Union {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        };
        let (value, certificate) = query
            .evaluate_with_occurrence_certificate(&model, &context, &registry)
            .unwrap();
        assert_eq!(value.rows().len(), 3);
        assert_eq!(certificate.row_count(), 3);
        let witness = RelationBaseWitness::from_occurrence_certificate(
            RevisionId::new(0),
            SemanticId::new(91_003),
            certificate.clone(),
            query.typecheck(&context, &registry).unwrap(),
            &context,
            &registry,
        )
        .unwrap();
        assert!(certificate.shares_occurrence_root_with(&witness));

        let difference = RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        }
        .prepare(&context, &registry)
        .unwrap();
        assert!(difference.emits_occurrence_certificate());
        let (value, certificate) = difference
            .evaluate_with_occurrence_certificate(&model, &context, &registry)
            .unwrap();
        assert_eq!(value.rows(), &[vec![Value::Text("beta".into())]]);
        let witness = RelationBaseWitness::from_occurrence_certificate(
            RevisionId::new(0),
            SemanticId::new(91_004),
            certificate.clone(),
            difference.result_type().clone(),
            &context,
            &registry,
        )
        .unwrap();
        assert!(certificate.shares_occurrence_root_with(&witness));
    }

    #[test]
    fn quotient_operators_emit_exact_occurrence_certificate_without_second_gamma_pass() {
        let text_eq = SemanticId::new(91_020);
        let i64_eq = SemanticId::new(91_021);
        let set_relation = SemanticId::new(91_022);
        let bag_relation = SemanticId::new(91_023);
        let target = SemanticId::new(91_024);
        let mut registry = SemanticRegistry::default();
        let text_digest =
            registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(912));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(912));
        schema
            .define_relation(RelationDef {
                id: set_relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: bag_relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        let context = SemanticContext { schema, environment };
        let mut model = FiniteModel::default();
        let rows = vec![
            vec![Value::Text("Alpha".into()), Value::I64(1)],
            vec![Value::Text("ALPHA".into()), Value::I64(2)],
            vec![Value::Text("Beta".into()), Value::I64(3)],
        ];
        model.relations.insert(set_relation, rows.clone());
        model.relations.insert(bag_relation, rows);

        let queries = [
            RelExpr::Project {
                input: Box::new(RelExpr::Scan(set_relation)),
                columns: vec![0],
            },
            RelExpr::Distinct {
                input: Box::new(RelExpr::Project {
                    input: Box::new(RelExpr::Scan(bag_relation)),
                    columns: vec![0],
                }),
                column_equivalences: vec![text_eq],
            },
        ];

        for query in queries {
            let prepared = query.prepare(&context, &registry).unwrap();
            assert!(prepared.emits_occurrence_certificate());
            let (value, certificate) = prepared
                .evaluate_with_occurrence_certificate(&model, &context, &registry)
                .unwrap();
            assert_eq!(value.rows().len(), 2);
            assert_eq!(certificate.row_count(), 2);
            let witness = RelationBaseWitness::from_occurrence_certificate(
                RevisionId::new(0),
                target,
                certificate.clone(),
                prepared.result_type().clone(),
                &context,
                &registry,
            )
            .unwrap();
            assert!(certificate.shares_occurrence_root_with(&witness));
        }

        let bag_projection = RelExpr::Project {
            input: Box::new(RelExpr::Scan(bag_relation)),
            columns: vec![0],
        }
        .prepare(&context, &registry)
        .unwrap();
        assert!(!bag_projection.emits_occurrence_certificate());
        assert_eq!(
            bag_projection
                .evaluate_with_occurrence_certificate(&model, &context, &registry)
                .unwrap_err(),
            RelQueryError::CanonicalObservationUnavailable
        );

        let group = RelExpr::Group {
            input: Box::new(RelExpr::Scan(bag_relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count {
                result_equivalence: i64_eq,
            },
        }
        .prepare(&context, &registry)
        .unwrap();
        assert!(!group.emits_occurrence_certificate());
        assert_eq!(
            group
                .evaluate_with_occurrence_certificate(&model, &context, &registry)
                .unwrap_err(),
            RelQueryError::CanonicalObservationUnavailable
        );
    }

    #[test]
    fn set_join_composes_child_full_row_certificates_without_output_recanonicalization() {
        let text_eq = SemanticId::new(91_080);
        let i64_eq = SemanticId::new(91_081);
        let left = SemanticId::new(91_082);
        let right = SemanticId::new(91_083);
        let target = SemanticId::new(91_084);
        let mut registry = SemanticRegistry::default();
        let text_digest =
            registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916));
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
        let context = SemanticContext { schema, environment };
        let mut model = FiniteModel::default();
        model.relations.insert(
            left,
            vec![
                vec![Value::Text("Alpha".into()), Value::I64(1)],
                vec![Value::Text("Beta".into()), Value::I64(2)],
            ],
        );
        model.relations.insert(
            right,
            vec![
                vec![Value::Text("ALPHA".into()), Value::I64(10)],
                vec![Value::Text("alpha".into()), Value::I64(11)],
                vec![Value::Text("Gamma".into()), Value::I64(12)],
            ],
        );

        let distinct = |relation| RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![text_eq, i64_eq],
        };
        let query = RelExpr::JoinEq {
            left: Box::new(distinct(left)),
            right: Box::new(distinct(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let prepared = query.prepare(&context, &registry).unwrap();
        assert!(prepared.emits_occurrence_certificate());
        let (value, certificate) = prepared
            .evaluate_with_occurrence_certificate(&model, &context, &registry)
            .unwrap();
        assert_eq!(value.rows().len(), 2);
        assert!(value.rows().iter().all(|row| {
            matches!(&row[0], Value::Text(value) if value.eq_ignore_ascii_case("alpha"))
                && matches!(&row[2], Value::Text(value) if value.eq_ignore_ascii_case("alpha"))
        }));
        assert_eq!(certificate.row_count(), 2);
        let witness = RelationBaseWitness::from_occurrence_certificate(
            RevisionId::new(0),
            target,
            certificate.clone(),
            prepared.result_type().clone(),
            &context,
            &registry,
        )
        .unwrap();
        assert!(certificate.shares_occurrence_root_with(&witness));

        let unsupported = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(distinct(right)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        }
        .prepare(&context, &registry)
        .unwrap();
        assert!(!unsupported.emits_occurrence_certificate());
        assert_eq!(
            unsupported
                .evaluate_with_occurrence_certificate(&model, &context, &registry)
                .unwrap_err(),
            RelQueryError::CanonicalObservationUnavailable
        );
    }

    #[test]
    #[ignore = "manual release-mode compositional Set Join occurrence certificate benchmark"]
    fn set_join_compositional_occurrence_certificate_scale_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let i64_eq = SemanticId::new(91_090);
        let left = SemanticId::new(91_091);
        let right = SemanticId::new(91_092);
        let target = SemanticId::new(91_093);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(917));
        environment.pin_module(i64_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(917));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![
                        TypeExpr::Scalar(ScalarType::I64),
                        TypeExpr::Scalar(ScalarType::I64),
                    ],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![i64_eq, i64_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext { schema, environment };
        let rows = 100_000usize;
        let mut model = FiniteModel::default();
        model.relations.insert(
            left,
            (0..rows)
                .map(|index| vec![Value::I64(index as i64), Value::I64(index as i64)])
                .collect(),
        );
        model.relations.insert(
            right,
            (0..rows)
                .map(|index| {
                    vec![
                        Value::I64(index as i64),
                        Value::I64((index as i64).wrapping_mul(3)),
                    ]
                })
                .collect(),
        );
        let distinct = |relation| RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(relation)),
            column_equivalences: vec![i64_eq, i64_eq],
        };
        let prepared = RelExpr::JoinEq {
            left: Box::new(distinct(left)),
            right: Box::new(distinct(right)),
            left_column: 0,
            right_column: 0,
            equivalence: i64_eq,
        }
        .prepare(&context, &registry)
        .unwrap();
        assert!(prepared.emits_occurrence_certificate());

        for run in 1..=3 {
            let baseline_start = Instant::now();
            let value = prepared.evaluate(&model, &context, &registry).unwrap();
            let witness = RelationBaseWitness::build(
                RevisionId::new(0),
                target,
                value.rows(),
                prepared.result_type().clone(),
                &context,
                &registry,
            )
            .unwrap();
            let baseline = baseline_start.elapsed();
            drop(witness);

            let certified_start = Instant::now();
            let (value, certificate) = prepared
                .evaluate_with_occurrence_certificate(&model, &context, &registry)
                .unwrap();
            let witness = RelationBaseWitness::from_occurrence_certificate(
                RevisionId::new(0),
                target,
                certificate,
                prepared.result_type().clone(),
                &context,
                &registry,
            )
            .unwrap();
            let certified = certified_start.elapsed();
            black_box((value, witness));

            eprintln!(
                "SET_JOIN_COMPOSED_OCC_CERT_PERF run={run} rows={rows} baseline_ms={:.3} certified_ms={:.3} ratio={:.3}x",
                baseline.as_secs_f64() * 1_000.0,
                certified.as_secs_f64() * 1_000.0,
                certified.as_secs_f64() / baseline.as_secs_f64(),
            );
        }
    }

    #[test]
    fn anti_join_composes_left_occurrence_certificate_without_full_row_recanonicalization() {
        let text_eq = SemanticId::new(91_060);
        let i64_eq = SemanticId::new(91_061);
        let source_bag = SemanticId::new(91_062);
        let direct_set = SemanticId::new(91_063);
        let blockers = SemanticId::new(91_064);
        let blockers_second = SemanticId::new(91_065);
        let target = SemanticId::new(91_066);
        let mut registry = SemanticRegistry::default();
        let text_digest =
            registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(914));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(914));
        schema
            .define_relation(RelationDef {
                id: source_bag,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: direct_set,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        for relation in [blockers, blockers_second] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                    semantics: RelationSemantics::Set {
                        column_equivalences: vec![text_eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext { schema, environment };
        let mut model = FiniteModel::default();
        let source_rows = vec![
            vec![Value::Text("Alpha".into()), Value::I64(1)],
            vec![Value::Text("ALPHA".into()), Value::I64(2)],
            vec![Value::Text("Beta".into()), Value::I64(3)],
            vec![Value::Text("Gamma".into()), Value::I64(4)],
        ];
        model.relations.insert(source_bag, source_rows.clone());
        model.relations.insert(direct_set, source_rows);
        model
            .relations
            .insert(blockers, vec![vec![Value::Text("alpha".into())]]);
        model
            .relations
            .insert(blockers_second, vec![vec![Value::Text("GAMMA".into())]]);

        let left = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(source_bag)),
            column_equivalences: vec![text_eq, i64_eq],
        };
        let first = RelExpr::AntiJoin {
            left: Box::new(left),
            right: Box::new(RelExpr::Scan(blockers)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let prepared = first.prepare(&context, &registry).unwrap();
        assert!(prepared.emits_occurrence_certificate());
        let baseline = prepared.evaluate(&model, &context, &registry).unwrap();
        let (certified, certificate) = prepared
            .evaluate_with_occurrence_certificate(&model, &context, &registry)
            .unwrap();
        assert_eq!(certified, baseline);
        assert_eq!(
            certified.rows(),
            &[
                vec![Value::Text("Beta".into()), Value::I64(3)],
                vec![Value::Text("Gamma".into()), Value::I64(4)],
            ]
        );
        let witness = RelationBaseWitness::from_occurrence_certificate(
            RevisionId::new(0),
            target,
            certificate.clone(),
            prepared.result_type().clone(),
            &context,
            &registry,
        )
        .unwrap();
        assert!(certificate.shares_occurrence_root_with(&witness));

        let nested = RelExpr::AntiJoin {
            left: Box::new(first),
            right: Box::new(RelExpr::Scan(blockers_second)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        }
        .prepare(&context, &registry)
        .unwrap();
        assert!(nested.emits_occurrence_certificate());
        let (nested_value, nested_certificate) = nested
            .evaluate_with_occurrence_certificate(&model, &context, &registry)
            .unwrap();
        assert_eq!(
            nested_value.rows(),
            &[vec![Value::Text("Beta".into()), Value::I64(3)]]
        );
        assert_eq!(nested_certificate.row_count(), 1);

        let unsupported_direct_scan = RelExpr::AntiJoin {
            left: Box::new(RelExpr::Scan(direct_set)),
            right: Box::new(RelExpr::Scan(blockers)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        }
        .prepare(&context, &registry)
        .unwrap();
        assert!(!unsupported_direct_scan.emits_occurrence_certificate());
        assert_eq!(
            unsupported_direct_scan
                .evaluate_with_occurrence_certificate(&model, &context, &registry)
                .unwrap_err(),
            RelQueryError::CanonicalObservationUnavailable
        );
    }

    #[test]
    fn physical_scan_witness_seeds_compositional_join_and_antijoin_without_recanonicalization() {
        let text_eq = SemanticId::new(91_067);
        let i64_eq = SemanticId::new(91_068);
        let source = SemanticId::new(91_069);
        let blockers = SemanticId::new(91_069_1);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(915));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(915));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::Text), TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Set { column_equivalences: vec![text_eq, i64_eq] },
        }).unwrap();
        schema.define_relation(RelationDef {
            id: blockers,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set { column_equivalences: vec![text_eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let source_rows = vec![
            vec![Value::Text("Alpha".into()), Value::I64(1)],
            vec![Value::Text("Beta".into()), Value::I64(2)],
            vec![Value::Text("Gamma".into()), Value::I64(3)],
        ];
        let mut model = FiniteModel::default();
        model.relations.insert(source, source_rows.clone());
        model.relations.insert(blockers, vec![vec![Value::Text("ALPHA".into())]]);
        let source_type = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &source_rows, source_type, &context, &registry,
        ).unwrap();
        let handles = (0..source_rows.len()).map(|slot| kernel_types::StableRowHandle { slot, generation: 0 }).collect::<Vec<_>>();
        let seed = witness.scan_occurrence_seed(&handles).unwrap();
        let scan_seeds = BTreeMap::from([(source, seed)]);
        let seeded_relations = BTreeSet::from([source]);

        let anti = RelExpr::AntiJoin {
            left: Box::new(RelExpr::Scan(source)),
            right: Box::new(RelExpr::Scan(blockers)),
            left_column: 0, right_column: 0, equivalence: text_eq,
        }.prepare(&context, &registry).unwrap();
        assert!(!anti.emits_occurrence_certificate());
        assert!(anti.emits_occurrence_certificate_with_scan_seeds(&seeded_relations));
        let baseline = anti.evaluate(&model, &context, &registry).unwrap();
        let (certified, cert) = anti.evaluate_with_occurrence_certificate_seeded(
            &model, &context, &registry, &scan_seeds,
        ).unwrap();
        assert_eq!(certified, baseline);
        assert_eq!(cert.row_count(), 2);

        let join = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(source)),
            right: Box::new(RelExpr::Scan(source)),
            left_column: 0, right_column: 0, equivalence: text_eq,
        }.prepare(&context, &registry).unwrap();
        assert!(!join.emits_occurrence_certificate());
        assert!(join.emits_occurrence_certificate_with_scan_seeds(&seeded_relations));
        let baseline = join.evaluate(&model, &context, &registry).unwrap();
        let (certified, cert) = join.evaluate_with_occurrence_certificate_seeded(
            &model, &context, &registry, &scan_seeds,
        ).unwrap();
        assert_eq!(certified, baseline);
        assert_eq!(cert.row_count(), 3);
    }

    #[test]
    fn scan_occurrence_seed_advances_in_logical_survivor_order_without_recanonicalizing_base() {
        let text_eq = SemanticId::new(91_069_10);
        let source = SemanticId::new(91_069_11);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916_1));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916_1));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set { column_equivalences: vec![text_eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let rows = vec![
            vec![Value::Text("Alpha".into())],
            vec![Value::Text("Beta".into())],
            vec![Value::Text("Gamma".into())],
        ];
        let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &rows, ty.clone(), &context, &registry,
        ).unwrap();
        let handles = (0..rows.len())
            .map(|slot| kernel_types::StableRowHandle { slot, generation: 0 })
            .collect::<Vec<_>>();
        let seed = witness.scan_occurrence_seed(&handles).unwrap();
        let delta = RelationDelta {
            removed: vec![vec![Value::Text("BETA".into())]],
            inserted: vec![vec![Value::Text("Delta".into())]],
            result_type: ty,
        };
        let next = seed.advance_logical_delta(&delta, &context, &registry).unwrap();

        let mut model = FiniteModel::default();
        model.relations.insert(source, vec![rows[0].clone(), rows[2].clone(), delta.inserted[0].clone()]);
        let prepared = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(source)),
            column_equivalences: vec![text_eq],
        }.prepare(&context, &registry).unwrap();
        let seeded = BTreeMap::from([(source, next)]);
        let value = prepared.evaluate_seeded(&model, &context, &registry, &seeded).unwrap();
        assert_eq!(value.rows(), model.relations[&source].to_vec().as_slice());
    }

    #[test]
    fn bag_base_witness_fifo_occurrences_match_logical_survivor_order() {
        let text_eq = SemanticId::new(91_069_120);
        let source = SemanticId::new(91_069_121);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916_31));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916_31));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag { column_equivalences: vec![text_eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let rows = vec![
            vec![Value::Text("Alpha".into())],
            vec![Value::Text("Beta".into())],
            vec![Value::Text("ALPHA".into())],
            vec![Value::Text("Gamma".into())],
        ];
        let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &rows, ty.clone(), &context, &registry,
        ).unwrap();
        let delta = RelationDelta {
            removed: vec![vec![Value::Text("alpha".into())]],
            inserted: vec![vec![Value::Text("Delta".into())]],
            result_type: ty,
        };
        let next_witness = witness.advance(RevisionId::new(1), &delta, &registry).unwrap();
        let seed = next_witness.logical_scan_occurrence_seed().unwrap();
        let expected_rows = vec![
            rows[1].clone(),
            rows[2].clone(),
            rows[3].clone(),
            delta.inserted[0].clone(),
        ];
        let expected = expected_rows.iter()
            .map(|row| canonical_row_key(row, &[text_eq], &context, &registry).unwrap())
            .collect::<Vec<_>>();
        let actual = seed.canonical_keys_by_row().iter().cloned().collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn bag_fifo_occurrence_bucket_survives_head_compaction() {
        let eq = SemanticId::new(91_069_122);
        let source = SemanticId::new(91_069_123);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916_32));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916_32));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag { column_equivalences: vec![eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let rows = (0..192).flat_map(|i| [vec![Value::I64(0)], vec![Value::I64(i + 1)]]).collect::<Vec<_>>();
        let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &rows, ty.clone(), &context, &registry,
        ).unwrap();
        let delta = RelationDelta {
            removed: (0..128).map(|_| vec![Value::I64(0)]).collect(),
            inserted: vec![vec![Value::I64(10_000)]],
            result_type: ty,
        };
        let next = witness.advance(RevisionId::new(1), &delta, &registry).unwrap();
        let seed = next.logical_scan_occurrence_seed().unwrap();
        let mut removals = 128usize;
        let mut expected_rows = Vec::new();
        for row in &rows {
            if row == &vec![Value::I64(0)] && removals != 0 {
                removals -= 1;
            } else {
                expected_rows.push(row.clone());
            }
        }
        expected_rows.push(vec![Value::I64(10_000)]);
        let expected = expected_rows.iter()
            .map(|row| canonical_row_key(row, &[eq], &context, &registry).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(seed.canonical_keys_by_row().iter().cloned().collect::<Vec<_>>(), expected);
    }

    #[test]
    #[ignore = "manual release-mode Bag witness-owned Scan evidence publication benchmark"]
    fn bag_runtime_scan_seed_publication_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;
        let eq = SemanticId::new(91_069_124);
        let source = SemanticId::new(91_069_125);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916_33));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916_33));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag { column_equivalences: vec![eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let rows = (0..100_000usize).map(|i| vec![Value::I64((i % 1024) as i64)]).collect::<Vec<_>>();
        let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &rows, ty.clone(), &context, &registry,
        ).unwrap();
        let handles = (0..rows.len()).map(|slot| kernel_types::StableRowHandle { slot, generation: 0 }).collect::<Vec<_>>();
        let dense = witness.scan_occurrence_seed(&handles).unwrap();
        let delta = RelationDelta {
            removed: vec![vec![Value::I64(7)]],
            inserted: vec![vec![Value::I64(2048)]],
            result_type: ty,
        };
        for run in 0..5 {
            let started = Instant::now();
            let next = black_box(&dense).advance_logical_delta(black_box(&delta), &context, &registry).unwrap();
            eprintln!("bag_dense_seed_publish run={run} ms={:.3} rows={}", started.elapsed().as_secs_f64()*1e3, next.row_count());
        }
        for run in 0..5 {
            let started = Instant::now();
            let next = black_box(&witness).advance(RevisionId::new(1), black_box(&delta), &registry).unwrap();
            let seed = next.logical_scan_occurrence_seed().unwrap();
            eprintln!("bag_witness_seed_publish run={run} ms={:.3} rows={}", started.elapsed().as_secs_f64()*1e3, seed.row_count());
        }
    }

    #[test]
    fn bag_seed_group_churn_keeps_witness_maintained_and_exact_views_coherent() {
        let text_eq = SemanticId::new(91_069_126);
        let i64_eq = SemanticId::new(91_069_127);
        let source = SemanticId::new(91_069_128);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916_34));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916_34));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::Text), TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag { column_equivalences: vec![text_eq, i64_eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let query = RelExpr::Group {
            input: Box::new(RelExpr::Scan(source)),
            group_columns: vec![0],
            group_equivalences: vec![text_eq],
            aggregate: AggregateSpec::Count { result_equivalence: i64_eq },
        };
        let output_ty = query.typecheck(&context, &registry).unwrap();
        let mut model = FiniteModel::default();
        model.relations.insert(source, (0..192usize).map(|i| {
            let name = match i % 3 { 0 => "Alpha", 1 => "BETA", _ => "gamma" };
            vec![Value::Text(name.into()), Value::I64(i as i64)]
        }).collect());
        let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let source_rows = model.relations[&source].to_vec();
        let mut witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &source_rows, ty.clone(), &context, &registry,
        ).unwrap();
        let seeds = BTreeMap::from([(source, witness.logical_scan_occurrence_seed().unwrap())]);
        let mut maintained = MaterializedRelPlanState::build_with_scan_seeds(
            &query, &model, &context, &registry, &seeds,
        ).unwrap();

        for step in 0..128usize {
            let removed = model.relations.get_mut(&source).unwrap().remove(0);
            let name = match step % 3 { 0 => "ALPHA", 1 => "beta", _ => "Gamma" };
            let inserted = vec![Value::Text(name.into()), Value::I64((10_000 + step) as i64)];
            model.relations.get_mut(&source).unwrap().push(inserted.clone());
            let delta = RelationDelta {
                removed: vec![removed],
                inserted: vec![inserted],
                result_type: ty.clone(),
            };
            witness = witness.advance(RevisionId::new((step + 1) as u64), &delta, &registry).unwrap();
            maintained.apply_relation_deltas(
                &BTreeMap::from([(source, delta)]), &context, &registry,
            ).unwrap();
            let maintained_value = maintained.output_value(&context, &registry).unwrap();
            let exact_value = query.evaluate(&model, &context, &registry).unwrap();
            assert!(relation_values_semantically_equivalent(
                &maintained_value, &exact_value, &output_ty, &context, &registry,
            ).unwrap());
        }

        let seed = witness.logical_scan_occurrence_seed().unwrap();
        let expected = model.relations[&source].iter().map(|row| {
            canonical_row_key(row, &[text_eq, i64_eq], &context, &registry).unwrap()
        }).collect::<Vec<_>>();
        assert_eq!(seed.canonical_keys_by_row().iter().cloned().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn set_base_witness_owns_logical_scan_evidence_across_bounded_delta() {
        let text_eq = SemanticId::new(91_069_12);
        let source = SemanticId::new(91_069_13);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916_3));
        environment.pin_module(text_eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916_3));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Set { column_equivalences: vec![text_eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let rows = vec![
            vec![Value::Text("Alpha".into())],
            vec![Value::Text("Beta".into())],
            vec![Value::Text("Gamma".into())],
        ];
        let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &rows, ty.clone(), &context, &registry,
        ).unwrap();
        let delta = RelationDelta {
            removed: vec![vec![Value::Text("BETA".into())]],
            inserted: vec![vec![Value::Text("Delta".into())]],
            result_type: ty,
        };
        let next_witness = witness.advance(RevisionId::new(1), &delta, &registry).unwrap();
        let seed = next_witness.logical_scan_occurrence_seed().unwrap();

        let mut model = FiniteModel::default();
        model.relations.insert(source, vec![rows[0].clone(), rows[2].clone(), delta.inserted[0].clone()]);
        let prepared = RelExpr::Distinct {
            input: Box::new(RelExpr::Scan(source)),
            column_equivalences: vec![text_eq],
        }.prepare(&context, &registry).unwrap();
        let seeded = BTreeMap::from([(source, seed)]);
        let value = prepared.evaluate_seeded(&model, &context, &registry, &seeded).unwrap();
        assert_eq!(value.rows(), model.relations[&source].to_vec().as_slice());
    }

    #[test]
    #[ignore = "manual release-mode seeded physical Scan certificate benchmark"]
    fn seeded_physical_scan_join_occurrence_certificate_scale_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let eq = SemanticId::new(91_069_2);
        let source = SemanticId::new(91_069_3);
        let target = SemanticId::new(91_069_4);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Set { column_equivalences: vec![eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let rows = (0..100_000).map(|raw| vec![Value::I64(raw)]).collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(source, rows.clone());
        let source_type = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let source_witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &rows, source_type, &context, &registry,
        ).unwrap();
        let handles = (0..rows.len()).map(|slot| kernel_types::StableRowHandle { slot, generation: 0 }).collect::<Vec<_>>();
        let query = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(source)),
            right: Box::new(RelExpr::Scan(source)),
            left_column: 0, right_column: 0, equivalence: eq,
        }.prepare(&context, &registry).unwrap();
        let target_type = query.result_type().clone();
        let seeded_relations = BTreeSet::from([source]);
        assert!(query.emits_occurrence_certificate_with_scan_seeds(&seeded_relations));
        let shared_seed = source_witness.scan_occurrence_seed(&handles).unwrap();

        for run in 0..4 {
            let started = Instant::now();
            let baseline_value = query.evaluate(black_box(&model), &context, &registry).unwrap();
            let baseline_rows = baseline_value.into_rows();
            let _baseline_witness = RelationBaseWitness::build(
                RevisionId::new(0), target, &baseline_rows, target_type.clone(), &context, &registry,
            ).unwrap();
            let baseline = started.elapsed();

            let started = Instant::now();
            let seed = source_witness.scan_occurrence_seed(black_box(&handles)).unwrap();
            let seed_cost = started.elapsed();
            let scan_seeds = BTreeMap::from([(source, seed)]);
            let eval_started = Instant::now();
            let (_certified_value, certificate) = query.evaluate_with_occurrence_certificate_seeded(
                black_box(&model), &context, &registry, &scan_seeds,
            ).unwrap();
            let _eval_cost = eval_started.elapsed();
            let adopt_started = Instant::now();
            let _certified_witness = RelationBaseWitness::from_occurrence_certificate(
                RevisionId::new(0), target, certificate, target_type.clone(), &context, &registry,
            ).unwrap();
            let _adopt_cost = adopt_started.elapsed();
            let certified = started.elapsed();

            let shared_started = Instant::now();
            let shared_scan_seeds = BTreeMap::from([(source, shared_seed.clone())]);
            let shared_clone_cost = shared_started.elapsed();
            let shared_eval_started = Instant::now();
            let (_shared_value, shared_certificate) = query.evaluate_with_occurrence_certificate_seeded(
                black_box(&model), &context, &registry, &shared_scan_seeds,
            ).unwrap();
            let shared_eval_cost = shared_eval_started.elapsed();
            let shared_adopt_started = Instant::now();
            let _shared_witness = RelationBaseWitness::from_occurrence_certificate(
                RevisionId::new(0), target, shared_certificate, target_type.clone(), &context, &registry,
            ).unwrap();
            let shared_adopt_cost = shared_adopt_started.elapsed();
            let shared_total = shared_started.elapsed();
            eprintln!(
                "seeded_scan_join run={run} baseline_ms={:.3} rebuilt_seed_ms={:.3} rebuilt_total_ms={:.3} rebuilt_ratio={:.3} shared_clone_ms={:.3} shared_eval_ms={:.3} shared_adopt_ms={:.3} shared_total_ms={:.3} shared_ratio={:.3}",
                baseline.as_secs_f64() * 1e3, seed_cost.as_secs_f64() * 1e3, certified.as_secs_f64() * 1e3,
                certified.as_secs_f64() / baseline.as_secs_f64(),
                shared_clone_cost.as_secs_f64() * 1e3, shared_eval_cost.as_secs_f64() * 1e3,
                shared_adopt_cost.as_secs_f64() * 1e3, shared_total.as_secs_f64() * 1e3,
                shared_total.as_secs_f64() / baseline.as_secs_f64(),
            );
        }
    }

    #[test]
    #[ignore = "manual release-mode runtime Scan seed publication benchmark"]
    fn runtime_scan_seed_logical_delta_publication_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let eq = SemanticId::new(91_069_20);
        let source = SemanticId::new(91_069_21);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(916_2));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(916_2));
        schema.define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Set { column_equivalences: vec![eq] },
        }).unwrap();
        let context = SemanticContext { schema, environment };
        let rows = (0..100_000).map(|raw| vec![Value::I64(raw)]).collect::<Vec<_>>();
        let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
        let witness = RelationBaseWitness::build(
            RevisionId::new(0), source, &rows, ty.clone(), &context, &registry,
        ).unwrap();
        let handles = (0..rows.len())
            .map(|slot| kernel_types::StableRowHandle { slot, generation: 0 })
            .collect::<Vec<_>>();
        let seed = witness.scan_occurrence_seed(&handles).unwrap();
        let delta = RelationDelta {
            removed: vec![vec![Value::I64(7)]],
            inserted: vec![vec![Value::I64(100_001)]],
            result_type: ty,
        };
        for run in 0..5 {
            let started = Instant::now();
            let next = black_box(&seed)
                .advance_logical_delta(black_box(&delta), &context, &registry)
                .unwrap();
            eprintln!(
                "runtime_scan_seed_publish run={run} ms={:.3} rows={}",
                started.elapsed().as_secs_f64() * 1e3,
                next.row_count(),
            );
        }

        for run in 0..5 {
            let started = Instant::now();
            let next_witness = black_box(&witness)
                .advance(RevisionId::new(1), black_box(&delta), &registry)
                .unwrap();
            let seed = next_witness.logical_scan_occurrence_seed().unwrap();
            eprintln!(
                "runtime_witness_owned_seed_publish run={run} ms={:.3} rows={}",
                started.elapsed().as_secs_f64() * 1e3,
                seed.row_count(),
            );
        }
    }

    #[test]
    #[ignore = "manual release-mode compositional AntiJoin occurrence certificate benchmark"]
    fn anti_join_compositional_occurrence_certificate_scale_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let text_eq = SemanticId::new(91_070);
        let i64_eq = SemanticId::new(91_071);
        let source = SemanticId::new(91_072);
        let blockers = SemanticId::new(91_073);
        let target = SemanticId::new(91_074);
        let mut registry = SemanticRegistry::default();
        let text_digest =
            registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(915));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let mut schema = Schema::new(SchemaRevisionId::new(915));
        schema
            .define_relation(RelationDef {
                id: source,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: blockers,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let context = SemanticContext { schema, environment };
        let rows = 100_000usize;
        let mut model = FiniteModel::default();
        model.relations.insert(
            source,
            (0..rows)
                .map(|index| {
                    vec![
                        Value::Text(format!("Key{index}")),
                        Value::I64(index as i64),
                    ]
                })
                .collect(),
        );
        model.relations.insert(
            blockers,
            (0..rows)
                .step_by(10)
                .map(|index| vec![Value::Text(format!("KEY{index}"))])
                .collect(),
        );

        let query = RelExpr::AntiJoin {
            left: Box::new(RelExpr::Distinct {
                input: Box::new(RelExpr::Scan(source)),
                column_equivalences: vec![text_eq, i64_eq],
            }),
            right: Box::new(RelExpr::Scan(blockers)),
            left_column: 0,
            right_column: 0,
            equivalence: text_eq,
        };
        let prepared = query.prepare(&context, &registry).unwrap();
        assert!(prepared.emits_occurrence_certificate());

        for run in 1..=3 {
            let baseline_start = Instant::now();
            let value = prepared.evaluate(&model, &context, &registry).unwrap();
            let witness = RelationBaseWitness::build(
                RevisionId::new(0),
                target,
                value.rows(),
                prepared.result_type().clone(),
                &context,
                &registry,
            )
            .unwrap();
            let baseline = baseline_start.elapsed();
            drop(witness);

            let certified_start = Instant::now();
            let (value, certificate) = prepared
                .evaluate_with_occurrence_certificate(&model, &context, &registry)
                .unwrap();
            let witness = RelationBaseWitness::from_occurrence_certificate(
                RevisionId::new(0),
                target,
                certificate,
                prepared.result_type().clone(),
                &context,
                &registry,
            )
            .unwrap();
            let certified = certified_start.elapsed();
            black_box((value, witness));

            eprintln!(
                "ANTI_JOIN_COMPOSED_OCC_CERT_PERF run={run} rows={rows} baseline_ms={:.3} certified_ms={:.3} ratio={:.3}x",
                baseline.as_secs_f64() * 1_000.0,
                certified.as_secs_f64() * 1_000.0,
                certified.as_secs_f64() / baseline.as_secs_f64(),
            );
        }
    }

    #[test]
    #[ignore = "manual release-mode quotient occurrence certificate benchmark"]
    fn quotient_operator_occurrence_certificate_scale_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let eq = SemanticId::new(91_040);
        let set_relation = SemanticId::new(91_041);
        let bag_relation = SemanticId::new(91_042);
        let target = SemanticId::new(91_043);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(913));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(913));
        schema
            .define_relation(RelationDef {
                id: set_relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq, eq],
                },
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: bag_relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq, eq],
                },
            })
            .unwrap();
        let context = SemanticContext { schema, environment };
        let rows = 100_000usize;
        let values = (0..rows)
            .map(|index| {
                vec![
                    Value::I64((index % 50_000) as i64),
                    Value::I64(index as i64),
                ]
            })
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(set_relation, values.clone());
        model.relations.insert(bag_relation, values);

        let queries = [
            (
                "project_set",
                RelExpr::Project {
                    input: Box::new(RelExpr::Scan(set_relation)),
                    columns: vec![0],
                },
            ),
            (
                "distinct",
                RelExpr::Distinct {
                    input: Box::new(RelExpr::Project {
                        input: Box::new(RelExpr::Scan(bag_relation)),
                        columns: vec![0],
                    }),
                    column_equivalences: vec![eq],
                },
            ),
            (
                "difference_set",
                RelExpr::Difference {
                    left: Box::new(RelExpr::Scan(set_relation)),
                    right: Box::new(RelExpr::FilterEqConst {
                        input: Box::new(RelExpr::Scan(set_relation)),
                        column: 0,
                        value: Value::I64(0),
                        equivalence: eq,
                    }),
                },
            ),
        ];

        for (label, query) in queries {
            let result_type = query.typecheck(&context, &registry).unwrap();
            for run in 1..=3 {
                let baseline_start = Instant::now();
                let value = query.evaluate(&model, &context, &registry).unwrap();
                let witness = RelationBaseWitness::build(
                    RevisionId::new(0),
                    target,
                    value.rows(),
                    result_type.clone(),
                    &context,
                    &registry,
                )
                .unwrap();
                let baseline = baseline_start.elapsed();
                drop(witness);

                let certified_start = Instant::now();
                let (value, certificate) = query
                    .evaluate_with_occurrence_certificate(&model, &context, &registry)
                    .unwrap();
                let witness = RelationBaseWitness::from_occurrence_certificate(
                    RevisionId::new(0),
                    target,
                    certificate,
                    result_type.clone(),
                    &context,
                    &registry,
                )
                .unwrap();
                let certified = certified_start.elapsed();
                black_box((value, witness));

                eprintln!(
                    "QUOTIENT_OCC_CERT_PERF op={label} run={run} rows={rows} baseline_ms={:.3} certified_ms={:.3} ratio={:.3}x",
                    baseline.as_secs_f64() * 1_000.0,
                    certified.as_secs_f64() * 1_000.0,
                    certified.as_secs_f64() / baseline.as_secs_f64(),
                );
            }
        }
    }

    #[test]
    #[ignore = "manual release-mode execution occurrence certificate benchmark"]
    fn union_set_execution_certificate_scale_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let eq = SemanticId::new(91_100);
        let left = SemanticId::new(91_101);
        let right = SemanticId::new(91_102);
        let target = SemanticId::new(91_103);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(911));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(911));
        for relation in [left, right] {
            schema
                .define_relation(RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                    semantics: RelationSemantics::Set {
                        column_equivalences: vec![eq],
                    },
                })
                .unwrap();
        }
        let context = SemanticContext { schema, environment };
        let mut model = FiniteModel::default();
        let side = 50_000usize;
        model.relations.insert(
            left,
            (0..side).map(|v| vec![Value::I64(v as i64)]).collect(),
        );
        model.relations.insert(
            right,
            (side..side * 2).map(|v| vec![Value::I64(v as i64)]).collect(),
        );
        let query = RelExpr::Union {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        };
        let result_type = query.typecheck(&context, &registry).unwrap();

        let baseline_start = Instant::now();
        let baseline_value = query.evaluate(&model, &context, &registry).unwrap();
        let baseline_witness = RelationBaseWitness::build(
            RevisionId::new(0),
            target,
            baseline_value.rows(),
            result_type.clone(),
            &context,
            &registry,
        )
        .unwrap();
        let baseline = baseline_start.elapsed();
        black_box(baseline_witness);

        let certified_start = Instant::now();
        let (certified_value, certificate) = query
            .evaluate_with_occurrence_certificate(&model, &context, &registry)
            .unwrap();
        let certified_witness = RelationBaseWitness::from_occurrence_certificate(
            RevisionId::new(0),
            target,
            certificate,
            result_type,
            &context,
            &registry,
        )
        .unwrap();
        let certified = certified_start.elapsed();
        black_box((certified_value, certified_witness));

        eprintln!(
            "UNION_SET_OCC_CERT_PERF rows={} baseline_ms={:.3} certified_ms={:.3} ratio={:.3}x",
            side * 2,
            baseline.as_secs_f64() * 1_000.0,
            certified.as_secs_f64() * 1_000.0,
            certified.as_secs_f64() / baseline.as_secs_f64(),
        );
    }

    #[test]
    fn nested_set_tree_reuses_scan_and_child_gamma_evidence_without_recanonicalization() {
        let eq = SemanticId::new(91_080);
        let left = SemanticId::new(91_081);
        let right = SemanticId::new(91_082);
        let blockers = SemanticId::new(91_083);
        let target = SemanticId::new(91_084);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(918));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(918));
        for relation in [left, right, blockers] {
            schema.define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Set { column_equivalences: vec![eq] },
            }).unwrap();
        }
        let context = SemanticContext { schema, environment };
        let left_rows = vec![vec![Value::I64(1)], vec![Value::I64(2)], vec![Value::I64(3)]];
        let right_rows = vec![vec![Value::I64(3)], vec![Value::I64(4)], vec![Value::I64(5)]];
        let blocker_rows = vec![vec![Value::I64(2)], vec![Value::I64(5)]];
        let mut model = FiniteModel::default();
        model.relations.insert(left, left_rows.clone());
        model.relations.insert(right, right_rows.clone());
        model.relations.insert(blockers, blocker_rows.clone());

        let one_col_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let seed_for = |relation, rows: &[Row]| {
            let witness = RelationBaseWitness::build(
                RevisionId::new(0), relation, rows, one_col_type.clone(), &context, &registry,
            ).unwrap();
            let handles = (0..rows.len())
                .map(|slot| kernel_types::StableRowHandle { slot, generation: 0 })
                .collect::<Vec<_>>();
            witness.scan_occurrence_seed(&handles).unwrap()
        };
        let scan_seeds = BTreeMap::from([
            (left, seed_for(left, &left_rows)),
            (right, seed_for(right, &right_rows)),
            (blockers, seed_for(blockers, &blocker_rows)),
        ]);
        let seeded_relations = BTreeSet::from([left, right, blockers]);

        let joined_left = RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(left)),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
        let projected_left = RelExpr::Project {
            input: Box::new(joined_left),
            columns: vec![0],
        };
        let filtered_right = RelExpr::AntiJoin {
            left: Box::new(RelExpr::Scan(right)),
            right: Box::new(RelExpr::Scan(blockers)),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
        let union = RelExpr::Union {
            left: Box::new(projected_left),
            right: Box::new(filtered_right),
        };
        let difference = RelExpr::Difference {
            left: Box::new(union),
            right: Box::new(RelExpr::Scan(blockers)),
        };
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(difference),
                columns: vec![0],
            }),
            column_equivalences: vec![eq],
        }.prepare(&context, &registry).unwrap();

        assert!(query.emits_occurrence_certificate_with_scan_seeds(&seeded_relations));
        let baseline = query.evaluate(&model, &context, &registry).unwrap();
        let (certified, certificate) = query.evaluate_with_occurrence_certificate_seeded(
            &model, &context, &registry, &scan_seeds,
        ).unwrap();
        assert_eq!(certified, baseline);
        assert_eq!(certificate.row_count(), certified.rows().len());
        let witness = RelationBaseWitness::from_occurrence_certificate(
            RevisionId::new(0), target, certificate, query.result_type().clone(), &context, &registry,
        ).unwrap();
        drop(witness);
    }

    #[test]
    #[ignore = "manual release-mode nested compositional occurrence certificate benchmark"]
    fn nested_set_tree_occurrence_certificate_scale_benchmark() {
        use std::hint::black_box;
        use std::time::Instant;

        let eq = SemanticId::new(91_085);
        let left = SemanticId::new(91_086);
        let right = SemanticId::new(91_087);
        let blockers = SemanticId::new(91_088);
        let target = SemanticId::new(91_089);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(919));
        environment.pin_module(eq, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(919));
        for relation in [left, right, blockers] {
            schema.define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Set { column_equivalences: vec![eq] },
            }).unwrap();
        }
        let context = SemanticContext { schema, environment };
        let n = 100_000usize;
        let left_rows = (0..n).map(|v| vec![Value::I64(v as i64)]).collect::<Vec<_>>();
        let right_rows = (n / 2..n + n / 2).map(|v| vec![Value::I64(v as i64)]).collect::<Vec<_>>();
        let blocker_rows = (0..n).step_by(8).map(|v| vec![Value::I64(v as i64)]).collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(left, left_rows.clone());
        model.relations.insert(right, right_rows.clone());
        model.relations.insert(blockers, blocker_rows.clone());
        let one_col_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let seed_for = |relation, rows: &[Row]| {
            let witness = RelationBaseWitness::build(
                RevisionId::new(0), relation, rows, one_col_type.clone(), &context, &registry,
            ).unwrap();
            let handles = (0..rows.len()).map(|slot| kernel_types::StableRowHandle { slot, generation: 0 }).collect::<Vec<_>>();
            witness.scan_occurrence_seed(&handles).unwrap()
        };
        let scan_seeds = BTreeMap::from([
            (left, seed_for(left, &left_rows)),
            (right, seed_for(right, &right_rows)),
            (blockers, seed_for(blockers, &blocker_rows)),
        ]);
        let query = RelExpr::Distinct {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::Difference {
                    left: Box::new(RelExpr::Union {
                        left: Box::new(RelExpr::Project {
                            input: Box::new(RelExpr::JoinEq {
                                left: Box::new(RelExpr::Scan(left)),
                                right: Box::new(RelExpr::Scan(left)),
                                left_column: 0,
                                right_column: 0,
                                equivalence: eq,
                            }),
                            columns: vec![0],
                        }),
                        right: Box::new(RelExpr::AntiJoin {
                            left: Box::new(RelExpr::Scan(right)),
                            right: Box::new(RelExpr::Scan(blockers)),
                            left_column: 0,
                            right_column: 0,
                            equivalence: eq,
                        }),
                    }),
                    right: Box::new(RelExpr::Scan(blockers)),
                }),
                columns: vec![0],
            }),
            column_equivalences: vec![eq],
        }.prepare(&context, &registry).unwrap();
        let target_type = query.result_type().clone();

        for run in 0..4 {
            let baseline_started = Instant::now();
            let baseline_value = query.evaluate(black_box(&model), &context, &registry).unwrap();
            let baseline_witness = RelationBaseWitness::build(
                RevisionId::new(0), target, baseline_value.rows(), target_type.clone(), &context, &registry,
            ).unwrap();
            black_box(baseline_witness);
            let baseline = baseline_started.elapsed();

            let certified_started = Instant::now();
            let (certified_value, certificate) = query.evaluate_with_occurrence_certificate_seeded(
                black_box(&model), &context, &registry, black_box(&scan_seeds),
            ).unwrap();
            let certified_witness = RelationBaseWitness::from_occurrence_certificate(
                RevisionId::new(0), target, certificate, target_type.clone(), &context, &registry,
            ).unwrap();
            black_box((certified_value, certified_witness));
            let certified = certified_started.elapsed();
            eprintln!(
                "nested_occ_cert run={run} baseline_ms={:.3} certified_ms={:.3} ratio={:.3}",
                baseline.as_secs_f64() * 1e3,
                certified.as_secs_f64() * 1e3,
                certified.as_secs_f64() / baseline.as_secs_f64(),
            );
        }
    }

}

#[cfg(test)]
mod p420_retention_tests {
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};
use super::*;

#[test]
fn pinned_set_witness_lineage_retains_path_copy_nodes_and_reclaims_old_unique_nodes() {
    let eq = SemanticId::new(91_070_420);
    let source = SemanticId::new(91_070_421);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(917_420));
    environment.pin_module(eq, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(917_420));
    schema
        .define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![eq],
            },
        })
        .unwrap();
    let context = SemanticContext { schema, environment };
    let mut rows = (0..4_096_i64)
        .map(|value| vec![Value::I64(value)])
        .collect::<Vec<_>>();
    let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
    let mut lineage = vec![
        RelationBaseWitness::build(
            RevisionId::new(0),
            source,
            &rows,
            ty.clone(),
            &context,
            &registry,
        )
        .unwrap(),
    ];

    for step in 0..64_usize {
        let removed = rows.remove(0);
        let inserted = vec![Value::I64(10_000 + step as i64)];
        rows.push(inserted.clone());
        let next = lineage
            .last()
            .unwrap()
            .advance(
                RevisionId::new((step + 1) as u64),
                &RelationDelta {
                    inserted: vec![inserted],
                    removed: vec![removed],
                    result_type: ty.clone(),
                },
                &registry,
            )
            .unwrap();
        lineage.push(next);
    }

    let baseline = lineage[0].storage_stats().total_nodes();
    let newly_retained = lineage
        .windows(2)
        .map(|pair| pair[1].structural_nodes_new_since(&pair[0]))
        .sum::<usize>();
    let retained_union = baseline + newly_retained;
    let naive_full_copy = baseline * lineage.len();
    eprintln!(
        "P420_SET_RETENTION baseline_nodes={baseline} snapshots={} newly_retained_nodes={newly_retained} retained_union_nodes={retained_union} naive_full_copy_nodes={naive_full_copy} union_to_naive={:.6}",
        lineage.len(),
        retained_union as f64 / naive_full_copy as f64,
    );
    assert!(baseline > 8_000, "fixture must exercise a non-trivial persistent tree");
    assert!(
        retained_union * 16 < naive_full_copy,
        "pinned snapshots retained too much structural storage: union={retained_union} naive={naive_full_copy}"
    );

    let probe = lineage[0].unique_storage_probe_against(&lineage[1]);
    assert!(probe.total_nodes() > 0);
    assert_eq!(probe.live_nodes(), probe.total_nodes());
    let oldest = lineage.remove(0);
    drop(oldest);
    assert!(
        probe.is_fully_reclaimed(),
        "nodes unique to the dropped oldest witness remained retained"
    );
}

#[test]
fn pinned_bag_fifo_witness_lineage_retains_path_copy_nodes_and_reclaims_old_unique_nodes() {
    let eq = SemanticId::new(91_070_422);
    let source = SemanticId::new(91_070_423);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(917_421));
    environment.pin_module(eq, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(917_421));
    schema
        .define_relation(RelationDef {
            id: source,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![eq],
            },
        })
        .unwrap();
    let context = SemanticContext { schema, environment };
    let rows = (0..4_096_i64)
        .map(|value| vec![Value::I64(value % 64)])
        .collect::<Vec<_>>();
    let ty = RelExpr::Scan(source).typecheck(&context, &registry).unwrap();
    let mut lineage = vec![
        RelationBaseWitness::build(
            RevisionId::new(0),
            source,
            &rows,
            ty.clone(),
            &context,
            &registry,
        )
        .unwrap(),
    ];

    for step in 0..64_usize {
        let value = (step % 64) as i64;
        let row = vec![Value::I64(value)];
        let next = lineage
            .last()
            .unwrap()
            .advance(
                RevisionId::new((step + 1) as u64),
                &RelationDelta {
                    inserted: vec![row.clone()],
                    removed: vec![row],
                    result_type: ty.clone(),
                },
                &registry,
            )
            .unwrap();
        lineage.push(next);
    }

    let baseline = lineage[0].storage_stats().total_nodes();
    let newly_retained = lineage
        .windows(2)
        .map(|pair| pair[1].structural_nodes_new_since(&pair[0]))
        .sum::<usize>();
    let retained_union = baseline + newly_retained;
    let naive_full_copy = baseline * lineage.len();
    eprintln!(
        "P420_BAG_RETENTION baseline_nodes={baseline} snapshots={} newly_retained_nodes={newly_retained} retained_union_nodes={retained_union} naive_full_copy_nodes={naive_full_copy} union_to_naive={:.6}",
        lineage.len(),
        retained_union as f64 / naive_full_copy as f64,
    );
    assert!(
        retained_union * 8 < naive_full_copy,
        "FIFO bag churn retained too much structural storage: union={retained_union} naive={naive_full_copy}"
    );

    let probe = lineage[0].unique_storage_probe_against(&lineage[1]);
    assert!(probe.total_nodes() > 0);
    let oldest = lineage.remove(0);
    drop(oldest);
    assert!(probe.is_fully_reclaimed());
}
}
