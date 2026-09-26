// HOSTILE[P174][TEST-LOCAL][CLEAN]: quotient/cyclic owner tests live with multiway internals,
// so their helper vocabulary does not inflate the root parent surface.
use super::*;
use crate::semantic_quotient_physical::test_support::{
    dense_projections_for_test, full_bfc_compiles_for_test, reset_dense_projections_for_test,
    support_atom_for_test, support_bfc_work_for_test, support_binding_for_test,
    support_contains_stable_key_for_test, support_cow_sharing_for_test,
    support_group_atoms_share_root_for_test, support_group_contains_key_for_test,
    support_rows_match_dense_for_test, support_stable_row_for_test,
};
use crate::storage_impl::test_support::PhysicalStoreTestExt as _;
use crate::{
    LayoutFamily, LayoutId, NativeColumn, NativeRelation, PhysicalCatalog, PreparedPlan,
    UnifiedArtifactId, prepare_with_catalog,
};
use kernel_model::Value;
use kernel_query::{RelExpr, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId};

fn sid(value: u64) -> SemanticId {
    SemanticId::new(u128::from(value))
}

fn only_semantic_quotient_support_for_test(
    store: &PhysicalStore,
) -> &Arc<MaterializedSemanticQuotientSupportState> {
    store
        .semantic_quotient_supports_for_test()
        .values()
        .next()
        .unwrap()
}

#[test]
fn gamma_quotient_specs_use_certified_refinement_closure() {
    let exact = sid(720);
    let ascii_ci = sid(721);
    let mut registry = SemanticRegistry::default();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(72));
    environment.pin_module(exact, exact_digest);
    environment.pin_module(ascii_ci, ci_digest);
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(72)),
        environment,
    };
    let a = MultiwayJoinColumnRef { leaf: 0, column: 0 };
    let b = MultiwayJoinColumnRef { leaf: 1, column: 0 };
    let c = MultiwayJoinColumnRef { leaf: 2, column: 0 };
    let predicates = [
        MultiwayJoinPredicate {
            left: a,
            right: b,
            equivalence: exact,
        },
        MultiwayJoinPredicate {
            left: b,
            right: c,
            equivalence: ascii_ci,
        },
    ];
    let specs = semantic_quotient_specs(&predicates, &context, &registry).unwrap();
    assert!(specs.contains(&(exact, vec![a, b])));
    assert!(specs.contains(&(ascii_ci, vec![a, b, c])));
    assert_eq!(specs.len(), 2);
}

#[test]
fn gamma_quotient_basis_changes_with_pinned_semantic_contract() {
    let first = sid(738);
    let second = sid(739);
    let mut registry = SemanticRegistry::default();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let context = |second_digest| {
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(76));
        environment.pin_module(first, exact_digest);
        environment.pin_module(second, second_digest);
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(76)),
            environment,
        }
    };
    let a = MultiwayJoinColumnRef { leaf: 0, column: 0 };
    let b = MultiwayJoinColumnRef { leaf: 1, column: 0 };
    let c = MultiwayJoinColumnRef { leaf: 2, column: 0 };
    let predicates = [
        MultiwayJoinPredicate {
            left: a,
            right: b,
            equivalence: first,
        },
        MultiwayJoinPredicate {
            left: b,
            right: c,
            equivalence: second,
        },
    ];
    let mixed = semantic_quotient_specs(&predicates, &context(ci_digest), &registry).unwrap();
    assert_eq!(mixed.len(), 2);
    let both_exact =
        semantic_quotient_specs(&predicates, &context(exact_digest), &registry).unwrap();
    assert_eq!(both_exact, vec![(first, vec![a, b, c])]);
}

#[test]
fn gamma_quotient_specs_drop_same_component_coarser_duplicate() {
    let exact = sid(735);
    let ascii_ci = sid(736);
    let mut registry = SemanticRegistry::default();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(75));
    environment.pin_module(exact, exact_digest);
    environment.pin_module(ascii_ci, ci_digest);
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(75)),
        environment,
    };
    let left = MultiwayJoinColumnRef { leaf: 0, column: 0 };
    let right = MultiwayJoinColumnRef { leaf: 1, column: 0 };
    let predicates = [
        MultiwayJoinPredicate {
            left,
            right,
            equivalence: ascii_ci,
        },
        MultiwayJoinPredicate {
            left,
            right,
            equivalence: exact,
        },
    ];
    let specs = semantic_quotient_specs(&predicates, &context, &registry).unwrap();
    assert_eq!(specs, vec![(exact, vec![left, right])]);
}

#[test]
fn gamma_quotient_specs_collapse_redundant_equality_clique() {
    let equivalence = sid(722);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(73));
    environment.pin_module(equivalence, digest);
    let context = SemanticContext {
        schema: Schema::new(SchemaRevisionId::new(73)),
        environment,
    };
    let vertices = (0..4)
        .map(|leaf| MultiwayJoinColumnRef { leaf, column: 0 })
        .collect::<Vec<_>>();
    let mut predicates = Vec::new();
    for left in 0..vertices.len() {
        for right in (left + 1)..vertices.len() {
            predicates.push(MultiwayJoinPredicate {
                left: vertices[left],
                right: vertices[right],
                equivalence,
            });
        }
    }
    assert_eq!(predicates.len(), 6);
    let specs = semantic_quotient_specs(&predicates, &context, &registry).unwrap();
    assert_eq!(specs, vec![(equivalence, vertices)]);
}

#[test]
fn quotient_hypergraph_gyo_accepts_chain_and_rejects_cycle() {
    let endpoint = |leaf| MultiwayJoinColumnRef { leaf, column: 0 };
    let chain = vec![
        (sid(750), vec![endpoint(0), endpoint(1)]),
        (sid(751), vec![endpoint(1), endpoint(2)]),
        (sid(752), vec![endpoint(2), endpoint(3)]),
    ];
    let order = quotient_hypergraph_search_order(4, &chain).unwrap();
    assert_eq!(order.len(), 4);
    assert_eq!(
        order.iter().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from([0, 1, 2, 3])
    );

    let cycle = vec![
        (sid(753), vec![endpoint(0), endpoint(1)]),
        (sid(754), vec![endpoint(1), endpoint(2)]),
        (sid(755), vec![endpoint(2), endpoint(0)]),
    ];
    assert!(quotient_hypergraph_search_order(3, &cycle).is_none());

    let nine_cycle = (0..9)
        .map(|leaf| {
            (
                sid(1_700 + leaf as u64),
                vec![endpoint(leaf), endpoint((leaf + 1) % 9)],
            )
        })
        .collect::<Vec<_>>();
    assert!(quotient_hypergraph_search_order(9, &nine_cycle).is_none());
    let cyclic_order = quotient_hypergraph_cyclic_search_order(9, &nine_cycle).unwrap();
    assert_eq!(cyclic_order.len(), 9);
    assert_eq!(
        cyclic_order.iter().copied().collect::<BTreeSet<_>>(),
        (0..9).collect::<BTreeSet<_>>()
    );
}

#[test]
fn quotient_hypergraph_large_generated_topology_stays_exact() {
    const LEAVES: usize = 256;
    let nested = (2..=LEAVES)
        .map(|size| (0..size).collect::<BTreeSet<_>>())
        .collect::<Vec<_>>();
    let maximal = quotient_maximal_hyperedges(nested, LEAVES);
    assert_eq!(maximal, vec![(0..LEAVES).collect::<BTreeSet<_>>()]);

    let endpoint = |leaf| MultiwayJoinColumnRef { leaf, column: 0 };
    let cycle = (0..LEAVES)
        .map(|leaf| {
            (
                sid(20_000 + leaf as u64),
                vec![endpoint(leaf), endpoint((leaf + 1) % LEAVES)],
            )
        })
        .collect::<Vec<_>>();
    assert!(quotient_hypergraph_search_order(LEAVES, &cycle).is_none());
    let order = quotient_hypergraph_cyclic_search_order(LEAVES, &cycle).unwrap();
    assert_eq!(order.len(), LEAVES);
    assert_eq!(
        order.into_iter().collect::<BTreeSet<_>>(),
        (0..LEAVES).collect::<BTreeSet<_>>()
    );
}

#[test]
fn bounded_cyclic_work_certificate_uses_sparse_masks_and_rejects_bad_order() {
    let row_counts = vec![200_000_usize; 10];
    let masks = row_counts
        .iter()
        .map(|rows| {
            let mut mask = vec![0_u64; rows.div_ceil(64)];
            mask[0] = 1;
            mask
        })
        .collect::<Vec<_>>();
    let order = (0..10).collect::<Vec<_>>();
    let mask_refs = masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let work =
        quotient_enumeration_work_upper_bound(&[], &mask_refs, &row_counts, &order, None).unwrap();
    assert!(work < MAX_BOUNDED_CYCLIC_ENUMERATION_WORK);

    let dense_masks = (0..10).map(|_| vec![u64::MAX; 4]).collect::<Vec<_>>();
    let dense_rows = vec![256_usize; 10];
    let dense_mask_refs = dense_masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let dense_work =
        quotient_enumeration_work_upper_bound(&[], &dense_mask_refs, &dense_rows, &order, None)
            .unwrap();
    assert!(dense_work > MAX_BOUNDED_CYCLIC_ENUMERATION_WORK);

    let malformed_masks = (0..10).map(|_| vec![1_u64]).collect::<Vec<_>>();
    let malformed_mask_refs = malformed_masks
        .iter()
        .map(Vec::as_slice)
        .collect::<Vec<_>>();
    assert!(
            quotient_enumeration_work_upper_bound(
                &[],
                &malformed_mask_refs,
                &row_counts,
                &order,
                None,
            )
            .is_none()
        );

    let mut invalid = order;
    invalid[9] = 0;
    assert!(
        quotient_enumeration_work_upper_bound(&[], &mask_refs, &row_counts, &invalid, None)
            .is_none()
    );
}

#[test]
fn bounded_cyclic_prefix_index_tightens_joint_constraint_domain() {
    let (constraints, masks, rows, order) = cyclic_prefix_fixture_for_test();
    let constraint_refs = constraints.iter().collect::<Vec<_>>();
    let mask_refs = masks.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let indexes = cyclic_prefix_candidate_indexes(&constraint_refs, &mask_refs, &order).unwrap();
    let leaf_two = indexes.by_leaf[2].as_ref().unwrap();
    assert_eq!(leaf_two.constraint_indices, vec![0, 1]);
    assert_eq!(leaf_two.max_bucket_len, 1);
    assert_eq!(leaf_two.buckets.len(), 4_096);

    let coarse =
        quotient_enumeration_work_upper_bound(&constraint_refs, &mask_refs, &rows, &order, None)
            .unwrap();
    let joint = quotient_enumeration_work_upper_bound(
        &constraint_refs,
        &mask_refs,
        &rows,
        &order,
        Some(&indexes),
    )
    .unwrap();
    assert!(joint < coarse, "joint={joint}, coarse={coarse}");
}

struct CyclicTextFixture {
    context: SemanticContext,
    registry: SemanticRegistry,
    relations: Vec<SemanticId>,
    bindings: Vec<LayoutBinding>,
    logical: RelExpr,
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
        logical,
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

fn install_sparse_cyclic_text_rows(
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
        let mut first = Vec::with_capacity(rows_per_leaf);
        let mut second = Vec::with_capacity(rows_per_leaf);
        first.push("A".to_owned());
        second.push("A".to_owned());
        for ordinal in 1..rows_per_leaf {
            let noise = format!("noise-{leaf}-{ordinal}");
            first.push(noise.clone());
            second.push(noise);
        }
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
fn quotient_materialization_invalidates_warmed_derived_dependency_cache() {
    let fixture = ten_way_non_gyo_text_cycle_fixture();
    let (mut store, _) = install_cyclic_text_rows(&fixture, 2);
    let relation = fixture.relations[0];
    let layout = fixture.bindings[0];

    let before = store.derived_artifact_ids_for_test(relation, layout.id);
    assert!(store.derived_artifact_cache_initialized_for_test());
    assert!(before.iter().all(|id| !matches!(
        id,
        UnifiedArtifactId::SemanticQuotientFactor(_)
            | UnifiedArtifactId::SemanticQuotientSupport(_)
    )));

    assert!(
        fixture
            .prepared
            .materialize_semantic_quotient_support(&mut store, &fixture.registry)
            .unwrap()
    );

    let after = store.derived_artifact_ids_for_test(relation, layout.id);
    assert!(
        after
            .iter()
            .any(|id| matches!(id, UnifiedArtifactId::SemanticQuotientFactor(_)))
    );
    assert!(
        after
            .iter()
            .any(|id| matches!(id, UnifiedArtifactId::SemanticQuotientSupport(_)))
    );
}

#[test]
fn ten_way_non_gyo_cycle_uses_bounded_cyclic_quotient_search() {
    let fixture = ten_way_non_gyo_text_cycle_fixture();
    let program = fixture.prepared.semantic_quotient_program.as_ref().unwrap();
    assert!(quotient_hypergraph_search_order(fixture.relations.len(), &program.specs).is_none());
    assert_eq!(
        program.search_certificate,
        Some(SemanticQuotientSearchCertificate::BoundedCyclic)
    );
    assert_eq!(
        program.hypergraph_order.as_ref().map(Vec::len),
        Some(fixture.relations.len())
    );
    let (store, mut model) = install_cyclic_text_rows(&fixture, 2);
    let program = fixture.prepared.semantic_quotient_program.as_ref().unwrap();
    let (native, stats) = fixture
        .prepared
        .physical()
        .execute_native_with_prepared_programs(
            &store,
            fixture.prepared.result_type(),
            fixture.prepared.semantic_context(),
            &fixture.registry,
            None,
            Some(program),
        )
        .unwrap();
    let reference = fixture
        .logical
        .evaluate(&model, &fixture.context, &fixture.registry)
        .unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 2);
    assert_eq!(stats.multiway_join_order_preserving_enumerations, 1);
    assert_eq!(stats.multiway_join_prepared_quotient_hits, 1);
    assert!(stats.multiway_join_cyclic_prefix_index_lookups > 0);

    let mut cached_store = store.clone();
    assert!(
        fixture
            .prepared
            .materialize_semantic_quotient_support(&mut cached_store, &fixture.registry)
            .unwrap()
    );
    let (cached, cached_stats) = fixture
        .prepared
        .execute_native_pinned(&cached_store, &fixture.registry)
        .unwrap();
    assert_eq!(cached, reference);
    assert_eq!(
        cached_stats.multiway_join_maintained_quotient_support_hits,
        1
    );

    let target_relation = *fixture.relations.last().unwrap();
    let target_layout = *fixture.bindings.last().unwrap();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&fixture.context, &fixture.registry)
        .unwrap();
    cached_store
        .apply_relation_delta(
            target_relation,
            target_layout,
            &RelationDelta {
                inserted: vec![vec![Value::Text("C".into()), Value::Text("C".into())]],
                removed: vec![vec![Value::Text("B".into()), Value::Text("B".into())]],
                result_type,
            },
            &fixture.context,
            &fixture.registry,
        )
        .unwrap();
    model.relations.insert(
        target_relation,
        vec![
            vec![Value::Text("A".into()), Value::Text("A".into())],
            vec![Value::Text("C".into()), Value::Text("C".into())],
        ],
    );
    let (after, after_stats) = fixture
        .prepared
        .execute_native_pinned(&cached_store, &fixture.registry)
        .unwrap();
    let after_reference = fixture
        .logical
        .evaluate(&model, &fixture.context, &fixture.registry)
        .unwrap();
    assert_eq!(after, after_reference);
    assert_eq!(after.rows().len(), 1);
    assert_eq!(
        after_stats.multiway_join_maintained_quotient_support_hits,
        1
    );
}

#[test]
fn sparse_non_gyo_cycle_visits_only_supported_ordinals() {
    let fixture = ten_way_non_gyo_text_cycle_fixture();
    let rows_per_leaf = 128;
    let (store, model) = install_sparse_cyclic_text_rows(&fixture, rows_per_leaf);
    let program = fixture.prepared.semantic_quotient_program.as_ref().unwrap();
    let (native, stats) = fixture
        .prepared
        .physical()
        .execute_native_with_prepared_programs(
            &store,
            fixture.prepared.result_type(),
            fixture.prepared.semantic_context(),
            &fixture.registry,
            None,
            Some(program),
        )
        .unwrap();
    let reference = fixture
        .logical
        .evaluate(&model, &fixture.context, &fixture.registry)
        .unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 1);
    assert_eq!(stats.scanned_rows, rows_per_leaf * fixture.relations.len());
    assert_eq!(stats.multiway_join_semantic_quotient_candidate_visits, 10);
    assert!(stats.multiway_join_semantic_quotient_candidate_visits * 100 < stats.scanned_rows);
}

#[test]
fn ten_way_non_gyo_cycle_rejects_uncertified_large_enumeration_before_dfs() {
    let fixture = ten_way_non_gyo_text_cycle_fixture();
    let program = fixture.prepared.semantic_quotient_program.as_ref().unwrap();
    let (store, _) = install_cyclic_text_rows(&fixture, 4);
    let mut leaves = Vec::new();
    let mut predicates = Vec::new();
    flatten_multiway_join_tree(
        fixture.prepared.physical(),
        &store,
        &mut leaves,
        &mut predicates,
    )
    .unwrap()
    .unwrap();
    let mut stats = ExecutionStats::default();
    let rows = execute_order_preserving_quotient_join(
        QuotientJoinExecutionRequest {
            leaves: &leaves,
            predicates: &predicates,
            search_order: program.hypergraph_order.as_ref().unwrap(),
            prepared_program: Some(program),
            store: &store,
            context: &fixture.context,
            registry: &fixture.registry,
        },
        &mut stats,
    )
    .unwrap();
    assert!(rows.is_none());
    assert_eq!(stats.multiway_join_order_preserving_enumerations, 0);
    assert_eq!(stats.multiway_join_cyclic_budget_rejections, 1);
}

// HOSTILE[P175][TEST-LOCAL][CLEAN]: local QCN delta/churn tests live with the multiway owner;
// root tests no longer force quotient state internals across the parent boundary.
fn three_relation_i64_context() -> (
    SemanticContext,
    SemanticRegistry,
    [SemanticId; 3],
    SemanticId,
) {
    let relations = [sid(700), sid(701), sid(702)];
    let equivalence = sid(703);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(700));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(700));
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
    (
        SemanticContext {
            schema,
            environment,
        },
        registry,
        relations,
        equivalence,
    )
}

fn scan_delta(
    relation: SemanticId,
    inserted: &[i64],
    removed: &[i64],
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> RelationDelta {
    RelationDelta {
        inserted: inserted
            .iter()
            .copied()
            .map(|value| vec![Value::I64(value)])
            .collect(),
        removed: removed
            .iter()
            .copied()
            .map(|value| vec![Value::I64(value)])
            .collect(),
        result_type: RelExpr::Scan(relation)
            .typecheck(context, registry)
            .unwrap(),
    }
}

fn local_dq_quotient_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    [SemanticId; 3],
    [LayoutBinding; 3],
    RelExpr,
    PreparedPlan,
    PhysicalStore,
    kernel_model::FiniteModel,
) {
    let (context, registry, relations @ [a, b, c], equivalence) = three_relation_i64_context();
    let bindings = [1_093_u128, 1_094, 1_095].map(|id| LayoutBinding {
        id: LayoutId(id),
        family: LayoutFamily::Columnar,
    });
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.into_iter().zip(bindings) {
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
    assert!(
        prepared
            .materialize_semantic_quotient_support(&mut store, &registry)
            .unwrap()
    );
    (
        context, registry, relations, bindings, logical, prepared, store, model,
    )
}

#[test]
fn gamma_quotient_support_uses_local_dq_for_arbitrary_deletions() {
    let (context, registry, [a, b, c], bindings, logical, prepared, mut store, mut model) =
        local_dq_quotient_fixture();
    let local_before = store.semantic_quotient_support_local_delta_updates();
    let handles_before = store.logical_row_handles(c, bindings[2]).unwrap();
    let suffix_delete = scan_delta(c, &[], &[20], &context, &registry);
    store
        .apply_relation_delta(c, bindings[2], &suffix_delete, &context, &registry)
        .unwrap();
    model.relations.get_mut(&c).unwrap().pop();
    let handles_after = store.logical_row_handles(c, bindings[2]).unwrap();
    assert_eq!(handles_after, handles_before[..handles_after.len()]);
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 1
    );
    let maintained = only_semantic_quotient_support_for_test(&store);
    let current_handles = [a, b, c]
        .into_iter()
        .zip(bindings)
        .map(|(relation, layout)| store.logical_row_handles(relation, layout).unwrap())
        .collect::<Vec<_>>();
    assert!(support_rows_match_dense_for_test(
        maintained,
        &current_handles
    ));
    let (after_suffix, suffix_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        after_suffix,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(
        suffix_stats.multiway_join_maintained_quotient_support_hits,
        1
    );

    let interior_delete = scan_delta(c, &[], &[10], &context, &registry);
    store
        .apply_relation_delta(c, bindings[2], &interior_delete, &context, &registry)
        .unwrap();
    model
        .relations
        .get_mut(&c)
        .unwrap()
        .retain(|row| row != &vec![Value::I64(10)]);
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 2
    );
    let (after_interior, interior_stats) =
        prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        interior_stats.multiway_join_maintained_quotient_support_hits,
        1
    );
    assert_eq!(
        after_interior,
        logical.evaluate(&model, &context, &registry).unwrap()
    );

    let duplicate_delete = scan_delta(b, &[], &[1], &context, &registry);
    store
        .apply_relation_delta(b, bindings[1], &duplicate_delete, &context, &registry)
        .unwrap();
    model.relations.get_mut(&b).unwrap().remove(0);
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 3
    );
    let (after_duplicate, duplicate_stats) =
        prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        after_duplicate,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(
        duplicate_stats.multiway_join_maintained_quotient_support_hits,
        1
    );

    let insertion = scan_delta(c, &[21], &[], &context, &registry);
    store
        .apply_relation_delta(c, bindings[2], &insertion, &context, &registry)
        .unwrap();
    model
        .relations
        .get_mut(&c)
        .unwrap()
        .push(vec![Value::I64(21)]);
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 4
    );
    let (after_insert, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(
        after_insert,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
}

#[test]
fn gamma_quotient_dense_projection_is_read_boundary_only() {
    let (context, registry, [_a, _b, c], bindings, _logical, prepared, mut store, _model) =
        local_dq_quotient_fixture();
    reset_dense_projections_for_test();

    let delta = scan_delta(c, &[], &[20], &context, &registry);
    store
        .apply_relation_delta(c, bindings[2], &delta, &context, &registry)
        .unwrap();
    assert_eq!(
        dense_projections_for_test(),
        0,
        "QCN writes must not materialize dense handles/keys/buckets"
    );

    let _ = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert!(
        dense_projections_for_test() > 0,
        "dense quotient projection belongs to the executor read boundary"
    );
}

#[test]
fn gamma_quotient_support_cow_shares_heavy_state_for_delete_only_delta() {
    let (context, registry, [_a, _b, c], bindings, _logical, _prepared, store, _model) =
        local_dq_quotient_fixture();
    let source = store.clone();
    let old_state = only_semantic_quotient_support_for_test(&source);
    let mut candidate = store;
    let suffix_delete = scan_delta(c, &[], &[20], &context, &registry);
    candidate
        .apply_relation_delta(c, bindings[2], &suffix_delete, &context, &registry)
        .unwrap();
    let new_state = only_semantic_quotient_support_for_test(&candidate);

    let sharing = support_cow_sharing_for_test(old_state, new_state);
    assert!(sharing.group_atoms);
    assert_eq!(sharing.atom_maps, vec![true, true, false]);
    assert_eq!(sharing.base_masks, vec![true, true, false]);
    assert_eq!(sharing.stable_rows, vec![true, true, false]);

    let (old_rows, _, _) = support_stable_row_for_test(old_state, 2);
    let (new_rows, new_storage_len, new_dense) = support_stable_row_for_test(new_state, 2);
    assert_eq!(new_rows, old_rows - 1);
    assert_eq!(new_storage_len, old_rows);
    assert_eq!(
        new_dense,
        candidate.logical_row_handles(c, bindings[2]).unwrap()
    );
    assert_eq!(
        source.logical_row_handles(c, bindings[2]).unwrap().len(),
        old_rows
    );
}

#[test]
fn gamma_quotient_novel_key_path_copies_semantic_directories() {
    let (context, registry, [a, _b, _c], bindings, _logical, _prepared, mut store, _model) =
        local_dq_quotient_fixture();
    let source = store.clone();
    let old_state = only_semantic_quotient_support_for_test(&source);
    let novel = kernel_semantics::CanonicalEqKey::I64(21);
    assert!(!support_contains_stable_key_for_test(old_state, &novel));
    assert!(!support_group_contains_key_for_test(old_state, &novel));

    let delta = scan_delta(a, &[21], &[], &context, &registry);
    store
        .apply_relation_delta(a, bindings[0], &delta, &context, &registry)
        .unwrap();
    let new_state = only_semantic_quotient_support_for_test(&store);

    assert!(support_contains_stable_key_for_test(new_state, &novel));
    assert!(support_group_contains_key_for_test(new_state, &novel));
    assert!(!support_group_atoms_share_root_for_test(
        old_state, new_state
    ));
    assert!(!support_contains_stable_key_for_test(old_state, &novel));
    assert!(
        !support_group_contains_key_for_test(old_state, &novel),
        "reader snapshot must retain the pre-insert semantic directories"
    );
}

#[test]
fn gamma_quotient_local_insertion_resurrects_greatest_fixed_point() {
    let (context, registry, [a, _b, _c], bindings, logical, prepared, mut store, mut model) =
        local_dq_quotient_fixture();
    let full_compile_before = full_bfc_compiles_for_test();
    let local_before = store.semantic_quotient_support_local_delta_updates();
    let old_row_handle = store
        .logical_rows_with_handles(a, bindings[0])
        .unwrap()
        .into_iter()
        .find_map(|(handle, row)| (row == vec![Value::I64(1)]).then_some(handle))
        .unwrap();
    let old_atom = support_atom_for_test(
        only_semantic_quotient_support_for_test(&store),
        0,
        old_row_handle,
    )
    .unwrap();

    let delete_support = scan_delta(a, &[], &[1], &context, &registry);
    store
        .apply_relation_delta(a, bindings[0], &delete_support, &context, &registry)
        .unwrap();
    model
        .relations
        .get_mut(&a)
        .unwrap()
        .retain(|row| row != &vec![Value::I64(1)]);
    let (without_support, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(without_support.rows().len(), 0);
    assert_eq!(
        without_support,
        logical.evaluate(&model, &context, &registry).unwrap()
    );

    let restore_support = scan_delta(a, &[1], &[], &context, &registry);
    store
        .apply_relation_delta(a, bindings[0], &restore_support, &context, &registry)
        .unwrap();
    model
        .relations
        .get_mut(&a)
        .unwrap()
        .push(vec![Value::I64(1)]);
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 2
    );

    let maintained = only_semantic_quotient_support_for_test(&store);
    let new_row_handle = store
        .logical_rows_with_handles(a, bindings[0])
        .unwrap()
        .into_iter()
        .find_map(|(handle, row)| (row == vec![Value::I64(1)]).then_some(handle))
        .unwrap();
    assert_eq!(new_row_handle.slot, old_row_handle.slot);
    assert!(new_row_handle.generation > old_row_handle.generation);
    let new_atom = support_atom_for_test(maintained, 0, new_row_handle).unwrap();
    assert_ne!(new_atom, old_atom);
    assert_eq!(support_atom_for_test(maintained, 0, old_row_handle), None);
    let (affected_atoms, atom_count) = support_bfc_work_for_test(maintained);
    assert!(affected_atoms > 0);
    assert!(
        affected_atoms < atom_count,
        "resurrection should repair only the selected death-witness cone"
    );
    assert_eq!(
        full_bfc_compiles_for_test(),
        full_compile_before,
        "local QCN delete+reinsert must not recompile the full grounded support program"
    );
    let rebuilt = build_semantic_quotient_support_state(
        support_binding_for_test(maintained),
        &store,
        &context,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert_eq!(maintained.as_ref(), &rebuilt);

    let (restored, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(restored.rows().len(), 100);
    assert_eq!(
        restored,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(stats.multiway_join_maintained_quotient_support_hits, 1);
}

#[test]
fn gamma_quotient_mixed_delta_refreshes_only_affected_component() {
    let (context, registry, [a, _b, _c], bindings, logical, prepared, mut store, mut model) =
        local_dq_quotient_fixture();
    let local_before = store.semantic_quotient_support_local_delta_updates();

    let remove_support_add_dead = scan_delta(a, &[21], &[1], &context, &registry);
    store
        .apply_relation_delta(
            a,
            bindings[0],
            &remove_support_add_dead,
            &context,
            &registry,
        )
        .unwrap();
    let rows = model.relations.get_mut(&a).unwrap();
    rows.retain(|row| row != &vec![Value::I64(1)]);
    rows.push(vec![Value::I64(21)]);
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 1
    );
    let maintained = only_semantic_quotient_support_for_test(&store);
    let rebuilt = build_semantic_quotient_support_state(
        support_binding_for_test(maintained),
        &store,
        &context,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert_eq!(maintained.as_ref(), &rebuilt);
    let (without_support, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(without_support.rows().len(), 0);
    assert_eq!(
        without_support,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(stats.multiway_join_maintained_quotient_support_hits, 1);

    let restore_support_remove_dead = scan_delta(a, &[1], &[21], &context, &registry);
    store
        .apply_relation_delta(
            a,
            bindings[0],
            &restore_support_remove_dead,
            &context,
            &registry,
        )
        .unwrap();
    let rows = model.relations.get_mut(&a).unwrap();
    rows.retain(|row| row != &vec![Value::I64(21)]);
    rows.push(vec![Value::I64(1)]);
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 2
    );
    let maintained = only_semantic_quotient_support_for_test(&store);
    let rebuilt = build_semantic_quotient_support_state(
        support_binding_for_test(maintained),
        &store,
        &context,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert_eq!(maintained.as_ref(), &rebuilt);
    let (restored, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(restored.rows().len(), 100);
    assert_eq!(
        restored,
        logical.evaluate(&model, &context, &registry).unwrap()
    );
    assert_eq!(stats.multiway_join_maintained_quotient_support_hits, 1);
}

#[test]
fn gamma_quotient_revision_batch_coalesces_support_refresh() {
    let (context, registry, [a, _b, c], bindings, _logical, _prepared, mut store, _model) =
        local_dq_quotient_fixture();
    let local_before = store.semantic_quotient_support_local_delta_updates();

    let a_delta = scan_delta(a, &[21], &[1], &context, &registry);
    let (_, a_physical) = store
        .apply_relation_delta_resolved_in_place_deferred_support(
            a,
            bindings[0],
            &a_delta,
            &context,
            &registry,
        )
        .unwrap();
    let c_delta = scan_delta(c, &[22], &[20], &context, &registry);
    let (_, c_physical) = store
        .apply_relation_delta_resolved_in_place_deferred_support(
            c,
            bindings[2],
            &c_delta,
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before
    );

    store
        .maintain_semantic_quotient_supports_for_changes(
            &[(a, bindings[0], &a_physical), (c, bindings[2], &c_physical)],
            &context,
            &registry,
        )
        .unwrap();
    assert_eq!(
        store.semantic_quotient_support_local_delta_updates(),
        local_before + 1
    );
    let maintained = only_semantic_quotient_support_for_test(&store);
    let rebuilt = build_semantic_quotient_support_state(
        support_binding_for_test(maintained),
        &store,
        &context,
        &registry,
    )
    .unwrap()
    .unwrap();
    assert_eq!(maintained.as_ref(), &rebuilt);
}
