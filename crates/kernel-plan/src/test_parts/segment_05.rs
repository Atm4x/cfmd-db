#[test]
fn retained_statistics_enable_safe_multiway_transient_costing() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(1_054),
        family: LayoutFamily::Columnar,
    };
    let equivalence = sid(101);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
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
        .install_semantic_statistics(binding, &context, &registry)
        .unwrap();
    let decision = multiway_right_access_estimate_for_test(
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
    assert_eq!(decision.family, JoinAccessKind::EphemeralI64);
    assert_eq!(decision.estimated_output_rows, 64);
}

#[test]
fn composite_semantic_statistics_use_the_same_canonical_key_contract_as_indexes() {
    let relation = sid(370);
    let text_equivalence = sid(371);
    let i64_equivalence = sid(372);
    let (context, registry) =
        text_i64_semantic_fixture(&[relation], text_equivalence, i64_equivalence, 46);
    let layout = LayoutBinding {
        id: LayoutId(1_057),
        family: LayoutFamily::Columnar,
    };
    let binding = SemanticIndexBinding {
        relation,
        layout,
        key_parts: vec![
            SemanticIndexKeyPart {
                column: 0,
                equivalence: text_equivalence,
            },
            SemanticIndexKeyPart {
                column: 1,
                equivalence: i64_equivalence,
            },
        ],
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            text_i64_native(&[("A", 1), ("a", 1), ("A", 2), ("B", 1)]),
        )
        .unwrap();
    let statistics = store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    assert_eq!(statistics.row_count, 4);
    assert_eq!(statistics.distinct_key_count, 3);

    store
        .install_semantic_index(binding.clone(), &context, &registry)
        .unwrap();
    assert_eq!(
        store.semantic_index(&binding).unwrap().distinct_key_count(),
        statistics.distinct_key_count
    );
}

#[test]
fn join_access_decision_prefers_canonical_bucket_over_one_shot_semantic_state() {
    let (context, registry, relation, equivalence, layout, mut store) =
        text_semantic_index_fixture();
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store.remove_semantic_index(&binding);
    let decision = observe_right_join_access_for_test(
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
    assert_eq!(decision.family, JoinAccessKind::FullScan);
}

#[test]
fn multiway_join_executes_transient_access_after_intermediate_join() {
    let (context, registry, relations, equivalence) = three_relation_i64_context();
    let [a, b, c] = relations;
    let bindings = [
        LayoutBinding {
            id: LayoutId(1_046),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1_047),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1_048),
            family: LayoutFamily::Columnar,
        },
    ];
    let mut catalog = PhysicalCatalog::default();
    for (relation, layout) in relations.into_iter().zip(bindings) {
        catalog.bind_relation(relation, layout);
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
    let values = [
        (1_i64..=8).collect::<Vec<_>>(),
        (1_i64..=512).collect::<Vec<_>>(),
        (1_i64..=512).collect::<Vec<_>>(),
    ];
    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for ((relation, layout), relation_values) in relations.into_iter().zip(bindings).zip(values) {
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
        model.relations.insert(
            relation,
            relation_values
                .into_iter()
                .map(|value| vec![Value::I64(value)])
                .collect(),
        );
    }

    let (native, stats) = prepared
        .physical()
        .execute_native(&store, prepared.result_type(), &context, &registry)
        .unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.multiway_join_reorders, 1);
    assert_eq!(stats.multiway_join_order_preserving_enumerations, 0);
    assert_eq!(stats.ephemeral_index_builds, 2);
    assert_eq!(stats.persisted_index_hits, 0);
}

#[test]
fn intermediate_transient_join_keeps_prebuild_admission_across_both_joins() {
    let (context, registry, relations, equivalence) = three_relation_i64_context();
    let [a, b, c] = relations;
    let bindings = [
        LayoutBinding {
            id: LayoutId(1_049),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1_050),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1_051),
            family: LayoutFamily::Columnar,
        },
    ];
    let mut store = PhysicalStore::default();
    for (relation, layout, values) in [
        (a, bindings[0], vec![1_i64]),
        (b, bindings[1], vec![1_i64; 50]),
        (c, bindings[2], vec![1_i64; 50]),
    ] {
        store
            .install(
                relation,
                layout,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(values.into())]).unwrap(),
            )
            .unwrap();
    }
    let scan = |relation, layout| Plan::Scan { relation, layout };
    let plan = Plan::JoinEq {
        left: Box::new(Plan::JoinEq {
            left: Box::new(scan(a, bindings[0])),
            right: Box::new(scan(b, bindings[1])),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        right: Box::new(scan(c, bindings[2])),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let mut stats = ExecutionStats::default();
    let rows = plan
        .execute_native_rows(&store, &context, &registry, &mut stats)
        .unwrap();
    assert_eq!(rows.len(), 2_500);
    assert_eq!(stats.ephemeral_index_builds, 2);
    assert_eq!(stats.persisted_index_hits, 0);
}

fn multiway_filter_store_and_model(
    context: &SemanticContext,
    registry: &SemanticRegistry,
    relations: [SemanticId; 3],
    bindings: [LayoutBinding; 3],
    equivalence: SemanticId,
) -> (PhysicalStore, kernel_model::FiniteModel) {
    let [a, b, c] = relations;
    let rows = [
        vec![(1_i64, 10_i64), (1, 10), (2, 20)],
        (1_i64..=1_000).map(|key| (key, key)).collect(),
        (1_i64..=1_000)
            .map(|key| {
                (
                    key,
                    if key == 1 {
                        10
                    } else if key == 2 {
                        20
                    } else {
                        0
                    },
                )
            })
            .collect(),
    ];
    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for ((relation, binding), relation_rows) in [a, b, c].into_iter().zip(bindings).zip(rows) {
        store
            .install(
                relation,
                binding,
                NativeRelation::typed_columnar(vec![
                    NativeColumn::I64(relation_rows.iter().map(|row| row.0).collect()),
                    NativeColumn::I64(relation_rows.iter().map(|row| row.1).collect()),
                ])
                .unwrap(),
            )
            .unwrap();
        model.relations.insert(
            relation,
            relation_rows
                .into_iter()
                .map(|(left, right)| vec![Value::I64(left), Value::I64(right)])
                .collect(),
        );
    }
    for (relation, binding) in [(b, bindings[1]), (c, bindings[2])] {
        store
            .install_semantic_index(
                SemanticIndexBinding::single(relation, binding, 0, equivalence),
                context,
                registry,
            )
            .unwrap();
    }
    (store, model)
}

fn multiway_filter_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    RelExpr,
    PreparedPlan,
    PhysicalStore,
    kernel_model::FiniteModel,
) {
    let relations = [sid(710), sid(711), sid(712)];
    let [a, b, c] = relations;
    let equivalence = sid(713);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(710));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(710));
    for relation in relations {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence, equivalence],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
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
    let logical = RelExpr::FilterEqColumns {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(a)),
            right: Box::new(RelExpr::JoinEq {
                left: Box::new(RelExpr::Scan(b)),
                right: Box::new(RelExpr::Scan(c)),
                left_column: 1,
                right_column: 0,
                equivalence,
            }),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        left_column: 1,
        right_column: 5,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let (store, model) =
        multiway_filter_store_and_model(&context, &registry, relations, bindings, equivalence);
    (context, registry, logical, prepared, store, model)
}

#[test]
fn multiway_join_planner_preserves_cross_relation_filter_predicates_and_bag_order() {
    let (context, registry, logical, prepared, store, model) = multiway_filter_fixture();
    let (native, stats) = prepared
        .physical()
        .execute_native(&store, prepared.result_type(), &context, &registry)
        .unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 3);
    assert_eq!(stats.multiway_join_reorders, 1);
    assert_eq!(stats.persisted_index_hits, 2);
}

fn independent_apnf_coordinate_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    RelExpr,
    PreparedPlan,
    PhysicalStore,
    kernel_model::FiniteModel,
) {
    let relations = [sid(3_100), sid(3_101), sid(3_102)];
    let equivalence = sid(3_103);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(3_100));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(3_100));
    schema
        .define_relation(RelationDef {
            id: relations[0],
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::I64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence, equivalence],
            },
        })
        .unwrap();
    for relation in &relations[1..] {
        schema
            .define_relation(RelationDef {
                id: *relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let bindings = [
        LayoutBinding {
            id: LayoutId(3_100),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(3_101),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(3_102),
            family: LayoutFamily::Columnar,
        },
    ];
    let mut physical_catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings) {
        physical_catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(relations[0])),
            right: Box::new(RelExpr::Scan(relations[1])),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        right: Box::new(RelExpr::Scan(relations[2])),
        left_column: 1,
        right_column: 0,
        equivalence,
    };
    let prepared =
        prepare_with_catalog(logical.clone(), &context, &registry, &physical_catalog).unwrap();
    let (store, model) = independent_apnf_coordinate_data(relations, bindings);
    (context, registry, logical, prepared, store, model)
}

fn independent_apnf_coordinate_data(
    relations: [SemanticId; 3],
    bindings: [LayoutBinding; 3],
) -> (PhysicalStore, kernel_model::FiniteModel) {
    let mut store = PhysicalStore::default();
    store
        .install(
            relations[0],
            bindings[0],
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(vec![1, 1, 2].into()),
                NativeColumn::I64(vec![2, 1, 1].into()),
            ])
            .unwrap(),
        )
        .unwrap();
    store
        .install(
            relations[1],
            bindings[1],
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1].into())]).unwrap(),
        )
        .unwrap();
    store
        .install(
            relations[2],
            bindings[2],
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![2].into())]).unwrap(),
        )
        .unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relations[0],
        vec![
            vec![Value::I64(1), Value::I64(2)],
            vec![Value::I64(1), Value::I64(1)],
            vec![Value::I64(2), Value::I64(1)],
        ],
    );
    model
        .relations
        .insert(relations[1], vec![vec![Value::I64(1)]]);
    model
        .relations
        .insert(relations[2], vec![vec![Value::I64(2)]]);
    (store, model)
}

#[test]
fn apnf_keeps_independent_query_coordinates_distinct_under_same_equivalence_law() {
    let (context, registry, logical, prepared, store, model) =
        independent_apnf_coordinate_fixture();
    assert_eq!(
        prepared
            .anchor_pullback_program
            .as_ref()
            .map(|program| program.predicates.len()),
        Some(2)
    );
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 1);
    assert_eq!(stats.multiway_join_apnf_executions, 1);
    assert_eq!(stats.multiway_join_prepared_quotient_hits, 0);
}

#[test]
fn apnf_allows_one_column_to_participate_in_distinct_semantic_laws() {
    let relations = [sid(3_110), sid(3_111), sid(3_112)];
    let exact = sid(3_113);
    let folded = sid(3_114);
    let mut registry = SemanticRegistry::default();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let folded_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(3_110));
    environment.pin_module(exact, exact_digest);
    environment.pin_module(folded, folded_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(3_110));
    for relation in relations {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![exact],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let bindings = [
        LayoutBinding {
            id: LayoutId(3_110),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(3_111),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(3_112),
            family: LayoutFamily::Columnar,
        },
    ];
    let mut physical_catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings) {
        physical_catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(relations[0])),
            right: Box::new(RelExpr::Scan(relations[1])),
            left_column: 0,
            right_column: 0,
            equivalence: exact,
        }),
        right: Box::new(RelExpr::Scan(relations[2])),
        left_column: 0,
        right_column: 0,
        equivalence: folded,
    };
    let prepared =
        prepare_with_catalog(logical.clone(), &context, &registry, &physical_catalog).unwrap();
    let mut store = PhysicalStore::default();
    for (relation, binding, value) in [
        (relations[0], bindings[0], "A"),
        (relations[1], bindings[1], "A"),
        (relations[2], bindings[2], "a"),
    ] {
        store
            .install(
                relation,
                binding,
                NativeRelation::typed_columnar(vec![NativeColumn::Text(vec![value.into()].into())])
                    .unwrap(),
            )
            .unwrap();
    }
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(relations[0], vec![vec![Value::Text("A".into())]]);
    model
        .relations
        .insert(relations[1], vec![vec![Value::Text("A".into())]]);
    model
        .relations
        .insert(relations[2], vec![vec![Value::Text("a".into())]]);

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 1);
    assert_eq!(stats.multiway_join_apnf_executions, 1);
}

fn randomized_cyclic_apnf_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    RelExpr,
    PreparedPlan,
    [SemanticId; 3],
    [LayoutBinding; 3],
) {
    let relations = [sid(3_120), sid(3_121), sid(3_122)];
    let equivalence = sid(3_123);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(3_120));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(3_120));
    for relation in relations {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let bindings = [
        LayoutBinding {
            id: LayoutId(3_120),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(3_121),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(3_122),
            family: LayoutFamily::Columnar,
        },
    ];
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::FilterEqColumns {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::JoinEq {
                left: Box::new(RelExpr::Scan(relations[0])),
                right: Box::new(RelExpr::Scan(relations[1])),
                left_column: 0,
                right_column: 0,
                equivalence,
            }),
            right: Box::new(RelExpr::Scan(relations[2])),
            left_column: 1,
            right_column: 0,
            equivalence,
        }),
        left_column: 0,
        right_column: 2,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    (context, registry, logical, prepared, relations, bindings)
}

fn generate_small_i64_bags(rng_state: &mut u64) -> [Vec<i64>; 3] {
    let mut generated = [Vec::new(), Vec::new(), Vec::new()];
    for rows in &mut generated {
        *rng_state = rng_state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let len = usize::try_from((*rng_state >> 61) + 1).unwrap();
        for _ in 0..len {
            *rng_state = rng_state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            rows.push(i64::try_from((*rng_state >> 60) & 0x3).unwrap());
        }
    }
    generated
}

#[test]
fn apnf_randomized_cyclic_triangle_matches_reference_with_duplicates() {
    let (context, registry, logical, prepared, relations, bindings) =
        randomized_cyclic_apnf_fixture();
    assert_eq!(
        prepared
            .anchor_pullback_program
            .as_ref()
            .map(|program| program.predicates.len()),
        Some(3)
    );
    let mut rng_state = 0x9E37_79B9_7F4A_7C15_u64;
    for case in 0..512 {
        let generated = generate_small_i64_bags(&mut rng_state);
        let mut store = PhysicalStore::default();
        let mut model = kernel_model::FiniteModel::default();
        for ((relation, binding), values) in relations
            .iter()
            .copied()
            .zip(bindings)
            .zip(generated.iter())
        {
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
                    .iter()
                    .copied()
                    .map(|value| vec![Value::I64(value)])
                    .collect(),
            );
        }
        let (native, execution_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
        let reference = logical.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(native, reference, "APNF cyclic differential case {case}");
        assert_eq!(execution_stats.multiway_join_apnf_executions, 1);
    }
}

#[test]
fn cost_model_uses_selective_persisted_text_semantic_index() {
    let relation = sid(344);
    let equivalence = sid(345);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(36));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(36));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let binding = LayoutBinding {
        id: LayoutId(938),
        family: LayoutFamily::Columnar,
    };
    let values = (0..64)
        .map(|index| {
            if index == 37 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(values.into())]).unwrap(),
        )
        .unwrap();
    store
        .install_semantic_index(
            SemanticIndexBinding::single(relation, binding, 0, equivalence),
            &context,
            &registry,
        )
        .unwrap();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let filter = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::Text("needle".into()),
        equivalence,
    };
    let prepared = prepare_with_catalog(filter, &context, &registry, &catalog).unwrap();
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(value.rows(), &[vec![Value::Text("Needle".into())]]);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.persisted_index_cost_rejections, 0);
    assert_eq!(stats.scanned_rows, 1);
}

fn advisor_text_fixture(
    relation: SemanticId,
    equivalence: SemanticId,
    revision: u64,
    layout_id: u64,
    values: Vec<String>,
) -> (
    SemanticContext,
    SemanticRegistry,
    LayoutBinding,
    PhysicalStore,
    PreparedPlan,
) {
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(revision));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let layout = LayoutBinding {
        id: LayoutId(layout_id.into()),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(values.into())]).unwrap(),
        )
        .unwrap();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::Text("needle".into()),
        equivalence,
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    (context, registry, layout, store, prepared)
}

#[test]
fn semantic_index_advisor_amortizes_build_and_reuses_one_shared_index() {
    let relation = sid(353);
    let equivalence = sid(354);
    let values = (0..64)
        .map(|index| {
            if index == 37 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let (context, registry, layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 39, 942, values);
    let workload = [
        SemanticIndexWorkloadSample {
            plan: prepared.physical().clone(),
            expected_executions: 1,
        },
        SemanticIndexWorkloadSample {
            plan: prepared.physical().clone(),
            expected_executions: 1,
        },
    ];

    let report = store
        .advise_semantic_indexes(
            &workload,
            SemanticIndexAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    let index = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    assert_eq!(report.created, vec![index.clone()]);
    assert!(report.rejected_unprofitable.is_empty());
    assert_eq!(report.managed_key_cells, 64);
    assert!(report.managed_estimated_bytes > report.managed_key_cells);
    assert_eq!(store.semantic_indexes_for_test().len(), 1);
    assert!(
        store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::SemanticIndex(index.clone()))
    );
    let memory = store.artifact_memory_report();
    assert_eq!(
        memory.families.get(&PhysicalArtifactFamily::SemanticIndex),
        Some(&PhysicalArtifactFamilyMemory {
            artifacts: 1,
            advisor_managed_artifacts: 1,
            estimated_retained_bytes: report.managed_estimated_bytes,
        })
    );

    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(value.rows(), &[vec![Value::Text("Needle".into())]]);
    assert_eq!(stats.persisted_index_hits, 1);
}

#[test]
fn observable_atom_convergence_retires_only_advisor_owned_legacy_state() {
    let relation = sid(3_530);
    let equivalence = sid(3_540);
    let values = (0..32).map(|index| format!("row-{index}")).collect();
    let (context, registry, layout, mut store, _) =
        advisor_text_fixture(relation, equivalence, 390, 9_420, values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);

    store
        .install_semantic_index(binding.clone(), &context, &registry)
        .unwrap();
    store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    store
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::SemanticStatistics(binding.clone()));

    let report = store
        .converge_observable_atom_candidate(binding.clone(), &context, &registry)
        .unwrap();

    assert!(report.created);
    assert!(report.retired_legacy_indexes.is_empty());
    assert_eq!(report.retired_legacy_statistics, vec![binding.clone()]);
    assert!(store.semantic_indexes_for_test().contains_key(&binding));
    assert!(!store.has_semantic_statistics_for_test(&binding));
    assert!(store.observable_atom_states_for_test().contains_key(&binding));
    assert!(
        store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::ObservableAtom(binding.clone()))
    );
    assert!(
        !store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::SemanticIndex(binding))
    );
}

#[test]
fn observable_atom_convergence_preserves_manual_observable_pin() {
    let relation = sid(3_531);
    let equivalence = sid(3_541);
    let values = (0..16).map(|index| format!("row-{index}")).collect();
    let (context, registry, layout, mut store, _) =
        advisor_text_fixture(relation, equivalence, 391, 9_421, values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);

    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    let report = store
        .converge_observable_atom_candidate(binding.clone(), &context, &registry)
        .unwrap();

    assert!(report.retained_manual_observable);
    assert!(!report.created);
    assert!(!report.rebuilt);
    assert!(store.observable_atom_states_for_test().contains_key(&binding));
    assert!(
        !store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::ObservableAtom(binding))
    );
}

#[test]
fn unified_observable_advisor_consumes_external_pressure_without_touching_manual_state() {
    let relation = sid(3_532);
    let equivalence = sid(3_542);
    let values = (0..64).map(|index| format!("row-{index}")).collect();
    let (context, registry, layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 392, 9_422, values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    let workload = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 2,
    }];

    let report = store
        .advise_unified_observable_atoms(
            &workload,
            UnifiedObservableAdvisorInputs {
                telemetry: &UnifiedAdvisorTelemetry::default(),
                policy: UnifiedAdvisorPolicy::default(),
                pressure_policy: PhysicalPressurePolicy {
                    max_process_rss_bytes: Some(100),
                    min_available_memory_bytes: None,
                },
                pressure_sample: PhysicalPressureSample {
                    process_rss_bytes: Some(101),
                    available_memory_bytes: None,
                },
            },
            &context,
            &registry,
        )
        .unwrap();

    assert_eq!(report.rejected_pressure, vec![binding.clone()]);
    assert!(!store.observable_atom_states_for_test().contains_key(&binding));

    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    let report = store
        .advise_unified_observable_atoms(
            &workload,
            UnifiedObservableAdvisorInputs {
                telemetry: &UnifiedAdvisorTelemetry::default(),
                policy: UnifiedAdvisorPolicy::default(),
                pressure_policy: PhysicalPressurePolicy {
                    max_process_rss_bytes: Some(100),
                    min_available_memory_bytes: None,
                },
                pressure_sample: PhysicalPressureSample {
                    process_rss_bytes: Some(101),
                    available_memory_bytes: None,
                },
            },
            &context,
            &registry,
        )
        .unwrap();
    assert!(report.rejected_pressure.is_empty());
    assert!(store.observable_atom_states_for_test().contains_key(&binding));
    assert!(
        !store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::ObservableAtom(binding))
    );
}

#[test]
fn unified_observable_advisor_uses_write_telemetry_and_retain_hysteresis() {
    let relation = sid(3_533);
    let equivalence = sid(3_543);
    let values = (0..64).map(|index| format!("row-{index}")).collect();
    let (context, registry, layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 393, 9_423, values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    let id = UnifiedArtifactId::ObservableAtom(binding.clone());
    let workload = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 2,
    }];
    let mut expensive_writes = UnifiedAdvisorTelemetry::default();
    expensive_writes.observe(
        id.clone(),
        ArtifactTelemetry {
            read_work_saved: 0,
            maintenance_work: u128::MAX,
            rebuild_work: 0,
        },
    );
    let report = store
        .advise_unified_observable_atoms(
            &workload,
            UnifiedObservableAdvisorInputs {
                telemetry: &expensive_writes,
                policy: UnifiedAdvisorPolicy::default(),
                pressure_policy: PhysicalPressurePolicy::default(),
                pressure_sample: PhysicalPressureSample::default(),
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.rejected_unprofitable, vec![binding.clone()]);

    let mut retain_signal = UnifiedAdvisorTelemetry::default();
    retain_signal.observe(
        id,
        ArtifactTelemetry {
            read_work_saved: 10,
            maintenance_work: 0,
            rebuild_work: 0,
        },
    );
    store
        .converge_observable_atom_candidate(binding.clone(), &context, &registry)
        .unwrap();
    let report = store
        .advise_unified_observable_atoms(
            &[],
            UnifiedObservableAdvisorInputs {
                telemetry: &retain_signal,
                policy: UnifiedAdvisorPolicy {
                    build_threshold: 20,
                    retain_threshold: 5,
                    ..UnifiedAdvisorPolicy::default()
                },
                pressure_policy: PhysicalPressurePolicy::default(),
                pressure_sample: PhysicalPressureSample::default(),
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.retained, vec![binding.clone()]);
    assert!(store.observable_atom_states_for_test().contains_key(&binding));
}

#[test]
fn semantic_statistics_advisor_does_not_recreate_obsolete_direct_join_statistics() {
    let (context, registry, [left_relation, right_relation, _], equivalence) =
        three_relation_i64_context();
    let left_layout = LayoutBinding {
        id: LayoutId(9_958),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(9_959),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left_relation, left_layout);
    catalog.bind_relation(right_relation, right_layout);
    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(left_relation)),
        right: Box::new(RelExpr::Scan(right_relation)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
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
    let workload = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 2,
    }];
    let binding = SemanticIndexBinding::single(right_relation, right_layout, 0, equivalence);
    let report = store
        .advise_semantic_statistics(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert!(report.created.is_empty());
    assert!(
        store
            .semantic_statistics(&binding, &context, &registry)
            .unwrap()
            .is_none()
    );
    let (_, execution) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(execution.ephemeral_index_builds, 1);
}

#[test]
fn semantic_statistics_compat_advisor_preserves_manual_statistics() {
    let (context, registry, [_, right_relation, _], equivalence) = three_relation_i64_context();
    let right_layout = LayoutBinding {
        id: LayoutId(9_957),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            right_relation,
            right_layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1; 128].into())]).unwrap(),
        )
        .unwrap();
    let binding = SemanticIndexBinding::single(right_relation, right_layout, 0, equivalence);
    store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    let report = store
        .advise_semantic_statistics(
            &[],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert!(report.evicted.is_empty());
    assert!(store.has_semantic_statistics_for_test(&binding));
    assert!(
        !store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::SemanticStatistics(binding))
    );
}

#[test]
fn semantic_statistics_advisor_does_not_duplicate_exact_persisted_join_cardinality() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(9_955),
        family: LayoutFamily::Columnar,
    };
    let plan = Plan::JoinEq {
        left: Box::new(Plan::Scan { relation, layout }),
        right: Box::new(Plan::Scan { relation, layout }),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1; 128].into())]).unwrap(),
        )
        .unwrap();
    let index = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    store.install_i64_index(index, &context, &registry).unwrap();
    let report = store
        .advise_semantic_statistics(
            &[SemanticIndexWorkloadSample {
                plan,
                expected_executions: 100,
            }],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert!(report.created.is_empty());
    assert!(store.semantic_statistics_empty_for_test());
}

#[test]
fn i64_index_advisor_amortizes_build_consumes_and_evicts_owned_index() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(9_960),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64((0_i64..64).collect())]).unwrap(),
        )
        .unwrap();
    let workload = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 2,
    }];
    let binding = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };

    let report = store
        .advise_i64_indexes(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.created, vec![binding]);
    assert!(report.managed_estimated_bytes > 0);
    assert!(store.i64_index(binding).is_some());
    assert!(
        store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::I64Index(binding))
    );
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(value.rows().len(), 64);
    assert_eq!(stats.persisted_index_hits, 1);

    let report = store
        .advise_i64_indexes(
            &[],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.evicted, vec![binding]);
    assert!(store.i64_index(binding).is_none());
}

#[test]
fn i64_index_advisor_uses_bucket_savings_and_respects_budget_and_manual_pin() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(9_961),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2, 3].into())]).unwrap(),
        )
        .unwrap();
    let binding = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    let one_shot = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 1,
    }];
    let report = store
        .advise_i64_indexes(
            &one_shot,
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.created, vec![binding]);
    assert!(store.i64_index(binding).is_some());
    let report = store
        .advise_i64_indexes(
            &[],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.evicted, vec![binding]);
    assert!(store.i64_index(binding).is_none());

    let repeated = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 2,
    }];
    let report = store
        .advise_i64_indexes(
            &repeated,
            PhysicalArtifactAdvisorPolicy {
                max_managed_estimated_bytes: 1,
                max_total_estimated_bytes: usize::MAX,
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.rejected_budget, vec![binding]);
    assert!(store.i64_index(binding).is_none());

    store
        .install_i64_index(binding, &context, &registry)
        .unwrap();
    store
        .advise_i64_indexes(
            &[],
            PhysicalArtifactAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert!(store.i64_index(binding).is_some());
    assert!(
        !store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::I64Index(binding))
    );
}

#[test]
fn semantic_index_advisor_enforces_estimated_byte_budget() {
    let relation = sid(368);
    let equivalence = sid(369);
    let values = (0..64)
        .map(|index| {
            if index == 37 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let (context, registry, layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 45, 950, values);
    let index = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    let report = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan: prepared.physical().clone(),
                expected_executions: 2,
            }],
            SemanticIndexAdvisorPolicy {
                max_managed_estimated_bytes: 1,
                ..SemanticIndexAdvisorPolicy::default()
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.rejected_budget, vec![index.clone()]);
    assert!(report.created.is_empty());
    assert_eq!(report.managed_estimated_bytes, 0);
    assert!(store.semantic_index(&index).is_none());
}

#[test]
fn semantic_index_advisor_global_budget_counts_other_physical_families() {
    let relation = sid(370);
    let equivalence = sid(371);
    let values = (0..64)
        .map(|index| {
            if index == 37 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let (context, registry, layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 46, 951, values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    let fixed = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;
    assert!(fixed > 0);

    let report = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan: prepared.physical().clone(),
                expected_executions: 2,
            }],
            SemanticIndexAdvisorPolicy {
                max_total_estimated_bytes: fixed.saturating_add(1),
                ..SemanticIndexAdvisorPolicy::default()
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.fixed_estimated_bytes, fixed);
    assert_eq!(report.total_estimated_bytes_after, fixed);
    assert_eq!(report.rejected_budget, vec![binding.clone()]);
    assert!(store.semantic_index(&binding).is_none());
    assert!(
        store
            .semantic_statistics(&binding, &context, &registry)
            .unwrap()
            .is_some()
    );
}

#[test]
fn semantic_index_advisor_discovers_access_path_below_project_boundary() {
    let relation = sid(366);
    let equivalence = sid(367);
    let values = (0..64)
        .map(|index| {
            if index == 23 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let (context, registry, layout, mut store, filter) =
        advisor_text_fixture(relation, equivalence, 44, 949, values);
    let plan = Plan::Project {
        input: Box::new(filter.physical().clone()),
        columns: vec![0],
    };
    let report = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan,
                expected_executions: 2,
            }],
            SemanticIndexAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(
        report.created,
        vec![SemanticIndexBinding::single(
            relation,
            layout,
            0,
            equivalence
        )]
    );
}

#[test]
fn semantic_index_advisor_rejects_one_shot_build_that_does_not_amortize() {
    let relation = sid(355);
    let equivalence = sid(356);
    let values = (0..64)
        .map(|index| {
            if index == 17 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let (context, registry, layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 40, 943, values);
    let index = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    let report = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan: prepared.physical().clone(),
                expected_executions: 1,
            }],
            SemanticIndexAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.rejected_unprofitable, vec![index.clone()]);
    assert!(report.created.is_empty());
    assert!(store.semantic_index(&index).is_none());
}

fn advisor_budget_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    PhysicalStore,
    Plan,
    Plan,
    SemanticIndexBinding,
    SemanticIndexBinding,
) {
    let relation_a = sid(357);
    let relation_b = sid(358);
    let equivalence = sid(359);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(41));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(41));
    for relation in [relation_a, relation_b] {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![equivalence],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let layout_a = LayoutBinding {
        id: LayoutId(944),
        family: LayoutFamily::Columnar,
    };
    let layout_b = LayoutBinding {
        id: LayoutId(945),
        family: LayoutFamily::Columnar,
    };
    let values = (0..64)
        .map(|index| {
            if index == 7 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation_a,
            layout_a,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(values.clone().into())])
                .unwrap(),
        )
        .unwrap();
    store
        .install(
            relation_b,
            layout_b,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(values.into())]).unwrap(),
        )
        .unwrap();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation_a, layout_a);
    catalog.bind_relation(relation_b, layout_b);
    let filter = |relation| RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::Text("needle".into()),
        equivalence,
    };
    let plan_a = prepare_with_catalog(filter(relation_a), &context, &registry, &catalog)
        .unwrap()
        .physical()
        .clone();
    let plan_b = prepare_with_catalog(filter(relation_b), &context, &registry, &catalog)
        .unwrap()
        .physical()
        .clone();
    let index_a = SemanticIndexBinding::single(relation_a, layout_a, 0, equivalence);
    let index_b = SemanticIndexBinding::single(relation_b, layout_b, 0, equivalence);
    (context, registry, store, plan_a, plan_b, index_a, index_b)
}

#[test]
fn semantic_index_advisor_budget_evicts_only_its_own_lower_value_index() {
    let (context, registry, mut store, plan_a, plan_b, index_a, index_b) = advisor_budget_fixture();
    let policy = SemanticIndexAdvisorPolicy {
        max_managed_key_cells: 64,
        ..SemanticIndexAdvisorPolicy::default()
    };

    let first = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan: plan_a,
                expected_executions: 3,
            }],
            policy,
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(first.created, vec![index_a.clone()]);

    store
        .install_semantic_index(index_b.clone(), &context, &registry)
        .unwrap();
    let second = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan: plan_b,
                expected_executions: 3,
            }],
            SemanticIndexAdvisorPolicy {
                max_managed_key_cells: 0,
                ..SemanticIndexAdvisorPolicy::default()
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(second.evicted, vec![index_a.clone()]);
    assert_eq!(second.reused_existing, vec![index_b.clone()]);
    assert!(store.semantic_index(&index_a).is_none());
    assert!(store.semantic_index(&index_b).is_some());
    assert!(
        !store
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::SemanticIndex(index_b.clone()))
    );
}

#[test]
fn semantic_index_advisor_rebuilds_stale_gamma_bound_index_before_reuse() {
    let relation = sid(360);
    let equivalence = sid(361);
    let values = (0..64)
        .map(|index| match index {
            10 => "Needle".to_owned(),
            11 => "needle".to_owned(),
            _ => format!("row-{index}"),
        })
        .collect::<Vec<_>>();
    let (context, mut registry, _layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 42, 946, values);
    let workload = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 3,
    }];
    let first = store
        .advise_semantic_indexes(
            &workload,
            SemanticIndexAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(first.created.len(), 1);

    let mut changed_context = context.clone();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    changed_context
        .environment
        .pin_module(equivalence, exact_digest);
    let rebuilt = store
        .advise_semantic_indexes(
            &workload,
            SemanticIndexAdvisorPolicy::default(),
            &changed_context,
            &registry,
        )
        .unwrap();
    assert_eq!(rebuilt.rebuilt.len(), 1);
    let index = &rebuilt.rebuilt[0];
    assert!(
        store
            .semantic_index(index)
            .unwrap()
            .compatible_with(&changed_context, &registry)
            .unwrap()
    );
}

#[test]
fn semantic_index_advisor_budget_credits_replaced_manual_stale_index() {
    let relation = sid(370);
    let equivalence = sid(371);
    let values = (0..64)
        .map(|index| {
            if index == 17 {
                "Needle".to_owned()
            } else {
                format!("row-{index}")
            }
        })
        .collect::<Vec<_>>();
    let (context, mut registry, layout, mut store, prepared) =
        advisor_text_fixture(relation, equivalence, 47, 952, values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store
        .install_semantic_index(binding.clone(), &context, &registry)
        .unwrap();
    let old_bytes =
        semantic_index_estimated_retained_bytes(store.semantic_indexes_for_test().get(&binding).unwrap());
    let current_bytes = store
        .artifact_memory_report()
        .total_estimated_retained_bytes;

    let mut changed_context = context.clone();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    changed_context
        .environment
        .pin_module(equivalence, exact_digest);
    let candidate = MaterializedSemanticIndexState::build(
        binding.clone(),
        store.installed(relation, layout).unwrap(),
        &changed_context,
        &registry,
    )
    .unwrap();
    let replacement_bytes = semantic_index_estimated_retained_bytes(&candidate);
    let exact_budget = current_bytes
        .saturating_sub(old_bytes)
        .saturating_add(replacement_bytes);
    let report = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan: prepared.physical().clone(),
                expected_executions: 3,
            }],
            SemanticIndexAdvisorPolicy {
                max_total_estimated_bytes: exact_budget,
                ..SemanticIndexAdvisorPolicy::default()
            },
            &changed_context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.rebuilt, vec![binding]);
    assert_eq!(report.total_estimated_bytes_after, exact_budget);
    assert!(report.rejected_budget.is_empty());
}

fn text_i64_semantic_fixture(
    relations: &[SemanticId],
    text_equivalence: SemanticId,
    i64_equivalence: SemanticId,
    revision: u64,
) -> (SemanticContext, SemanticRegistry) {
    let mut registry = SemanticRegistry::default();
    let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
    environment.pin_module(text_equivalence, text_digest);
    environment.pin_module(i64_equivalence, i64_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(revision));
    for &relation in relations {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_equivalence, i64_equivalence],
                },
            })
            .unwrap();
    }
    (
        SemanticContext {
            schema,
            environment,
        },
        registry,
    )
}

fn text_i64_rows(values: &[(&str, i64)]) -> Vec<kernel_query::Row> {
    values
        .iter()
        .map(|(text, number)| vec![Value::Text((*text).into()), Value::I64(*number)])
        .collect()
}

fn text_i64_native(values: &[(&str, i64)]) -> NativeRelation {
    NativeRelation::typed_columnar(vec![
        NativeColumn::Text(values.iter().map(|(text, _)| (*text).into()).collect()),
        NativeColumn::I64(values.iter().map(|(_, number)| *number).collect()),
    ])
    .unwrap()
}

#[test]
fn semantic_index_advisor_builds_profitable_composite_join_index() {
    let left_relation = sid(362);
    let right_relation = sid(363);
    let text_equivalence = sid(364);
    let i64_equivalence = sid(365);
    let (context, registry) = text_i64_semantic_fixture(
        &[left_relation, right_relation],
        text_equivalence,
        i64_equivalence,
        43,
    );
    let left_layout = LayoutBinding {
        id: LayoutId(947),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(948),
        family: LayoutFamily::Columnar,
    };
    let left_values = [("K-7", 7), ("K-37", 37), ("missing", 999)];
    let right_values = (0..128)
        .map(|index| (format!("K-{index}"), i64::from(index)))
        .collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(left_relation, left_layout, text_i64_native(&left_values))
        .unwrap();
    store
        .install(
            right_relation,
            right_layout,
            NativeRelation::typed_columnar(vec![
                NativeColumn::Text(right_values.iter().map(|(text, _)| text.clone()).collect()),
                NativeColumn::I64(right_values.iter().map(|(_, number)| *number).collect()),
            ])
            .unwrap(),
        )
        .unwrap();
    let query = RelExpr::FilterEqColumns {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left_relation)),
            right: Box::new(RelExpr::Scan(right_relation)),
            left_column: 0,
            right_column: 0,
            equivalence: text_equivalence,
        }),
        left_column: 1,
        right_column: 3,
        equivalence: i64_equivalence,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left_relation, left_layout);
    catalog.bind_relation(right_relation, right_layout);
    let prepared = prepare_with_catalog(query.clone(), &context, &registry, &catalog).unwrap();

    let report = store
        .advise_semantic_indexes(
            &[SemanticIndexWorkloadSample {
                plan: prepared.physical().clone(),
                expected_executions: 1,
            }],
            SemanticIndexAdvisorPolicy::default(),
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(report.created.len(), 1);
    assert_eq!(report.created[0].relation, right_relation);
    assert_eq!(report.created[0].key_parts.len(), 2);

    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(stats.persisted_index_hits, 1);
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(left_relation, text_i64_rows(&left_values));
    model.relations.insert(
        right_relation,
        right_values
            .iter()
            .map(|(text, number)| vec![Value::Text(text.clone()), Value::I64(*number)])
            .collect(),
    );
    let reference = query.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(value, reference);
}

#[test]
fn composite_mixed_type_semantic_index_drives_filter_chain_and_maintains_delta() {
    let relation = sid(346);
    let text_equivalence = sid(347);
    let i64_equivalence = sid(348);
    let (context, registry) =
        text_i64_semantic_fixture(&[relation], text_equivalence, i64_equivalence, 37);
    let binding = LayoutBinding {
        id: LayoutId(939),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            text_i64_native(&[("A", 1), ("a", 2), ("A", 2), ("B", 2), ("C", 3), ("D", 4)]),
        )
        .unwrap();
    let index_binding = SemanticIndexBinding {
        relation,
        layout: binding,
        key_parts: vec![
            SemanticIndexKeyPart {
                column: 0,
                equivalence: text_equivalence,
            },
            SemanticIndexKeyPart {
                column: 1,
                equivalence: i64_equivalence,
            },
        ],
    };
    store
        .install_semantic_index(index_binding.clone(), &context, &registry)
        .unwrap();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let filter = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("a".into()),
            equivalence: text_equivalence,
        }),
        column: 1,
        value: Value::I64(2),
        equivalence: i64_equivalence,
    };
    let prepared = prepare_with_catalog(filter.clone(), &context, &registry, &catalog).unwrap();
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        value.rows(),
        &[
            vec![Value::Text("a".into()), Value::I64(2)],
            vec![Value::Text("A".into()), Value::I64(2)],
        ]
    );
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.scanned_rows, 2);

    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    store
        .apply_relation_delta(
            relation,
            binding,
            &RelationDelta {
                inserted: vec![vec![Value::Text("a".into()), Value::I64(3)]],
                removed: vec![vec![Value::Text("A".into()), Value::I64(2)]],
                result_type,
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(store.semantic_index(&index_binding).unwrap().row_count(), 6);
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        value.rows(),
        &[vec![Value::Text("A".into()), Value::I64(2)]]
    );
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.scanned_rows, 1);
}

#[test]
fn composite_mixed_key_semantic_index_fuses_column_filtered_join() {
    let left_relation = sid(349);
    let right_relation = sid(350);
    let text_equivalence = sid(351);
    let i64_equivalence = sid(352);
    let (context, registry) = text_i64_semantic_fixture(
        &[left_relation, right_relation],
        text_equivalence,
        i64_equivalence,
        38,
    );
    let left_layout = LayoutBinding {
        id: LayoutId(940),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(941),
        family: LayoutFamily::Columnar,
    };
    let left_values = [("A", 1), ("A", 2), ("B", 2)];
    let right_values = [("a", 1), ("a", 3), ("B", 2), ("b", 3), ("C", 9), ("D", 10)];
    let left_rows = text_i64_rows(&left_values);
    let right_rows = text_i64_rows(&right_values);
    let mut store = PhysicalStore::default();
    store
        .install(left_relation, left_layout, text_i64_native(&left_values))
        .unwrap();
    store
        .install(right_relation, right_layout, text_i64_native(&right_values))
        .unwrap();
    store
        .install_semantic_index(
            SemanticIndexBinding {
                relation: right_relation,
                layout: right_layout,
                key_parts: vec![
                    SemanticIndexKeyPart {
                        column: 1,
                        equivalence: i64_equivalence,
                    },
                    SemanticIndexKeyPart {
                        column: 0,
                        equivalence: text_equivalence,
                    },
                ],
            },
            &context,
            &registry,
        )
        .unwrap();

    let query = RelExpr::FilterEqColumns {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left_relation)),
            right: Box::new(RelExpr::Scan(right_relation)),
            left_column: 0,
            right_column: 0,
            equivalence: text_equivalence,
        }),
        left_column: 1,
        right_column: 3,
        equivalence: i64_equivalence,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left_relation, left_layout);
    catalog.bind_relation(right_relation, right_layout);
    let prepared = prepare_with_catalog(query.clone(), &context, &registry, &catalog).unwrap();
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        value.rows(),
        &[
            vec![
                Value::Text("A".into()),
                Value::I64(1),
                Value::Text("a".into()),
                Value::I64(1),
            ],
            vec![
                Value::Text("B".into()),
                Value::I64(2),
                Value::Text("B".into()),
                Value::I64(2),
            ],
        ]
    );
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.persisted_index_cost_rejections, 0);
    assert_eq!(stats.scanned_rows, left_rows.len());

    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(left_relation, left_rows);
    model.relations.insert(right_relation, right_rows);
    let reference = query.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(value, reference);
}

#[test]
fn composite_mixed_key_observable_atom_drives_persisted_join_without_legacy_index() {
    let left_relation = sid(449);
    let right_relation = sid(450);
    let text_equivalence = sid(451);
    let i64_equivalence = sid(452);
    let (context, registry) = text_i64_semantic_fixture(
        &[left_relation, right_relation],
        text_equivalence,
        i64_equivalence,
        48,
    );
    let left_layout = LayoutBinding {
        id: LayoutId(1040),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(1041),
        family: LayoutFamily::Columnar,
    };
    let left_values = [("A", 1), ("A", 2), ("B", 2)];
    let right_values = [("a", 1), ("a", 3), ("B", 2), ("b", 3), ("C", 9), ("D", 10)];
    let mut store = PhysicalStore::default();
    store
        .install(left_relation, left_layout, text_i64_native(&left_values))
        .unwrap();
    store
        .install(right_relation, right_layout, text_i64_native(&right_values))
        .unwrap();
    let binding = SemanticIndexBinding {
        relation: right_relation,
        layout: right_layout,
        key_parts: vec![
            SemanticIndexKeyPart {
                column: 1,
                equivalence: i64_equivalence,
            },
            SemanticIndexKeyPart {
                column: 0,
                equivalence: text_equivalence,
            },
        ],
    };
    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    assert!(store.semantic_indexes_for_test().is_empty());
    assert!(store.observable_atom_state(&binding).is_some());

    let query = RelExpr::FilterEqColumns {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(left_relation)),
            right: Box::new(RelExpr::Scan(right_relation)),
            left_column: 0,
            right_column: 0,
            equivalence: text_equivalence,
        }),
        left_column: 1,
        right_column: 3,
        equivalence: i64_equivalence,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left_relation, left_layout);
    catalog.bind_relation(right_relation, right_layout);
    let prepared = prepare_with_catalog(query.clone(), &context, &registry, &catalog).unwrap();
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.persisted_index_cost_rejections, 0);
    assert_eq!(stats.scanned_rows, left_values.len());

    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(left_relation, text_i64_rows(&left_values));
    model
        .relations
        .insert(right_relation, text_i64_rows(&right_values));
    assert_eq!(value, query.evaluate(&model, &context, &registry).unwrap());
}

#[test]
fn persisted_text_semantic_index_maintains_delta_and_pins_gamma() {
    let (context, mut registry, relation, equivalence, binding, mut store) =
        text_semantic_index_fixture();
    let index_binding = SemanticIndexBinding::single(relation, binding, 0, equivalence);
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    store
        .apply_relation_delta(
            relation,
            binding,
            &RelationDelta {
                inserted: vec![vec![Value::Text("C".into())]],
                removed: vec![vec![Value::Text("A".into())]],
                result_type,
            },
            &context,
            &registry,
        )
        .unwrap();
    let index = store.semantic_index(&index_binding).unwrap();
    assert_eq!(index.row_count(), 3);
    assert_eq!(
        index
            .probe_value(&Value::Text("A".into()), &context, &registry)
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        index
            .probe_value(&Value::Text("c".into()), &context, &registry)
            .unwrap()
            .unwrap()
            .len(),
        1
    );

    let mut changed_context = context.clone();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    changed_context
        .environment
        .pin_module(equivalence, exact_digest);
    let changed_type = RelExpr::Scan(relation)
        .typecheck(&changed_context, &registry)
        .unwrap();
    assert_eq!(
        store.apply_relation_delta(
            relation,
            binding,
            &RelationDelta {
                inserted: vec![vec![Value::Text("D".into())]],
                removed: Vec::new(),
                result_type: changed_type,
            },
            &changed_context,
            &registry,
        ),
        Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild)
    );

    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let exact_filter = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::Text("A".into()),
        equivalence,
    };
    let prepared =
        prepare_with_catalog(exact_filter, &changed_context, &registry, &catalog).unwrap();
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert!(value.rows().is_empty());
    assert_eq!(stats.persisted_index_hits, 0);
}

#[test]
fn native_runtime_refuses_layout_family_mismatch() {
    let mut store = PhysicalStore::default();
    let relation = sid(1);
    let binding = LayoutBinding {
        id: LayoutId(902),
        family: LayoutFamily::Columnar,
    };
    assert_eq!(
        store.install(relation, binding, NativeRelation::row_store(vec![])),
        Err(PhysicalExecutionError::LayoutFamilyMismatch)
    );
}

#[test]
fn typed_stateful_distinct_consumes_batch_selection_without_full_input_rows() {
    let relation = sid(400);
    let equivalence = sid(401);
    let (context, registry) = two_i64_column_context(relation, equivalence, 50);
    let logical = RelExpr::Distinct {
        input: Box::new(RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::I64(1),
                equivalence,
            }),
            columns: vec![1],
        }),
        column_equivalences: vec![equivalence],
    };
    let binding = LayoutBinding {
        id: LayoutId(970),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let source = [(1_i64, 10_i64), (1, 10), (1, 20), (2, 10)];
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(source.iter().map(|row| row.0).collect()),
                NativeColumn::I64(source.iter().map(|row| row.1).collect()),
            ])
            .unwrap(),
        )
        .unwrap();
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        source
            .iter()
            .map(|(key, payload)| vec![Value::I64(*key), Value::I64(*payload)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows(), &[vec![Value::I64(10)], vec![Value::I64(20)]]);
    assert_eq!(stats.typed_batch_chain_hits, 1);
    assert_eq!(stats.typed_stateful_batch_hits, 1);
}

#[test]
fn typed_stateful_distinct_and_group_preserve_text_ci_semantics() {
    let relation = sid(410);
    let text_eq = sid(411);
    let count_eq = sid(412);
    let mut registry = SemanticRegistry::default();
    let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let count_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(54));
    environment.pin_module(text_eq, text_digest);
    environment.pin_module(count_eq, count_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(54));
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
    let distinct = RelExpr::Distinct {
        input: Box::new(RelExpr::Scan(relation)),
        column_equivalences: vec![text_eq],
    };
    let group = RelExpr::Group {
        input: Box::new(RelExpr::Scan(relation)),
        group_columns: vec![0],
        group_equivalences: vec![text_eq],
        aggregate: AggregateSpec::Count {
            result_equivalence: count_eq,
        },
    };
    let binding = LayoutBinding {
        id: LayoutId(974),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared_distinct =
        prepare_with_catalog(distinct.clone(), &context, &registry, &catalog).unwrap();
    let prepared_group =
        prepare_with_catalog(group.clone(), &context, &registry, &catalog).unwrap();
    let source = ["A".to_owned(), "a".to_owned(), "B".to_owned()];
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(source.to_vec().into())])
                .unwrap(),
        )
        .unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        source
            .iter()
            .cloned()
            .map(|value| vec![Value::Text(value)])
            .collect(),
    );

    let (native_distinct, distinct_stats) = prepared_distinct
        .execute_native_pinned(&store, &registry)
        .unwrap();
    assert_eq!(
        native_distinct,
        distinct.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(distinct_stats.typed_stateful_batch_hits, 1);

    let (native_group, group_stats) = prepared_group
        .execute_native_pinned(&store, &registry)
        .unwrap();
    assert_eq!(
        native_group,
        group.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(
        native_group.rows(),
        &[
            vec![Value::Text("A".to_owned()), Value::I64(2)],
            vec![Value::Text("B".to_owned()), Value::I64(1)],
        ]
    );
    assert_eq!(group_stats.typed_stateful_batch_hits, 1);
}

