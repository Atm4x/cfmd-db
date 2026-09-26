#[test]
fn nine_way_raw_cycle_collapses_to_certified_common_quotient_without_semantic_drift() {
    const LEAVES: usize = 9;
    let relations = (0..LEAVES)
        .map(|offset| sid(1_820 + offset as u64))
        .collect::<Vec<_>>();
    let equivalences = (0..LEAVES)
        .map(|offset| sid(1_840 + offset as u64))
        .collect::<Vec<_>>();
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1_820));
    let mut schema = Schema::new(SchemaRevisionId::new(1_820));
    for equivalence in &equivalences {
        environment.pin_module(*equivalence, digest);
    }
    for relation in &relations {
        schema
            .define_relation(RelationDef {
                id: *relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalences[0]],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let bindings = (0..LEAVES)
        .map(|offset| LayoutBinding {
            id: LayoutId(1_820 + offset as u128),
            family: LayoutFamily::Columnar,
        })
        .collect::<Vec<_>>();
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings.iter().copied()) {
        catalog.bind_relation(relation, binding);
    }

    let mut logical = RelExpr::Scan(relations[0]);
    for leaf in 1..LEAVES {
        logical = RelExpr::JoinEq {
            left: Box::new(logical),
            right: Box::new(RelExpr::Scan(relations[leaf])),
            left_column: leaf - 1,
            right_column: 0,
            equivalence: equivalences[leaf - 1],
        };
    }
    logical = RelExpr::FilterEqColumns {
        input: Box::new(logical),
        left_column: LEAVES - 1,
        right_column: 0,
        equivalence: equivalences[LEAVES - 1],
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let program = prepared.semantic_quotient_program.as_ref().unwrap();
    assert_eq!(program.specs.len(), 1);
    assert_eq!(program.specs[0].1.len(), LEAVES);
    assert_eq!(program.hypergraph_order.as_ref().unwrap().len(), LEAVES);

    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for (relation, binding) in relations.iter().copied().zip(bindings.iter().copied()) {
        let values = vec![1_i64, 2];
        store
            .install(
                relation,
                binding,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(values.clone().into())])
                    .unwrap(),
            )
            .unwrap();
        model.relations.insert(
            relation,
            values
                .into_iter()
                .map(|value| vec![Value::I64(value)])
                .collect(),
        );
    }
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 2);
    assert_eq!(stats.multiway_join_apnf_executions, 1);
    assert_eq!(stats.multiway_join_prepared_quotient_hits, 0);
}

struct CyclicTextFixture {
    context: SemanticContext,
    registry: SemanticRegistry,
    relations: Vec<SemanticId>,
    bindings: Vec<LayoutBinding>,
    prepared: PreparedPlan,
}

fn ten_way_non_gyo_text_cycle_fixture() -> CyclicTextFixture {
    const LEAVES: usize = 10;
    let relations = (0..LEAVES)
        .map(|offset| sid(1_900 + offset as u64))
        .collect::<Vec<_>>();
    let exact = sid(1_920);
    let ascii_ci = sid(1_921);
    let mut registry = SemanticRegistry::default();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1_900));
    environment.pin_module(exact, exact_digest);
    environment.pin_module(ascii_ci, ci_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1_900));
    for relation in &relations {
        schema
            .define_relation(RelationDef {
                id: *relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![exact, ascii_ci],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let bindings = (0..LEAVES)
        .map(|offset| LayoutBinding {
            id: LayoutId(1_900 + offset as u128),
            family: LayoutFamily::Columnar,
        })
        .collect::<Vec<_>>();
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings.iter().copied()) {
        catalog.bind_relation(relation, binding);
    }

    let mut logical = RelExpr::Scan(relations[0]);
    for (leaf, relation) in relations.iter().copied().enumerate().skip(1) {
        let edge = leaf - 1;
        let (column, equivalence) = if edge % 2 == 0 {
            (0, exact)
        } else {
            (1, ascii_ci)
        };
        logical = RelExpr::JoinEq {
            left: Box::new(logical),
            right: Box::new(RelExpr::Scan(relation)),
            left_column: (leaf - 1) * 2 + column,
            right_column: column,
            equivalence,
        };
    }
    logical = RelExpr::FilterEqColumns {
        input: Box::new(logical),
        left_column: (LEAVES - 1) * 2 + 1,
        right_column: 1,
        equivalence: ascii_ci,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    CyclicTextFixture {
        context,
        registry,
        relations,
        bindings,
        prepared,
    }
}

fn install_cyclic_text_rows(
    fixture: &CyclicTextFixture,
    rows_per_leaf: usize,
) -> (PhysicalStore, kernel_model::FiniteModel) {
    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for (leaf, (relation, binding)) in fixture
        .relations
        .iter()
        .copied()
        .zip(fixture.bindings.iter().copied())
        .enumerate()
    {
        let (first, second) = if rows_per_leaf == 2 {
            let second = if leaf % 2 == 0 {
                vec!["a".to_owned(), "b".to_owned()]
            } else {
                vec!["A".to_owned(), "B".to_owned()]
            };
            (vec!["A".to_owned(), "B".to_owned()], second)
        } else {
            (
                vec!["A".to_owned(); rows_per_leaf],
                vec!["a".to_owned(); rows_per_leaf],
            )
        };
        store
            .install(
                relation,
                binding,
                NativeRelation::typed_columnar(vec![
                    NativeColumn::Text(first.clone().into()),
                    NativeColumn::Text(second.clone().into()),
                ])
                .unwrap(),
            )
            .unwrap();
        model.relations.insert(
            relation,
            first
                .into_iter()
                .zip(second)
                .map(|(first, second)| vec![Value::Text(first), Value::Text(second)])
                .collect(),
        );
    }
    (store, model)
}

#[test]
fn four_way_subset_join_uses_order_preserving_gamma_quotients() {
    let FourWaySubsetFixture {
        context,
        registry,
        relations,
        equivalence,
        bindings,
        logical,
        prepared,
    } = four_way_subset_fixture();
    let values = [
        (
            (1_i64..=1_000).collect::<Vec<_>>(),
            (1_i64..=1_000).collect::<Vec<_>>(),
        ),
        (vec![1_i64; 1_000], (1_i64..=1_000).collect::<Vec<_>>()),
        (vec![0_i64], vec![1_i64]),
        (vec![0_i64, 0], vec![1_i64, 2]),
    ];
    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for ((relation, layout), (first, second)) in relations.into_iter().zip(bindings).zip(values) {
        install_two_i64_fixture_relation(&mut store, &mut model, relation, layout, first, second);
    }
    for (relation, layout, column) in [
        (relations[0], bindings[0], 0_usize),
        (relations[0], bindings[0], 1),
        (relations[1], bindings[1], 0),
        (relations[1], bindings[1], 1),
        (relations[2], bindings[2], 1),
        (relations[3], bindings[3], 1),
    ] {
        store
            .install_semantic_statistics(
                SemanticIndexBinding::single(relation, layout, column, equivalence),
                &context,
                &registry,
            )
            .unwrap();
    }

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 2);
    assert_eq!(stats.multiway_join_apnf_executions, 1);
    assert_eq!(stats.multiway_join_prepared_quotient_hits, 0);
    assert_eq!(stats.scanned_rows, 2_003);
}

fn assert_quotient_support_cache_survives_delta(
    prepared: &PreparedPlan,
    store: &mut PhysicalStore,
    registry: &SemanticRegistry,
    logical: &RelExpr,
    model: &mut kernel_model::FiniteModel,
    context: &SemanticContext,
    target: (SemanticId, LayoutBinding),
) {
    let (before, before_stats) = prepared.execute_native_pinned(store, registry).unwrap();
    assert_eq!(before, logical.evaluate(model, context, registry).unwrap());
    assert_eq!(before_stats.multiway_join_prepared_quotient_hits, 1);
    assert_eq!(before_stats.multiway_join_maintained_quotient_key_hits, 140);
    assert!(
        prepared
            .materialize_semantic_quotient_support(store, registry)
            .unwrap()
    );
    assert!(
        !prepared
            .materialize_semantic_quotient_support(store, registry)
            .unwrap()
    );
    let (cached, cached_stats) = prepared.execute_native_pinned(store, registry).unwrap();
    assert_eq!(cached, before);
    assert_eq!(
        cached_stats.multiway_join_maintained_quotient_support_hits,
        1
    );
    assert_eq!(cached_stats.multiway_join_maintained_quotient_key_hits, 0);

    let (relation, layout) = target;
    let before_invalid = store.clone();
    let invalid = scan_delta(relation, &[], &[999], context, registry);
    assert_eq!(
        store.apply_relation_delta(relation, layout, &invalid, context, registry),
        Err(RelQueryError::InconsistentIncrementalDelta.into())
    );
    assert_eq!(*store, before_invalid);

    let delta = scan_delta(relation, &[21], &[20], context, registry);
    store
        .apply_relation_delta(relation, layout, &delta, context, registry)
        .unwrap();
    model.relations.get_mut(&relation).unwrap().pop();
    model
        .relations
        .get_mut(&relation)
        .unwrap()
        .push(vec![Value::I64(21)]);
    let (after, after_stats) = prepared.execute_native_pinned(store, registry).unwrap();
    assert_eq!(after, logical.evaluate(model, context, registry).unwrap());
    assert_eq!(after_stats.multiway_join_prepared_quotient_hits, 1);
    assert_eq!(
        after_stats.multiway_join_maintained_quotient_support_hits,
        1
    );
    assert_eq!(after_stats.multiway_join_maintained_quotient_key_hits, 0);
}

fn assert_quotient_artifact_memory_inventory(store: &PhysicalStore) {
    let memory = store.artifact_memory_report();
    for (family, expected) in [
        (PhysicalArtifactFamily::SemanticStatistics, 3),
        (PhysicalArtifactFamily::SemanticQuotientFactor, 3),
        (PhysicalArtifactFamily::SemanticQuotientSupport, 1),
    ] {
        assert_eq!(
            memory.families.get(&family).map(|entry| entry.artifacts),
            Some(expected)
        );
    }
    assert!(memory.total_estimated_retained_bytes > 0);
}

fn quotient_factor_advisor_fixture(
    with_statistics: bool,
) -> (
    SemanticContext,
    SemanticRegistry,
    PreparedPlan,
    PhysicalStore,
    [SemanticIndexBinding; 3],
) {
    let (context, registry, [a, b, c], equivalence) = three_relation_i64_context();
    let bindings = [1_280_u128, 1_281, 1_282].map(|id| LayoutBinding {
        id: LayoutId(id),
        family: LayoutFamily::Columnar,
    });
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in [a, b, c].into_iter().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(a)),
            right: Box::new(RelExpr::Scan(b)),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        right: Box::new(RelExpr::Scan(c)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical, &context, &registry, &catalog).unwrap();
    let values = [
        (1_i64..=20).collect::<Vec<_>>(),
        vec![1_i64; 100],
        (1_i64..=20).collect::<Vec<_>>(),
    ];
    let mut store = PhysicalStore::default();
    for ((relation, layout), relation_values) in [a, b, c].into_iter().zip(bindings).zip(values) {
        store
            .install(
                relation,
                layout,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(relation_values.into())])
                    .unwrap(),
            )
            .unwrap();
        if with_statistics {
            store
                .install_semantic_statistics(
                    SemanticIndexBinding::single(relation, layout, 0, equivalence),
                    &context,
                    &registry,
                )
                .unwrap();
        }
    }
    let factor_bindings = [a, b, c].map(|relation| {
        let index = [a, b, c]
            .iter()
            .position(|candidate| *candidate == relation)
            .unwrap();
        SemanticIndexBinding::single(relation, bindings[index], 0, equivalence)
    });
    (context, registry, prepared, store, factor_bindings)
}

#[test]
fn quotient_factor_advisor_amortizes_build_and_evicts_only_owned_factors() {
    let (context, registry, prepared, mut store, factor_bindings) =
        quotient_factor_advisor_fixture(true);
    let workload = [SemanticQuotientFactorWorkloadSample {
        plan: &prepared,
        expected_executions: 2,
    }];
    let (_, before_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(before_stats.multiway_join_apnf_executions, 1);
    assert_eq!(before_stats.multiway_join_prepared_quotient_hits, 0);
    assert_eq!(before_stats.multiway_join_maintained_quotient_key_hits, 0);
    let report = store
        .advise_semantic_quotient_factors(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.created.len(), 3);
    assert!(report.rejected_unprofitable.is_empty());
    for binding in &factor_bindings {
        assert!(store.semantic_quotient_factor(binding).is_some());
    }
    let (_, after_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(after_stats.multiway_join_prepared_quotient_hits, 1);
    assert_eq!(after_stats.multiway_join_maintained_quotient_key_hits, 140);
    assert_eq!(
        store
            .artifact_memory_report()
            .families
            .get(&PhysicalArtifactFamily::SemanticQuotientFactor)
            .map(|memory| memory.advisor_managed_artifacts),
        Some(3)
    );
    assert_eq!(
        estimated_quotient_build_work_for_test(
            prepared.physical(),
            prepared.semantic_quotient_program.as_ref(),
            &store,
            &context,
            &registry,
        )
        .unwrap(),
        0
    );
    let epoch = store.transition_epoch();
    let retained = store
        .advise_semantic_quotient_factors(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(retained.retained.len(), 3);
    assert_eq!(store.transition_epoch(), epoch);

    let report = store
        .advise_semantic_quotient_factors(
            &[],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.evicted.len(), 3);
    assert!(
        factor_bindings
            .iter()
            .all(|binding| store.semantic_quotient_factor(binding).is_none())
    );

    assert_eq!(
        prepared
            .materialize_semantic_quotient_factors(&mut store, &registry)
            .unwrap(),
        3
    );
    let report = store
        .advise_semantic_quotient_factors(
            &[],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert!(report.evicted.is_empty());
    assert!(
        factor_bindings
            .iter()
            .all(|binding| store.semantic_quotient_factor(binding).is_some())
    );
}

#[test]
fn quotient_factor_advisor_rejects_one_shot_and_respects_global_memory_budget() {
    let (context, registry, prepared, mut store, _) = quotient_factor_advisor_fixture(true);
    let one_shot = [SemanticQuotientFactorWorkloadSample {
        plan: &prepared,
        expected_executions: 1,
    }];
    let report = store
        .advise_semantic_quotient_factors(
            &one_shot,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.rejected_unprofitable.len(), 3);
    assert!(report.created.is_empty());

    let repeated = [SemanticQuotientFactorWorkloadSample {
        plan: &prepared,
        expected_executions: 2,
    }];
    let fixed = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    let report = store
        .advise_semantic_quotient_factors(
            &repeated,
            PhysicalArtifactAdvisorPolicy {
                max_managed_estimated_bytes: usize::MAX,
                max_total_estimated_bytes: fixed.saturating_add(1),
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.rejected_budget.len(), 3);
    assert!(report.created.is_empty());
    assert_eq!(report.fixed_estimated_bytes, fixed);
}

#[test]
fn manual_quotient_factor_materialization_pins_advisor_owned_factors() {
    let (context, registry, prepared, mut store, factor_bindings) =
        quotient_factor_advisor_fixture(true);
    let workload = [SemanticQuotientFactorWorkloadSample {
        plan: &prepared,
        expected_executions: 2,
    }];
    let created = store
        .advise_semantic_quotient_factors(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(created.created.len(), 3);
    let before_pin_epoch = store.transition_epoch();
    assert_eq!(
        prepared
            .materialize_semantic_quotient_factors(&mut store, &registry)
            .unwrap(),
        0
    );
    assert_eq!(store.transition_epoch(), before_pin_epoch + 1);
    assert_eq!(
        store
            .artifact_memory_report()
            .families
            .get(&PhysicalArtifactFamily::SemanticQuotientFactor)
            .map(|memory| memory.advisor_managed_artifacts),
        Some(0)
    );
    let empty = store
        .advise_semantic_quotient_factors(
            &[],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert!(empty.evicted.is_empty());
    assert!(
        factor_bindings
            .iter()
            .all(|binding| store.semantic_quotient_factor(binding).is_some())
    );
}

#[test]
fn quotient_factor_advisor_does_not_materialize_unused_qcn_path() {
    let (context, registry, prepared, mut store, factor_bindings) =
        quotient_factor_advisor_fixture(false);
    let workload = [SemanticQuotientFactorWorkloadSample {
        plan: &prepared,
        expected_executions: 100,
    }];
    let report = store
        .advise_semantic_quotient_factors(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert!(report.created.is_empty());
    assert!(report.retained.is_empty());
    assert!(
        factor_bindings
            .iter()
            .all(|binding| store.semantic_quotient_factor(binding).is_none())
    );
}

#[test]
fn quotient_factor_advisor_rejects_over_budget_non_gyo_cyclic_path() {
    let fixture = ten_way_non_gyo_text_cycle_fixture();
    let (mut store, _) = install_cyclic_text_rows(&fixture, 4);
    let factor_bindings = fixture
        .prepared
        .semantic_quotient_factor_bindings(&store)
        .unwrap();
    assert!(!factor_bindings.is_empty());
    let workload = [SemanticQuotientFactorWorkloadSample {
        plan: &fixture.prepared,
        expected_executions: 100,
    }];
    let report = store
        .advise_semantic_quotient_factors(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &fixture.context,
            &fixture.registry,
        )
        .unwrap();
    assert!(report.created.is_empty());
    assert!(report.retained.is_empty());
    assert!(
        factor_bindings
            .iter()
            .all(|binding| store.semantic_quotient_factor(binding).is_none())
    );
}

#[test]
fn prepared_gamma_quotient_factors_reuse_delta_maintained_canonical_keys() {
    let (context, registry, [a, b, c], equivalence) = three_relation_i64_context();
    let bindings = [1_090_u128, 1_091, 1_092].map(|id| LayoutBinding {
        id: LayoutId(id),
        family: LayoutFamily::Columnar,
    });
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in [a, b, c].into_iter().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(a)),
            right: Box::new(RelExpr::Scan(b)),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        right: Box::new(RelExpr::Scan(c)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let values = [
        (1_i64..=20).collect::<Vec<_>>(),
        vec![1_i64; 100],
        (1_i64..=20).collect::<Vec<_>>(),
    ];
    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for ((relation, layout), relation_values) in [a, b, c].into_iter().zip(bindings).zip(values) {
        store
            .install(
                relation,
                layout,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(
                    relation_values.clone().into(),
                )])
                .unwrap(),
            )
            .unwrap();
        store
            .install_semantic_statistics(
                SemanticIndexBinding::single(relation, layout, 0, equivalence),
                &context,
                &registry,
            )
            .unwrap();
        model.relations.insert(
            relation,
            relation_values
                .into_iter()
                .map(|value| vec![Value::I64(value)])
                .collect(),
        );
    }
    assert_eq!(
        prepared
            .materialize_semantic_quotient_factors(&mut store, &registry)
            .unwrap(),
        3
    );
    assert_eq!(
        prepared
            .materialize_semantic_quotient_factors(&mut store, &registry)
            .unwrap(),
        0
    );
    let factor_binding = SemanticIndexBinding::single(a, bindings[0], 0, equivalence);
    assert!(store.semantic_index(&factor_binding).is_none());
    assert!(store.semantic_quotient_factor(&factor_binding).is_some());

    assert_quotient_support_cache_survives_delta(
        &prepared,
        &mut store,
        &registry,
        &logical,
        &mut model,
        &context,
        (c, bindings[2]),
    );
    assert_quotient_artifact_memory_inventory(&store);
}

fn assert_structural_quotient_factor_hits(
    prepared: &PreparedPlan,
    store: &PhysicalStore,
    registry: &SemanticRegistry,
    expected_rows: usize,
) {
    let mut stats = ExecutionStats::default();
    let rows = execute_order_preserving_quotient_join_for_test(
        prepared.physical(),
        prepared.semantic_quotient_program.as_ref(),
        store,
        prepared.semantic_context(),
        registry,
        &mut stats,
    )
    .unwrap()
    .unwrap();
    assert_eq!(rows.len(), expected_rows);
    assert_eq!(stats.multiway_join_maintained_quotient_key_hits, 6);
}

#[test]
fn structural_gamma_quotient_factors_are_materialized_and_delta_maintained() {
    let relations = [sid(1_200), sid(1_201), sid(1_202)];
    let structural = sid(1_203);
    let ci = sid(1_204);
    let mut registry = SemanticRegistry::default();
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1_200));
    environment.pin_module(ci, ci_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1_200));
    schema
        .define_structural_equivalence(structural, StructuralEquivalenceDef::Option { inner: ci })
        .unwrap();
    for relation in relations {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Option(Box::new(TypeExpr::Scalar(
                    ScalarType::Text,
                )))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![structural],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let bindings = [1_200_u128, 1_201, 1_202].map(|id| LayoutBinding {
        id: LayoutId(id),
        family: LayoutFamily::RowStore,
    });
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.into_iter().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let [a, b, c] = relations;
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(a)),
            right: Box::new(RelExpr::Scan(b)),
            left_column: 0,
            right_column: 0,
            equivalence: structural,
        }),
        right: Box::new(RelExpr::Scan(c)),
        left_column: 0,
        right_column: 0,
        equivalence: structural,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let option_text = |value: &str| Value::Option(Some(Box::new(Value::Text(value.into()))));
    let relation_rows = [
        vec![vec![option_text("Alpha")], vec![option_text("Beta")]],
        vec![vec![option_text("alpha")], vec![option_text("beta")]],
        vec![vec![option_text("ALPHA")], vec![option_text("BETA")]],
    ];
    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for ((relation, binding), rows) in relations.into_iter().zip(bindings).zip(relation_rows) {
        store
            .install(relation, binding, NativeRelation::row_store(rows.clone()))
            .unwrap();
        model.relations.insert(relation, rows);
    }
    assert_eq!(
        prepared
            .materialize_semantic_quotient_factors(&mut store, &registry)
            .unwrap(),
        3
    );
    let (before, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        before,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(before.rows().len(), 2);
    assert_structural_quotient_factor_hits(&prepared, &store, &registry, 2);

    let delta = RelationDelta {
        inserted: vec![vec![option_text("Gamma")]],
        removed: vec![vec![option_text("ALPHA")]],
        result_type: RelExpr::Scan(c).typecheck(&context, &registry).unwrap(),
    };
    store
        .apply_relation_delta(c, bindings[2], &delta, &context, &registry)
        .unwrap();
    let c_rows = model.relations.get_mut(&c).unwrap();
    c_rows.remove(0);
    c_rows.push(vec![option_text("Gamma")]);
    let (after, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        after,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(after.rows().len(), 1);
    assert_structural_quotient_factor_hits(&prepared, &store, &registry, 1);
}

#[test]
fn structural_observable_atom_drives_persisted_join_without_primitive_gate() {
    let left_relation = sid(1_210);
    let right_relation = sid(1_211);
    let structural = sid(1_212);
    let ci = sid(1_213);
    let mut registry = SemanticRegistry::default();
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1_210));
    environment.pin_module(ci, ci_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1_210));
    schema
        .define_structural_equivalence(structural, StructuralEquivalenceDef::Option { inner: ci })
        .unwrap();
    for relation in [left_relation, right_relation] {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Option(Box::new(TypeExpr::Scalar(
                    ScalarType::Text,
                )))],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![structural],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let left_layout = LayoutBinding {
        id: LayoutId(1_210),
        family: LayoutFamily::RowStore,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(1_211),
        family: LayoutFamily::RowStore,
    };
    let option_text = |value: &str| Value::Option(Some(Box::new(Value::Text(value.into()))));
    let left_rows = vec![vec![option_text("ALPHA")], vec![option_text("missing")]];
    let mut right_rows = vec![vec![option_text("Alpha")], vec![option_text("alpha")]];
    for index in 0..126 {
        right_rows.push(vec![option_text(&format!("other-{index}"))]);
    }
    let mut store = PhysicalStore::default();
    store
        .install(
            left_relation,
            left_layout,
            NativeRelation::row_store(left_rows.clone()),
        )
        .unwrap();
    store
        .install(
            right_relation,
            right_layout,
            NativeRelation::row_store(right_rows.clone()),
        )
        .unwrap();
    let binding = SemanticIndexBinding::single(right_relation, right_layout, 0, structural);
    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    assert!(store.semantic_indexes_for_test().is_empty());

    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(left_relation)),
        right: Box::new(RelExpr::Scan(right_relation)),
        left_column: 0,
        right_column: 0,
        equivalence: structural,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left_relation, left_layout);
    catalog.bind_relation(right_relation, right_layout);
    let prepared = prepare_with_catalog(query.clone(), &context, &registry, &catalog).unwrap();
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.scanned_rows, left_rows.len());
    assert_eq!(value.rows().len(), 2);

    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(left_relation, left_rows);
    model.relations.insert(right_relation, right_rows);
    assert_eq!(value, query.evaluate(&model, &context, &registry).unwrap());
}

#[test]
fn persisted_structural_semantic_index_is_consumed_and_delta_maintained() {
    let relation = sid(1_220);
    let structural = sid(1_221);
    let ci = sid(1_222);
    let unrelated = sid(1_223);
    let mut registry = SemanticRegistry::default();
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let unrelated_digest = registry.install_equivalence_revision(EquivalenceModule::I64Exact, 1);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1_220));
    environment.pin_module(ci, ci_digest);
    environment.pin_module(unrelated, unrelated_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1_220));
    schema
        .define_structural_equivalence(structural, StructuralEquivalenceDef::Option { inner: ci })
        .unwrap();
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Option(Box::new(TypeExpr::Scalar(
                ScalarType::Text,
            )))],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![structural],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let layout = LayoutBinding {
        id: LayoutId(1_220),
        family: LayoutFamily::RowStore,
    };
    let option_text = |value: &str| Value::Option(Some(Box::new(Value::Text(value.into()))));
    let mut rows = vec![vec![option_text("Alpha")], vec![option_text("alpha")]];
    for index in 0..64 {
        rows.push(vec![option_text(&format!("other-{index}"))]);
    }
    let mut store = PhysicalStore::default();
    store
        .install(relation, layout, NativeRelation::row_store(rows))
        .unwrap();
    let binding = SemanticIndexBinding::single(relation, layout, 0, structural);
    store
        .install_semantic_index(binding.clone(), &context, &registry)
        .unwrap();
    let index = store.semantic_index(&binding).unwrap();

    let mut unrelated_gamma_change = context.clone();
    let unrelated_v2 = registry.install_equivalence_revision(EquivalenceModule::I64Exact, 2);
    unrelated_gamma_change
        .environment
        .pin_module(unrelated, unrelated_v2);
    assert!(
        index
            .compatible_with(&unrelated_gamma_change, &registry)
            .unwrap(),
        "unrelated Γ module drift must not invalidate structural key cache"
    );

    let mut leaf_gamma_change = context.clone();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    leaf_gamma_change.environment.pin_module(ci, exact_digest);
    assert!(
        !index
            .compatible_with(&leaf_gamma_change, &registry)
            .unwrap(),
        "canonical leaf digest drift must force structural key rebuild"
    );

    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: option_text("ALPHA"),
        equivalence: structural,
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let (before, before_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(before.rows().len(), 2);
    assert_eq!(before_stats.persisted_index_hits, 1);

    let delta = RelationDelta {
        inserted: vec![vec![option_text("ALpHa")]],
        removed: vec![vec![option_text("alpha")]],
        result_type: RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap(),
    };
    store
        .apply_relation_delta(relation, layout, &delta, &context, &registry)
        .unwrap();
    let index = store.semantic_index(&binding).unwrap();
    assert_eq!(index.row_count(), 66);
    let (after, after_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(after.rows().len(), 2);
    assert_eq!(after_stats.persisted_index_hits, 1);
}

#[test]
fn multiway_join_planner_reassociates_contiguous_tree_and_reuses_indexes_after_intermediate() {
    let (context, registry, [a, b, c], equivalence) = three_relation_i64_context();
    let bindings = [
        LayoutBinding {
            id: LayoutId(1000),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1001),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1002),
            family: LayoutFamily::Columnar,
        },
    ];
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in [a, b, c].into_iter().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(a)),
        right: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(b)),
            right: Box::new(RelExpr::Scan(c)),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let a_values = vec![1_i64, 2];
    let b_values = (1_i64..=1_000).collect::<Vec<_>>();
    let c_values = (1_i64..=1_000).collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    for (relation, binding, values) in [
        (a, bindings[0], a_values.clone()),
        (b, bindings[1], b_values.clone()),
        (c, bindings[2], c_values.clone()),
    ] {
        store
            .install(
                relation,
                binding,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(values.into())]).unwrap(),
            )
            .unwrap();
    }
    for (relation, binding) in [(b, bindings[1]), (c, bindings[2])] {
        store
            .install_semantic_index(
                SemanticIndexBinding::single(relation, binding, 0, equivalence),
                &context,
                &registry,
            )
            .unwrap();
    }

    let (native, stats) = prepared
        .physical()
        .execute_native(&store, prepared.result_type(), &context, &registry)
        .unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        a,
        a_values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    model.relations.insert(
        b,
        b_values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    model.relations.insert(
        c,
        c_values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.multiway_join_reorders, 1);
    assert_eq!(stats.multiway_join_order_preserving_enumerations, 0);
    assert_eq!(stats.persisted_index_hits, 2);
    assert_eq!(native.rows().len(), 2);
}

#[test]
#[ignore = "diagnostic release benchmark"]
fn pass44_multiway_join_reassociation_benchmark() {
    let (context, registry, [a, b, c], equivalence) = three_relation_i64_context();
    let bindings = [
        LayoutBinding {
            id: LayoutId(1010),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1011),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1012),
            family: LayoutFamily::Columnar,
        },
    ];
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in [a, b, c].into_iter().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(a)),
        right: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(b)),
            right: Box::new(RelExpr::Scan(c)),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical, &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    for (relation, binding, values) in [
        (a, bindings[0], (1_i64..=50).collect::<Vec<_>>()),
        (b, bindings[1], (1_i64..=10_000).collect::<Vec<_>>()),
        (c, bindings[2], (1_i64..=10_000).collect::<Vec<_>>()),
    ] {
        store
            .install(
                relation,
                binding,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(values.into())]).unwrap(),
            )
            .unwrap();
    }
    for (relation, binding) in [(b, bindings[1]), (c, bindings[2])] {
        store
            .install_semantic_index(
                SemanticIndexBinding::single(relation, binding, 0, equivalence),
                &context,
                &registry,
            )
            .unwrap();
    }
    let optimized =
        optimize_contiguous_multiway_join_for_test(prepared.physical(), &store, &context, &registry)
            .unwrap()
            .unwrap();
    let mut baseline_ns = Vec::new();
    let mut optimized_ns = Vec::new();
    for _ in 0..7 {
        let start = std::time::Instant::now();
        let baseline = execute_multiway_join_candidate_for_test(
            prepared.physical(),
            &store,
            &context,
            &registry,
            &mut ExecutionStats::default(),
        )
        .unwrap();
        baseline_ns.push(start.elapsed().as_nanos());

        let start = std::time::Instant::now();
        let optimized_rows = execute_multiway_join_candidate_for_test(
            &optimized,
            &store,
            &context,
            &registry,
            &mut ExecutionStats::default(),
        )
        .unwrap();
        optimized_ns.push(start.elapsed().as_nanos());
        assert_eq!(baseline, optimized_rows);
    }
    baseline_ns.sort_unstable();
    optimized_ns.sort_unstable();
    let baseline_median = baseline_ns[baseline_ns.len() / 2];
    let optimized_median = optimized_ns[optimized_ns.len() / 2];
    let ratio_milli = baseline_median
        .saturating_mul(1_000)
        .checked_div(optimized_median.max(1))
        .unwrap_or(u128::MAX);
    println!(
        "PASS44_MULTIWAY_BENCH baseline_median_ns={baseline_median} optimized_median_ns={optimized_median} ratio_milli={ratio_milli}"
    );
}

#[test]
fn join_access_decision_unifies_persisted_and_ephemeral_i64_families() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(1_044),
        family: LayoutFamily::Columnar,
    };
    let equivalence = sid(101);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64((0_i64..100).collect())])
                .unwrap(),
        )
        .unwrap();
    let i64_binding = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence,
    };
    let semantic_binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store
        .install_i64_index(i64_binding, &context, &registry)
        .unwrap();
    store
        .install_semantic_index(semantic_binding.clone(), &context, &registry)
        .unwrap();

    let both = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 100,
            right_relation: relation,
            right_layout: layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(both.family, JoinAccessKind::PersistedI64);

    store.remove_i64_index(i64_binding);
    let semantic_only = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 100,
            right_relation: relation,
            right_layout: layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(semantic_only.family, JoinAccessKind::PersistedSemantic);

    store
        .install_observable_atom_state(semantic_binding.clone(), &context, &registry)
        .unwrap();
    store.remove_semantic_index(&semantic_binding);
    let samf_only = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 100,
            right_relation: relation,
            right_layout: layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(samf_only.family, JoinAccessKind::PersistedSemantic);

    store.remove_observable_atom_state(&semantic_binding);
    let transient = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 100,
            right_relation: relation,
            right_layout: layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(transient.family, JoinAccessKind::EphemeralI64);
}

#[test]
fn multiway_and_direct_join_planning_share_the_same_right_access_decision() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(1_045),
        family: LayoutFamily::Columnar,
    };
    let equivalence = sid(101);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64((0_i64..128).collect())])
                .unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(
            I64IndexBinding {
                relation,
                layout,
                key_column: 0,
                equivalence,
            },
            &context,
            &registry,
        )
        .unwrap();
    let direct = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 64,
            right_relation: relation,
            right_layout: layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    let multiway = multiway_right_access_estimate_for_test(
        &store,
        relation,
        layout,
        128,
        0,
        equivalence,
        64,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(multiway, direct);
}

#[test]
fn multiway_costing_does_not_credit_unbuilt_transient_distinctness() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(1_052),
        family: LayoutFamily::Columnar,
    };
    let equivalence = sid(101);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64((0_i64..128).collect())])
                .unwrap(),
        )
        .unwrap();
    let direct = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 64,
            right_relation: relation,
            right_layout: layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(direct.family, JoinAccessKind::EphemeralI64);

    let multiway = multiway_right_access_estimate_for_test(
        &store,
        relation,
        layout,
        128,
        0,
        equivalence,
        64,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(multiway.family, JoinAccessKind::FullScan);
}

#[test]
fn semantic_statistics_are_maintained_with_relation_delta() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(1_053),
        family: LayoutFamily::Columnar,
    };
    let equivalence = sid(101);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 1, 2].into())]).unwrap(),
        )
        .unwrap();
    let initial = store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    assert_eq!(
        initial,
        SemanticKeyStatistics {
            row_count: 3,
            distinct_key_count: 2,
        }
    );

    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    store
        .apply_relation_delta(relation, layout, &delta, &context, &registry)
        .unwrap();
    assert_eq!(
        store
            .semantic_statistics(&binding, &context, &registry)
            .unwrap(),
        Some(SemanticKeyStatistics {
            row_count: 3,
            distinct_key_count: 3,
        })
    );
    let second = scan_delta(relation, &[2], &[1], &context, &registry);
    store
        .apply_relation_delta(relation, layout, &second, &context, &registry)
        .unwrap();
    assert_eq!(
        store
            .semantic_statistics(&binding, &context, &registry)
            .unwrap(),
        Some(SemanticKeyStatistics {
            row_count: 3,
            distinct_key_count: 2,
        })
    );
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![9, 10].into())]).unwrap(),
        )
        .unwrap();
    assert_eq!(
        store
            .semantic_statistics(&binding, &context, &registry)
            .unwrap(),
        None
    );
}

#[test]
fn inconsistent_statistics_are_ignored_as_non_authoritative_physical_state() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(1_058),
        family: LayoutFamily::Columnar,
    };
    let equivalence = sid(101);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2, 3].into())]).unwrap(),
        )
        .unwrap();
    store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    store.set_semantic_statistics_row_count(&binding, 99);
    assert_eq!(
        store
            .semantic_statistics(&binding, &context, &registry)
            .unwrap(),
        None
    );
    let decision = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 3,
            right_relation: relation,
            right_layout: layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(decision.family, JoinAccessKind::EphemeralI64);
}

#[test]
fn retained_statistics_refine_cardinality_without_changing_i64_access_family() {
    let (context, registry, [left_relation, right_relation, _], equivalence) =
        three_relation_i64_context();
    let left_layout = LayoutBinding {
        id: LayoutId(1_055),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(1_056),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left_relation, left_layout);
    catalog.bind_relation(right_relation, right_layout);
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(left_relation)),
        right: Box::new(RelExpr::Scan(right_relation)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical, &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            left_relation,
            left_layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64((1_i64..=64).collect())])
                .unwrap(),
        )
        .unwrap();
    store
        .install(
            right_relation,
            right_layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1; 128].into())]).unwrap(),
        )
        .unwrap();
    let binding = SemanticIndexBinding::single(right_relation, right_layout, 0, equivalence);
    store
        .install_semantic_statistics(binding, &context, &registry)
        .unwrap();

    let decision = observe_right_join_access_for_test(
        JoinAccessProbe {
            left_rows: 64,
            right_relation,
            right_layout,
            right_column: 0,
            equivalence,
            allow_ephemeral: true,
        },
        &store,
        &context,
        &registry,
    )
    .unwrap();
    assert_eq!(decision.family, JoinAccessKind::EphemeralI64);
    let (value, execution) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(value.rows().len(), 128);
    assert_eq!(execution.ephemeral_index_builds, 1);
}

