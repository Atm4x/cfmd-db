use std::process::Command;

use kernel_schema::{
    FieldDef, RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext,
    SemanticEnvironment, StructuralEquivalenceDef, StructuralOrderingDef, TypeExpr,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
use kernel_types::{EntityId, MaterializationId, RevisionId, SchemaRevisionId, SemanticEnvId};

use super::*;

fn sid(value: u64) -> SemanticId {
    SemanticId(u128::from(value))
}

fn two_i64_column_context(
    relation: SemanticId,
    equivalence: SemanticId,
    revision: u64,
) -> (SemanticContext, SemanticRegistry) {
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(revision));
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
    (
        SemanticContext {
            schema,
            environment,
        },
        registry,
    )
}

fn planning_context() -> (SemanticContext, SemanticRegistry, SemanticId) {
    let relation = sid(100);
    let equivalence = sid(101);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    (
        SemanticContext {
            schema,
            environment,
        },
        registry,
        relation,
    )
}

fn install_ordered_view_backend(
    relation: SemanticId,
    binding: LayoutBinding,
    data: NativeRelation,
    logical: &RelExpr,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    revision: RevisionId,
) -> (PreparedPlan, PhysicalStore) {
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), context, registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store.install(relation, binding, data).unwrap();
    store.bind_revision(revision).unwrap();
    (prepared, store)
}

fn ordered_view_test_backends(rows: &[kernel_query::Row]) -> Vec<(LayoutBinding, NativeRelation)> {
    let i64_column = |column: usize| {
        rows.iter()
            .map(|row| match row[column] {
                Value::I64(value) => value,
                _ => unreachable!(),
            })
            .collect::<Vec<_>>()
    };
    vec![
        (
            LayoutBinding {
                id: LayoutId(9_995_010),
                family: LayoutFamily::RowStore,
            },
            NativeRelation::row_store(rows.to_vec()),
        ),
        (
            LayoutBinding {
                id: LayoutId(9_995_011),
                family: LayoutFamily::Columnar,
            },
            NativeRelation::columnar(vec![
                rows.iter().map(|row| row[0].clone()).collect(),
                rows.iter().map(|row| row[1].clone()).collect(),
            ])
            .unwrap(),
        ),
        (
            LayoutBinding {
                id: LayoutId(9_995_012),
                family: LayoutFamily::Columnar,
            },
            NativeRelation::i64_columnar(vec![i64_column(0), i64_column(1)]).unwrap(),
        ),
        (
            LayoutBinding {
                id: LayoutId(9_995_013),
                family: LayoutFamily::Columnar,
            },
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(i64_column(0).into()),
                NativeColumn::I64(i64_column(1).into()),
            ])
            .unwrap(),
        ),
    ]
}

fn two_text_relation_context(
    set_semantics: bool,
) -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    SemanticId,
    SemanticId,
) {
    let left = sid(9_980_100);
    let right = sid(9_980_101);
    let equivalence = sid(9_980_102);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9_980_100));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(9_980_100));
    let semantics = if set_semantics {
        RelationSemantics::Set {
            column_equivalences: vec![equivalence],
        }
    } else {
        RelationSemantics::Bag {
            column_equivalences: vec![equivalence],
        }
    };
    for relation in [left, right] {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: semantics.clone(),
            })
            .unwrap();
    }
    (
        SemanticContext {
            schema,
            environment,
        },
        registry,
        left,
        right,
        equivalence,
    )
}

fn algebraic_filter_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    SemanticId,
    SemanticId,
    SemanticId,
    LayoutBinding,
    PhysicalStore,
    PhysicalCatalog,
) {
    let relation = sid(9_990_100);
    let field = sid(9_990_101);
    let product_equivalence = sid(9_990_102);
    let text_equivalence = sid(9_990_103);
    let i64_equivalence = sid(9_990_104);
    let mut registry = SemanticRegistry::default();
    let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9_990_100));
    environment.pin_module(text_equivalence, text_digest);
    environment.pin_module(i64_equivalence, i64_digest);
    let product_type = TypeExpr::Product(BTreeMap::from([(
        field,
        TypeExpr::Scalar(ScalarType::Text),
    )]));
    let mut schema = Schema::new(SchemaRevisionId::new(9_990_100));
    schema
        .define_structural_equivalence(
            product_equivalence,
            StructuralEquivalenceDef::Product {
                fields: BTreeMap::from([(field, text_equivalence)]),
            },
        )
        .unwrap();
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![product_type.clone(), TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![product_equivalence, i64_equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let rows = vec![
        vec![named_product(field, "Alice"), Value::I64(1)],
        vec![named_product(field, "ALICE"), Value::I64(2)],
        vec![named_product(field, "Bob"), Value::I64(3)],
    ];
    let layout = LayoutBinding {
        id: LayoutId(9_990_100),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_from_rows(
                &rows,
                &[product_type, TypeExpr::Scalar(ScalarType::I64)],
            )
            .unwrap(),
        )
        .unwrap();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    (
        context,
        registry,
        relation,
        field,
        product_equivalence,
        i64_equivalence,
        layout,
        store,
        catalog,
    )
}

#[test]
fn algebraic_typed_filter_uses_structural_canonical_keys_without_row_materialization() {
    let (context, registry, relation, field, product_equivalence, _, _, store, catalog) =
        algebraic_filter_fixture();
    let filter = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: named_product(field, "ALICE"),
        equivalence: product_equivalence,
    };
    let projected = RelExpr::Project {
        input: Box::new(filter.clone()),
        columns: vec![1],
    };
    let prepared = prepare_with_catalog(projected, &context, &registry, &catalog).unwrap();
    let (projected, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        projected.rows(),
        &[vec![Value::I64(1)], vec![Value::I64(2)]]
    );
    assert_eq!(stats.typed_batch_chain_hits, 1);

    let prepared = prepare_with_catalog(filter, &context, &registry, &catalog).unwrap();
    let (filtered, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        filtered.rows(),
        &[
            vec![named_product(field, "Alice"), Value::I64(1)],
            vec![named_product(field, "ALICE"), Value::I64(2)],
        ]
    );
    assert_eq!(stats.typed_batch_chain_hits, 1);
}

#[test]
fn algebraic_structural_filter_feeds_typed_group_without_row_materialization() {
    let (
        context,
        registry,
        relation,
        field,
        product_equivalence,
        i64_equivalence,
        _,
        store,
        catalog,
    ) = algebraic_filter_fixture();
    let filtered = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: named_product(field, "alice"),
        equivalence: product_equivalence,
    };
    let grouped = RelExpr::Group {
        input: Box::new(filtered),
        group_columns: Vec::new(),
        group_equivalences: Vec::new(),
        aggregate: AggregateSpec::Count {
            result_equivalence: i64_equivalence,
        },
    };
    let prepared = prepare_with_catalog(grouped, &context, &registry, &catalog).unwrap();
    let (grouped, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(grouped.rows(), &[vec![Value::I64(2)]]);
    assert_eq!(stats.typed_batch_chain_hits, 1);
    assert_eq!(stats.typed_stateful_batch_hits, 1);
}

fn assert_physical_store_catalog_root_sharing(
    left: &PhysicalStore,
    right: &PhysicalStore,
    relations_shared: bool,
    i64_indexes_shared: bool,
) {
    assert_eq!(
        left.relations_for_test().shares_root_with(right.relations_for_test()),
        relations_shared
    );
    assert_eq!(
        left.i64_indexes_for_test().shares_root_with(right.i64_indexes_for_test()),
        i64_indexes_shared
    );
    assert!(
        left.semantic_indexes_for_test()
            .shares_root_with(right.semantic_indexes_for_test())
    );
    assert!(
        left.semantic_quotient_factors_for_test()
            .shares_root_with(right.semantic_quotient_factors_for_test())
    );
    assert!(
        left.semantic_quotient_supports_for_test()
            .shares_root_with(right.semantic_quotient_supports_for_test())
    );
    assert!(
        left.shares_semantic_statistics_root_for_test(right)
    );
    assert!(
        left.advisor_managed_artifacts_for_test()
            .shares_root_with(right.advisor_managed_artifacts_for_test())
    );
}

#[test]
fn physical_store_clone_cow_isolates_relation_mutation() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(9_991_000),
        family: LayoutFamily::Columnar,
    };
    let mut original = PhysicalStore::default();
    original
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
        )
        .unwrap();
    let index = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    original
        .install_i64_index(index, &context, &registry)
        .unwrap();

    let mut candidate = original.clone();
    assert_physical_store_catalog_root_sharing(&original, &candidate, true, true);
    let original_root = original.relations_for_test().get(&(relation, layout.id)).unwrap();
    let candidate_root = candidate.relations_for_test().get(&(relation, layout.id)).unwrap();
    assert!(Arc::ptr_eq(original_root, candidate_root));
    assert!(Arc::ptr_eq(
        original.i64_indexes_for_test().get(&index).unwrap(),
        candidate.i64_indexes_for_test().get(&index).unwrap(),
    ));

    candidate
        .apply_relation_delta_resolved(
            relation,
            layout,
            &scan_delta(relation, &[3], &[1], &context, &registry),
            &context,
            &registry,
        )
        .unwrap();

    assert_physical_store_catalog_root_sharing(&original, &candidate, false, false);

    assert!(!Arc::ptr_eq(
        original.relations_for_test().get(&(relation, layout.id)).unwrap(),
        candidate.relations_for_test().get(&(relation, layout.id)).unwrap(),
    ));
    assert!(!Arc::ptr_eq(
        original.i64_indexes_for_test().get(&index).unwrap(),
        candidate.i64_indexes_for_test().get(&index).unwrap(),
    ));
    assert!(original.i64_index(index).unwrap().probe_len_for_test(1).is_some());
    assert!(original.i64_index(index).unwrap().probe_len_for_test(3).is_none());
    assert!(candidate.i64_index(index).unwrap().probe_len_for_test(1).is_none());
    assert!(candidate.i64_index(index).unwrap().probe_len_for_test(3).is_some());

    let original_rows = original
        .installed(relation, layout)
        .unwrap()
        .scan_positions()
        .map(|position| {
            materialize_native_row(
                &original.installed(relation, layout).unwrap().data,
                position,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let candidate_rows = candidate
        .installed(relation, layout)
        .unwrap()
        .scan_positions()
        .map(|position| {
            materialize_native_row(
                &candidate.installed(relation, layout).unwrap().data,
                position,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        original_rows,
        vec![vec![Value::I64(1)], vec![Value::I64(2)]]
    );
    assert_eq!(
        candidate_rows,
        vec![vec![Value::I64(2)], vec![Value::I64(3)]]
    );
}

#[test]
fn physical_store_derived_dependency_contour_is_reused_until_topology_changes() {
    let (context, registry, relation) = planning_context();
    let layout = LayoutBinding {
        id: LayoutId(9_991_002),
        family: LayoutFamily::Columnar,
    };
    let mut original = PhysicalStore::default();
    original
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
        )
        .unwrap();
    let index = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    original
        .install_i64_index(index, &context, &registry)
        .unwrap();

    assert_eq!(
        original.derived_artifact_ids_for_test(relation, layout.id),
        vec![UnifiedArtifactId::I64Index(index)]
    );

    let mut candidate = original.clone();
    assert!(original.shares_derived_artifact_cache_for_test(&candidate));
    candidate
        .apply_relation_delta_resolved(
            relation,
            layout,
            &scan_delta(relation, &[3], &[], &context, &registry),
            &context,
            &registry,
        )
        .unwrap();
    assert!(original.shares_derived_artifact_cache_for_test(&candidate));

    let statistics = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    candidate
        .install_semantic_statistics(statistics.clone(), &context, &registry)
        .unwrap();
    assert!(!candidate.derived_artifact_cache_initialized_for_test());
    let rebuilt = candidate.derived_artifact_ids_for_test(relation, layout.id);
    assert!(!original.shares_derived_artifact_cache_for_test(&candidate));
    assert!(rebuilt.contains(&UnifiedArtifactId::I64Index(index)));
    assert!(rebuilt.contains(&UnifiedArtifactId::SemanticStatistics(statistics)));
}

#[test]
#[ignore = "diagnostic clone benchmark; run explicitly in release mode"]
fn benchmark_persistent_physical_store_clone_tax_with_and_without_index() {
    use std::hint::black_box;
    use std::time::Instant;

    let (context, registry, relation) = planning_context();
    let equivalence = sid(101);
    let layout = LayoutBinding {
        id: LayoutId(9_991_001),
        family: LayoutFamily::Columnar,
    };
    let rows = (0..200_000_i64).collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(rows.into())]).unwrap(),
        )
        .unwrap();

    let start = Instant::now();
    for _ in 0..50 {
        black_box(store.clone());
    }
    let relation_only = start.elapsed();

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
    let start = Instant::now();
    for _ in 0..50 {
        black_box(store.clone());
    }
    let indexed = start.elapsed();
    println!(
        "relation_only_clone_ns={} indexed_clone_ns={}",
        relation_only.as_nanos() / 50,
        indexed.as_nanos() / 50
    );
}

#[test]
fn persistent_physical_vec_path_copy_preserves_old_snapshot() {
    let original = PersistentPhysicalVec::from_vec((0_u64..10_000).collect());
    let mut candidate = original.clone();
    candidate.push(10_000);
    assert_eq!(original.len(), 10_000);
    assert_eq!(candidate.len(), 10_001);
    assert_eq!(original.get(9_999), Some(&9_999));
    assert_eq!(candidate.get(10_000), Some(&10_000));

    let removed = candidate.swap_remove(17);
    assert_eq!(removed, 17);
    assert_eq!(original.get(17), Some(&17));
    assert_eq!(candidate.len(), 10_000);
    assert_eq!(candidate.get(17), Some(&10_000));
}

#[test]
fn physical_store_family_directory_path_copies_only_touched_binding() {
    let mut store = PhysicalStore::default();
    for index in 0_u128..4096 {
        store.relations_mut_for_test().insert(
            (SemanticId(index), LayoutId(index)),
            Arc::new(InstalledRelation::new(
                NativeRelation::row_store(Vec::new()),
            )),
        );
    }
    let snapshot = store.clone();
    assert!(snapshot.relations_for_test().shares_root_with(store.relations_for_test()));

    let target = (SemanticId(2048), LayoutId(2048));
    let untouched = (SemanticId(17), LayoutId(17));
    let relation = store.relation_entry_mut(target).unwrap();
    Arc::make_mut(relation).data = NativeRelation::row_store(vec![vec![Value::I64(7)]]);

    assert!(!snapshot.relations_for_test().shares_root_with(store.relations_for_test()));
    assert!(Arc::ptr_eq(
        snapshot.relations_for_test().get(&untouched).unwrap(),
        store.relations_for_test().get(&untouched).unwrap()
    ));
    assert_eq!(
        native_row_count(&snapshot.relations_for_test().get(&target).unwrap().data),
        0
    );
    assert_eq!(
        native_row_count(&store.relations_for_test().get(&target).unwrap().data),
        1
    );
}

#[test]
fn columnar_relation_snapshot_mutation_path_copies_row_pages() {
    let row_count = 4096_usize;
    let value_column = PersistentPhysicalVec::from_vec(
        (0..row_count)
            .map(|index| Value::Text(format!("value-{index}")))
            .collect(),
    );
    let i64_column = PersistentPhysicalVec::from_vec(
        (0..row_count)
            .map(|index| i64::try_from(index).unwrap())
            .collect(),
    );

    let mut value_relation = NativeRelation::Columnar {
        columns: vec![value_column],
        row_count,
    };
    let value_snapshot = value_relation.clone();
    remove_native_row(&mut value_relation, 2048).unwrap();
    push_native_row(
        &mut value_relation,
        &vec![Value::Text("replacement".into())],
    )
    .unwrap();
    let (
        NativeRelation::Columnar {
            columns: value_columns,
            ..
        },
        NativeRelation::Columnar {
            columns: old_value_columns,
            ..
        },
    ) = (&value_relation, &value_snapshot)
    else {
        panic!("expected value columnar relations");
    };
    assert!(value_columns[0].shares_page_with(&old_value_columns[0], 256));
    assert!(!value_columns[0].shares_page_with(&old_value_columns[0], 2048));
    assert_eq!(old_value_columns[0][2048], Value::Text("value-2048".into()));
    assert_eq!(
        value_columns[0].last(),
        Some(&Value::Text("replacement".into()))
    );

    let mut i64_relation = NativeRelation::I64Columnar {
        columns: vec![i64_column],
        row_count,
    };
    let i64_snapshot = i64_relation.clone();
    remove_native_row(&mut i64_relation, 2048).unwrap();
    push_native_row(&mut i64_relation, &vec![Value::I64(-1)]).unwrap();
    let (
        NativeRelation::I64Columnar {
            columns: i64_columns,
            ..
        },
        NativeRelation::I64Columnar {
            columns: old_i64_columns,
            ..
        },
    ) = (&i64_relation, &i64_snapshot)
    else {
        panic!("expected i64 columnar relations");
    };
    assert!(i64_columns[0].shares_page_with(&old_i64_columns[0], 256));
    assert!(!i64_columns[0].shares_page_with(&old_i64_columns[0], 2048));
    assert_eq!(old_i64_columns[0][2048], 2048);
    assert_eq!(i64_columns[0].last(), Some(&-1));

    let typed_column = NativeColumn::I64(PersistentPhysicalVec::from_vec(
        (0..row_count)
            .map(|index| i64::try_from(index).unwrap())
            .collect(),
    ));
    let mut typed_relation = NativeRelation::TypedColumnar {
        columns: vec![typed_column],
        row_count,
    };
    let typed_snapshot = typed_relation.clone();
    remove_native_row(&mut typed_relation, 2048).unwrap();
    push_native_row(&mut typed_relation, &vec![Value::I64(-2)]).unwrap();
    let (
        NativeRelation::TypedColumnar {
            columns: typed_columns,
            ..
        },
        NativeRelation::TypedColumnar {
            columns: old_typed_columns,
            ..
        },
    ) = (&typed_relation, &typed_snapshot)
    else {
        panic!("expected typed columnar relations");
    };
    let (NativeColumn::I64(values), NativeColumn::I64(old_values)) =
        (&typed_columns[0], &old_typed_columns[0])
    else {
        panic!("expected typed i64 columns");
    };
    assert!(values.shares_page_with(old_values, 256));
    assert!(!values.shares_page_with(old_values, 2048));
    assert_eq!(old_values[2048], 2048);
    assert_eq!(values.last(), Some(&-2));
}

#[test]
fn batched_nested_algebraic_removal_rebuilds_once_and_preserves_logical_handles() {
    let ty = TypeExpr::Option(Box::new(TypeExpr::Scalar(ScalarType::I64)));
    let option_i64 = |value| Value::Option(Some(Box::new(Value::I64(value))));
    let values = (0_i64..6).map(option_i64).collect::<Vec<_>>();
    let column = NativeColumn::algebraic(&values, &ty).unwrap();
    let relation = NativeRelation::typed_columnar(vec![column]).unwrap();
    let mut installed = super::InstalledRelation::new(relation);
    assert!(installed.removal_rebuilds_data());
    let removed = [
        installed.row_id_at(1).unwrap(),
        installed.row_id_at(4).unwrap(),
    ];
    installed.remove_rows(&removed).unwrap();

    let rows = installed
        .scan_positions()
        .map(|position| materialize_native_row(&installed.data, position).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        [0_i64, 2, 3, 5]
            .into_iter()
            .map(|value| vec![option_i64(value)])
            .collect::<Vec<_>>()
    );
    assert!(installed.position(removed[0]).is_none());
    assert!(installed.position(removed[1]).is_none());

    let reused = installed.push_row(&vec![option_i64(6)]).unwrap();
    assert_eq!(reused.slot, removed[1].slot);
    assert_eq!(reused.generation, removed[1].generation + 1);
}

#[test]
#[ignore = "diagnostic persistent RowStore delta benchmark"]
fn benchmark_persistent_row_store_single_insert_scaling() {
    use std::hint::black_box;
    use std::time::Instant;

    let (context, registry, relation) = planning_context();
    for row_count in [10_000_u64, 100_000, 300_000] {
        let layout = LayoutBinding {
            id: LayoutId(9_991_100 + u128::from(row_count)),
            family: LayoutFamily::RowStore,
        };
        let rows = (0..row_count)
            .map(|value| vec![Value::I64(i64::try_from(value).unwrap())])
            .collect::<Vec<_>>();
        let mut store = PhysicalStore::default();
        store
            .install(relation, layout, NativeRelation::row_store(rows))
            .unwrap();
        let result_type = RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap();
        let delta = RelationDelta {
            inserted: vec![vec![Value::I64(i64::try_from(row_count).unwrap())]],
            removed: Vec::new(),
            result_type,
        };
        let start = Instant::now();
        store
            .apply_relation_delta(relation, layout, &delta, &context, &registry)
            .unwrap();
        black_box(&store);
        println!("rows={row_count} insert_ns={}", start.elapsed().as_nanos());

        let typed_layout = LayoutBinding {
            id: LayoutId(9_992_100 + u128::from(row_count)),
            family: LayoutFamily::Columnar,
        };
        let mut typed_store = PhysicalStore::default();
        typed_store
            .install(
                relation,
                typed_layout,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(
                    (0..row_count)
                        .map(|value| i64::try_from(value).unwrap())
                        .collect(),
                )])
                .unwrap(),
            )
            .unwrap();
        let typed_delta = RelationDelta {
            inserted: vec![vec![Value::I64(i64::try_from(row_count).unwrap())]],
            removed: Vec::new(),
            result_type: RelExpr::Scan(relation)
                .typecheck(&context, &registry)
                .unwrap(),
        };
        let start = Instant::now();
        typed_store
            .apply_relation_delta(relation, typed_layout, &typed_delta, &context, &registry)
            .unwrap();
        black_box(&typed_store);
        println!(
            "rows={row_count} typed_insert_ns={}",
            start.elapsed().as_nanos()
        );
    }
}

#[test]
fn native_columnar_filter_project_matches_logical_reference_without_full_row_materialization() {
    let (context, registry, relation) = planning_context();
    let equivalence = match context.schema.relation(relation).unwrap().semantics.clone() {
        RelationSemantics::Bag {
            column_equivalences,
        }
        | RelationSemantics::Set {
            column_equivalences,
        } => column_equivalences[0],
    };
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(2),
            equivalence,
        }),
        columns: vec![0],
    };
    let binding = LayoutBinding {
        id: LayoutId(900),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::columnar(vec![vec![
                Value::I64(1),
                Value::I64(2),
                Value::I64(2),
                Value::I64(3),
            ]])
            .unwrap(),
        )
        .unwrap();
    let (native, stats) = prepared
        .execute_native(&store, &context, &registry)
        .unwrap();

    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        vec![
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(2)],
            vec![Value::I64(3)],
        ],
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.scanned_rows, 4);
    assert_eq!(stats.values_read, 6);
    assert_eq!(stats.output_rows, 2);
}

#[test]
fn compositional_typed_batch_chain_handles_nested_filters_projects_and_bag_promotion() {
    let relation = sid(370);
    let equivalence = sid(371);
    let (context, registry) = two_i64_column_context(relation, equivalence, 41);
    let logical = RelExpr::PromoteToBag(Box::new(RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::I64(2),
                    equivalence,
                }),
                columns: vec![1, 0],
            }),
            column: 0,
            value: Value::I64(20),
            equivalence,
        }),
        columns: vec![1],
    }));
    let binding = LayoutBinding {
        id: LayoutId(950),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(vec![1, 1, 2, 2].into()),
                NativeColumn::I64(vec![10, 20, 20, 30].into()),
            ])
            .unwrap(),
        )
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        vec![(1_i64, 10_i64), (1, 20), (2, 20), (2, 30)]
            .into_iter()
            .map(|(left, right)| vec![Value::I64(left), Value::I64(right)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows(), &[vec![Value::I64(2)]]);
    assert_eq!(stats.typed_batch_chain_hits, 1);
    assert_eq!(stats.fused_join_project_hits, 0);
    assert_eq!(stats.scanned_rows, 4);
    assert_eq!(stats.values_read, 7);
}

#[test]
fn typed_batch_chain_preserves_logical_order_after_physical_swap_remove() {
    let relation = sid(374);
    let equivalence = sid(375);
    let (context, registry) = two_i64_column_context(relation, equivalence, 43);
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(2),
            equivalence,
        }),
        columns: vec![1],
    };
    let binding = LayoutBinding {
        id: LayoutId(952),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(vec![1, 2, 2, 3].into()),
                NativeColumn::I64(vec![10, 20, 30, 40].into()),
            ])
            .unwrap(),
        )
        .unwrap();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    store
        .apply_relation_delta(
            relation,
            binding,
            &RelationDelta {
                inserted: Vec::new(),
                removed: vec![vec![Value::I64(1), Value::I64(10)]],
                result_type,
            },
            &context,
            &registry,
        )
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(native.rows(), &[vec![Value::I64(20)], vec![Value::I64(30)]]);
    assert_eq!(stats.typed_batch_chain_hits, 1);

    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        vec![
            vec![Value::I64(2), Value::I64(20)],
            vec![Value::I64(2), Value::I64(30)],
            vec![Value::I64(3), Value::I64(40)],
        ],
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
}

#[test]
fn typed_i64_columnar_matches_generic_columnar_and_logical_reference() {
    let (context, registry, relation) = planning_context();
    let equivalence = sid(101);
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(2),
            equivalence,
        }),
        columns: vec![0],
    };
    let generic_binding = LayoutBinding {
        id: LayoutId(908),
        family: LayoutFamily::Columnar,
    };
    let typed_binding = LayoutBinding {
        id: LayoutId(909),
        family: LayoutFamily::Columnar,
    };
    let values = vec![1_i64, 2, 2, 3];

    let mut generic_catalog = PhysicalCatalog::default();
    generic_catalog.bind_relation(relation, generic_binding);
    let generic =
        prepare_with_catalog(logical.clone(), &context, &registry, &generic_catalog).unwrap();
    let mut generic_store = PhysicalStore::default();
    generic_store
        .install(
            relation,
            generic_binding,
            NativeRelation::columnar(vec![values.iter().copied().map(Value::I64).collect()])
                .unwrap(),
        )
        .unwrap();

    let mut typed_catalog = PhysicalCatalog::default();
    typed_catalog.bind_relation(relation, typed_binding);
    let typed = prepare_with_catalog(logical.clone(), &context, &registry, &typed_catalog).unwrap();
    let mut typed_store = PhysicalStore::default();
    typed_store
        .install(
            relation,
            typed_binding,
            NativeRelation::i64_columnar(vec![values.clone()]).unwrap(),
        )
        .unwrap();

    let (generic_value, generic_stats) = generic
        .execute_native_pinned(&generic_store, &registry)
        .unwrap();
    let (typed_value, typed_stats) = typed
        .execute_native_pinned(&typed_store, &registry)
        .unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(typed_value, reference);
    assert_eq!(generic_value, reference);
    assert_eq!(typed_stats, generic_stats);
}

#[test]
#[allow(clippy::too_many_lines)]
fn mixed_typed_columnar_covers_all_scalar_carriers_and_matches_logical_reference() {
    let relation = sid(320);
    let entity_type = sid(321);
    let equivalences = [
        sid(322),
        sid(323),
        sid(324),
        sid(325),
        sid(326),
        sid(327),
        sid(328),
    ];
    let modules = [
        EquivalenceModule::UnitExact,
        EquivalenceModule::BoolExact,
        EquivalenceModule::I64Exact,
        EquivalenceModule::F64Bitwise,
        EquivalenceModule::TextAsciiCaseInsensitive,
        EquivalenceModule::LiveEntityIdExact(entity_type),
        EquivalenceModule::HistoricalEntityIdExact(entity_type),
    ];
    let mut registry = SemanticRegistry::default();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(32));
    for (equivalence, module) in equivalences.into_iter().zip(modules) {
        environment.pin_module(equivalence, registry.install_equivalence(module));
    }
    let mut schema = Schema::new(SchemaRevisionId::new(32));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::Unit),
                TypeExpr::Scalar(ScalarType::Bool),
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::F64),
                TypeExpr::Scalar(ScalarType::Text),
                TypeExpr::Scalar(ScalarType::LiveEntityRef(entity_type)),
                TypeExpr::Scalar(ScalarType::HistoricalEntityId(entity_type)),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: equivalences.to_vec(),
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let ids = [
        kernel_types::EntityId::new(1),
        kernel_types::EntityId::new(2),
        kernel_types::EntityId::new(3),
    ];
    let text = vec!["A".to_owned(), "b".to_owned(), "a".to_owned()];
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 4,
            value: Value::Text("a".to_owned()),
            equivalence: equivalences[4],
        }),
        columns: vec![0, 1, 2, 3, 4, 5, 6],
    };
    let binding = LayoutBinding {
        id: LayoutId(932),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let typed = NativeRelation::typed_columnar(vec![
        NativeColumn::Unit(3),
        NativeColumn::Bool(vec![true, false, true].into()),
        NativeColumn::I64(vec![10, 20, 30].into()),
        NativeColumn::F64Bits(vec![1.0_f64.to_bits(), 2.0_f64.to_bits(), 3.0_f64.to_bits()].into()),
        NativeColumn::Text(text.clone().into()),
        NativeColumn::LiveEntityIds {
            entity_type,
            values: ids.to_vec().into(),
        },
        NativeColumn::HistoricalEntityIds {
            entity_type,
            values: ids.to_vec().into(),
        },
    ])
    .unwrap();
    let mut store = PhysicalStore::default();
    store.install(relation, binding, typed).unwrap();
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();

    let rows = (0..3)
        .map(|index| {
            vec![
                Value::Unit,
                Value::Bool([true, false, true][index]),
                Value::I64([10, 20, 30][index]),
                Value::F64Bits([1.0_f64, 2.0_f64, 3.0_f64][index].to_bits()),
                Value::Text(text[index].clone()),
                Value::LiveEntityRef {
                    entity_type,
                    id: ids[index],
                },
                Value::HistoricalEntityId {
                    entity_type,
                    id: ids[index],
                },
            ]
        })
        .collect();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.scanned_rows, 3);
    assert_eq!(stats.values_read, 17);
    assert_eq!(stats.output_rows, 2);

    for (key_column, equivalence) in equivalences.into_iter().enumerate() {
        let index = SemanticIndexBinding::single(relation, binding, key_column, equivalence);
        store
            .install_semantic_index(index.clone(), &context, &registry)
            .unwrap();
        assert_eq!(store.semantic_index(&index).unwrap().row_count(), 3);
    }
}

#[test]
fn mixed_typed_columnar_rejects_shape_and_schema_mismatch() {
    assert_eq!(
        NativeRelation::typed_columnar(vec![
            NativeColumn::I64(vec![1, 2].into()),
            NativeColumn::Bool(vec![true].into()),
        ]),
        Err(PhysicalExecutionError::ColumnShapeMismatch)
    );

    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(933),
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
            NativeRelation::typed_columnar(vec![NativeColumn::Bool(vec![true].into())]).unwrap(),
        )
        .unwrap();
    assert_eq!(
        prepared.execute_native_pinned(&store, &registry),
        Err(PhysicalExecutionError::PhysicalTypeMismatch)
    );
}

#[test]
fn dense_live_entity_column_round_trips_and_filters_with_revision_local_ids() {
    let entity_type = sid(340);
    let external = [
        kernel_types::EntityId::new(10),
        kernel_types::EntityId::new(20),
        kernel_types::EntityId::new(30),
    ];
    let dense =
        Arc::new(DenseEntityIds::compile(&external.into_iter().collect::<BTreeSet<_>>()).unwrap());
    let column =
        NativeColumn::dense_live_entity_ids(entity_type, Arc::clone(&dense), external.to_vec())
            .unwrap();

    assert!(matches!(
        column.value_at(1),
        Value::LiveEntityRef { entity_type: actual, id }
            if actual == entity_type && id == external[1]
    ));
    let bound = kernel_semantics::BoundPrimitivePredicate::LiveEntityId {
        entity_type,
        id: external[2],
    };
    assert!(!typed_column_matches_bound_for_test(&column, 0, &bound).unwrap());
    assert!(typed_column_matches_bound_for_test(&column, 2, &bound).unwrap());

    let selected = column.select_positions(&[2, 0]).unwrap();
    assert!(matches!(
        selected.value_at(0),
        Value::LiveEntityRef { id, .. } if id == external[2]
    ));
}

#[test]
fn artifact_memory_report_deduplicates_shared_dense_identity_backing() {
    let entity_type = sid(372);
    let external = [
        kernel_types::EntityId::new(10),
        kernel_types::EntityId::new(20),
        kernel_types::EntityId::new(30),
    ];
    let dense =
        Arc::new(DenseEntityIds::compile(&external.into_iter().collect::<BTreeSet<_>>()).unwrap());
    let first =
        NativeColumn::dense_live_entity_ids(entity_type, Arc::clone(&dense), external.to_vec())
            .unwrap();
    let second =
        NativeColumn::dense_live_entity_ids(entity_type, Arc::clone(&dense), external.to_vec())
            .unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            sid(373),
            LayoutBinding {
                id: LayoutId(952),
                family: LayoutFamily::Columnar,
            },
            NativeRelation::typed_columnar(vec![first, second]).unwrap(),
        )
        .unwrap();
    let memory = store.artifact_memory_report();
    assert_eq!(
        memory
            .families
            .get(&PhysicalArtifactFamily::SharedDenseIdentityMap)
            .map(|entry| entry.artifacts),
        Some(1)
    );
    assert_eq!(
        memory
            .families
            .get(&PhysicalArtifactFamily::RelationLayout)
            .map(|entry| entry.artifacts),
        Some(1)
    );
}

fn named_product(field: SemanticId, name: &str) -> Value {
    Value::Product(BTreeMap::from([(field, Value::Text(name.to_owned()))]))
}

fn algebraic_dense_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    SemanticId,
    SemanticId,
    SemanticId,
    [kernel_types::EntityId; 3],
    LayoutBinding,
    PhysicalStore,
) {
    let relation = sid(9_990_200);
    let field = sid(9_990_201);
    let product_equivalence = sid(9_990_202);
    let text_equivalence = sid(9_990_203);
    let live_equivalence = sid(9_990_204);
    let entity_type = sid(9_990_205);
    let mut registry = SemanticRegistry::default();
    let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let live_digest =
        registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(entity_type));
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9_990_200));
    environment.pin_module(text_equivalence, text_digest);
    environment.pin_module(live_equivalence, live_digest);
    let product_type = TypeExpr::Product(BTreeMap::from([(
        field,
        TypeExpr::Scalar(ScalarType::Text),
    )]));
    let mut schema = Schema::new(SchemaRevisionId::new(9_990_200));
    schema
        .define_structural_equivalence(
            product_equivalence,
            StructuralEquivalenceDef::Product {
                fields: BTreeMap::from([(field, text_equivalence)]),
            },
        )
        .unwrap();
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                product_type.clone(),
                TypeExpr::Scalar(ScalarType::LiveEntityRef(entity_type)),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![product_equivalence, live_equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let external = [
        kernel_types::EntityId::new(101),
        kernel_types::EntityId::new(202),
        kernel_types::EntityId::new(303),
    ];
    let dense =
        Arc::new(DenseEntityIds::compile(&external.into_iter().collect::<BTreeSet<_>>()).unwrap());
    let algebraic = NativeColumn::algebraic(
        &[
            named_product(field, "Alice"),
            named_product(field, "ALICE"),
            named_product(field, "Bob"),
        ],
        &product_type,
    )
    .unwrap();
    let dense_refs =
        NativeColumn::dense_live_entity_ids(entity_type, Arc::clone(&dense), external.to_vec())
            .unwrap();
    let layout = LayoutBinding {
        id: LayoutId(9_990_200),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![algebraic, dense_refs]).unwrap(),
        )
        .unwrap();
    (
        context,
        registry,
        relation,
        field,
        product_equivalence,
        entity_type,
        external,
        layout,
        store,
    )
}

#[test]
fn algebraic_structural_filter_composes_with_dense_live_entity_projection() {
    let (
        context,
        registry,
        relation,
        field,
        product_equivalence,
        entity_type,
        external,
        layout,
        mut store,
    ) = algebraic_dense_fixture();
    let filter = |store: &PhysicalStore, name: &str| {
        let mut stats = ExecutionStats::default();
        let rows = execute_fused_filter_project_scan_for_test(
            store,
            relation,
            layout,
            0,
            &named_product(field, name),
            product_equivalence,
            &[1],
            &context,
            &registry,
            &mut stats,
        )
        .unwrap();
        assert_eq!(stats.scanned_rows, 3);
        rows
    };
    assert_eq!(
        filter(&store, "aLiCe"),
        vec![
            vec![Value::LiveEntityRef {
                entity_type,
                id: external[0],
            }],
            vec![Value::LiveEntityRef {
                entity_type,
                id: external[1],
            }],
        ]
    );
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    store
        .apply_relation_delta(
            relation,
            layout,
            &RelationDelta {
                inserted: vec![vec![
                    named_product(field, "alice"),
                    Value::LiveEntityRef {
                        entity_type,
                        id: external[0],
                    },
                ]],
                removed: vec![vec![
                    named_product(field, "Alice"),
                    Value::LiveEntityRef {
                        entity_type,
                        id: external[0],
                    },
                ]],
                result_type,
            },
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(
        filter(&store, "ALICE"),
        vec![
            vec![Value::LiveEntityRef {
                entity_type,
                id: external[1],
            }],
            vec![Value::LiveEntityRef {
                entity_type,
                id: external[0],
            }],
        ]
    );
}

#[test]
fn mixed_typed_predicate_kernels_cover_every_scalar_carrier() {
    let entity_type = sid(350);
    let ids = [
        kernel_types::EntityId::new(1),
        kernel_types::EntityId::new(2),
        kernel_types::EntityId::new(3),
    ];
    let columns = vec![
        NativeColumn::Unit(3),
        NativeColumn::Bool(vec![false, true, false].into()),
        NativeColumn::I64(vec![1, 2, 3].into()),
        NativeColumn::F64Bits(vec![1.0_f64.to_bits(), 2.0_f64.to_bits(), 3.0_f64.to_bits()].into()),
        NativeColumn::Text(vec!["a".into(), "B".into(), "c".into()].into()),
        NativeColumn::LiveEntityIds {
            entity_type,
            values: ids.to_vec().into(),
        },
        NativeColumn::HistoricalEntityIds {
            entity_type,
            values: ids.to_vec().into(),
        },
    ];
    let cases = vec![
        (0, kernel_semantics::BoundPrimitivePredicate::Unit, 3),
        (1, kernel_semantics::BoundPrimitivePredicate::Bool(true), 1),
        (2, kernel_semantics::BoundPrimitivePredicate::I64(2), 1),
        (
            3,
            kernel_semantics::BoundPrimitivePredicate::F64Bits(2.0_f64.to_bits()),
            1,
        ),
        (
            4,
            kernel_semantics::BoundPrimitivePredicate::TextExact("B".into()),
            1,
        ),
        (
            4,
            kernel_semantics::BoundPrimitivePredicate::TextAsciiCaseInsensitive("b".into()),
            1,
        ),
        (
            5,
            kernel_semantics::BoundPrimitivePredicate::LiveEntityId {
                entity_type,
                id: ids[1],
            },
            1,
        ),
        (
            6,
            kernel_semantics::BoundPrimitivePredicate::HistoricalEntityId {
                entity_type,
                id: ids[1],
            },
            1,
        ),
    ];
    for (predicate_column, bound, expected_rows) in cases {
        let rows =
            execute_bound_typed_filter_for_test(&columns, &columns[predicate_column], &bound, &[2], 3)
                .unwrap();
        assert_eq!(rows.len(), expected_rows);
        if expected_rows == 1 {
            assert_eq!(rows, vec![vec![Value::I64(2)]]);
        }
    }
}

#[test]
fn native_row_store_filter_project_matches_logical_reference() {
    let (context, registry, relation) = planning_context();
    let equivalence = match context.schema.relation(relation).unwrap().semantics.clone() {
        RelationSemantics::Bag {
            column_equivalences,
        }
        | RelationSemantics::Set {
            column_equivalences,
        } => column_equivalences[0],
    };
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(2),
            equivalence,
        }),
        columns: vec![0],
    };
    let binding = LayoutBinding {
        id: LayoutId(901),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let rows = vec![
        vec![Value::I64(1)],
        vec![Value::I64(2)],
        vec![Value::I64(2)],
        vec![Value::I64(3)],
    ];
    let mut store = PhysicalStore::default();
    store
        .install(relation, binding, NativeRelation::row_store(rows.clone()))
        .unwrap();
    let (native, stats) = prepared
        .execute_native(&store, &context, &registry)
        .unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.scanned_rows, 4);
    assert_eq!(stats.output_rows, 2);
}

#[test]
fn native_distinct_matches_logical_reference() {
    let (context, registry, relation) = planning_context();
    let equivalence = sid(101);
    let logical = RelExpr::Distinct {
        input: Box::new(RelExpr::Scan(relation)),
        column_equivalences: vec![equivalence],
    };
    let binding = LayoutBinding {
        id: LayoutId(904),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let rows = vec![
        vec![Value::I64(1)],
        vec![Value::I64(1)],
        vec![Value::I64(2)],
    ];
    let mut store = PhysicalStore::default();
    store
        .install(relation, binding, NativeRelation::row_store(rows.clone()))
        .unwrap();
    let (native, _) = prepared
        .execute_native(&store, &context, &registry)
        .unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
}

#[test]
fn native_nested_loop_join_matches_logical_reference() {
    let (context, registry, relation) = planning_context();
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let binding = LayoutBinding {
        id: LayoutId(905),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let rows = vec![
        vec![Value::I64(1)],
        vec![Value::I64(1)],
        vec![Value::I64(2)],
    ];
    let mut store = PhysicalStore::default();
    store
        .install(relation, binding, NativeRelation::row_store(rows.clone()))
        .unwrap();
    let (native, _) = prepared
        .execute_native(&store, &context, &registry)
        .unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
}

#[test]
fn native_bag_difference_uses_gamma_monus_and_matches_logical_reference() {
    let (context, registry, left, right, _equivalence) = two_text_relation_context(false);
    let logical = RelExpr::Difference {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
    };
    let left_layout = LayoutBinding {
        id: LayoutId(9_980_110),
        family: LayoutFamily::RowStore,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(9_980_111),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left, left_layout);
    catalog.bind_relation(right, right_layout);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    assert!(matches!(prepared.physical(), Plan::Difference { .. }));

    let left_rows = vec![
        vec![Value::Text("A".into())],
        vec![Value::Text("A".into())],
        vec![Value::Text("A".into())],
        vec![Value::Text("B".into())],
    ];
    let right_rows = vec![
        vec![Value::Text("a".into())],
        vec![Value::Text("a".into())],
        vec![Value::Text("C".into())],
    ];
    let mut store = PhysicalStore::default();
    store
        .install(
            left,
            left_layout,
            NativeRelation::row_store(left_rows.clone()),
        )
        .unwrap();
    store
        .install(
            right,
            right_layout,
            NativeRelation::row_store(right_rows.clone()),
        )
        .unwrap();

    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(left, left_rows);
    model.relations.insert(right, right_rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(
        native,
        RelationValue::Bag(vec![
            vec![Value::Text("A".into())],
            vec![Value::Text("B".into())],
        ])
    );
}

#[test]
fn native_set_difference_removes_gamma_equivalent_support() {
    let (context, registry, left, right, _equivalence) = two_text_relation_context(true);
    let logical = RelExpr::Difference {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
    };
    let left_layout = LayoutBinding {
        id: LayoutId(9_980_120),
        family: LayoutFamily::RowStore,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(9_980_121),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left, left_layout);
    catalog.bind_relation(right, right_layout);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            left,
            left_layout,
            NativeRelation::row_store(vec![
                vec![Value::Text("A".into())],
                vec![Value::Text("B".into())],
            ]),
        )
        .unwrap();
    store
        .install(
            right,
            right_layout,
            NativeRelation::row_store(vec![vec![Value::Text("a".into())]]),
        )
        .unwrap();
    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(native.rows(), &[vec![Value::Text("B".into())]]);
}

#[test]
fn native_anti_join_uses_zero_cross_blocker_and_preserves_left_multiplicity() {
    let (context, registry, left, right, equivalence) = two_text_relation_context(false);
    let logical = RelExpr::AntiJoin {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let left_layout = LayoutBinding {
        id: LayoutId(9_980_130),
        family: LayoutFamily::RowStore,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(9_980_131),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left, left_layout);
    catalog.bind_relation(right, right_layout);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    assert!(matches!(prepared.physical(), Plan::AntiJoin { .. }));
    let left_rows = vec![
        vec![Value::Text("A".into())],
        vec![Value::Text("A".into())],
        vec![Value::Text("B".into())],
        vec![Value::Text("C".into())],
        vec![Value::Text("C".into())],
    ];
    let right_rows = vec![
        vec![Value::Text("a".into())],
        vec![Value::Text("a".into())],
        vec![Value::Text("b".into())],
    ];
    let mut store = PhysicalStore::default();
    store
        .install(
            left,
            left_layout,
            NativeRelation::row_store(left_rows.clone()),
        )
        .unwrap();
    store
        .install(
            right,
            right_layout,
            NativeRelation::row_store(right_rows.clone()),
        )
        .unwrap();
    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(left, left_rows);
    model.relations.insert(right, right_rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(
        native,
        RelationValue::Bag(vec![
            vec![Value::Text("C".into())],
            vec![Value::Text("C".into())],
        ])
    );
}

#[test]
fn typed_filter_columns_stays_native_through_project_boundary() {
    let relation = sid(9_980_150);
    let equivalence = sid(9_980_151);
    let (context, registry) = two_i64_column_context(relation, equivalence, 9_980_150);
    let binding = LayoutBinding {
        id: LayoutId(9_980_150),
        family: LayoutFamily::Columnar,
    };
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqColumns {
            input: Box::new(RelExpr::Scan(relation)),
            left_column: 0,
            right_column: 1,
            equivalence,
        }),
        columns: vec![1],
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let left = vec![1_i64, 2, 3, 4];
    let right = vec![1_i64, 9, 3, 8];
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(left.clone().into()),
                NativeColumn::I64(right.clone().into()),
            ])
            .unwrap(),
        )
        .unwrap();
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        left.into_iter()
            .zip(right)
            .map(|(left, right)| vec![Value::I64(left), Value::I64(right)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(
        native,
        RelationValue::Bag(vec![vec![Value::I64(1)], vec![Value::I64(3)]])
    );
    assert_eq!(stats.typed_stateful_producer_hits, 1);
    assert_eq!(stats.typed_batch_chain_hits, 1);
}

#[test]
fn typed_set_difference_uses_native_support_subtraction() {
    let (context, registry, left, right, _equivalence) = two_text_relation_context(true);
    let left_layout = LayoutBinding {
        id: LayoutId(9_980_142),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(9_980_143),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left, left_layout);
    catalog.bind_relation(right, right_layout);
    let mut store = PhysicalStore::default();
    store
        .install(
            left,
            left_layout,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(
                vec!["A".into(), "B".into()].into(),
            )])
            .unwrap(),
        )
        .unwrap();
    store
        .install(
            right,
            right_layout,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(vec!["a".into()].into())])
                .unwrap(),
        )
        .unwrap();
    let prepared = prepare_with_catalog(
        RelExpr::Difference {
            left: Box::new(RelExpr::Scan(left)),
            right: Box::new(RelExpr::Scan(right)),
        },
        &context,
        &registry,
        &catalog,
    )
    .unwrap();
    let (value, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(value.rows(), &[vec![Value::Text("B".into())]]);
    assert_eq!(stats.typed_stateful_producer_hits, 1);
    assert_eq!(stats.typed_batch_chain_hits, 2);
}

#[test]
fn typed_difference_and_anti_join_use_native_stateful_producers() {
    let (context, registry, left, right, equivalence) = two_text_relation_context(false);
    let left_layout = LayoutBinding {
        id: LayoutId(9_980_140),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(9_980_141),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(left, left_layout);
    catalog.bind_relation(right, right_layout);
    let left_values = vec!["A".into(), "A".into(), "B".into(), "C".into()];
    let right_values = vec!["a".into(), "b".into()];
    let mut store = PhysicalStore::default();
    store
        .install(
            left,
            left_layout,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(left_values.into())]).unwrap(),
        )
        .unwrap();
    store
        .install(
            right,
            right_layout,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(right_values.into())]).unwrap(),
        )
        .unwrap();

    let difference = RelExpr::Difference {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
    };
    let prepared_difference =
        prepare_with_catalog(difference, &context, &registry, &catalog).unwrap();
    let (difference_value, difference_stats) = prepared_difference
        .execute_native_pinned(&store, &registry)
        .unwrap();
    assert_eq!(
        difference_value,
        RelationValue::Bag(vec![
            vec![Value::Text("A".into())],
            vec![Value::Text("C".into())],
        ])
    );
    assert_eq!(difference_stats.typed_stateful_producer_hits, 1);
    assert_eq!(difference_stats.typed_batch_chain_hits, 2);

    let anti_join = RelExpr::AntiJoin {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared_anti_join =
        prepare_with_catalog(anti_join, &context, &registry, &catalog).unwrap();
    let (anti_join_value, anti_join_stats) = prepared_anti_join
        .execute_native_pinned(&store, &registry)
        .unwrap();
    assert_eq!(
        anti_join_value,
        RelationValue::Bag(vec![vec![Value::Text("C".into())]])
    );
    assert_eq!(anti_join_stats.typed_stateful_producer_hits, 1);
    assert_eq!(anti_join_stats.typed_batch_chain_hits, 2);
}

#[test]
fn structural_top_k_with_ties_uses_semantic_order_class_not_physical_tie_break() {
    let relation = sid(9_980_200);
    let field = sid(9_980_201);
    let text_eq = sid(9_980_202);
    let text_order = sid(9_980_203);
    let product_eq = sid(9_980_204);
    let product_order = sid(9_980_205);
    let mut registry = SemanticRegistry::default();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9_980_200));
    environment.pin_module(
        text_eq,
        registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive),
    );
    environment.pin_module(
        text_order,
        registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive),
    );
    let product_type = TypeExpr::Product(BTreeMap::from([(
        field,
        TypeExpr::Scalar(ScalarType::Text),
    )]));
    let mut schema = Schema::new(SchemaRevisionId::new(9_980_200));
    schema
        .define_structural_equivalence(
            product_eq,
            StructuralEquivalenceDef::Product {
                fields: BTreeMap::from([(field, text_eq)]),
            },
        )
        .unwrap();
    schema
        .define_structural_ordering(
            product_order,
            StructuralOrderingDef::Product {
                fields: vec![(field, text_order)],
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
    let logical = RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        ordering: product_order,
        direction: OrderDirection::Ascending,
        k: 1,
    };
    let layout = LayoutBinding {
        id: LayoutId(9_980_200),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let rows = vec![
        vec![named_product(field, "a")],
        vec![named_product(field, "A")],
        vec![named_product(field, "b")],
    ];
    let mut store = PhysicalStore::default();
    store
        .install(relation, layout, NativeRelation::row_store(rows.clone()))
        .unwrap();
    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 2);
}

#[test]
fn columnar_join_selects_indexed_i64_path_and_preserves_bag_multiplicity() {
    let (context, registry, relation) = planning_context();
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let binding = LayoutBinding {
        id: LayoutId(934),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    assert!(matches!(prepared.physical(), Plan::JoinEq { .. }));
    let mut values = (1_i64..=64).collect::<Vec<_>>();
    values.insert(1, 1);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(values.clone().into())]).unwrap(),
        )
        .unwrap();
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.scanned_rows, 130);
    assert_eq!(stats.output_rows, 67);
    assert_eq!(stats.ephemeral_index_builds, 1);
}

#[test]
fn identity_project_over_scan_preserves_indexed_join_access_path() {
    let (context, registry, relation) = planning_context();
    let identity = || RelExpr::Project {
        input: Box::new(RelExpr::Scan(relation)),
        columns: vec![0],
    };
    let logical = RelExpr::JoinEq {
        left: Box::new(identity()),
        right: Box::new(identity()),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let binding = LayoutBinding {
        id: LayoutId(1934),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    assert!(matches!(prepared.physical(), Plan::JoinEq { .. }));

    let mut values = (1_i64..=64).collect::<Vec<_>>();
    values.insert(1, 1);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(values.clone().into())]).unwrap(),
        )
        .unwrap();
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.ephemeral_index_builds, 1);
    assert_eq!(stats.output_rows, 67);
}

#[test]
fn identity_project_join_is_not_hidden_from_join_batch_fusion() {
    let (context, registry, relation) = planning_context();
    let identity = || RelExpr::Project {
        input: Box::new(RelExpr::Scan(relation)),
        columns: vec![0],
    };
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(identity()),
            right: Box::new(identity()),
            left_column: 0,
            right_column: 0,
            equivalence: sid(101),
        }),
        columns: vec![0],
    };
    let layout = LayoutBinding {
        id: LayoutId(1_935),
        family: LayoutFamily::Columnar,
    };
    let index = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let values = (1_i64..=64).collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(values.clone().into())]).unwrap(),
        )
        .unwrap();
    store.install_i64_index(index, &context, &registry).unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    assert_eq!(
        native,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.typed_batch_chain_hits, 1);
    assert_eq!(stats.fused_join_project_hits, 1);
}

#[test]
fn identity_projects_do_not_hide_prepared_multiway_quotient_program() {
    let equivalence = sid(7_491);
    let (context, registry, relations) = n_way_i64_context(3, 7_492, equivalence);
    let bindings = [7_492_u128, 7_493, 7_494].map(|id| LayoutBinding {
        id: LayoutId(id),
        family: LayoutFamily::Columnar,
    });
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let projected = relations
        .iter()
        .copied()
        .map(|relation| RelExpr::Project {
            input: Box::new(RelExpr::Scan(relation)),
            columns: vec![0],
        })
        .collect::<Vec<_>>();
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::JoinEq {
            left: Box::new(projected[0].clone()),
            right: Box::new(projected[1].clone()),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        right: Box::new(projected[2].clone()),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    assert!(prepared.semantic_quotient_program.is_some());

    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for (relation, binding) in relations.iter().copied().zip(bindings) {
        let values = vec![1_i64, 2, 3];
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
    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        native,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
}

#[test]
fn indexed_i64_join_project_fuses_without_materializing_full_join_rows() {
    let (context, registry, relation) = planning_context();
    let join = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let logical = RelExpr::Project {
        input: Box::new(join),
        columns: vec![0],
    };
    let binding = LayoutBinding {
        id: LayoutId(942),
        family: LayoutFamily::Columnar,
    };
    let index_binding = I64IndexBinding {
        relation,
        layout: binding,
        key_column: 0,
        equivalence: sid(101),
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut values = (1_i64..=64).collect::<Vec<_>>();
    values.insert(1, 1);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(values.clone().into())]).unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(index_binding, &context, &registry)
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::I64(value)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.ephemeral_index_builds, 0);
    assert_eq!(stats.fused_join_project_hits, 1);
    assert_eq!(stats.output_rows, 67);
}

#[test]
fn typed_batch_dag_keeps_join_filter_project_unmaterialized_until_output() {
    let relation = sid(372);
    let equivalence = sid(373);
    let (context, registry) = two_i64_column_context(relation, equivalence, 42);
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::JoinEq {
                left: Box::new(RelExpr::Scan(relation)),
                right: Box::new(RelExpr::Scan(relation)),
                left_column: 0,
                right_column: 0,
                equivalence,
            }),
            column: 3,
            value: Value::I64(20),
            equivalence,
        }),
        columns: vec![1, 2],
    };
    let binding = LayoutBinding {
        id: LayoutId(951),
        family: LayoutFamily::Columnar,
    };
    let index_binding = I64IndexBinding {
        relation,
        layout: binding,
        key_column: 0,
        equivalence,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut source_rows = vec![(1_i64, 10_i64), (1, 20), (2, 20)];
    source_rows.extend((3_i64..64).map(|key| (key, 1_000 + key)));
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(source_rows.iter().map(|row| row.0).collect()),
                NativeColumn::I64(source_rows.iter().map(|row| row.1).collect()),
            ])
            .unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(index_binding, &context, &registry)
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        source_rows
            .iter()
            .copied()
            .map(|(key, payload)| vec![Value::I64(key), Value::I64(payload)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(
        native.rows(),
        &[
            vec![Value::I64(10), Value::I64(1)],
            vec![Value::I64(20), Value::I64(1)],
            vec![Value::I64(20), Value::I64(2)],
        ]
    );
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.ephemeral_index_builds, 0);
    assert_eq!(stats.typed_batch_chain_hits, 1);
    assert_eq!(stats.fused_join_project_hits, 1);
}

