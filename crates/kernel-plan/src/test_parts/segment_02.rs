#[test]
fn persisted_i64_index_is_reused_and_maintained_by_relation_delta() {
    let (context, registry, relation) = planning_context();
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let binding = LayoutBinding {
        id: LayoutId(936),
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
            NativeRelation::typed_columnar(vec![NativeColumn::I64((1_i64..=64).collect())])
                .unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(
            I64IndexBinding {
                relation,
                layout: binding,
                key_column: 0,
                equivalence: sid(101),
            },
            &context,
            &registry,
        )
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(native.rows().len(), 64);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.ephemeral_index_builds, 0);

    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let delta = RelationDelta {
        inserted: vec![vec![Value::I64(65)]],
        removed: vec![vec![Value::I64(1)]],
        result_type,
    };
    store
        .apply_relation_delta(relation, binding, &delta, &context, &registry)
        .unwrap();
    let index = store
        .i64_index(I64IndexBinding {
            relation,
            layout: binding,
            key_column: 0,
            equivalence: sid(101),
        })
        .unwrap();
    assert_eq!(index.row_count(), 64);
    assert!(index.probe_len_for_test(1).is_none());
    assert_eq!(index.probe_len_for_test(2).unwrap(), 1);
    assert_eq!(index.probe_len_for_test(65).unwrap(), 1);
}

#[test]
fn storage_resolved_delta_updates_recursive_scan_without_semantic_relookup() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(960),
        family: LayoutFamily::Columnar,
    };
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        vec![
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(3)],
        ],
    );
    let query = RelExpr::Scan(relation);
    let mut maintained =
        kernel_query::MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();

    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2, 3].into())]).unwrap(),
        )
        .unwrap();
    let rows = store.logical_rows_with_handles(relation, binding).unwrap();
    maintained.attach_storage_rows(relation, &rows).unwrap();

    let result_type = query.typecheck(&context, &registry).unwrap();
    let delta = RelationDelta {
        inserted: vec![vec![Value::I64(4)]],
        removed: vec![vec![Value::I64(1)]],
        result_type,
    };
    let resolved = store
        .apply_relation_delta_resolved(relation, binding, &delta, &context, &registry)
        .unwrap();
    let mut resolved_map = BTreeMap::new();
    resolved_map.insert(relation, resolved.clone());
    let emitted = maintained
        .apply_storage_resolved_deltas(&resolved_map, &context, &registry)
        .unwrap();
    assert_eq!(emitted, delta);

    let output = maintained.output_value(&context, &registry).unwrap();
    let mut values = output
        .rows()
        .iter()
        .map(|row| match row.as_slice() {
            [Value::I64(value)] => *value,
            _ => unreachable!(),
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(values, vec![2, 3, 4]);

    let before = maintained.clone();
    assert_eq!(
        maintained.apply_storage_resolved_deltas(&resolved_map, &context, &registry),
        Err(RelQueryError::InconsistentIncrementalDelta)
    );
    assert_eq!(maintained, before);
}

fn test_materialization_id() -> MaterializationId {
    MaterializationId::new(1)
}

fn target_revision_for(
    runtime: &RuntimeRevisionBundle,
    target_revision: u64,
    mutations: &[RevisionRelationMutation<'_>],
    registry: &SemanticRegistry,
) -> kernel_revision::Revision {
    let mut state = runtime.revision().state().clone();
    for mutation in mutations {
        let relation = RelExpr::Scan(mutation.relation);
        let old = relation
            .evaluate(
                &state.model,
                runtime.revision().semantic_context(),
                registry,
            )
            .unwrap();
        let next = mutation
            .delta
            .apply_to_value(old, runtime.revision().semantic_context(), registry)
            .unwrap();
        state
            .model
            .relations
            .insert(mutation.relation, next.into_rows());
    }
    kernel_revision::Revision::build(
        RevisionId::new(target_revision),
        runtime.revision().semantic_context(),
        registry,
        state,
    )
    .unwrap()
}

fn revision_with_same_state(
    runtime: &RuntimeRevisionBundle,
    target_revision: u64,
    registry: &SemanticRegistry,
) -> kernel_revision::Revision {
    kernel_revision::Revision::build(
        RevisionId::new(target_revision),
        runtime.revision().semantic_context(),
        registry,
        runtime.revision().state().clone(),
    )
    .unwrap()
}

fn prepare_runtime_revision(
    runtime: &RuntimeRevisionBundle,
    target_revision: u64,
    mutations: &[RevisionRelationMutation<'_>],
    registry: &SemanticRegistry,
) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
    let target = target_revision_for(runtime, target_revision, mutations, registry);
    runtime.prepare_revision_for_test(&RevisionTransitionRequest {
        target_revision: &target,
        mutations,
        registry,
    })
}

fn prepare_runtime_cell_revision(
    runtime: &RuntimeRevisionCell,
    target_revision: u64,
    mutations: &[RevisionRelationMutation<'_>],
    registry: &SemanticRegistry,
) -> Result<PreparedRuntimeRevisionTransition, PhysicalExecutionError> {
    let snapshot = runtime.snapshot()?;
    let target = target_revision_for(snapshot.root(), target_revision, mutations, registry);
    runtime.prepare_revision_for_test(&RevisionTransitionRequest {
        target_revision: &target,
        mutations,
        registry,
    })
}

fn scan_runtime_bundle(
    revision: u64,
    layout_id: u64,
    values: &[i64],
) -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    LayoutBinding,
    RuntimeRevisionBundle,
) {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(u128::from(layout_id)),
        family: LayoutFamily::Columnar,
    };
    let rows = values
        .iter()
        .copied()
        .map(|value| vec![Value::I64(value)])
        .collect::<Vec<_>>();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows);
    let query = RelExpr::Scan(relation);
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(values.to_vec().into())])
                .unwrap(),
        )
        .unwrap();
    let revision = kernel_revision::Revision::build(
        RevisionId::new(revision),
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
            query,
        }],
        &registry,
    )
    .unwrap();
    (context, registry, relation, binding, runtime)
}

#[test]
fn runtime_bundle_clone_shares_large_persistent_directories() {
    let (_context, _registry, relation, binding, mut runtime) =
        scan_runtime_bundle(8_970_001, 8_970_002, &[1, 2, 3]);
    for offset in 1_u64..=4_096 {
        runtime.insert_relation_layout_for_test(
            SemanticId::new(9_000_000_u128 + u128::from(offset)),
            LayoutBinding {
                id: LayoutId(9_100_000_u128 + u128::from(offset)),
                family: LayoutFamily::Columnar,
            },
        );
    }

    let snapshot = runtime.clone_for_test();
    assert!(
        runtime.relation_layouts_share_root_with_for_test(&snapshot)
    );
    assert!(
        runtime.materialization_specs_share_root_with_for_test(&snapshot)
    );
    assert!(
        runtime.materializations_share_root_with_for_test(&snapshot)
    );

    runtime.insert_relation_layout_for_test(
        relation,
        LayoutBinding {
            id: LayoutId(binding.id.0 + 1),
            family: binding.family,
        },
    );
    assert!(
        !runtime.relation_layouts_share_root_with_for_test(&snapshot)
    );
    assert_eq!(snapshot.relation_layout(relation), Some(binding));
    assert!(
        runtime.materialization_specs_share_root_with_for_test(&snapshot)
    );
    assert!(
        runtime.materializations_share_root_with_for_test(&snapshot)
    );
}

fn two_independent_materialization_runtime() -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    SemanticId,
    MaterializationId,
    MaterializationId,
    RuntimeRevisionBundle,
) {
    let left = sid(8_972_001);
    let right = sid(8_972_002);
    let equivalence = sid(8_972_003);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(8_972_001));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(8_972_001));
    for relation in [left, right] {
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
    let left_rows = vec![vec![Value::I64(1)], vec![Value::I64(2)]];
    let right_rows = vec![vec![Value::I64(10)], vec![Value::I64(20)]];
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(left, left_rows.clone());
    model.relations.insert(right, right_rows.clone());
    let revision = kernel_revision::Revision::build(
        RevisionId::new(8_972_010),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let left_binding = LayoutBinding {
        id: LayoutId(8_972_011),
        family: LayoutFamily::RowStore,
    };
    let right_binding = LayoutBinding {
        id: LayoutId(8_972_012),
        family: LayoutFamily::RowStore,
    };
    let mut store = PhysicalStore::default();
    store
        .install(left, left_binding, NativeRelation::row_store(left_rows))
        .unwrap();
    store
        .install(right, right_binding, NativeRelation::row_store(right_rows))
        .unwrap();
    let left_id = MaterializationId::new(8_972_101);
    let right_id = MaterializationId::new(8_972_102);
    let runtime = RuntimeRevisionBundle::build(
        revision,
        store,
        BTreeMap::from([(left, left_binding), (right, right_binding)]),
        &[
            RuntimeMaterializationSpec {
                id: left_id,
                query: RelExpr::Scan(left),
            },
            RuntimeMaterializationSpec {
                id: right_id,
                query: RelExpr::Scan(right),
            },
        ],
        &registry,
    )
    .unwrap();
    (context, registry, left, right, left_id, right_id, runtime)
}

#[test]
fn runtime_relation_transition_updates_only_dependency_consumers() {
    let (context, registry, left, right, left_id, right_id, runtime) =
        two_independent_materialization_runtime();
    assert_eq!(runtime.materialization(left_id).unwrap().revision(), None);
    assert_eq!(runtime.materialization(right_id).unwrap().revision(), None);
    assert!(runtime.materialization_dependency_contains_for_test(left, left_id));
    assert!(!runtime.materialization_dependency_contains_for_test(left, right_id));
    assert!(runtime.materialization_dependency_contains_for_test(right, right_id));
    let right_before = runtime.materialization(right_id).unwrap().clone();
    let right_epoch = right_before.transition_epoch();

    let delta = scan_delta(left, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation: left,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 8_972_020, &mutations, &registry);
    let prepared = runtime
        .prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &target,
            mutations: &mutations,
            registry: &registry,
        })
        .unwrap();

    assert_eq!(prepared.output_deltas().len(), 1);
    assert_eq!(prepared.output_deltas()[&left_id], delta);
    assert!(!prepared.output_deltas().contains_key(&right_id));
    assert_eq!(
        prepared.candidate_materialization_for_test(right_id).unwrap(),
        &right_before
    );
    assert_eq!(
        prepared
            .candidate_materialization_for_test(right_id)
            .unwrap()
            .transition_epoch(),
        right_epoch
    );
    assert_eq!(
        prepared.candidate_materialization_revision_for_test(right_id),
        Some(RevisionId::new(8_972_020))
    );
}

#[test]
fn runtime_relation_transition_reuses_unchanged_bundle_directories() {
    let (context, registry, relation, _binding, runtime) =
        scan_runtime_bundle(8_971_001, 8_971_002, &[1, 2, 3]);
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let target = target_revision_for(
        &runtime,
        8_971_003,
        &[RevisionRelationMutation {
            relation,
            delta: &delta,
        }],
        &registry,
    );
    let prepared = runtime
        .prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &target,
            mutations: &[RevisionRelationMutation {
                relation,
                delta: &delta,
            }],
            registry: &registry,
        })
        .unwrap();

    let (layouts_shared, specs_shared, materializations_shared) =
        prepared.candidate_directories_share_with_for_test(&runtime);
    assert!(layouts_shared);
    assert!(specs_shared);
    assert!(!materializations_shared);
}

fn two_text_recovery_bundle(
    revision_id: u64,
    short_values: &[&str],
    long_values: &[&str],
) -> (
    SemanticContext,
    SemanticRegistry,
    SemanticIndexBinding,
    SemanticIndexBinding,
    RuntimeRevisionBundle,
) {
    assert_eq!(short_values.len(), long_values.len());
    let relation = sid(9_930_000 + revision_id);
    let equivalence = sid(9_940_000 + revision_id);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision_id));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(revision_id));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::Text),
                TypeExpr::Scalar(ScalarType::Text),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence, equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let rows = short_values
        .iter()
        .zip(long_values)
        .map(|(short, long)| vec![Value::Text((*short).into()), Value::Text((*long).into())])
        .collect::<Vec<_>>();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows.clone());
    let revision = kernel_revision::Revision::build(
        RevisionId::new(revision_id),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let layout = LayoutBinding {
        id: LayoutId(u128::from(9_950_000 + revision_id)),
        family: LayoutFamily::Columnar,
    };
    let mut physical = PhysicalStore::default();
    physical
        .install(
            relation,
            layout,
            NativeRelation::typed_from_rows(
                &rows,
                &[
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
            )
            .unwrap(),
        )
        .unwrap();
    let short = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    let long = SemanticIndexBinding::single(relation, layout, 1, equivalence);
    physical
        .install_semantic_index(short.clone(), &context, &registry)
        .unwrap();
    physical
        .install_semantic_index(long.clone(), &context, &registry)
        .unwrap();
    physical
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::SemanticIndex(short.clone()));
    physical
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::SemanticIndex(long.clone()));
    let root = RuntimeRevisionBundle::build(
        revision,
        physical,
        BTreeMap::from([(relation, layout)]),
        &[],
        &registry,
    )
    .unwrap();
    (context, registry, short, long, root)
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

fn relation_cube_certificate(
    base: &RelationValue,
    final_endpoint: &RelationValue,
) -> kernel_change::RewriteResidualCubeCertificate<RelationValue, Value> {
    fn rewrite(
        spec: u128,
        endpoint: &RelationValue,
    ) -> kernel_change::PreparedRewrite<RelationValue, Value> {
        kernel_change::PreparedRewrite {
            spec: RewriteSpecId(SemanticId::new(spec)),
            explicit_inputs: Vec::new(),
            effect: kernel_change::RewriteEffect::Replace(endpoint.clone()),
            law_set: RewriteLawSetId(SemanticId::new(90_000 + spec)),
        }
    }

    fn register_pair(
        registry: &mut kernel_change::RewriteResidualFamilyRegistry,
        id: u128,
        left: &kernel_change::PreparedRewrite<RelationValue, Value>,
        right: &kernel_change::PreparedRewrite<RelationValue, Value>,
        right_after_left: &kernel_change::PreparedRewrite<RelationValue, Value>,
        left_after_right: &kernel_change::PreparedRewrite<RelationValue, Value>,
    ) {
        registry
            .register(kernel_change::RewriteResidualFamilySpec {
                id: kernel_change::RewriteResidualFamilyId(SemanticId::new(id)),
                key: kernel_change::RewriteResidualFamilyKey {
                    left: left.into(),
                    right: right.into(),
                },
                right_after_left: right_after_left.into(),
                left_after_right: left_after_right.into(),
            })
            .unwrap();
    }

    let a = rewrite(90_101, final_endpoint);
    let b = rewrite(90_102, final_endpoint);
    let c = rewrite(90_103, final_endpoint);
    let witness = kernel_change::RewriteResidualCubeWitness {
        b_after_a: rewrite(90_111, final_endpoint),
        a_after_b: rewrite(90_112, final_endpoint),
        c_after_a: rewrite(90_113, final_endpoint),
        a_after_c: rewrite(90_114, final_endpoint),
        c_after_b: rewrite(90_115, final_endpoint),
        b_after_c: rewrite(90_116, final_endpoint),
        c_after_ab: rewrite(90_121, final_endpoint),
        b_after_ac: rewrite(90_122, final_endpoint),
        c_after_ba: rewrite(90_121, final_endpoint),
        a_after_bc: rewrite(90_123, final_endpoint),
        b_after_ca: rewrite(90_124, final_endpoint),
        a_after_cb: rewrite(90_125, final_endpoint),
    };
    let mut registry = kernel_change::RewriteResidualFamilyRegistry::default();
    register_pair(
        &mut registry,
        90_201,
        &a,
        &b,
        &witness.b_after_a,
        &witness.a_after_b,
    );
    register_pair(
        &mut registry,
        90_202,
        &a,
        &c,
        &witness.c_after_a,
        &witness.a_after_c,
    );
    register_pair(
        &mut registry,
        90_203,
        &b,
        &c,
        &witness.c_after_b,
        &witness.b_after_c,
    );
    register_pair(
        &mut registry,
        90_204,
        &witness.b_after_a,
        &witness.c_after_a,
        &witness.c_after_ab,
        &witness.b_after_ac,
    );
    register_pair(
        &mut registry,
        90_205,
        &witness.a_after_b,
        &witness.c_after_b,
        &witness.c_after_ba,
        &witness.a_after_bc,
    );
    register_pair(
        &mut registry,
        90_206,
        &witness.a_after_c,
        &witness.b_after_c,
        &witness.b_after_ca,
        &witness.a_after_cb,
    );
    registry.certify_cube(base, &a, &b, &c, witness).unwrap()
}

fn relation_residual_chain_certificate(
    base: &RelationValue,
    final_endpoint: &RelationValue,
) -> kernel_change::RevisionEffectResidualChainCertificate<RelationValue, Value> {
    fn rewrite(
        spec: u128,
        endpoint: &RelationValue,
    ) -> kernel_change::PreparedRewrite<RelationValue, Value> {
        kernel_change::PreparedRewrite {
            spec: RewriteSpecId(SemanticId::new(spec)),
            explicit_inputs: Vec::new(),
            effect: kernel_change::RewriteEffect::Replace(endpoint.clone()),
            law_set: RewriteLawSetId(SemanticId::new(91_000 + spec)),
        }
    }

    let left = rewrite(91_101, final_endpoint);
    let right = rewrite(91_102, final_endpoint);
    let right_after_left = rewrite(91_111, final_endpoint);
    let left_after_right = rewrite(91_112, final_endpoint);
    let mut residuals = kernel_change::RewriteResidualFamilyRegistry::default();
    residuals
        .register(kernel_change::RewriteResidualFamilySpec {
            id: kernel_change::RewriteResidualFamilyId(SemanticId::new(91_201)),
            key: kernel_change::RewriteResidualFamilyKey {
                left: (&left).into(),
                right: (&right).into(),
            },
            right_after_left: (&right_after_left).into(),
            left_after_right: (&left_after_right).into(),
        })
        .unwrap();
    let left_ideal = kernel_change::RevisionEffectIdeal::new(vec![kernel_change::RevisionEffect {
        id: kernel_change::RevisionEffectId(91_301),
        prerequisites: BTreeSet::new(),
        payload: left,
    }])
    .unwrap();
    let right_ideal =
        kernel_change::RevisionEffectIdeal::new(vec![kernel_change::RevisionEffect {
            id: kernel_change::RevisionEffectId(91_302),
            prerequisites: BTreeSet::new(),
            payload: right,
        }])
        .unwrap();
    left_ideal
        .certify_registered_residual_chain(
            &right_ideal,
            base,
            &residuals,
            &kernel_change::RewriteSequentialFamilyRegistry::default(),
            kernel_change::RevisionEffectResidualChainWitness {
                first_right_after_left: right_after_left,
                first_left_after_right: left_after_right,
                subsequent: Vec::new(),
            },
        )
        .unwrap()
}

fn runtime_physical_i64_values(
    runtime: &RuntimeRevisionBundle,
    relation: SemanticId,
    binding: LayoutBinding,
) -> Vec<i64> {
    let installed = runtime
        .physical_store()
        .installed(relation, binding)
        .unwrap();
    let mut values = installed
        .scan_positions()
        .map(|position| materialize_native_row(&installed.data, position).unwrap())
        .map(|row| match row.as_slice() {
            [Value::I64(value)] => *value,
            _ => panic!("expected one I64 column"),
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    values
}

fn runtime_maintained_i64_values(
    runtime: &RuntimeRevisionBundle,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> Vec<i64> {
    let mut values = runtime
        .materialization(test_materialization_id())
        .unwrap()
        .output_value(context, registry)
        .unwrap()
        .rows()
        .iter()
        .map(|row| match row.as_slice() {
            [Value::I64(value)] => *value,
            _ => panic!("expected one I64 column"),
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    values
}

#[test]
fn revision_prepare_is_invisible_until_sealed_publish() {
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(40, 961, &[1, 2, 3]);
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let before = runtime.clone_for_test();
    let prepared = prepare_runtime_revision(&runtime, 41, &mutations, &registry).unwrap();

    assert_eq!(prepared.source_revision(), RevisionId::new(40));
    assert_eq!(prepared.target_revision(), RevisionId::new(41));
    assert_eq!(
        prepared.descriptor().relation_deltas().unwrap()[&relation],
        delta
    );
    assert_eq!(prepared.output_deltas()[&test_materialization_id()], delta);
    assert_eq!(runtime, before);

    let cell = RuntimeRevisionCell::new(runtime);
    let sealed = prepared.seal(&cell).unwrap();
    assert_eq!(sealed.source_revision(), RevisionId::new(40));
    assert_eq!(sealed.target_revision(), RevisionId::new(41));
    assert_eq!(
        sealed.descriptor().relation_deltas().unwrap()[&relation],
        delta
    );
    let emitted = sealed.publish();
    let runtime = cell.snapshot().unwrap();

    assert_eq!(emitted[&test_materialization_id()], delta);
    assert_eq!(runtime.revision_id(), RevisionId::new(41));
    assert_eq!(
        runtime.revision().state().model.relations[&relation],
        vec![
            vec![Value::I64(2)],
            vec![Value::I64(3)],
            vec![Value::I64(4)],
        ]
    );
    assert_eq!(
        runtime.physical_store().revision(),
        Some(RevisionId::new(41))
    );
    assert_eq!(
        runtime.materialization_revision(test_materialization_id()),
        Some(RevisionId::new(41))
    );
    assert_eq!(
        runtime
            .materialization(test_materialization_id())
            .unwrap()
            .revision(),
        None
    );
    assert_eq!(
        runtime_physical_i64_values(&runtime, relation, binding),
        vec![2, 3, 4]
    );
    assert_eq!(
        runtime_maintained_i64_values(&runtime, &context, &registry),
        vec![2, 3, 4]
    );
}

#[test]
fn relation_rewrite_prepare_preserves_intent_and_uses_existing_vmf_dtc_boundary() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(440, 9961, &[1, 2, 3]);
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let old = RelExpr::Scan(relation)
        .evaluate(&runtime.revision().state().model, &context, &registry)
        .unwrap();
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(44_001)),
        law_set: RewriteLawSetId(SemanticId::new(44_002)),
        footprint: kernel_change::RewriteFootprint::default(),
    };
    let rewrite = delta
        .prepare_relation_rewrite(&old, &context, &registry, &spec, vec![Value::I64(4)])
        .unwrap();
    let mutation = RevisionRelationMutation {
        relation,
        delta: &delta,
    };
    let target = target_revision_for(&runtime, 441, &[mutation], &registry);
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &rewrite,
    }];
    let prepared = runtime
        .prepare_rewrites_for_test(&RevisionRewriteTransitionRequest {
            target_revision: &target,
            rewrites: &rewrites,
            registry: &registry,
        })
        .unwrap();

    assert_eq!(
        prepared.rewrite_intents()[&relation],
        RuntimeRewriteIntent {
            spec: spec.id,
            law_set: spec.law_set,
        }
    );
    assert_eq!(
        prepared.descriptor().relation_deltas().unwrap()[&relation],
        delta
    );

    let cell = RuntimeRevisionCell::new(runtime);
    let sealed = prepared.seal(&cell).unwrap();
    assert_eq!(sealed.rewrite_intents()[&relation].spec, spec.id);
    let _ = sealed.publish();
    assert_eq!(cell.snapshot().unwrap().revision_id(), RevisionId::new(441));
}

#[test]
fn relation_rewrite_prepare_rejects_effect_not_derived_from_its_delta() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(450, 9962, &[1, 2, 3]);
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let old = RelExpr::Scan(relation)
        .evaluate(&runtime.revision().state().model, &context, &registry)
        .unwrap();
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(45_001)),
        law_set: RewriteLawSetId(SemanticId::new(45_002)),
        footprint: kernel_change::RewriteFootprint::default(),
    };
    let mut rewrite = delta
        .prepare_relation_rewrite(&old, &context, &registry, &spec, Vec::<Value>::new())
        .unwrap();
    rewrite.rewrite.effect = kernel_change::RewriteEffect::Replace(old.clone());
    let mutation = RevisionRelationMutation {
        relation,
        delta: &delta,
    };
    let target = target_revision_for(&runtime, 451, &[mutation], &registry);
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &rewrite,
    }];

    assert!(matches!(
        runtime.prepare_rewrites_for_test(&RevisionRewriteTransitionRequest {
            target_revision: &target,
            rewrites: &rewrites,
            registry: &registry,
        }),
        Err(PhysicalExecutionError::RewriteEffectMismatch(found)) if found == relation
    ));
}

#[test]
fn derived_rewrite_endpoint_certificate_is_bound_to_exact_delta_payload() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(452, 9963, &[1, 2, 3]);
    let certified_delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let certified_mutation = RevisionRelationMutation {
        relation,
        delta: &certified_delta,
    };
    let target = target_revision_for(&runtime, 453, &[certified_mutation], &registry);

    let different_delta = scan_delta(relation, &[5], &[1], &context, &registry);
    let old = RelExpr::Scan(relation)
        .evaluate(&runtime.revision().state().model, &context, &registry)
        .unwrap();
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(45_201)),
        law_set: RewriteLawSetId(SemanticId::new(45_202)),
        footprint: kernel_change::RewriteFootprint::default(),
    };
    let different_rewrite = different_delta
        .prepare_relation_rewrite(&old, &context, &registry, &spec, Vec::<Value>::new())
        .unwrap();
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &different_rewrite,
    }];

    assert!(matches!(
        runtime.prepare_rewrites_derived_for_test(
            target,
            BTreeMap::from([(relation, certified_delta.clone())]),
            &rewrites,
            &registry,
        ),
        Err(PhysicalExecutionError::LogicalRevisionMutationMismatch)
    ));
}

#[test]
fn derived_rewrite_effect_is_checked_against_certified_target_endpoint() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(454, 9964, &[1, 2, 3]);
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutation = RevisionRelationMutation {
        relation,
        delta: &delta,
    };
    let target = target_revision_for(&runtime, 455, &[mutation], &registry);

    let old = RelExpr::Scan(relation)
        .evaluate(&runtime.revision().state().model, &context, &registry)
        .unwrap();
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(45_401)),
        law_set: RewriteLawSetId(SemanticId::new(45_402)),
        footprint: kernel_change::RewriteFootprint::default(),
    };
    let mut rewrite = delta
        .prepare_relation_rewrite(&old, &context, &registry, &spec, Vec::<Value>::new())
        .unwrap();
    rewrite.rewrite.effect = kernel_change::RewriteEffect::Replace(old);
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &rewrite,
    }];

    assert!(matches!(
        runtime.prepare_rewrites_derived_for_test(
            target,
            BTreeMap::from([(relation, delta.clone())]),
            &rewrites,
            &registry,
        ),
        Err(PhysicalExecutionError::RewriteEffectMismatch(found)) if found == relation
    ));
}

#[test]
fn seal_rejects_nonzero_candidate_violation_state_before_publication() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(42, 9_980_042, &[1, 2, 3]);
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let mut prepared = prepare_runtime_revision(&runtime, 43, &mutations, &registry).unwrap();
    prepared.inject_candidate_violation_for_test(
            kernel_validation::DynamicViolationWitness::RelationUniqueness {
                relation,
                row_key: Vec::new(),
            },
        1,
    );

    let cell = RuntimeRevisionCell::new(runtime);
    let before = cell.snapshot().unwrap();
    assert!(matches!(
        prepared.seal(&cell),
        Err(PhysicalExecutionError::CandidateViolationStateNonZero)
    ));
    assert_eq!(cell.snapshot().unwrap(), before);
}

#[test]
fn vmf_invariant_closure_certificate_is_zero_and_revision_bound() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(43, 9_980_043, &[1, 2, 3]);
    let certificate = runtime.invariant_closure_certificate().unwrap();
    assert_eq!(certificate.revision(), RevisionId::new(43));
    assert_eq!(
        certificate.semantic_revision(),
        runtime.revision().semantic_revision()
    );

    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let mut prepared = prepare_runtime_revision(&runtime, 44, &mutations, &registry).unwrap();
    let candidate_certificate = prepared.candidate_invariant_closure_certificate_for_test().unwrap();
    assert_eq!(candidate_certificate.revision(), RevisionId::new(44));

    prepared.inject_candidate_violation_for_test(
            kernel_validation::DynamicViolationWitness::RelationUniqueness {
                relation,
                row_key: Vec::new(),
            },
        1,
    );
    assert!(matches!(
        prepared.candidate_invariant_closure_certificate_for_test(),
        Err(PhysicalExecutionError::CandidateViolationStateNonZero)
    ));
}

#[test]
fn coherent_resolution_binding_requires_cube_endpoint_and_candidate_vmf_closure() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(45, 9_980_045, &[1]);
    let old = RelExpr::Scan(relation)
        .evaluate(
            &runtime.revision().state().model,
            runtime.revision().semantic_context(),
            &registry,
        )
        .unwrap();
    let delta = scan_delta(relation, &[7], &[1], &context, &registry);
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(45_101)),
        law_set: RewriteLawSetId(SemanticId::new(45_102)),
        footprint: kernel_change::RewriteFootprint::default(),
    };
    let rewrite = delta
        .prepare_relation_rewrite(&old, &context, &registry, &spec, Vec::<Value>::new())
        .unwrap();
    let mutation = RevisionRelationMutation {
        relation,
        delta: &delta,
    };
    let target = target_revision_for(&runtime, 46, &[mutation], &registry);
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &rewrite,
    }];
    let prepared = runtime
        .prepare_rewrites_for_test(&RevisionRewriteTransitionRequest {
            target_revision: &target,
            rewrites: &rewrites,
            registry: &registry,
        })
        .unwrap();
    let final_endpoint = rewrite.rewrite.apply(&old);
    let cube = relation_cube_certificate(&old, &final_endpoint);
    let coherent = prepared
        .bind_coherent_resolution(relation, cube, &registry)
        .unwrap();
    assert_eq!(coherent.target_revision(), RevisionId::new(46));
    assert_eq!(coherent.cube().common_endpoint(), &final_endpoint);
    assert_eq!(coherent.invariant_closure().revision(), RevisionId::new(46));

    let prepared = runtime
        .prepare_rewrites_for_test(&RevisionRewriteTransitionRequest {
            target_revision: &target,
            rewrites: &rewrites,
            registry: &registry,
        })
        .unwrap();
    let wrong_cube = relation_cube_certificate(&old, &old);
    assert!(matches!(
        prepared.bind_coherent_resolution(relation, wrong_cube, &registry),
        Err(PhysicalExecutionError::ResolutionCoherenceEndpointMismatch)
    ));
}

#[test]
fn runtime_observation_guard_classifies_prepared_transition_via_dtc() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(44, 9_980_044, &[1, 2, 3]);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(99),
        equivalence: sid(101),
    };
    let guard = runtime.observe_query(&query, &registry).unwrap();
    assert_eq!(guard.source_revision(), RevisionId::new(44));
    assert_eq!(guard.source_relations(), &BTreeSet::from([relation]));

    let unaffected_delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let unaffected_mutations = [RevisionRelationMutation {
        relation,
        delta: &unaffected_delta,
    }];
    let unaffected =
        prepare_runtime_revision(&runtime, 45, &unaffected_mutations, &registry).unwrap();
    assert_eq!(
        guard
            .impact_prepared(&runtime, &unaffected, &registry)
            .unwrap(),
        Impact::Unaffected
    );
    assert_eq!(
        guard
            .impact_prepared_by_recompute_oracle(&runtime, &unaffected, &registry)
            .unwrap(),
        Impact::Unaffected
    );

    let changed_delta = scan_delta(relation, &[99], &[2], &context, &registry);
    let changed_mutations = [RevisionRelationMutation {
        relation,
        delta: &changed_delta,
    }];
    let changed = prepare_runtime_revision(&runtime, 46, &changed_mutations, &registry).unwrap();
    assert_eq!(
        guard
            .impact_prepared(&runtime, &changed, &registry)
            .unwrap(),
        Impact::Changed
    );
    assert_eq!(
        guard
            .impact_prepared_by_recompute_oracle(&runtime, &changed, &registry)
            .unwrap(),
        Impact::Changed
    );
}

#[test]
fn bounded_repair_accepts_one_observation_preserving_candidate() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(64, 9_980_064, &[1, 2, 3]);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(99),
        equivalence: sid(101),
    };
    let guard = runtime.observe_query(&query, &registry).unwrap();
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let borrowed = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 65, &borrowed, &registry);
    let provider = vec![RuntimeRepairCandidate::RelationData {
        target_revision: target,
        mutations: vec![RuntimeRepairRelationMutation { relation, delta }],
    }];

    let outcome = runtime
        .prepare_bounded_repair(&guard, &provider, RepairSearchPolicy::default(), &registry)
        .unwrap();
    let RepairSearchOutcome::Prepared { transition, report } = outcome else {
        panic!("expected unique repair")
    };
    assert_eq!(
        transition.descriptor().target_revision(),
        RevisionId::new(65)
    );
    assert_eq!(report.supplied, 1);
    assert_eq!(report.examined, 1);
    assert_eq!(report.accepted, 1);
    assert_eq!(report.rejected_observation_change, 0);
}

#[test]
fn bounded_repair_accepts_tsc_transport_across_definitionally_equivalent_context() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(640, 9_980_640, &[1, 2, 3]);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(99),
        equivalence: sid(101),
    };
    let guard = runtime.observe_query(&query, &registry).unwrap();

    let mut target_context = context.clone();
    target_context.schema.revision = SchemaRevisionId::new(641);
    let transport =
        kernel_transport::DefinitionalTransport::verify(&context, &target_context, &registry)
            .unwrap();
    let mut target_state = runtime.revision().state().clone();
    target_state.model.relations.insert(
        relation,
        vec![
            vec![Value::I64(2)],
            vec![Value::I64(3)],
            vec![Value::I64(4)],
        ],
    );
    let target = kernel_revision::Revision::build(
        RevisionId::new(641),
        &target_context,
        &registry,
        target_state,
    )
    .unwrap();
    let provider = vec![RuntimeRepairCandidate::TransportedFullRevision {
        target_revision: target,
        observation_transport: Box::new(RuntimeRepairObservationTransport::Definitional(transport)),
    }];

    let outcome = runtime
        .prepare_bounded_repair(&guard, &provider, RepairSearchPolicy::default(), &registry)
        .unwrap();
    let RepairSearchOutcome::Prepared { report, .. } = outcome else {
        panic!("verified TSC repair should be accepted")
    };
    assert_eq!(report.accepted, 1);
    assert_eq!(report.rejected_transport, 0);
    assert_eq!(report.rejected_observation_change, 0);
}

#[test]
fn bounded_repair_accepts_equivalent_semantic_implementation_transport() {
    let (context, mut registry, relation, _, runtime) =
        scan_runtime_bundle(646, 9_980_646, &[1, 2, 3]);
    let guard = runtime
        .observe_query(&RelExpr::Scan(relation), &registry)
        .unwrap();

    let upgraded = registry.install_equivalence_revision(EquivalenceModule::I64Exact, 2);
    let mut target_context = context.clone();
    target_context.environment.revision = SemanticEnvId::new(647);
    target_context.environment.pin_module(sid(101), upgraded);
    let transport = kernel_transport::EquivalentSemanticEnvironmentTransport::verify(
        &context,
        &target_context,
        &registry,
    )
    .unwrap();
    let target = kernel_revision::Revision::build(
        RevisionId::new(647),
        &target_context,
        &registry,
        runtime.revision().state().clone(),
    )
    .unwrap();
    let provider = vec![RuntimeRepairCandidate::TransportedFullRevision {
        target_revision: target,
        observation_transport: Box::new(
            RuntimeRepairObservationTransport::EquivalentSemanticEnvironment(transport),
        ),
    }];

    let outcome = runtime
        .prepare_bounded_repair(&guard, &provider, RepairSearchPolicy::default(), &registry)
        .unwrap();
    let RepairSearchOutcome::Prepared { report, .. } = outcome else {
        panic!("same semantic contract implementation upgrade should preserve observation")
    };
    assert_eq!(report.accepted, 1);
    assert_eq!(report.rejected_transport, 0);
}

#[test]
fn bounded_repair_rejects_cross_context_candidate_without_transport_witness() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(642, 9_980_642, &[1, 2, 3]);
    let guard = runtime
        .observe_query(&RelExpr::Scan(relation), &registry)
        .unwrap();
    let mut target_context = context.clone();
    target_context.schema.revision = SchemaRevisionId::new(643);
    let target = kernel_revision::Revision::build(
        RevisionId::new(643),
        &target_context,
        &registry,
        runtime.revision().state().clone(),
    )
    .unwrap();
    let provider = vec![RuntimeRepairCandidate::FullRevision {
        target_revision: target,
    }];

    let outcome = runtime
        .prepare_bounded_repair(&guard, &provider, RepairSearchPolicy::default(), &registry)
        .unwrap();
    let RepairSearchOutcome::NoRepair(report) = outcome else {
        panic!("cross-context repair without TSC witness must be rejected")
    };
    assert_eq!(report.rejected_transport, 1);
    assert_eq!(report.accepted, 0);
}

#[test]
fn bounded_repair_tsc_still_rejects_changed_target_observation() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(644, 9_980_644, &[1, 2, 3]);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(99),
        equivalence: sid(101),
    };
    let guard = runtime.observe_query(&query, &registry).unwrap();
    let mut target_context = context.clone();
    target_context.schema.revision = SchemaRevisionId::new(645);
    let transport =
        kernel_transport::DefinitionalTransport::verify(&context, &target_context, &registry)
            .unwrap();
    let mut target_state = runtime.revision().state().clone();
    target_state
        .model
        .relations
        .get_mut(&relation)
        .unwrap()
        .push(vec![Value::I64(99)]);
    let target = kernel_revision::Revision::build(
        RevisionId::new(645),
        &target_context,
        &registry,
        target_state,
    )
    .unwrap();
    let provider = vec![RuntimeRepairCandidate::TransportedFullRevision {
        target_revision: target,
        observation_transport: Box::new(RuntimeRepairObservationTransport::Definitional(transport)),
    }];

    let outcome = runtime
        .prepare_bounded_repair(&guard, &provider, RepairSearchPolicy::default(), &registry)
        .unwrap();
    let RepairSearchOutcome::NoRepair(report) = outcome else {
        panic!("transport does not excuse observation change")
    };
    assert_eq!(report.rejected_observation_change, 1);
    assert_eq!(report.accepted, 0);
}

#[test]
fn bounded_repair_rejects_observation_change_and_reports_no_repair() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(66, 9_980_066, &[1, 2, 3]);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(99),
        equivalence: sid(101),
    };
    let guard = runtime.observe_query(&query, &registry).unwrap();
    let delta = scan_delta(relation, &[99], &[1], &context, &registry);
    let borrowed = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 67, &borrowed, &registry);
    let provider = vec![RuntimeRepairCandidate::RelationData {
        target_revision: target,
        mutations: vec![RuntimeRepairRelationMutation { relation, delta }],
    }];

    let outcome = runtime
        .prepare_bounded_repair(&guard, &provider, RepairSearchPolicy::default(), &registry)
        .unwrap();
    let RepairSearchOutcome::NoRepair(report) = outcome else {
        panic!("observation-changing candidate must not repair")
    };
    assert_eq!(report.supplied, 1);
    assert_eq!(report.examined, 1);
    assert_eq!(report.accepted, 0);
    assert_eq!(report.rejected_observation_change, 1);
}

#[test]
fn bounded_repair_reports_ambiguity_for_two_valid_candidates() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(68, 9_980_068, &[1, 2, 3]);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(99),
        equivalence: sid(101),
    };
    let guard = runtime.observe_query(&query, &registry).unwrap();

    let delta_a = scan_delta(relation, &[4], &[1], &context, &registry);
    let borrowed_a = [RevisionRelationMutation {
        relation,
        delta: &delta_a,
    }];
    let target_a = target_revision_for(&runtime, 69, &borrowed_a, &registry);
    let delta_b = scan_delta(relation, &[5], &[2], &context, &registry);
    let borrowed_b = [RevisionRelationMutation {
        relation,
        delta: &delta_b,
    }];
    let target_b = target_revision_for(&runtime, 70, &borrowed_b, &registry);
    let provider = vec![
        RuntimeRepairCandidate::RelationData {
            target_revision: target_a,
            mutations: vec![RuntimeRepairRelationMutation {
                relation,
                delta: delta_a,
            }],
        },
        RuntimeRepairCandidate::RelationData {
            target_revision: target_b,
            mutations: vec![RuntimeRepairRelationMutation {
                relation,
                delta: delta_b,
            }],
        },
    ];

    let outcome = runtime
        .prepare_bounded_repair(&guard, &provider, RepairSearchPolicy::default(), &registry)
        .unwrap();
    let RepairSearchOutcome::Ambiguous {
        valid_candidates,
        report,
    } = outcome
    else {
        panic!("two valid repairs must be ambiguous")
    };
    assert_eq!(valid_candidates, 2);
    assert_eq!(report.supplied, 2);
    assert_eq!(report.examined, 2);
    assert_eq!(report.accepted, 2);
}

#[test]
fn bounded_repair_enforces_candidate_budget_before_evaluation() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(71, 9_980_071, &[1, 2, 3]);
    let guard = runtime
        .observe_query(&RelExpr::Scan(relation), &registry)
        .unwrap();
    let delta_a = scan_delta(relation, &[4], &[1], &context, &registry);
    let borrowed_a = [RevisionRelationMutation {
        relation,
        delta: &delta_a,
    }];
    let target_a = target_revision_for(&runtime, 72, &borrowed_a, &registry);
    let delta_b = scan_delta(relation, &[5], &[2], &context, &registry);
    let borrowed_b = [RevisionRelationMutation {
        relation,
        delta: &delta_b,
    }];
    let target_b = target_revision_for(&runtime, 73, &borrowed_b, &registry);
    let provider = vec![
        RuntimeRepairCandidate::RelationData {
            target_revision: target_a,
            mutations: vec![RuntimeRepairRelationMutation {
                relation,
                delta: delta_a,
            }],
        },
        RuntimeRepairCandidate::RelationData {
            target_revision: target_b,
            mutations: vec![RuntimeRepairRelationMutation {
                relation,
                delta: delta_b,
            }],
        },
    ];

    assert_eq!(
        runtime
            .prepare_bounded_repair(
                &guard,
                &provider,
                RepairSearchPolicy { max_candidates: 1 },
                &registry,
            )
            .unwrap(),
        RepairSearchOutcome::BudgetExceeded {
            supplied: 2,
            maximum: 1,
        }
    );
}

#[test]
fn runtime_observation_guard_is_root_lineage_bound() {
    let (context, registry, relation, _, source) = scan_runtime_bundle(47, 9_980_047, &[1, 2]);
    let (_, _, _, _, identical_other_root) = scan_runtime_bundle(47, 9_980_047, &[1, 2]);
    let guard = source
        .observe_query(&RelExpr::Scan(relation), &registry)
        .unwrap();
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let prepared =
        prepare_runtime_revision(&identical_other_root, 48, &mutations, &registry).unwrap();
    assert_eq!(
        guard.impact_prepared(&source, &prepared, &registry),
        Err(PhysicalExecutionError::RevisionBindingMismatch)
    );
}

#[test]
fn reader_snapshot_remains_on_old_root_after_atomic_publication() {
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(45, 976, &[1, 2, 3]);
    let cell = RuntimeRevisionCell::new(runtime);
    let old_reader = cell.snapshot().unwrap();
    assert_eq!(old_reader.root_version().raw(), 0);

    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let prepared = prepare_runtime_cell_revision(&cell, 46, &mutations, &registry).unwrap();
    let _ = prepared.seal(&cell).unwrap().publish();
    let new_reader = cell.snapshot().unwrap();

    assert_eq!(old_reader.revision_id(), RevisionId::new(45));
    assert_eq!(new_reader.revision_id(), RevisionId::new(46));
    assert_eq!(old_reader.root_version().raw(), 0);
    assert_eq!(new_reader.root_version().raw(), 1);
    assert_eq!(
        runtime_physical_i64_values(&old_reader, relation, binding),
        vec![1, 2, 3]
    );
    assert_eq!(
        runtime_physical_i64_values(&new_reader, relation, binding),
        vec![2, 3, 4]
    );
    assert_eq!(
        runtime_maintained_i64_values(&old_reader, &context, &registry),
        vec![1, 2, 3]
    );
    assert_eq!(
        runtime_maintained_i64_values(&new_reader, &context, &registry),
        vec![2, 3, 4]
    );
}

#[test]
fn prepared_transition_cannot_cross_identical_runtime_root_lineages() {
    let (context, registry, relation, _, source) = scan_runtime_bundle(47, 977, &[1, 2]);
    let (_, _, _, _, identical) = scan_runtime_bundle(47, 977, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let prepared = prepare_runtime_revision(&source, 48, &mutations, &registry).unwrap();
    let wrong_cell = RuntimeRevisionCell::new(identical);

    assert!(matches!(
        prepared.seal(&wrong_cell),
        Err(PhysicalExecutionError::StalePreparedTransition)
    ));
    assert_eq!(
        wrong_cell.snapshot().unwrap().revision_id(),
        RevisionId::new(47)
    );
}

#[test]
fn semantic_context_change_is_rejected_without_publication() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(50, 962, &[1, 2]);
    let mut wrong_context = context.clone();
    wrong_context.schema.revision = SchemaRevisionId::new(999_962);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let mut target_state = runtime.revision().state().clone();
    target_state
        .model
        .relations
        .insert(relation, vec![vec![Value::I64(2)], vec![Value::I64(3)]]);
    let target = kernel_revision::Revision::build(
        RevisionId::new(51),
        &wrong_context,
        &registry,
        target_state,
    )
    .unwrap();
    let before = runtime.clone_for_test();

    assert_eq!(
        runtime.prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &target,
            mutations: &mutations,
            registry: &registry,
        }),
        Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild)
    );
    assert_eq!(runtime, before);
}

#[test]
fn prepared_transition_rejects_different_same_revision_bundle() {
    let (context, registry, relation, _, source) = scan_runtime_bundle(60, 963, &[1, 2]);
    let (_, _, _, _, different) = scan_runtime_bundle(60, 963, &[7, 8]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let prepared = prepare_runtime_revision(&source, 61, &mutations, &registry).unwrap();
    let different_before = different.clone_for_test();
    let different_cell = RuntimeRevisionCell::new(different);

    assert!(matches!(
        prepared.seal(&different_cell),
        Err(PhysicalExecutionError::StalePreparedTransition)
    ));
    assert_eq!(different_cell.snapshot().unwrap().root(), &different_before);
}

#[test]
fn physical_index_change_after_prepare_makes_transition_stale() {
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(70, 964, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let prepared = prepare_runtime_revision(&runtime, 71, &mutations, &registry).unwrap();
    let cell = RuntimeRevisionCell::new(runtime);
    let before_index = cell.snapshot().unwrap();
    cell.install_i64_index(
        I64IndexBinding {
            relation,
            layout: binding,
            key_column: 0,
            equivalence: sid(101),
        },
        &registry,
    )
    .unwrap();
    let after_index = cell.snapshot().unwrap();
    assert_eq!(before_index.revision_id(), after_index.revision_id());
    assert_eq!(before_index.root_version().raw(), 0);
    assert_eq!(after_index.root_version().raw(), 1);

    assert!(matches!(
        prepared.seal(&cell),
        Err(PhysicalExecutionError::StalePreparedTransition)
    ));
    assert_eq!(cell.snapshot().unwrap(), after_index);
}

#[test]
fn semantic_statistics_publish_as_reconstructible_runtime_state() {
    let values = (0_i64..64).collect::<Vec<_>>();
    let (context, registry, relation, layout, runtime) = scan_runtime_bundle(73, 966, &values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    let delta = scan_delta(relation, &[100], &[0], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let prepared = prepare_runtime_revision(&runtime, 74, &mutations, &registry).unwrap();
    let cell = RuntimeRevisionCell::new(runtime);
    let old_reader = cell.snapshot().unwrap();

    let statistics = cell
        .install_semantic_statistics(binding.clone(), &registry)
        .unwrap();
    let new_reader = cell.snapshot().unwrap();
    assert_eq!(statistics.row_count, 64);
    assert_eq!(statistics.distinct_key_count, 64);
    assert_eq!(old_reader.root_version().raw(), 0);
    assert_eq!(new_reader.root_version().raw(), 1);
    assert_eq!(old_reader.revision_id(), new_reader.revision_id());
    assert_eq!(
        old_reader
            .physical_store()
            .semantic_statistics(&binding, &context, &registry)
            .unwrap(),
        None
    );
    assert_eq!(
        new_reader
            .physical_store()
            .semantic_statistics(&binding, &context, &registry)
            .unwrap(),
        Some(statistics)
    );
    assert!(matches!(
        prepared.seal(&cell),
        Err(PhysicalExecutionError::StalePreparedTransition)
    ));
}

#[test]
fn observable_atom_state_publishes_as_reconstructible_runtime_state() {
    let values = (0_i64..32).collect::<Vec<_>>();
    let (context, registry, relation, layout, runtime) =
        scan_runtime_bundle(9_978_200, 9_978_201, &values);
    let binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    let cell = RuntimeRevisionCell::new(runtime);
    let old_reader = cell.snapshot().unwrap();

    cell.install_observable_atom_state(binding.clone(), &registry)
        .unwrap();
    let new_reader = cell.snapshot().unwrap();
    assert_eq!(old_reader.revision_id(), new_reader.revision_id());
    assert_eq!(old_reader.root_version().raw(), 0);
    assert_eq!(new_reader.root_version().raw(), 1);
    assert!(
        old_reader
            .physical_store()
            .observable_atom_state(&binding)
            .is_none()
    );
    let state = new_reader
        .physical_store()
        .observable_atom_state(&binding)
        .unwrap();
    assert_eq!(state.row_count(), 32);
    assert_eq!(state.atom_count(), 32);
    assert_eq!(state.catalog.revision(), context.revision());
}

#[test]
fn semantic_index_advisor_publishes_one_runtime_root_and_noop_does_not_republish() {
    let values = (0_i64..64).collect::<Vec<_>>();
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(72, 965, &values);
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let workload = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 2,
    }];
    let index = SemanticIndexBinding::single(relation, binding, 0, sid(101));
    let cell = RuntimeRevisionCell::new(runtime);
    let old_reader = cell.snapshot().unwrap();

    let first = cell
        .advise_semantic_indexes(&workload, SemanticIndexAdvisorPolicy::default(), &registry)
        .unwrap();
    let new_reader = cell.snapshot().unwrap();
    assert_eq!(first.created, vec![index.clone()]);
    assert_eq!(old_reader.root_version().raw(), 0);
    assert_eq!(new_reader.root_version().raw(), 1);
    assert!(old_reader.physical_store().semantic_index(&index).is_none());
    assert!(new_reader.physical_store().semantic_index(&index).is_some());

    let second = cell
        .advise_semantic_indexes(&workload, SemanticIndexAdvisorPolicy::default(), &registry)
        .unwrap();
    assert_eq!(second.retained, vec![index]);
    assert_eq!(cell.snapshot().unwrap().root_version().raw(), 1);
}

#[test]
fn semantic_statistics_advisor_no_longer_materializes_direct_join_statistics() {
    let values = vec![1_i64; 128];
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(73, 966, &values);
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let workload = [SemanticIndexWorkloadSample {
        plan: prepared.physical().clone(),
        expected_executions: 1,
    }];
    let statistics = SemanticIndexBinding::single(relation, binding, 0, sid(101));
    let cell = RuntimeRevisionCell::new(runtime);
    let old_reader = cell.snapshot().unwrap();

    let first = cell
        .advise_semantic_statistics(
            &workload,
            PhysicalArtifactAdvisorPolicy::default(),
            &registry,
        )
        .unwrap();
    let new_reader = cell.snapshot().unwrap();
    assert!(first.created.is_empty());
    assert!(first.retained.is_empty());
    assert_eq!(old_reader.root_version().raw(), 0);
    assert_eq!(new_reader.root_version().raw(), 0);
    assert!(
        new_reader
            .physical_store()
            .semantic_statistics(&statistics, &context, &registry)
            .unwrap()
            .is_none()
    );
}

#[test]
fn competing_prepared_transitions_from_same_revision_cannot_both_publish() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(100, 967, &[1, 2]);
    let delta_a = scan_delta(relation, &[3], &[1], &context, &registry);
    let delta_b = scan_delta(relation, &[4], &[2], &context, &registry);
    let mutations_a = [RevisionRelationMutation {
        relation,
        delta: &delta_a,
    }];
    let mutations_b = [RevisionRelationMutation {
        relation,
        delta: &delta_b,
    }];
    let prepared_a = prepare_runtime_revision(&runtime, 101, &mutations_a, &registry).unwrap();
    let prepared_b = prepare_runtime_revision(&runtime, 102, &mutations_b, &registry).unwrap();
    let cell = RuntimeRevisionCell::new(runtime);

    let _ = prepared_a.seal(&cell).unwrap().publish();
    let after_a = cell.snapshot().unwrap();
    assert!(matches!(
        prepared_b.seal(&cell),
        Err(PhysicalExecutionError::StalePreparedTransition)
    ));
    assert_eq!(cell.snapshot().unwrap(), after_a);
    assert_eq!(after_a.revision_id(), RevisionId::new(101));
}

#[test]
fn sequential_transitions_carry_revision_and_stable_handles_forward() {
    let (context, registry, relation, binding, runtime) = scan_runtime_bundle(110, 968, &[1, 2]);
    let initial_handles = runtime
        .physical_store()
        .logical_row_handles(relation, binding)
        .unwrap();
    assert_eq!(initial_handles[0].generation, 0);
    let cell = RuntimeRevisionCell::new(runtime);

    let first = scan_delta(relation, &[3], &[1], &context, &registry);
    let first_mutations = [RevisionRelationMutation {
        relation,
        delta: &first,
    }];
    let _ = prepare_runtime_cell_revision(&cell, 111, &first_mutations, &registry)
        .unwrap()
        .seal(&cell)
        .unwrap()
        .publish();
    let first_snapshot = cell.snapshot().unwrap();
    let first_handles = first_snapshot
        .physical_store()
        .logical_row_handles(relation, binding)
        .unwrap();
    let reused = first_handles
        .iter()
        .copied()
        .find(|handle| handle.slot == initial_handles[0].slot)
        .unwrap();
    assert_eq!(reused.generation, 1);
    drop(first_snapshot);

    let second = scan_delta(relation, &[4], &[3], &context, &registry);
    let second_mutations = [RevisionRelationMutation {
        relation,
        delta: &second,
    }];
    let _ = prepare_runtime_cell_revision(&cell, 112, &second_mutations, &registry)
        .unwrap()
        .seal(&cell)
        .unwrap()
        .publish();
    let runtime = cell.snapshot().unwrap();
    let second_handles = runtime
        .physical_store()
        .logical_row_handles(relation, binding)
        .unwrap();
    let reused_again = second_handles
        .iter()
        .copied()
        .find(|handle| handle.slot == initial_handles[0].slot)
        .unwrap();

    assert_eq!(reused_again.generation, 2);
    assert_eq!(runtime.revision_id(), RevisionId::new(112));
    assert_eq!(
        runtime_physical_i64_values(&runtime, relation, binding),
        vec![2, 4]
    );
    assert_eq!(
        runtime_maintained_i64_values(&runtime, &context, &registry),
        vec![2, 4]
    );
}

#[test]
fn revision_prepare_rejects_same_source_and_target_without_mutation() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(119, 970, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 119, &mutations, &registry);
    let before = runtime.clone_for_test();

    assert_eq!(
        runtime.prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &target,
            mutations: &mutations,
            registry: &registry,
        }),
        Err(PhysicalExecutionError::InvalidRevisionTransition)
    );
    assert_eq!(runtime, before);
}

#[test]
fn sealed_transition_drop_aborts_without_publication() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(125, 971, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let before = runtime.clone_for_test();
    let prepared = prepare_runtime_revision(&runtime, 126, &mutations, &registry).unwrap();
    let cell = RuntimeRevisionCell::new(runtime);
    let sealed = prepared.seal(&cell).unwrap();
    assert_eq!(sealed.source_revision(), RevisionId::new(125));
    assert_eq!(sealed.target_revision(), RevisionId::new(126));
    drop(sealed);

    let runtime = cell.snapshot().unwrap();
    assert_eq!(runtime.root(), &before);
    assert_eq!(runtime.revision_id(), RevisionId::new(125));
}

#[test]
fn bootstrap_rejects_logical_physical_divergence() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(972),
        family: LayoutFamily::Columnar,
    };
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(relation, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
    let revision = kernel_revision::Revision::build(
        RevisionId::new(130),
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
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 9].into())]).unwrap(),
        )
        .unwrap();

    assert!(matches!(
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
        Err(PhysicalExecutionError::LogicalPhysicalStateMismatch(found)) if found == relation
    ));
}

#[test]
fn target_revision_state_must_match_logical_mutation_descriptor() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(140, 973, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let mismatched_target = revision_with_same_state(&runtime, 141, &registry);
    let before = runtime.clone_for_test();

    assert_eq!(
        runtime.prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &mismatched_target,
            mutations: &mutations,
            registry: &registry,
        }),
        Err(PhysicalExecutionError::LogicalRevisionMutationMismatch)
    );
    assert_eq!(runtime, before);
}

#[test]
fn logical_transition_validation_reconstructs_only_affected_relation() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(145, 9731, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 146, &mutations, &registry);

    let prepared = runtime
        .prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &target,
            mutations: &mutations,
            registry: &registry,
        })
        .unwrap();

    assert_eq!(prepared.source_revision(), RevisionId::new(145));
    assert_eq!(prepared.target_revision(), RevisionId::new(146));
    let rows = target.state().model.relations.get(&relation).unwrap();
    assert_eq!(rows, &vec![vec![Value::I64(2)], vec![Value::I64(3)]]);
}

#[test]
fn certified_relation_endpoint_skips_global_provenance_recheck_but_not_delta_replay() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(147, 9732, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];

    let expected = target_revision_for(&runtime, 148, &mutations, &registry);
    let expected_rows = expected.state().model.relations[&relation].to_vec();
    let mut candidate = runtime.revision().relation_update_candidate();
    candidate.replace_relation_rows(relation, expected_rows);
    let certified = candidate.build(RevisionId::new(148), &registry).unwrap();
    assert!(
        certified.certifies_relation_only_from(runtime.revision(), &BTreeSet::from([relation]),)
    );
    runtime
        .prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &certified,
            mutations: &mutations,
            registry: &registry,
        })
        .unwrap();

    let mut wrong_candidate = runtime.revision().relation_update_candidate();
    wrong_candidate.replace_relation_rows(relation, vec![vec![Value::I64(99)]]);
    let wrong = wrong_candidate
        .build(RevisionId::new(149), &registry)
        .unwrap();
    assert!(wrong.certifies_relation_only_from(runtime.revision(), &BTreeSet::from([relation]),));
    assert_eq!(
        runtime.prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &wrong,
            mutations: &mutations,
            registry: &registry,
        }),
        Err(PhysicalExecutionError::LogicalRevisionMutationMismatch),
    );
}

#[test]
fn materialization_registry_advances_all_registered_plans_atomically() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(974),
        family: LayoutFamily::Columnar,
    };
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(relation, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
    let revision = kernel_revision::Revision::build(
        RevisionId::new(150),
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
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
        )
        .unwrap();
    let scan_id = MaterializationId::new(10);
    let filter_id = MaterializationId::new(11);
    let filter_query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(2),
        equivalence: sid(101),
    };
    let runtime = RuntimeRevisionBundle::build(
        revision,
        store,
        BTreeMap::from([(relation, binding)]),
        &[
            RuntimeMaterializationSpec {
                id: scan_id,
                query: RelExpr::Scan(relation),
            },
            RuntimeMaterializationSpec {
                id: filter_id,
                query: filter_query.clone(),
            },
        ],
        &registry,
    )
    .unwrap();
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
    }];
    let target = target_revision_for(&runtime, 151, &mutations, &registry);
    let prepared = runtime
        .prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &target,
            mutations: &mutations,
            registry: &registry,
        })
        .unwrap();
    assert_eq!(
        prepared.descriptor().semantic_revision(),
        Some(context.revision())
    );
    assert_eq!(prepared.output_deltas()[&scan_id], delta);
    assert!(prepared.output_deltas()[&filter_id].is_empty());
    let cell = RuntimeRevisionCell::new(runtime);
    let emitted = prepared.seal(&cell).unwrap().publish();
    let runtime = cell.snapshot().unwrap();

    assert_eq!(emitted.len(), 2);
    assert_eq!(runtime.revision_id(), RevisionId::new(151));
    for id in [scan_id, filter_id] {
        assert_eq!(
            runtime.materialization_revision(id),
            Some(RevisionId::new(151))
        );
        assert_eq!(runtime.materialization(id).unwrap().revision(), None);
    }
    let expected_filter = filter_query
        .evaluate(&runtime.revision().state().model, &context, &registry)
        .unwrap();
    assert_eq!(
        runtime
            .materialization(filter_id)
            .unwrap()
            .output_value(&context, &registry)
            .unwrap(),
        expected_filter
    );
}

#[test]
fn duplicate_materialization_id_is_rejected_at_bootstrap() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(975),
        family: LayoutFamily::Columnar,
    };
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, vec![vec![Value::I64(1)]]);
    let revision = kernel_revision::Revision::build(
        RevisionId::new(160),
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
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1].into())]).unwrap(),
        )
        .unwrap();
    let duplicate = MaterializationId::new(99);

    assert_eq!(
        RuntimeRevisionBundle::build(
            revision,
            store,
            BTreeMap::from([(relation, binding)]),
            &[
                RuntimeMaterializationSpec {
                    id: duplicate,
                    query: RelExpr::Scan(relation),
                },
                RuntimeMaterializationSpec {
                    id: duplicate,
                    query: RelExpr::Scan(relation),
                },
            ],
            &registry,
        ),
        Err(PhysicalExecutionError::DuplicateMaterialization(duplicate))
    );
}

#[test]
fn revision_bound_states_reject_legacy_semantic_mutation_entrypoints() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(966),
        family: LayoutFamily::Columnar,
    };
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(relation, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
    let query = RelExpr::Scan(relation);
    let mut maintained =
        MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
        )
        .unwrap();
    let rows = store.logical_rows_with_handles(relation, binding).unwrap();
    maintained.attach_storage_rows(relation, &rows).unwrap();
    store.bind_revision(RevisionId::new(90)).unwrap();
    maintained.bind_revision(RevisionId::new(90)).unwrap();
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let store_before = store.clone();

    assert_eq!(
        store.apply_relation_delta(relation, binding, &delta, &context, &registry),
        Err(PhysicalExecutionError::RevisionBoundMutationRequiresPreparedTransition)
    );
    assert_eq!(store, store_before);
    assert_eq!(
        store.install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![9, 9].into())]).unwrap(),
        ),
        Err(PhysicalExecutionError::RevisionBoundMutationRequiresPreparedTransition)
    );
    assert_eq!(store, store_before);

    let resolved = StorageResolvedRelationDelta::from_parts(
        relation,
        delta,
        vec![rows[0].0],
        vec![kernel_types::StableRowHandle {
            slot: rows[0].0.slot,
            generation: rows[0].0.generation + 1,
        }],
    );
    let mut resolved_map = BTreeMap::new();
    resolved_map.insert(relation, resolved);
    let plan_before = maintained.clone();
    assert_eq!(
        maintained.apply_storage_resolved_deltas(&resolved_map, &context, &registry),
        Err(RelQueryError::RevisionBoundMutationRequiresPreparedTransition)
    );
    assert_eq!(maintained, plan_before);
}

fn two_relation_context() -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    SemanticId,
    SemanticId,
) {
    let left = sid(200);
    let right = sid(201);
    let equivalence = sid(202);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(200));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(200));
    for relation in [left, right] {
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
        left,
        right,
        equivalence,
    )
}

fn join_runtime_bundle(
    revision: u64,
) -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    SemanticId,
    LayoutBinding,
    LayoutBinding,
    RelExpr,
    RuntimeRevisionBundle,
) {
    let (context, registry, left, right, equivalence) = two_relation_context();
    let left_binding = LayoutBinding {
        id: LayoutId(980),
        family: LayoutFamily::Columnar,
    };
    let right_binding = LayoutBinding {
        id: LayoutId(981),
        family: LayoutFamily::Columnar,
    };
    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(left, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
    model
        .relations
        .insert(right, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
    let mut store = PhysicalStore::default();
    store
        .install(
            left,
            left_binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
        )
        .unwrap();
    store
        .install(
            right,
            right_binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
        )
        .unwrap();
    let revision = kernel_revision::Revision::build(
        RevisionId::new(revision),
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
        BTreeMap::from([(left, left_binding), (right, right_binding)]),
        &[RuntimeMaterializationSpec {
            id: test_materialization_id(),
            query: query.clone(),
        }],
        &registry,
    )
    .unwrap();
    (
        context,
        registry,
        left,
        right,
        left_binding,
        right_binding,
        query,
        runtime,
    )
}

fn advisor_i64_recovery_bundle(
    revision_id: u64,
    left_values: &[i64],
    right_values: &[i64],
) -> (
    SemanticRegistry,
    I64IndexBinding,
    I64IndexBinding,
    RuntimeRevisionBundle,
) {
    let (context, registry, left, right, equivalence) = two_relation_context();
    let left_layout = LayoutBinding {
        id: LayoutId(9_182),
        family: LayoutFamily::Columnar,
    };
    let right_layout = LayoutBinding {
        id: LayoutId(9_183),
        family: LayoutFamily::Columnar,
    };
    let mut model = kernel_model::FiniteModel::default();
    for (relation, values) in [(left, left_values), (right, right_values)] {
        model.relations.insert(
            relation,
            values
                .iter()
                .copied()
                .map(|value| vec![Value::I64(value)])
                .collect(),
        );
    }
    let revision = kernel_revision::Revision::build(
        RevisionId::new(revision_id),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let mut store = PhysicalStore::default();
    for (relation, layout, values) in [
        (left, left_layout, left_values),
        (right, right_layout, right_values),
    ] {
        store
            .install(
                relation,
                layout,
                NativeRelation::typed_columnar(vec![NativeColumn::I64(values.to_vec().into())])
                    .unwrap(),
            )
            .unwrap();
    }
    let left_index = I64IndexBinding {
        relation: left,
        layout: left_layout,
        key_column: 0,
        equivalence,
    };
    let right_index = I64IndexBinding {
        relation: right,
        layout: right_layout,
        key_column: 0,
        equivalence,
    };
    for binding in [left_index, right_index] {
        store
            .install_i64_index(binding, &context, &registry)
            .unwrap();
        store
            .advisor_managed_artifacts_mut()
            .insert(UnifiedArtifactId::I64Index(binding));
    }
    let root = RuntimeRevisionBundle::build(
        revision,
        store,
        BTreeMap::from([(left, left_layout), (right, right_layout)]),
        &[],
        &registry,
    )
    .unwrap();
    (registry, left_index, right_index, root)
}

