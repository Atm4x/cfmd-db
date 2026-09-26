#[test]
fn typed_stateful_group_count_consumes_filtered_projected_batch_selection() {
    let relation = sid(402);
    let equivalence = sid(403);
    let (context, registry) = two_i64_column_context(relation, equivalence, 51);
    let logical = RelExpr::Group {
        input: Box::new(RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::I64(1),
                equivalence,
            }),
            columns: vec![1],
        }),
        group_columns: vec![0],
        group_equivalences: vec![equivalence],
        aggregate: AggregateSpec::Count {
            result_equivalence: equivalence,
        },
    };
    let binding = LayoutBinding {
        id: LayoutId(971),
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
    assert_eq!(
        native.rows(),
        &[
            vec![Value::I64(10), Value::I64(2)],
            vec![Value::I64(20), Value::I64(1)]
        ]
    );
    assert_eq!(stats.typed_batch_chain_hits, 1);
    assert_eq!(stats.typed_stateful_batch_hits, 1);
}

#[test]
fn typed_stateful_group_exact_f64_sum_reads_only_group_key_and_aggregate_column() {
    let relation = sid(404);
    let i64_eq = sid(405);
    let f64_eq = sid(406);
    let mut registry = SemanticRegistry::default();
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(52));
    environment.pin_module(i64_eq, i64_digest);
    environment.pin_module(f64_eq, f64_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(52));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::F64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![i64_eq, f64_eq],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let logical = RelExpr::Group {
        input: Box::new(RelExpr::Scan(relation)),
        group_columns: vec![0],
        group_equivalences: vec![i64_eq],
        aggregate: AggregateSpec::ExactF64Sum {
            value_column: 1,
            result_equivalence: f64_eq,
        },
    };
    let binding = LayoutBinding {
        id: LayoutId(972),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let source = [(1_i64, 1.5_f64), (1, 2.25), (2, -0.75)];
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(source.iter().map(|row| row.0).collect()),
                NativeColumn::F64Bits(source.iter().map(|row| row.1.to_bits()).collect()),
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
            .map(|(key, value)| vec![Value::I64(*key), Value::F64Bits(value.to_bits())])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.typed_stateful_batch_hits, 1);
}

#[test]
fn typed_stateful_top_k_with_ties_sorts_positions_before_materializing_rows() {
    let relation = sid(407);
    let equivalence = sid(408);
    let (mut context, mut registry) = two_i64_column_context(relation, equivalence, 53);
    let ordering = sid(409);
    let ordering_digest = registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let logical = RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Project {
            input: Box::new(RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(relation)),
                column: 0,
                value: Value::I64(1),
                equivalence,
            }),
            columns: vec![1],
        }),
        column: 0,
        ordering,
        direction: OrderDirection::Ascending,
        k: 2,
    };
    let binding = LayoutBinding {
        id: LayoutId(973),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let source = [(1_i64, 30_i64), (1, 20), (1, 20), (1, 10), (2, 0)];
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
    assert_eq!(
        native.rows(),
        &[
            vec![Value::I64(10)],
            vec![Value::I64(20)],
            vec![Value::I64(20)]
        ]
    );
    assert_eq!(stats.typed_batch_chain_hits, 1);
    assert_eq!(stats.typed_stateful_batch_hits, 1);
}

#[test]
fn typed_group_top_k_producer_chain_stays_columnar_until_final_output() {
    let relation = sid(420);
    let equivalence = sid(421);
    let (mut context, mut registry) = two_i64_column_context(relation, equivalence, 55);
    let ordering = sid(422);
    let ordering_digest = registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let group = RelExpr::Group {
        input: Box::new(RelExpr::Scan(relation)),
        group_columns: vec![1],
        group_equivalences: vec![equivalence],
        aggregate: AggregateSpec::Count {
            result_equivalence: equivalence,
        },
    };
    let top_k = RelExpr::TopKWithTies {
        input: Box::new(group),
        column: 1,
        ordering,
        direction: OrderDirection::Ascending,
        k: 1,
    };
    let filtered = RelExpr::FilterEqConst {
        input: Box::new(top_k),
        column: 1,
        value: Value::I64(1),
        equivalence,
    };
    let logical = RelExpr::Project {
        input: Box::new(filtered),
        columns: vec![0],
    };
    let binding = LayoutBinding {
        id: LayoutId(975),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let source = [(0_i64, 10_i64), (0, 10), (0, 20), (0, 30), (0, 30)];
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
            .map(|(left, right)| vec![Value::I64(*left), Value::I64(*right)])
            .collect(),
    );
    assert_eq!(
        native,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(native.rows(), &[vec![Value::I64(20)]]);
    assert_eq!(stats.typed_stateful_producer_hits, 1);
    assert_eq!(stats.typed_stateful_batch_hits, 0);
}

#[test]
fn typed_text_group_producer_uses_primitive_canonical_keys() {
    let relation = sid(423);
    let text_equivalence = sid(424);
    let count_equivalence = sid(425);
    let mut registry = SemanticRegistry::default();
    let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let count_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(56));
    environment.pin_module(text_equivalence, text_digest);
    environment.pin_module(count_equivalence, count_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(56));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![text_equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns: vec![0],
            group_equivalences: vec![text_equivalence],
            aggregate: AggregateSpec::Count {
                result_equivalence: count_equivalence,
            },
        }),
        columns: vec![0],
    };
    let binding = LayoutBinding {
        id: LayoutId(976),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let values = vec!["A".to_owned(), "a".to_owned(), "B".to_owned()];
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(values.clone().into())])
                .unwrap(),
        )
        .unwrap();
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::Text(value)])
            .collect(),
    );
    assert_eq!(
        native,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(stats.typed_stateful_producer_hits, 1);
}

#[test]
fn typed_f64_top_k_producer_stays_columnar_until_project_boundary() {
    let relation = sid(426);
    let equivalence = sid(427);
    let ordering = sid(428);
    let mut registry = SemanticRegistry::default();
    let equivalence_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
    let ordering_digest = registry.install_ordering(kernel_semantics::OrderingModule::F64Total);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(57));
    environment.pin_module(equivalence, equivalence_digest);
    environment.pin_module(ordering, ordering_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(57));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::F64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::TopKWithTies {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            ordering,
            direction: OrderDirection::Ascending,
            k: 2,
        }),
        columns: vec![0],
    };
    let binding = LayoutBinding {
        id: LayoutId(977),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let values = [-0.0_f64, 1.0, -1.0, 0.0];
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::F64Bits(
                values.iter().map(|value| value.to_bits()).collect(),
            )])
            .unwrap(),
        )
        .unwrap();
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::F64Bits(value.to_bits())])
            .collect(),
    );
    assert_eq!(
        native,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(stats.typed_stateful_producer_hits, 1);
}

#[test]
fn typed_top_k_can_feed_group_without_row_materialization() {
    let relation = sid(429);
    let equivalence = sid(430);
    let (mut context, mut registry) = two_i64_column_context(relation, equivalence, 58);
    let ordering = sid(431);
    let ordering_digest = registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let top_k = RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Scan(relation)),
        column: 1,
        ordering,
        direction: OrderDirection::Descending,
        k: 2,
    };
    let group = RelExpr::Group {
        input: Box::new(top_k),
        group_columns: vec![0],
        group_equivalences: vec![equivalence],
        aggregate: AggregateSpec::Count {
            result_equivalence: equivalence,
        },
    };
    let logical = RelExpr::Project {
        input: Box::new(group),
        columns: vec![0, 1],
    };
    let binding = LayoutBinding {
        id: LayoutId(978),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let source = [(1_i64, 10_i64), (1, 20), (2, 30), (2, 40)];
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
            .map(|(key, value)| vec![Value::I64(*key), Value::I64(*value)])
            .collect(),
    );
    assert_eq!(
        native,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(native.rows(), &[vec![Value::I64(2), Value::I64(2)]]);
    assert_eq!(stats.typed_stateful_producer_hits, 1);
}

#[test]
fn typed_i64_top_k_descending_selects_high_threshold_with_ties() {
    let mut positions = vec![0, 1, 2, 3, 4];
    let values = [1_i64, 9, 8, 8, 2];
    let mut stats = ExecutionStats::default();
    select_top_k_i64_positions_for_test(
        &mut positions,
        &values,
        OrderDirection::Descending,
        2,
        &mut stats,
    );
    assert_eq!(
        positions
            .iter()
            .map(|&position| values[position])
            .collect::<Vec<_>>(),
        vec![9, 8, 8]
    );
}

#[test]
fn typed_f64_top_k_uses_bounded_semantic_frontier() {
    let ordering = sid(9_771);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_ordering(OrderingModule::F64Total);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9_771));
    environment.pin_module(ordering, digest);
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(9_771)),
        environment,
    };
    let values = (0..1_000)
        .rev()
        .map(|value| f64::from(value).to_bits())
        .collect::<Vec<_>>();
    let column = NativeColumn::F64Bits(values.clone().into());
    let mut positions = (0..values.len()).collect::<Vec<_>>();
    let mut stats = ExecutionStats::default();
    select_top_k_semantic_positions_for_test(
        &mut positions,
        &column,
        0,
        ordering,
        OrderDirection::Ascending,
        2,
        &context,
        &registry,
        &mut stats,
    )
    .unwrap();
    assert_eq!(positions.len(), 2);
    assert_eq!(values[positions[0]], 0.0_f64.to_bits());
    assert_eq!(values[positions[1]], 1.0_f64.to_bits());
    assert_eq!(stats.values_read, values.len());
}

#[test]
fn native_group_count_matches_logical_reference_including_empty_global_group() {
    let (context, registry, relation) = planning_context();
    for (group_columns, rows) in [
        (
            vec![0],
            vec![
                vec![Value::I64(1)],
                vec![Value::I64(1)],
                vec![Value::I64(2)],
            ],
        ),
        (Vec::new(), Vec::new()),
    ] {
        let group_equivalences = if group_columns.is_empty() {
            Vec::new()
        } else {
            vec![sid(101)]
        };
        let logical = RelExpr::Group {
            input: Box::new(RelExpr::Scan(relation)),
            group_columns,
            group_equivalences,
            aggregate: AggregateSpec::Count {
                result_equivalence: sid(101),
            },
        };
        let binding = LayoutBinding {
            id: LayoutId(903),
            family: LayoutFamily::RowStore,
        };
        let mut catalog = PhysicalCatalog::default();
        catalog.bind_relation(relation, binding);
        let prepared =
            prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
        let mut store = PhysicalStore::default();
        store
            .install(relation, binding, NativeRelation::row_store(rows.clone()))
            .unwrap();
        let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
        let mut model = kernel_model::FiniteModel::default();
        model.relations.insert(relation, rows);
        let reference = logical.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(native, reference);
    }
}

#[test]
fn native_group_exact_f64_sum_matches_logical_reference() {
    let relation = sid(210);
    let equivalence = sid(211);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(2));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(2));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::F64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let logical = RelExpr::Group {
        input: Box::new(RelExpr::Scan(relation)),
        group_columns: Vec::new(),
        group_equivalences: Vec::new(),
        aggregate: AggregateSpec::ExactF64Sum {
            value_column: 0,
            result_equivalence: equivalence,
        },
    };
    let binding = LayoutBinding {
        id: LayoutId(910),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let rows = [1.5_f64, 2.25, -0.75]
        .into_iter()
        .map(|value| vec![Value::F64Bits(value.to_bits())])
        .collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(relation, binding, NativeRelation::row_store(rows.clone()))
        .unwrap();
    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
}

#[test]
fn native_top_k_with_ties_matches_logical_reference() {
    let (mut context, mut registry, relation) = planning_context();
    let ordering = sid(102);
    let digest = registry.install_ordering(kernel_semantics::OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, digest);
    let logical = RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        ordering,
        direction: OrderDirection::Ascending,
        k: 2,
    };
    let binding = LayoutBinding {
        id: LayoutId(906),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let rows = vec![
        vec![Value::I64(3)],
        vec![Value::I64(2)],
        vec![Value::I64(1)],
        vec![Value::I64(2)],
    ];
    let mut store = PhysicalStore::default();
    store
        .install(relation, binding, NativeRelation::row_store(rows.clone()))
        .unwrap();
    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
}

#[test]
fn ordered_view_cursor_is_revision_gamma_bound_and_layout_independent_under_ties() {
    let relation = sid(9_995_000);
    let equivalence = sid(9_995_001);
    let ordering = sid(9_995_002);
    let revision = RevisionId::new(9_995_000);
    let (mut context, mut registry) = two_i64_column_context(relation, equivalence, 9_995_000);
    let ordering_digest = registry.install_ordering(OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let logical = RelExpr::Scan(relation);
    let rows = vec![
        vec![Value::I64(2), Value::I64(4)],
        vec![Value::I64(1), Value::I64(2)],
        vec![Value::I64(1), Value::I64(1)],
        vec![Value::I64(2), Value::I64(3)],
        vec![Value::I64(1), Value::I64(1)],
    ];
    let expected = [
        vec![Value::I64(1), Value::I64(1)],
        vec![Value::I64(1), Value::I64(1)],
        vec![Value::I64(1), Value::I64(2)],
        vec![Value::I64(2), Value::I64(3)],
        vec![Value::I64(2), Value::I64(4)],
    ];
    let mut prepared_and_stores = Vec::new();
    for (binding, data) in ordered_view_test_backends(&rows) {
        prepared_and_stores.push(install_ordered_view_backend(
            relation, binding, data, &logical, &context, &registry, revision,
        ));
    }
    let spec = OrderedViewSpec {
        column: 0,
        ordering,
        direction: OrderDirection::Ascending,
    };
    let first_view = prepared_and_stores[0]
        .0
        .ordered_view(spec.clone(), &registry)
        .unwrap();
    let first_snapshot = first_view
        .snapshot_native_pinned(&prepared_and_stores[0].1, &registry)
        .unwrap();
    let first = first_snapshot.page(None, 2).unwrap();
    assert_eq!(first.rows, expected[..2]);
    let cursor = first.next_cursor.unwrap();
    assert_eq!(cursor.revision(), revision);
    assert_eq!(cursor.semantic_revision(), context.revision());
    for (prepared, store) in &prepared_and_stores {
        let view = prepared.ordered_view(spec.clone(), &registry).unwrap();
        let snapshot = view.snapshot_native_pinned(store, &registry).unwrap();
        let second = snapshot.page(Some(&cursor), 2).unwrap();
        assert_eq!(second.rows, expected[2..4]);
        let third = snapshot.page(second.next_cursor.as_ref(), 2).unwrap();
        assert_eq!(third.rows, expected[4..]);
        assert!(third.next_cursor.is_none());
    }
    let mut wrong_revision_store = prepared_and_stores[0].1.clone();
    wrong_revision_store.set_revision_for_test(Some(RevisionId::new(9_995_999)));
    let wrong_revision_snapshot = first_view
        .snapshot_native_pinned(&wrong_revision_store, &registry)
        .unwrap();
    assert_eq!(
        wrong_revision_snapshot.page(Some(&cursor), 2),
        Err(OrderedViewError::CursorBindingMismatch)
    );
    let descending = prepared_and_stores[0]
        .0
        .ordered_view(
            OrderedViewSpec {
                direction: OrderDirection::Descending,
                ..spec.clone()
            },
            &registry,
        )
        .unwrap();
    let descending_snapshot = descending
        .snapshot_native_pinned(&prepared_and_stores[0].1, &registry)
        .unwrap();
    assert_eq!(
        descending_snapshot.page(None, 8).unwrap().rows,
        vec![
            vec![Value::I64(2), Value::I64(3)],
            vec![Value::I64(2), Value::I64(4)],
            vec![Value::I64(1), Value::I64(1)],
            vec![Value::I64(1), Value::I64(1)],
            vec![Value::I64(1), Value::I64(2)],
        ]
    );
    assert_eq!(
        descending_snapshot.page(Some(&cursor), 2),
        Err(OrderedViewError::CursorBindingMismatch)
    );
    ordered_view_rejects_semantic_revision_drift(
        relation,
        equivalence,
        ordering,
        logical,
        &prepared_and_stores[0].1,
        &cursor,
    );
}

fn ordered_view_rejects_semantic_revision_drift(
    relation: SemanticId,
    equivalence: SemanticId,
    ordering: SemanticId,
    logical: RelExpr,
    store: &PhysicalStore,
    cursor: &OrderedViewCursor,
) {
    let (mut context, mut registry) = two_i64_column_context(relation, equivalence, 9_995_000);
    let ordering_digest = registry.install_ordering(OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let unrelated = sid(9_995_099);
    let unrelated_digest = registry.install_equivalence(EquivalenceModule::BoolExact);
    context.environment.pin_module(unrelated, unrelated_digest);
    let binding = LayoutBinding {
        id: LayoutId(9_995_010),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical, &context, &registry, &catalog).unwrap();
    let view = prepared
        .ordered_view(
            OrderedViewSpec {
                column: 0,
                ordering,
                direction: OrderDirection::Ascending,
            },
            &registry,
        )
        .unwrap();
    let snapshot = view.snapshot_native_pinned(store, &registry).unwrap();
    assert_eq!(
        snapshot.page(Some(cursor), 2),
        Err(OrderedViewError::CursorBindingMismatch)
    );
}

#[test]
fn ordered_view_requires_revision_binding_and_congruent_ordering() {
    let (mut context, mut registry, relation) = planning_context();
    let ordering = sid(9_995_100);
    let ordering_digest = registry.install_ordering(OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let binding = LayoutBinding {
        id: LayoutId(9_995_100),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared =
        prepare_with_catalog(RelExpr::Scan(relation), &context, &registry, &catalog).unwrap();
    let view = prepared
        .ordered_view(
            OrderedViewSpec {
                column: 0,
                ordering,
                direction: OrderDirection::Ascending,
            },
            &registry,
        )
        .unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::row_store(vec![vec![Value::I64(1)]]),
        )
        .unwrap();
    assert_eq!(
        view.snapshot_native_pinned(&store, &registry),
        Err(OrderedViewError::UnboundPhysicalRevision)
    );
    store.bind_revision(RevisionId::new(1)).unwrap();
    let snapshot = view.snapshot_native_pinned(&store, &registry).unwrap();
    assert_eq!(snapshot.page(None, 0), Err(OrderedViewError::ZeroPageSize));
}

#[test]
fn ordered_view_snapshot_compacts_bag_multiplicity_and_seeks_within_run() {
    let (mut context, mut registry, relation) = planning_context();
    let ordering = sid(9_995_101);
    let ordering_digest = registry.install_ordering(OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let binding = LayoutBinding {
        id: LayoutId(9_995_101),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared =
        prepare_with_catalog(RelExpr::Scan(relation), &context, &registry, &catalog).unwrap();
    let view = prepared
        .ordered_view(
            OrderedViewSpec {
                column: 0,
                ordering,
                direction: OrderDirection::Ascending,
            },
            &registry,
        )
        .unwrap();
    let row = vec![Value::I64(7)];
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::row_store(vec![row.clone(); 10_000]),
        )
        .unwrap();
    store.bind_revision(RevisionId::new(9_995_101)).unwrap();

    let snapshot = view.snapshot_native_pinned(&store, &registry).unwrap();
    assert_eq!(snapshot.run_count_for_test(), 1);
    assert_eq!(
        snapshot.first_run_count_for_test().unwrap(),
        10_000
    );

    let first = snapshot.page(None, 3).unwrap();
    assert_eq!(first.rows, vec![row.clone(); 3]);
    let first_cursor = first.next_cursor.unwrap();
    assert_eq!(first_cursor.occurrence, 2);
    let second = snapshot.page(Some(&first_cursor), 3).unwrap();
    assert_eq!(second.rows, vec![row; 3]);
    assert_eq!(second.next_cursor.as_ref().unwrap().occurrence, 5);

    let mut invalid = first_cursor;
    invalid.occurrence = 10_000;
    assert_eq!(
        snapshot.page(Some(&invalid), 3),
        Err(OrderedViewError::CursorPositionMismatch)
    );
}

fn ordered_materialization_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    LayoutBinding,
    RelExpr,
    SemanticId,
    RuntimeRevisionBundle,
) {
    let (mut context, mut registry, relation) = planning_context();
    let ordering = sid(9_995_102);
    let ordering_digest = registry.install_ordering(OrderingModule::I64Ascending);
    context.environment.pin_module(ordering, ordering_digest);
    let binding = LayoutBinding {
        id: LayoutId(9_995_102),
        family: LayoutFamily::Columnar,
    };
    let query = RelExpr::Scan(relation);
    let rows = [3_i64, 1, 2, 2];
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        rows.iter()
            .copied()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(rows.to_vec().into())]).unwrap(),
        )
        .unwrap();
    let revision = kernel_revision::Revision::build(
        RevisionId::new(9_995_102),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let runtime = RuntimeRevisionBundle::build(
        revision,
        store,
        BTreeMap::from([(relation, binding)]),
        &[RuntimeMaterializationSpec {
            id: test_materialization_id(),
            query: query.clone(),
        }],
        &registry,
    )
    .unwrap();
    (
        context, registry, relation, binding, query, ordering, runtime,
    )
}

#[test]
fn ordered_view_snapshot_can_use_runtime_maintained_materialization_without_plan_execution() {
    let (context, registry, relation, binding, query, ordering, runtime) =
        ordered_materialization_fixture();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(query.clone(), &context, &registry, &catalog).unwrap();
    let view = prepared
        .ordered_view(
            OrderedViewSpec {
                column: 0,
                ordering,
                direction: OrderDirection::Ascending,
            },
            &registry,
        )
        .unwrap();
    let snapshot = view
        .snapshot_materialized_pinned(&runtime, test_materialization_id(), &registry)
        .unwrap();
    let page = snapshot.page(None, 8).unwrap();
    assert_eq!(
        page.rows,
        vec![
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(2)],
            vec![Value::I64(3)],
        ]
    );
    assert_eq!(snapshot.revision, runtime.revision_id());
    assert!(page.next_cursor.is_none());
    assert_eq!(
        view.snapshot_materialized_pinned(&runtime, MaterializationId::new(9_995_999), &registry,),
        Err(OrderedViewError::UnknownMaterialization(
            MaterializationId::new(9_995_999,)
        ))
    );

    let projected = prepare_with_catalog(
        RelExpr::Project {
            input: Box::new(query),
            columns: vec![0],
        },
        &context,
        &registry,
        &catalog,
    )
    .unwrap();
    let projected_view = projected
        .ordered_view(
            OrderedViewSpec {
                column: 0,
                ordering,
                direction: OrderDirection::Ascending,
            },
            &registry,
        )
        .unwrap();
    assert_eq!(
        projected_view
            .snapshot_materialized_pinned(&runtime, test_materialization_id(), &registry,),
        Err(OrderedViewError::MaterializationQueryMismatch)
    );
}

#[test]
fn ordered_view_advances_from_certified_materialization_output_delta() {
    let (context, registry, relation, binding, query, ordering, runtime) =
        ordered_materialization_fixture();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let view = prepared
        .ordered_view(
            OrderedViewSpec {
                column: 0,
                ordering,
                direction: OrderDirection::Ascending,
            },
            &registry,
        )
        .unwrap();
    let snapshot = view
        .snapshot_materialized_pinned(&runtime, test_materialization_id(), &registry)
        .unwrap();
    let old_page = snapshot.page(None, 2).unwrap();
    let old_cursor = old_page.next_cursor.unwrap();

    let delta = scan_delta(relation, &[0, 4], &[2], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let transition = prepare_runtime_revision(&runtime, 9_995_103, &mutations, &registry).unwrap();
    let output_delta = transition
        .output_deltas()
        .get(&test_materialization_id())
        .unwrap();
    assert_eq!(output_delta.inserted.len(), 2);
    assert_eq!(output_delta.removed.len(), 1);

    let advanced = transition
        .advance_ordered_view(&snapshot, test_materialization_id(), &registry)
        .unwrap();
    assert_eq!(advanced.revision, RevisionId::new(9_995_103));
    assert_eq!(
        advanced.page(None, 16).unwrap().rows,
        vec![
            vec![Value::I64(0)],
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(3)],
            vec![Value::I64(4)],
        ]
    );
    assert_eq!(
        advanced.page(Some(&old_cursor), 2),
        Err(OrderedViewError::CursorBindingMismatch)
    );

    let mut stale = snapshot.clone();
    stale.revision = RevisionId::new(9_995_999);
    assert_eq!(
        transition.advance_ordered_view(&stale, test_materialization_id(), &registry),
        Err(OrderedViewError::SnapshotBindingMismatch)
    );
}

#[test]
fn typed_i64_columnar_rejects_non_i64_schema() {
    let relation = sid(200);
    let equivalence = sid(201);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1));
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
        id: LayoutId(907),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared =
        prepare_with_catalog(RelExpr::Scan(relation), &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::i64_columnar(vec![vec![1, 2, 3]]).unwrap(),
        )
        .unwrap();
    assert_eq!(
        prepared.execute_native_pinned(&store, &registry),
        Err(PhysicalExecutionError::PhysicalTypeMismatch)
    );
}

#[test]
fn baseline_lowering_is_exact_round_trip_for_every_operator_shape() {
    let expr = RelExpr::PromoteToBag(Box::new(RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Group {
            input: Box::new(RelExpr::Distinct {
                input: Box::new(RelExpr::Project {
                    input: Box::new(RelExpr::FilterEqConst {
                        input: Box::new(RelExpr::JoinEq {
                            left: Box::new(RelExpr::Scan(sid(1))),
                            right: Box::new(RelExpr::Scan(sid(2))),
                            left_column: 0,
                            right_column: 1,
                            equivalence: sid(10),
                        }),
                        column: 0,
                        value: Value::I64(7),
                        equivalence: sid(10),
                    }),
                    columns: vec![0, 2],
                }),
                column_equivalences: vec![sid(10), sid(11)],
            }),
            group_columns: vec![0],
            group_equivalences: vec![sid(10)],
            aggregate: AggregateSpec::Count {
                result_equivalence: sid(12),
            },
        }),
        column: 1,
        ordering: sid(20),
        direction: OrderDirection::Descending,
        k: 5,
    }));

    let plan = Plan::lower_baseline(&expr);
    assert_eq!(plan.to_logical_expr(), expr);
    assert_eq!(
        plan.shape(),
        PlanShape {
            nodes: 9,
            scans: 2,
            filters: 1,
            projects: 1,
            joins: 1,
            differences: 0,
            anti_joins: 0,
            distincts: 1,
            groups: 1,
            top_k: 1,
            bag_promotions: 1,
        }
    );
    assert_eq!(plan.shape().stateful_operator_upper_bound(), 4);
    assert_eq!(plan.shape().nodes, logical_node_count(&expr));
}

#[test]
fn plan_shape_counts_difference_and_anti_join_as_stateful_producers() {
    let expr = RelExpr::Difference {
        left: Box::new(RelExpr::AntiJoin {
            left: Box::new(RelExpr::Scan(sid(1))),
            right: Box::new(RelExpr::Scan(sid(2))),
            left_column: 0,
            right_column: 0,
            equivalence: sid(10),
        }),
        right: Box::new(RelExpr::Scan(sid(3))),
    };
    let shape = Plan::lower_baseline(&expr).shape();
    assert_eq!(shape.differences, 1);
    assert_eq!(shape.anti_joins, 1);
    assert_eq!(shape.stateful_operator_upper_bound(), 2);
    assert_eq!(shape.nodes, logical_node_count(&expr));
}

#[test]
fn baseline_choices_are_explicit_and_contain_no_opaque_operator_variant() {
    let join = Plan::lower_baseline(&RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(sid(1))),
        right: Box::new(RelExpr::Scan(sid(2))),
        left_column: 0,
        right_column: 0,
        equivalence: sid(3),
    });
    assert!(matches!(join, Plan::JoinEq { .. }));

    let top_k = Plan::lower_baseline(&RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Scan(sid(1))),
        column: 0,
        ordering: sid(4),
        direction: OrderDirection::Ascending,
        k: 3,
    });
    assert!(matches!(top_k, Plan::TopKWithTies { .. }));
}

#[test]
fn lowering_crosses_the_generic_checked_certificate_boundary() {
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::Scan(sid(1))),
        columns: vec![0],
    };
    let checked = certify_baseline_lowering(logical.clone()).unwrap();
    assert_eq!(checked.spec().logical, logical);
    assert_eq!(checked.spec().physical.to_logical_expr(), logical);
    assert_eq!(
        checked.certificate(),
        &LoweringCertificate::ExactLogicalRoundTrip
    );
}

#[test]
fn lowering_checker_rejects_a_plan_that_changes_logical_meaning() {
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::Scan(sid(1))),
        columns: vec![0],
    };
    let physical = Plan::Project {
        input: Box::new(Plan::Scan {
            relation: sid(1),
            layout: LayoutBinding::LOGICAL_MODEL_ROWS,
        }),
        columns: vec![1],
    };
    let spec = LoweringSpec { logical, physical };
    let result = kernel_proof::verify_certificate::<LoweringChecker>(
        &spec,
        LoweringCertificate::ExactLogicalRoundTrip,
    );
    assert!(matches!(result, Err(LoweringError::LogicalMeaningChanged)));
}

#[test]
fn prepared_plan_requires_logical_typecheck_before_lowering() {
    let (context, registry, relation) = planning_context();
    let invalid = RelExpr::Project {
        input: Box::new(RelExpr::Scan(relation)),
        columns: vec![1],
    };
    assert!(matches!(
        prepare_baseline(invalid, &context, &registry),
        Err(PlanPrepareError::Query(RelQueryError::ColumnOutOfBounds))
    ));

    let valid = RelExpr::Project {
        input: Box::new(RelExpr::Scan(relation)),
        columns: vec![0],
    };
    let prepared = prepare_baseline(valid.clone(), &context, &registry).unwrap();
    assert_eq!(prepared.logical(), &valid);
    assert_eq!(prepared.physical().to_logical_expr(), valid);
    assert_eq!(prepared.semantic_context(), &context);
}

#[test]
fn lowering_preserves_native_relation_layout_without_forced_conversion_node() {
    let relation = sid(77);
    let layout = LayoutBinding {
        id: LayoutId(9001),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::Scan(relation)),
        columns: vec![0],
    };
    let checked = certify_lowering_with_catalog(logical.clone(), &catalog).unwrap();
    let Plan::Project { input, .. } = &checked.spec().physical else {
        unreachable!();
    };
    assert!(matches!(
        input.as_ref(),
        Plan::Scan {
            relation: actual,
            layout: actual_layout,
        } if *actual == relation && *actual_layout == layout
    ));
    assert_eq!(checked.spec().physical.to_logical_expr(), logical);
    assert_eq!(checked.spec().physical.shape().nodes, 2);
}

#[test]
fn prepared_plan_preserves_catalog_layout_after_typecheck() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(42),
        family: LayoutFamily::KeyValue,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let prepared =
        prepare_with_catalog(RelExpr::Scan(relation), &context, &registry, &catalog).unwrap();
    assert!(matches!(
        prepared.physical(),
        Plan::Scan {
            relation: actual,
            layout: actual_layout,
        } if *actual == relation && *actual_layout == layout
    ));
    assert_eq!(prepared.result_type().columns.len(), 1);
    assert_eq!(prepared.semantic_context(), &context);
}

#[test]
fn prepared_plan_reference_execution_rejects_semantic_context_drift() {
    let (context, registry, relation) = planning_context();
    let prepared = prepare_baseline(RelExpr::Scan(relation), &context, &registry).unwrap();
    let mut drifted = context.clone();
    drifted.environment = SemanticEnvironment::new(SemanticEnvId::new(999));
    assert_eq!(
        prepared.reference_execute(&kernel_model::FiniteModel::default(), &drifted, &registry),
        Err(RelQueryError::SemanticRevisionMismatch)
    );
}

#[test]
fn generational_slots_reuse_metadata_without_reviving_stale_handles() {
    let mut relation = InstalledRelation::new(
        NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![10, 20, 30].into())]).unwrap(),
    );
    let stale = relation.row_id_at(0).unwrap();
    assert_eq!(relation.remove_row(0).unwrap(), stale);
    assert_eq!(relation.position(stale), None);
    assert_eq!(relation.slots.len(), 3);
    assert_eq!(relation.free_slots.len(), 1);

    let replacement = relation.push_row(&vec![Value::I64(40)]).unwrap();
    assert_eq!(replacement.slot, stale.slot);
    assert_eq!(replacement.generation, stale.generation + 1);
    assert_eq!(relation.position(stale), None);
    assert_eq!(relation.position(replacement), Some(2));
    assert_eq!(relation.slots.len(), 3);
    assert!(relation.free_slots.is_empty());

    let values = relation
        .scan_positions()
        .map(|position| materialize_native_row(&relation.data, position).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            vec![Value::I64(20)],
            vec![Value::I64(30)],
            vec![Value::I64(40)],
        ]
    );
}

#[test]
fn generation_exhaustion_is_rejected_before_any_relation_mutation() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(943),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
        )
        .unwrap();
    {
        let installed = store
            .relation_entry_mut((relation, binding.id))
            .unwrap();
        let installed = Arc::make_mut(installed);
        installed.row_ids[0].generation = u64::MAX;
        installed.slots[0].generation = u64::MAX;
        installed.logical_head = Some(installed.row_ids[0]);
        installed.slots[1].previous = Some(installed.row_ids[0]);
        installed.slots[0].next = Some(installed.row_ids[1]);
    }
    let before = store.clone();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let delta = RelationDelta {
        inserted: Vec::new(),
        removed: vec![vec![Value::I64(1)]],
        result_type,
    };
    assert_eq!(
        store.apply_relation_delta(relation, binding, &delta, &context, &registry),
        Err(PhysicalExecutionError::HandleGenerationExhausted)
    );
    assert_eq!(store, before);
}

#[test]
fn high_churn_slot_metadata_is_bounded_by_peak_live_rows() {
    let mut relation = InstalledRelation::new(
        NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![0].into())]).unwrap(),
    );
    let first = relation.row_id_at(0).unwrap();
    for value in 1_i64..=10_000 {
        let current = relation.row_id_at(0).unwrap();
        relation.remove_row(0).unwrap();
        relation.push_row(&vec![Value::I64(value)]).unwrap();
        assert_eq!(relation.slots.len(), 1);
        assert!(relation.free_slots.is_empty());
        assert_eq!(relation.scan_positions().collect::<Vec<_>>(), vec![0]);
        assert_eq!(relation.position(current), None);
    }
    let current = relation.row_id_at(0).unwrap();
    assert_eq!(current.slot, first.slot);
    assert_eq!(current.generation, 10_000);
    assert_eq!(relation.position(first), None);
    assert_eq!(relation.position(current), Some(0));
}

#[test]
fn durable_commit_linearizes_before_runtime_publication_and_recovers_from_logical_wal() {
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(300, 980, &[1, 2]);
    let base_revision = runtime.revision().clone();
    let specs = [RuntimeMaterializationSpec {
        id: test_materialization_id(),
        query: RelExpr::Scan(relation),
    }];
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 301, &mutations, &registry);
    let cell = RuntimeRevisionCell::new(runtime);
    let mut wal = kernel_durability::SimulatedRevisionWal::new();

    let receipt = cell
        .commit_revision_durable(
            ClientTransactionId::new(1001),
            &RevisionTransitionRequest {
                target_revision: &target,
                mutations: &mutations,
                registry: &registry,
            },
            &mut wal,
        )
        .unwrap();

    assert_eq!(receipt.durable.target_revision(), RevisionId::new(301));
    let live = cell.snapshot().unwrap();
    assert_eq!(live.revision_id(), RevisionId::new(301));
    assert_eq!(
        runtime_physical_i64_values(live.root(), relation, binding),
        vec![2, 3]
    );

    let scan = kernel_durability::scan_wal(wal.crash_image(), base_revision.id()).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(301));
    let recovered = recover_runtime_bundle(&base_revision, &scan, &specs, &[], &registry).unwrap();
    assert_eq!(recovered.revision(), live.revision());
    assert_eq!(
        runtime_physical_i64_values(&recovered, relation, LayoutBinding::RECOVERY_ROW_STORE,),
        vec![2, 3]
    );
    assert_eq!(
        runtime_maintained_i64_values(&recovered, &context, &registry),
        vec![2, 3]
    );
}

#[test]
fn durable_recovery_replays_multiple_nonconsecutive_revision_ids_idempotently() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(310, 981, &[1, 2]);
    let base_revision = runtime.revision().clone();
    let cell = RuntimeRevisionCell::new(runtime);
    let mut wal = kernel_durability::SimulatedRevisionWal::new();

    let delta_a = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations_a = [RevisionRelationMutation {
        relation,
        delta: &delta_a,
    }];
    let snapshot = cell.snapshot().unwrap();
    let target_a = target_revision_for(snapshot.root(), 400, &mutations_a, &registry);
    drop(snapshot);
    cell.commit_revision_durable(
        ClientTransactionId::new(1002),
        &RevisionTransitionRequest {
            target_revision: &target_a,
            mutations: &mutations_a,
            registry: &registry,
        },
        &mut wal,
    )
    .unwrap();

    let delta_b = scan_delta(relation, &[4], &[2], &context, &registry);
    let mutations_b = [RevisionRelationMutation {
        relation,
        delta: &delta_b,
    }];
    let snapshot = cell.snapshot().unwrap();
    let target_b = target_revision_for(snapshot.root(), 900, &mutations_b, &registry);
    drop(snapshot);
    cell.commit_revision_durable(
        ClientTransactionId::new(1003),
        &RevisionTransitionRequest {
            target_revision: &target_b,
            mutations: &mutations_b,
            registry: &registry,
        },
        &mut wal,
    )
    .unwrap();

    let scan = kernel_durability::scan_wal(wal.crash_image(), base_revision.id()).unwrap();
    let first = replay_durable_revisions(&base_revision, &scan, &registry).unwrap();
    let second = replay_durable_revisions(&base_revision, &scan, &registry).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.id(), RevisionId::new(900));
    assert_eq!(first, *cell.snapshot().unwrap().revision());
}

struct FailCommitDurability {
    inner: kernel_durability::SimulatedRevisionWal,
}

impl kernel_durability::RevisionDurability for FailCommitDurability {
    fn durably_prepare(
        &mut self,
        descriptor: &kernel_durability::DurableRevisionDescriptor,
    ) -> Result<kernel_durability::DurablePrepareToken, kernel_durability::DurabilityError> {
        self.inner.durably_prepare(descriptor)
    }

    fn durably_commit(
        &mut self,
        _prepared: kernel_durability::DurablePrepareToken,
    ) -> Result<kernel_durability::DurableCommitReceipt, kernel_durability::DurabilityError> {
        Err(kernel_durability::DurabilityError::Poisoned)
    }
}

#[test]
fn commit_durability_failure_after_seal_fail_stops_runtime_until_recovery() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(320, 982, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 321, &mutations, &registry);
    let cell = RuntimeRevisionCell::new(runtime);
    let mut durability = FailCommitDurability {
        inner: kernel_durability::SimulatedRevisionWal::new(),
    };

    assert!(matches!(
        cell.commit_revision_durable(
            ClientTransactionId::new(1004),
            &RevisionTransitionRequest {
                target_revision: &target,
                mutations: &mutations,
                registry: &registry,
            },
            &mut durability,
        ),
        Err(DurableRuntimeCommitError::CommitDurabilityUncertain(
            kernel_durability::DurabilityError::Poisoned
        ))
    ));
    assert_eq!(
        cell.snapshot(),
        Err(PhysicalExecutionError::RuntimeRecoveryRequired)
    );
    let scan =
        kernel_durability::scan_wal(durability.inner.crash_image(), RevisionId::new(320)).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(320));
}

struct InvalidateAfterPrepare<'a> {
    inner: kernel_durability::SimulatedRevisionWal,
    live: &'a RuntimeRevisionCell,
    index: I64IndexBinding,
    registry: &'a SemanticRegistry,
}

impl kernel_durability::RevisionDurability for InvalidateAfterPrepare<'_> {
    fn durably_prepare(
        &mut self,
        descriptor: &kernel_durability::DurableRevisionDescriptor,
    ) -> Result<kernel_durability::DurablePrepareToken, kernel_durability::DurabilityError> {
        let prepared = self.inner.durably_prepare(descriptor)?;
        self.live
            .install_i64_index(self.index, self.registry)
            .expect("reconstructible index publication");
        Ok(prepared)
    }

    fn durably_commit(
        &mut self,
        prepared: kernel_durability::DurablePrepareToken,
    ) -> Result<kernel_durability::DurableCommitReceipt, kernel_durability::DurabilityError> {
        self.inner.durably_commit(prepared)
    }
}

#[test]
fn stale_after_durable_prepare_leaves_only_uncommitted_wal_prepare() {
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(330, 983, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 331, &mutations, &registry);
    let cell = RuntimeRevisionCell::new(runtime);
    let mut durability = InvalidateAfterPrepare {
        inner: kernel_durability::SimulatedRevisionWal::new(),
        live: &cell,
        index: I64IndexBinding {
            relation,
            layout: binding,
            key_column: 0,
            equivalence: sid(101),
        },
        registry: &registry,
    };

    assert!(matches!(
        cell.commit_revision_durable(
            ClientTransactionId::new(1005),
            &RevisionTransitionRequest {
                target_revision: &target,
                mutations: &mutations,
                registry: &registry,
            },
            &mut durability,
        ),
        Err(DurableRuntimeCommitError::Runtime(
            PhysicalExecutionError::StalePreparedTransition
        ))
    ));
    assert_eq!(cell.snapshot().unwrap().revision_id(), RevisionId::new(330));
    let scan =
        kernel_durability::scan_wal(durability.inner.crash_image(), RevisionId::new(330)).unwrap();
    assert_eq!(scan.durable_revision(), RevisionId::new(330));
    assert!(scan.committed().is_empty());
}

#[test]
fn runtime_bootstrap_rejects_semantically_equal_bag_reordering() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(4242),
        family: LayoutFamily::Columnar,
    };
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(relation, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
    let revision = kernel_revision::Revision::build(
        RevisionId::new(500),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![2, 1].into())]).unwrap(),
        )
        .unwrap();

    assert_eq!(
        RuntimeRevisionBundle::build(
            revision,
            store,
            BTreeMap::from([(relation, binding)]),
            &[RuntimeMaterializationSpec {
                id: test_materialization_id(),
                query: RelExpr::Scan(relation),
            }],
            &registry,
        ),
        Err(PhysicalExecutionError::LogicalPhysicalStateMismatch(
            relation
        ))
    );
}

#[test]
fn resolved_leaf_delete_preserves_authoritative_scan_order() {
    let (context, registry, relation, _binding, runtime) =
        scan_runtime_bundle(600, 4600, &[1, 2, 3]);
    let delta = scan_delta(relation, &[], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let cell = RuntimeRevisionCell::new(runtime);
    let prepared = prepare_runtime_cell_revision(&cell, 601, &mutations, &registry).unwrap();
    let _ = prepared.seal(&cell).unwrap().publish();
    let after = cell.snapshot().unwrap();

    let logical = after.revision().state().model.relations[&relation]
        .iter()
        .map(|row| match row.as_slice() {
            [Value::I64(value)] => *value,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    let maintained = after
        .materialization(test_materialization_id())
        .unwrap()
        .output_value(&context, &registry)
        .unwrap()
        .rows()
        .iter()
        .map(|row| match row.as_slice() {
            [Value::I64(value)] => *value,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();

    assert_eq!(logical, vec![2, 3]);
    assert_eq!(maintained, logical);
}

fn durable_test_dir(name: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("cfmd-plan-{name}-{}-{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn coarse_observable_core_fixture(
    dir: &std::path::Path,
) -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    LayoutBinding,
    SemanticIndexBinding,
    DurableRuntime,
) {
    let relation = sid(5_372);
    let equivalence = sid(5_371);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(5_372));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(5_372));
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
    let rows = vec![
        vec![Value::Text("Alpha".into())],
        vec![Value::Text("ALPHA".into())],
        vec![Value::Text("Beta".into())],
    ];
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows.clone());
    let revision = kernel_revision::Revision::build(
        RevisionId::new(5_372),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let layout = LayoutBinding {
        id: LayoutId(9_372),
        family: LayoutFamily::RowStore,
    };
    let mut physical = PhysicalStore::default();
    physical
        .install(relation, layout, NativeRelation::row_store(rows))
        .unwrap();
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    physical
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    let root = RuntimeRevisionBundle::build(
        revision,
        physical,
        BTreeMap::from([(relation, layout)]),
        &[],
        &registry,
    )
    .unwrap();
    let runtime = DurableRuntime::create(root, dir, &registry).unwrap();
    (context, registry, relation, layout, binding, runtime)
}

#[test]
fn durable_runtime_owner_restarts_from_checkpoint_plus_wal_tail() {
    let dir = durable_test_dir("restart-tail");
    let (context, registry, relation, layout, root) = scan_runtime_bundle(500, 1200, &[1, 2]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let snapshot = runtime.snapshot().unwrap();
    let target = target_revision_for(snapshot.root(), 501, &mutations, &registry);
    drop(snapshot);
    runtime
        .commit_revision(
            ClientTransactionId::new(2001),
            &RevisionTransitionRequest {
                target_revision: &target,
                mutations: &mutations,
                registry: &registry,
            },
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision_id(), RevisionId::new(501));
    assert_eq!(
        runtime_physical_i64_values(snapshot.root(), relation, layout),
        vec![2, 3]
    );
    assert_eq!(
        runtime_maintained_i64_values(snapshot.root(), &context, &registry),
        vec![2, 3]
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_runtime_checkpoint_rotates_base_then_replays_only_new_tail() {
    let dir = durable_test_dir("checkpoint-tail");
    let (context, registry, relation, layout, root) = scan_runtime_bundle(510, 1201, &[1, 2]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();

    let delta_a = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations_a = [RevisionRelationMutation {
        relation,
        delta: &delta_a,
    }];
    let snapshot = runtime.snapshot().unwrap();
    let target_a = target_revision_for(snapshot.root(), 511, &mutations_a, &registry);
    drop(snapshot);
    runtime
        .commit_revision(
            ClientTransactionId::new(2002),
            &RevisionTransitionRequest {
                target_revision: &target_a,
                mutations: &mutations_a,
                registry: &registry,
            },
        )
        .unwrap();
    let checkpoint = runtime.checkpoint().unwrap();
    assert_eq!(checkpoint.generation, 2);
    assert_eq!(checkpoint.base_revision, RevisionId::new(511));

    let delta_b = scan_delta(relation, &[4], &[2], &context, &registry);
    let mutations_b = [RevisionRelationMutation {
        relation,
        delta: &delta_b,
    }];
    let snapshot = runtime.snapshot().unwrap();
    let target_b = target_revision_for(snapshot.root(), 512, &mutations_b, &registry);
    drop(snapshot);
    runtime
        .commit_revision(
            ClientTransactionId::new(2003),
            &RevisionTransitionRequest {
                target_revision: &target_b,
                mutations: &mutations_b,
                registry: &registry,
            },
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision_id(), RevisionId::new(512));
    assert_eq!(
        runtime_physical_i64_values(snapshot.root(), relation, layout),
        vec![3, 4]
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_runtime_compaction_preserves_reopenability() {
    let dir = durable_test_dir("compact");
    let (_, registry, _relation, _, root) = scan_runtime_bundle(520, 1202, &[1, 2]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);
    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().revision_id(),
        RevisionId::new(520)
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_checkpoint_rebuilds_physical_index_recipes_on_reopen() {
    let dir = durable_test_dir("physical-recipes");
    let (context, registry, relation, layout, root) = scan_runtime_bundle(515, 1215, &[1, 2, 3, 3]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    runtime.install_semantic_index(binding, &registry).unwrap();
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();

    runtime.with_durability_for_test(|durability| {
        assert_eq!(durability.physical_artifact_specs().len(), 2);
    });

    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let snapshot = runtime.snapshot().unwrap();
    let target = target_revision_for(snapshot.root(), 516, &mutations, &registry);
    drop(snapshot);
    runtime
        .commit_revision(
            ClientTransactionId::new(2516),
            &RevisionTransitionRequest {
                target_revision: &target,
                mutations: &mutations,
                registry: &registry,
            },
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision_id(), RevisionId::new(516));
    let recovered_binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    assert_eq!(
        snapshot
            .physical_store()
            .observable_atom_state(&recovered_binding)
            .unwrap()
            .row_count(),
        4
    );
    assert!(
        snapshot
            .physical_store()
            .semantic_index(&recovered_binding)
            .is_none()
    );
    assert!(
        !snapshot
            .physical_store()
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::ObservableAtom(
                recovered_binding.clone()
            ))
    );

    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(4),
        equivalence: sid(101),
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let (result, stats) = prepared
        .execute_native_pinned(snapshot.physical_store(), &registry)
        .unwrap();
    assert_eq!(result.rows(), &[vec![Value::I64(4)]]);
    assert_eq!(stats.persisted_index_hits, 1);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_typed_layout_and_i64_index_rebuild_and_serve_after_compaction() {
    let dir = durable_test_dir("typed-layout-i64-recovery");
    let values = (1_i64..=64).collect::<Vec<_>>();
    let (context, registry, relation, layout, root) = scan_runtime_bundle(5_180, 9_180, &values);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let index = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    runtime.install_i64_index(index, &registry).unwrap();
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.root().relation_layout(relation), Some(layout));
    assert!(matches!(
        &snapshot
            .physical_store()
            .installed(relation, layout)
            .unwrap()
            .data,
        NativeRelation::TypedColumnar { columns, row_count }
            if *row_count == 64 && matches!(columns.as_slice(), [NativeColumn::I64(values)] if values.len() == 64)
    ));
    let recovered_index = snapshot.physical_store().i64_index(index).unwrap();
    assert_eq!(recovered_index.row_count(), 64);
    assert_eq!(recovered_index.probe_len_for_test(32).unwrap(), 1);

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
    let (result, stats) = prepared
        .execute_native_pinned(snapshot.physical_store(), &registry)
        .unwrap();
    assert_eq!(result.rows().len(), 64);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.ephemeral_index_builds, 0);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_layout_recipe_round_trips_supported_native_representations() {
    let (context, registry, relation) = planning_context();
    let rows = vec![vec![Value::I64(1)], vec![Value::I64(2)]];
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows.clone());
    let revision = kernel_revision::Revision::build(
        RevisionId::new(5_179),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let cases = [
        (
            "row",
            LayoutBinding {
                id: LayoutId(9_179),
                family: LayoutFamily::RowStore,
            },
            NativeRelation::row_store(rows.clone()),
        ),
        (
            "value-columnar",
            LayoutBinding {
                id: LayoutId(9_180),
                family: LayoutFamily::Columnar,
            },
            NativeRelation::columnar(vec![vec![Value::I64(1), Value::I64(2)]]).unwrap(),
        ),
        (
            "i64-columnar",
            LayoutBinding {
                id: LayoutId(9_181),
                family: LayoutFamily::Columnar,
            },
            NativeRelation::i64_columnar(vec![vec![1, 2]]).unwrap(),
        ),
    ];
    for (name, layout, native) in cases {
        let dir = durable_test_dir(name);
        let mut physical = PhysicalStore::default();
        physical.install(relation, layout, native.clone()).unwrap();
        let root = RuntimeRevisionBundle::build(
            revision.clone(),
            physical,
            BTreeMap::from([(relation, layout)]),
            &[],
            &registry,
        )
        .unwrap();
        drop(DurableRuntime::create(root, &dir, &registry).unwrap());
        let reopened = DurableRuntime::open(&dir).unwrap();
        let snapshot = reopened.snapshot().unwrap();
        assert_eq!(snapshot.root().relation_layout(relation), Some(layout));
        assert_eq!(
            snapshot
                .physical_store()
                .installed(relation, layout)
                .unwrap()
                .data,
            native
        );
        drop(snapshot);
        drop(reopened);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn assert_recovered_dense_live_ref_layout(
    snapshot: &RuntimeRevisionSnapshot,
    relation: SemanticId,
    layout: LayoutBinding,
    source_dense: &Arc<DenseEntityIds>,
    first: EntityId,
    second: EntityId,
) {
    let recovered_dense = snapshot.revision().dense_entity_ids();
    assert!(!Arc::ptr_eq(source_dense, &recovered_dense));
    let installed = snapshot
        .physical_store()
        .installed(relation, layout)
        .unwrap();
    let NativeRelation::TypedColumnar { columns, row_count } = &installed.data else {
        panic!("typed relation layout was not recovered");
    };
    assert_eq!(*row_count, 2);
    let [NativeColumn::DenseLiveEntityIds { ids, values, .. }] = columns.as_slice() else {
        panic!("recovered live-ref column was not rebound to dense local ids");
    };
    assert!(Arc::ptr_eq(ids, &recovered_dense));
    assert_eq!(
        values
            .iter()
            .map(|&local| recovered_dense.external(local).unwrap())
            .collect::<Vec<_>>(),
        vec![first, second]
    );
}

#[test]
fn durable_typed_live_ref_layout_rebinds_fresh_revision_local_dense_ids() {
    let dir = durable_test_dir("typed-live-ref-recovery");
    let relation = sid(5_181);
    let entity_type = sid(5_182);
    let equivalence = sid(5_183);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(entity_type));
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(5_181));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(5_181));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(entity_type))],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let first = EntityId::new(5_181);
    let second = EntityId::new(5_182);
    let rows = [first, second]
        .into_iter()
        .map(|id| vec![Value::LiveEntityRef { entity_type, id }])
        .collect::<Vec<_>>();
    let mut state = kernel_model::DatabaseState::default();
    for entity in [first, second] {
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
        state
            .model
            .carriers
            .entry(entity_type)
            .or_default()
            .insert(entity);
    }
    state.model.relations.insert(relation, rows);
    let revision =
        kernel_revision::Revision::build(RevisionId::new(5_181), &context, &registry, state)
            .unwrap();
    let source_dense = revision.dense_entity_ids();
    let layout = LayoutBinding {
        id: LayoutId(9_181),
        family: LayoutFamily::Columnar,
    };
    let dense_column = NativeColumn::dense_live_entity_ids(
        entity_type,
        Arc::clone(&source_dense),
        vec![first, second],
    )
    .unwrap();
    let mut physical = PhysicalStore::default();
    physical
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![dense_column]).unwrap(),
        )
        .unwrap();
    let root = RuntimeRevisionBundle::build(
        revision,
        physical,
        BTreeMap::from([(relation, layout)]),
        &[],
        &registry,
    )
    .unwrap();
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_recovered_dense_live_ref_layout(
        &snapshot,
        relation,
        layout,
        &source_dense,
        first,
        second,
    );
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_initial_recipes_rebuild_samf_statistics_and_quotient_factor_with_ownership() {
    let dir = durable_test_dir("physical-recipes-initial");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(516, 1216, &[1, 1, 2, 3]);
    let binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    root.physical_store_mut_for_test()
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    root.physical_store_mut_for_test()
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    let installed = root.physical_store_mut_for_test().installed(relation, layout).unwrap();
    let factor = MaterializedSemanticQuotientFactorState::build(
        binding.clone(),
        installed,
        &context,
        &registry,
    )
    .unwrap();
    root.physical_store_mut_for_test()
        .semantic_quotient_factors_mut()
        .insert(binding.clone(), Arc::new(factor));
    root.physical_store_mut_for_test()
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::SemanticQuotientFactor(binding.clone()));

    assert_eq!(root.durable_physical_artifact_specs().len(), 4);
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());
    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    let recovered = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    assert!(
        snapshot
            .physical_store()
            .has_semantic_statistics_for_test(&recovered)
    );
    assert!(
        snapshot
            .physical_store()
            .semantic_quotient_factors_for_test()
            .contains_key(&recovered)
    );
    assert!(
        snapshot
            .physical_store()
            .observable_atom_states_for_test()
            .contains_key(&recovered)
    );
    assert!(
        snapshot
            .physical_store()
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::SemanticQuotientFactor(
                recovered.clone()
            ))
    );
    assert!(
        !snapshot
            .physical_store()
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::SemanticStatistics(recovered.clone()))
    );
    assert!(
        !snapshot
            .physical_store()
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::ObservableAtom(recovered))
    );
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn checkpoint_rehydrates_observable_atom_core_without_semantic_rebuild_work() {
    let dir = durable_test_dir("observable-atom-core-rehydrate-pass84");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_274, 9_274, &[1, 1, 2, 3]);
    let binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    root.physical_store_mut_for_test()
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    runtime.checkpoint().unwrap();
    drop(runtime);

    let (durability, _) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(durability.artifact_cores().len(), 1);
    drop(durability);

    let (reopened, report) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_key_evaluations: 0,
            max_advisor_rebuild_semantic_work_units: 0,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(report.rehydrated.len(), 1);
    assert!(report.rebuilt.is_empty());
    assert_eq!(report.attempted_rebuild_key_evaluations, 0);
    assert_eq!(report.attempted_rebuild_semantic_work_units, 0);
    let snapshot = reopened.snapshot().unwrap();
    let atom = snapshot
        .physical_store()
        .observable_atom_states_for_test()
        .get(&binding)
        .unwrap();
    assert_eq!(atom.row_count(), 4);
    assert_eq!(atom.distinct_key_count(), 3);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wal_tail_replays_observable_atom_core_before_rehydration() {
    let dir = durable_test_dir("observable-atom-core-wal-replay-pass84");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_273, 9_273, &[1, 1, 2, 3]);
    let binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    root.physical_store_mut_for_test()
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    runtime.checkpoint().unwrap();

    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    runtime
        .commit_derived_relation_data(
            ClientTransactionId::new(0x5273),
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(5_273),
                target_revision: RevisionId::new(5_274),
                mutations: &mutations,
            },
        )
        .unwrap();
    drop(runtime);

    let (reopened, report) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_key_evaluations: 0,
            max_advisor_rebuild_semantic_work_units: 0,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().revision_id(),
        RevisionId::new(5_274)
    );
    assert_eq!(report.rehydrated.len(), 1);
    assert!(report.rebuilt.is_empty());
    assert_eq!(report.attempted_rebuild_key_evaluations, 0);
    let snapshot = reopened.snapshot().unwrap();
    let atom = snapshot
        .physical_store()
        .observable_atom_states_for_test()
        .get(&binding)
        .unwrap();
    assert_eq!(atom.row_count(), 4);
    assert_eq!(atom.distinct_key_count(), 4);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

