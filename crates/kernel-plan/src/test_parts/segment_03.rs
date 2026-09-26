#[test]
fn multi_relation_revision_publishes_one_coherent_join_snapshot() {
    let (context, registry, left, right, left_binding, right_binding, query, runtime) =
        join_runtime_bundle(200);
    let left_delta = scan_delta(left, &[3], &[1], &context, &registry);
    let right_delta = scan_delta(right, &[3], &[2], &context, &registry);
    let mutations = [
        RevisionRelationMutation {
            relation: left,
            delta: &left_delta,
        },
        RevisionRelationMutation {
            relation: right,
            delta: &right_delta,
        },
    ];
    let before = runtime.clone_for_test();
    let prepared = prepare_runtime_revision(&runtime, 201, &mutations, &registry).unwrap();
    assert_eq!(runtime, before);
    let cell = RuntimeRevisionCell::new(runtime);
    let _ = prepared.seal(&cell).unwrap().publish();
    let runtime = cell.snapshot().unwrap();

    let reference = query
        .evaluate(&runtime.revision().state().model, &context, &registry)
        .unwrap();
    let maintained = runtime
        .materialization(test_materialization_id())
        .unwrap()
        .output_value(&context, &registry)
        .unwrap();

    assert_eq!(runtime.revision_id(), RevisionId::new(201));
    assert_eq!(
        runtime.physical_store().revision(),
        Some(RevisionId::new(201))
    );
    assert_eq!(
        runtime.materialization_revision(test_materialization_id()),
        Some(RevisionId::new(201))
    );
    assert_eq!(
        runtime_physical_i64_values(&runtime, left, left_binding),
        vec![2, 3]
    );
    assert_eq!(
        runtime_physical_i64_values(&runtime, right, right_binding),
        vec![1, 3]
    );
    assert_eq!(maintained, reference);
    assert_eq!(maintained.rows(), &[vec![Value::I64(3), Value::I64(3)]]);
}

#[test]
fn failure_in_second_relation_of_batch_leaves_live_bundle_unchanged() {
    let (context, registry, left, right, _, _, _, runtime) = join_runtime_bundle(210);
    let left_delta = scan_delta(left, &[3], &[1], &context, &registry);
    let invalid_right_delta = scan_delta(right, &[3], &[99], &context, &registry);
    let mutations = [
        RevisionRelationMutation {
            relation: left,
            delta: &left_delta,
        },
        RevisionRelationMutation {
            relation: right,
            delta: &invalid_right_delta,
        },
    ];
    let target = revision_with_same_state(&runtime, 211, &registry);
    let before = runtime.clone_for_test();

    assert!(
        runtime
            .prepare_revision_for_test(&RevisionTransitionRequest {
                target_revision: &target,
                mutations: &mutations,
                registry: &registry,
            })
            .is_err()
    );
    assert_eq!(runtime, before);
}

#[test]
fn duplicate_relation_in_revision_batch_is_rejected_before_prepare() {
    let (context, registry, relation, _, runtime) = scan_runtime_bundle(220, 982, &[1, 2]);
    let first = scan_delta(relation, &[3], &[1], &context, &registry);
    let second = scan_delta(relation, &[4], &[2], &context, &registry);
    let mutations = [
        RevisionRelationMutation {
            relation,
            delta: &first,
        },
        RevisionRelationMutation {
            relation,
            delta: &second,
        },
    ];
    let target = revision_with_same_state(&runtime, 221, &registry);
    let before = runtime.clone_for_test();

    assert!(matches!(
        runtime.prepare_revision_for_test(&RevisionTransitionRequest {
            target_revision: &target,
            mutations: &mutations,
            registry: &registry,
        }),
        Err(PhysicalExecutionError::DuplicateRelationMutation(found)) if found == relation
    ));
    assert_eq!(runtime, before);
}

#[test]
fn invalid_relation_delta_is_rejected_before_relation_or_index_mutation() {
    let (context, registry, relation) = planning_context();
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
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2, 3].into())]).unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(index_binding, &context, &registry)
        .unwrap();
    let before = store.clone();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let missing_remove = RelationDelta {
        inserted: vec![vec![Value::I64(4)]],
        removed: vec![vec![Value::I64(99)]],
        result_type: result_type.clone(),
    };
    assert_eq!(
        store.apply_relation_delta(relation, binding, &missing_remove, &context, &registry),
        Err(RelQueryError::InconsistentIncrementalDelta.into())
    );
    assert_eq!(store, before);

    let wrong_insert = RelationDelta {
        inserted: vec![vec![Value::Text("wrong".into())]],
        removed: Vec::new(),
        result_type,
    };
    assert_eq!(
        store.apply_relation_delta(relation, binding, &wrong_insert, &context, &registry),
        Err(PhysicalExecutionError::PhysicalTypeMismatch)
    );
    assert_eq!(store, before);
}

#[test]
fn atomic_relation_delta_updates_relation_and_persisted_index_together() {
    let (context, registry, relation) = planning_context();
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let binding = LayoutBinding {
        id: LayoutId(939),
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
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 1, 2].into())]).unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(index_binding, &context, &registry)
        .unwrap();

    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let delta = RelationDelta {
        inserted: vec![vec![Value::I64(3)]],
        removed: vec![vec![Value::I64(1)]],
        result_type,
    };
    store
        .apply_relation_delta(relation, binding, &delta, &context, &registry)
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let new_rows = vec![
        vec![Value::I64(1)],
        vec![Value::I64(2)],
        vec![Value::I64(3)],
    ];
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, new_rows);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.ephemeral_index_builds, 0);
    assert_eq!(store.i64_index(index_binding).unwrap().row_count(), 3);
}

#[test]
fn relation_removal_warms_and_maintains_internal_full_row_occurrence_atom() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(1938),
        family: LayoutFamily::Columnar,
    };
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(
                (0_i64..128).collect::<Vec<_>>().into(),
            )])
            .unwrap(),
        )
        .unwrap();
    assert!(store.row_occurrence_atoms_for_test().is_empty());

    for removed in [64_i64, 96_i64] {
        let delta = RelationDelta {
            inserted: Vec::new(),
            removed: vec![vec![Value::I64(removed)]],
            result_type: result_type.clone(),
        };
        store
            .apply_relation_delta(relation, binding, &delta, &context, &registry)
            .unwrap();
        let state = store
            .row_occurrence_atoms_for_test()
            .get(&(relation, binding.id))
            .unwrap();
        assert_eq!(
            state.row_count(),
            native_row_count(&store.installed(relation, binding).unwrap().data)
        );
    }
    assert_eq!(store.row_occurrence_atoms_for_test().len(), 1);
    assert!(store.observable_atom_states_for_test().is_empty());
    assert!(
        store
            .durable_physical_artifact_specs()
            .iter()
            .all(|spec| !matches!(
                spec,
                DurablePhysicalArtifactSpec::ObservableAtom { relation: candidate, .. }
                    if *candidate == relation
            ))
    );
}

#[test]
fn maintained_i64_index_matches_fresh_rebuild_across_bag_delta_sequence() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(938),
        family: LayoutFamily::Columnar,
    };
    let index_binding = I64IndexBinding {
        relation,
        layout: binding,
        key_column: 0,
        equivalence: sid(101),
    };
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let mut values = vec![1_i64, 1, 2];
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
    let steps = [
        (vec![3_i64], vec![1_i64]),
        (Vec::new(), vec![1_i64]),
        (vec![2_i64, 4_i64], Vec::new()),
        (vec![1_i64], vec![3_i64, 2_i64]),
    ];
    for (inserted, removed) in steps {
        for value in &removed {
            let position = values
                .iter()
                .position(|candidate| candidate == value)
                .unwrap();
            values.remove(position);
        }
        values.extend(inserted.iter().copied());
        let delta = RelationDelta {
            inserted: inserted
                .into_iter()
                .map(|value| vec![Value::I64(value)])
                .collect(),
            removed: removed
                .into_iter()
                .map(|value| vec![Value::I64(value)])
                .collect(),
            result_type: result_type.clone(),
        };
        store
            .apply_relation_delta(relation, binding, &delta, &context, &registry)
            .unwrap();

        let current =
            NativeRelation::typed_columnar(vec![NativeColumn::I64(values.clone().into())]).unwrap();
        let rebuilt_relation = InstalledRelation::new(current);
        let rebuilt =
            build_i64_index_state_for_test(index_binding, &rebuilt_relation, &context, &registry)
                .unwrap();
        let maintained = store.i64_index(index_binding).unwrap();
        assert_eq!(maintained.row_count(), rebuilt.row_count());
        assert_eq!(
            maintained.bucket_keys(),
            rebuilt.bucket_keys()
        );
        for key in rebuilt.bucket_keys() {
            assert_eq!(
                maintained.probe_len_for_test(key).unwrap(),
                rebuilt.probe_len_for_test(key).unwrap()
            );
        }
    }
}

#[test]
fn stable_row_handles_survive_position_shifts_without_payload_aliasing() {
    let relation = sid(360);
    let equivalence = sid(361);
    let (context, registry) = two_i64_column_context(relation, equivalence, 36);
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let binding = LayoutBinding {
        id: LayoutId(940),
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
    let mut rows = vec![(1_i64, 10_i64), (1, 20), (2, 30), (3, 40)];
    rows.extend((5_i64..=64).map(|key| (key, 1_000 + key)));
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(rows.iter().map(|row| row.0).collect()),
                NativeColumn::I64(rows.iter().map(|row| row.1).collect()),
            ])
            .unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(index_binding, &context, &registry)
        .unwrap();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();

    for (inserted, removed) in [
        (None, Some((1_i64, 10_i64))),
        (None, Some((2_i64, 30_i64))),
        (Some((1_i64, 50_i64)), None),
    ] {
        if let Some(row) = removed {
            let position = rows.iter().position(|candidate| *candidate == row).unwrap();
            rows.remove(position);
        }
        if let Some(row) = inserted {
            rows.push(row);
        }
        let delta = RelationDelta {
            inserted: inserted
                .into_iter()
                .map(|(key, payload)| vec![Value::I64(key), Value::I64(payload)])
                .collect(),
            removed: removed
                .into_iter()
                .map(|(key, payload)| vec![Value::I64(key), Value::I64(payload)])
                .collect(),
            result_type: result_type.clone(),
        };
        store
            .apply_relation_delta(relation, binding, &delta, &context, &registry)
            .unwrap();

        let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
        let mut model = kernel_model::FiniteModel::default();
        model.relations.insert(
            relation,
            rows.iter()
                .map(|(key, payload)| vec![Value::I64(*key), Value::I64(*payload)])
                .collect(),
        );
        let reference = logical.evaluate(&model, &context, &registry).unwrap();
        assert_eq!(native, reference);
        assert_eq!(stats.persisted_index_hits, 1);
        assert_eq!(stats.ephemeral_index_builds, 0);
    }
}

#[test]
fn dense_slot_delete_repairs_only_the_swapped_row_position() {
    let relation = sid(362);
    let equivalence = sid(363);
    let (context, registry) = two_i64_column_context(relation, equivalence, 37);
    let binding = LayoutBinding {
        id: LayoutId(941),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64((0_i64..8).collect()),
                NativeColumn::I64((100_i64..108).collect()),
            ])
            .unwrap(),
        )
        .unwrap();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let delta = RelationDelta {
        inserted: Vec::new(),
        removed: vec![vec![Value::I64(2), Value::I64(102)]],
        result_type,
    };
    store
        .apply_relation_delta(relation, binding, &delta, &context, &registry)
        .unwrap();

    let installed = store.installed(relation, binding).unwrap();
    assert_eq!(
        installed.position(PhysicalRowId {
            slot: 2,
            generation: 0
        }),
        None
    );
    assert_eq!(
        installed.position(PhysicalRowId {
            slot: 7,
            generation: 0
        }),
        Some(2)
    );
    assert_eq!(
        installed.position(PhysicalRowId {
            slot: 3,
            generation: 0
        }),
        Some(3)
    );
    assert_eq!(
        installed.position(PhysicalRowId {
            slot: 6,
            generation: 0
        }),
        Some(6)
    );
    assert_eq!(
        installed.scan_positions().collect::<Vec<_>>(),
        vec![0, 1, 3, 4, 5, 6, 2]
    );
}

#[test]
fn reinstalling_physical_relation_invalidates_persisted_i64_index() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(937),
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
    assert!(
        store
            .i64_index(I64IndexBinding {
                relation,
                layout: binding,
                key_column: 0,
                equivalence: sid(101)
            })
            .is_some()
    );
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![3, 4].into())]).unwrap(),
        )
        .unwrap();
    assert!(
        store
            .i64_index(I64IndexBinding {
                relation,
                layout: binding,
                key_column: 0,
                equivalence: sid(101)
            })
            .is_none()
    );
}

#[test]
fn adaptive_indexed_join_falls_back_for_non_i64_columnar_keys() {
    let relation = sid(340);
    let equivalence = sid(341);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(34));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(34));
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
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let binding = LayoutBinding {
        id: LayoutId(935),
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
    let (native, _) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        values
            .into_iter()
            .map(|value| vec![Value::Text(value)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
}

#[test]
fn large_text_join_uses_canonical_bucket_without_one_shot_semantic_state() {
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
        id: LayoutId(936),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut values = vec!["Needle".to_owned(), "needle".to_owned()];
    values.extend((2..128).map(|value| format!("key-{value}")));
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
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(native.rows().len(), 130);
    assert_eq!(stats.persisted_index_hits, 0);
    assert_eq!(stats.ephemeral_index_builds, 0);
}

fn text_semantic_index_fixture() -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    SemanticId,
    LayoutBinding,
    PhysicalStore,
) {
    let relation = sid(342);
    let equivalence = sid(343);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(35));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(35));
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
        id: LayoutId(937),
        family: LayoutFamily::Columnar,
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(
                vec!["A".into(), "a".into(), "B".into()].into(),
            )])
            .unwrap(),
        )
        .unwrap();
    store
        .install_semantic_index(
            SemanticIndexBinding::single(relation, binding, 0, equivalence),
            &context,
            &registry,
        )
        .unwrap();
    (context, registry, relation, equivalence, binding, store)
}

#[test]
fn observable_atom_state_matches_legacy_semantic_index_and_statistics_views() {
    let (context, registry, relation, equivalence, layout, mut store) =
        text_semantic_index_fixture();
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();

    let value = Value::Text("a".into());
    let mut legacy = store
        .semantic_index(&binding)
        .unwrap()
        .probe_value(&value, &context, &registry)
        .unwrap()
        .unwrap()
        .iter()
        .copied()
        .collect::<Vec<_>>();
    let mut atom_rows = store
        .observable_atom_probe_value(&binding, 0, &value, &context, &registry)
        .unwrap();
    legacy.sort_unstable();
    atom_rows.sort_unstable();
    assert_eq!(atom_rows, legacy);
    assert_eq!(
        store
            .observable_atom_count_value(&binding, 0, &value, &context, &registry)
            .unwrap(),
        2
    );
    let state = store.observable_atom_state(&binding).unwrap();
    assert_eq!(state.row_count(), 3);
    assert_eq!(state.atom_count(), 2);
    let violations = state.uniqueness_violation_measure().unwrap();
    assert_eq!(violations.witness_count(), 1);
    assert_eq!(violations.iter().next().map(|(_, mass)| mass), Some(1));

    // SAMF is now an execution capability in its own right. Legacy index/statistics
    // objects remain independent adapters/oracles rather than owners of the SAMF state.
    store.remove_semantic_index(&binding);
    store.remove_semantic_statistics(&binding);
    assert_eq!(
        store
            .semantic_statistics(&binding, &context, &registry)
            .unwrap(),
        Some(SemanticKeyStatistics {
            row_count: 3,
            distinct_key_count: 2,
        })
    );
    let first_row = store
        .installed(relation, layout)
        .unwrap()
        .row_id_at(0)
        .unwrap();
    assert!(
        store
            .semantic_quotient_single_key(&binding, first_row, &context, &registry)
            .unwrap()
            .is_some()
    );
}

#[test]
fn observable_atom_state_maintains_exact_fibers_through_relation_delta() {
    let (context, registry, relation, equivalence, layout, mut store) =
        text_semantic_index_fixture();
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    store
        .apply_relation_delta(
            relation,
            layout,
            &RelationDelta {
                inserted: vec![vec![Value::Text("C".into())]],
                removed: vec![vec![Value::Text("A".into())]],
                result_type,
            },
            &context,
            &registry,
        )
        .unwrap();

    assert_eq!(
            store
                .observable_atom_count_value(
                    &binding,
                    0,
                    &Value::Text("a".into()),
                    &context,
                    &registry,
                )
                .unwrap(),
            1
        );
    assert_eq!(
            store
                .observable_atom_count_value(
                    &binding,
                    0,
                    &Value::Text("c".into()),
                    &context,
                    &registry,
                )
                .unwrap(),
            1
        );
    assert_eq!(
        store.observable_atom_state(&binding).unwrap().row_count(),
        3
    );
}

#[test]
fn observable_atom_state_is_invalidated_on_layout_replacement() {
    let (context, registry, relation, equivalence, layout, mut store) =
        text_semantic_index_fixture();
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![NativeColumn::Text(vec!["z".into()].into())])
                .unwrap(),
        )
        .unwrap();
    assert!(store.observable_atom_state(&binding).is_none());
}

#[test]
fn observable_atom_state_rejects_gamma_drift_before_relation_mutation() {
    let (context, mut registry, relation, equivalence, layout, mut store) =
        text_semantic_index_fixture();
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();
    store.remove_semantic_index(&binding);
    let before = store.clone();

    let mut changed = context.clone();
    let exact = registry.install_equivalence(EquivalenceModule::TextExact);
    changed.environment.pin_module(equivalence, exact);
    let result_type = RelExpr::Scan(relation)
        .typecheck(&changed, &registry)
        .unwrap();
    assert!(
        store
            .semantic_statistics(&binding, &changed, &registry)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        store.apply_relation_delta(
            relation,
            layout,
            &RelationDelta {
                inserted: vec![vec![Value::Text("C".into())]],
                removed: Vec::new(),
                result_type,
            },
            &changed,
            &registry,
        ),
        Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild)
    ));
    assert_eq!(store, before);
}

#[test]
fn observable_atom_state_factors_multi_column_joint_and_projected_fibers() {
    let relation = sid(9_978_100);
    let equivalence = sid(9_978_101);
    let (context, registry) = two_i64_column_context(relation, equivalence, 9_978_100);
    let layout = LayoutBinding {
        id: LayoutId(9_978_100),
        family: LayoutFamily::Columnar,
    };
    let binding = SemanticIndexBinding {
        relation,
        layout,
        key_parts: vec![
            SemanticIndexKeyPart {
                column: 0,
                equivalence,
            },
            SemanticIndexKeyPart {
                column: 1,
                equivalence,
            },
        ],
    };
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(vec![1, 1, 2].into()),
                NativeColumn::I64(vec![10, 20, 10].into()),
            ])
            .unwrap(),
        )
        .unwrap();
    store
        .install_observable_atom_state(binding.clone(), &context, &registry)
        .unwrap();

    let joint = store
        .observable_atom_probe_values(
            &binding,
            &[&Value::I64(1), &Value::I64(10)],
            &context,
            &registry,
        )
        .unwrap()
        .unwrap();
    assert_eq!(joint.len(), 1);
    assert_eq!(
        store
            .observable_atom_probe_value(&binding, 0, &Value::I64(1), &context, &registry)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .observable_atom_probe_value(&binding, 1, &Value::I64(10), &context, &registry)
            .unwrap()
            .len(),
        2
    );
    let state = store.observable_atom_state(&binding).unwrap();
    assert_eq!(state.atom_count(), 3);
    assert_eq!(
        state.product_projection().source(),
        &[state.product_observable()]
    );
    assert_eq!(state.product_projection().target().len(), 2);
}

#[test]
fn semantic_statistics_are_invalidated_by_gamma_change() {
    let (context, mut registry, relation, equivalence, layout, mut store) =
        text_semantic_index_fixture();
    let binding = SemanticIndexBinding::single(relation, layout, 0, equivalence);
    store.remove_semantic_index(&binding);
    let statistics = store
        .install_semantic_statistics(binding.clone(), &context, &registry)
        .unwrap();
    assert_eq!(statistics.distinct_key_count, 2);

    let mut changed_context = context.clone();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    changed_context
        .environment
        .pin_module(equivalence, exact_digest);
    assert_eq!(
        store
            .semantic_statistics(&binding, &changed_context, &registry)
            .unwrap(),
        None
    );
    let result_type = RelExpr::Scan(relation)
        .typecheck(&changed_context, &registry)
        .unwrap();
    let delta = RelationDelta {
        inserted: vec![vec![Value::Text("C".into())]],
        removed: Vec::new(),
        result_type,
    };
    assert!(matches!(
        store.apply_relation_delta(relation, layout, &delta, &changed_context, &registry),
        Err(PhysicalExecutionError::SemanticContextTransitionRequiresRebuild)
    ));
}

#[test]
fn join_reuses_tiny_persisted_semantic_index_while_filter_can_still_reject_it() {
    let (context, registry, relation, equivalence, binding, store) = text_semantic_index_fixture();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let join = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let filter = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::Text("a".into()),
        equivalence,
    };
    let prepared_join = prepare_with_catalog(join, &context, &registry, &catalog).unwrap();
    let prepared_filter = prepare_with_catalog(filter, &context, &registry, &catalog).unwrap();
    let (join_value, join_stats) = prepared_join
        .execute_native_pinned(&store, &registry)
        .unwrap();
    assert_eq!(join_value.rows().len(), 5);
    assert_eq!(join_stats.persisted_index_hits, 1);
    assert_eq!(join_stats.persisted_index_cost_rejections, 0);
    assert_eq!(join_stats.ephemeral_index_builds, 0);
    let (filter_value, filter_stats) = prepared_filter
        .execute_native_pinned(&store, &registry)
        .unwrap();
    assert_eq!(filter_value.rows().len(), 2);
    assert_eq!(filter_stats.persisted_index_hits, 0);
    assert_eq!(filter_stats.persisted_index_cost_rejections, 1);
    assert_eq!(filter_stats.scanned_rows, 3);
}

#[test]
fn join_reuses_tiny_persisted_i64_index_instead_of_rebuilding_bucket() {
    let (context, registry, relation) = planning_context();
    let binding = LayoutBinding {
        id: LayoutId(997),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: sid(101),
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![NativeColumn::I64(vec![1, 2].into())]).unwrap(),
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
    let mut model = kernel_model::FiniteModel::default();
    model
        .relations
        .insert(relation, vec![vec![Value::I64(1)], vec![Value::I64(2)]]);
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.persisted_index_cost_rejections, 0);
    assert_eq!(stats.ephemeral_index_builds, 0);
}

#[test]
fn join_batch_reuses_tiny_persisted_i64_index() {
    let relation = sid(346);
    let equivalence = sid(347);
    let (context, registry) = two_i64_column_context(relation, equivalence, 43);
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(relation)),
            right: Box::new(RelExpr::Scan(relation)),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        columns: vec![1, 3],
    };
    let binding = LayoutBinding {
        id: LayoutId(998),
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
                NativeColumn::I64(vec![1, 2].into()),
                NativeColumn::I64(vec![10, 20].into()),
            ])
            .unwrap(),
        )
        .unwrap();
    store
        .install_i64_index(
            I64IndexBinding {
                relation,
                layout: binding,
                key_column: 0,
                equivalence,
            },
            &context,
            &registry,
        )
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        vec![
            vec![Value::I64(1), Value::I64(10)],
            vec![Value::I64(2), Value::I64(20)],
        ],
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.ephemeral_index_builds, 0);
    assert_eq!(stats.fused_join_project_hits, 1);
    assert_eq!(stats.persisted_index_cost_rejections, 0);
}

#[test]
fn join_project_reuses_persisted_semantic_index_before_building_ephemeral_i64() {
    let relation = sid(348);
    let equivalence = sid(349);
    let (context, registry) = two_i64_column_context(relation, equivalence, 44);
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(relation)),
            right: Box::new(RelExpr::Scan(relation)),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        columns: vec![1, 3],
    };
    let layout = LayoutBinding {
        id: LayoutId(999),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let keys = (0_i64..128).collect::<Vec<_>>();
    let payload = (1_000_i64..1_128).collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(keys.clone().into()),
                NativeColumn::I64(payload.clone().into()),
            ])
            .unwrap(),
        )
        .unwrap();
    store
        .install_semantic_index(
            SemanticIndexBinding::single(relation, layout, 0, equivalence),
            &context,
            &registry,
        )
        .unwrap();

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        keys.into_iter()
            .zip(payload)
            .map(|(key, value)| vec![Value::I64(key), Value::I64(value)])
            .collect(),
    );
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.persisted_index_hits, 1);
    assert_eq!(stats.ephemeral_index_builds, 0);
    assert_eq!(stats.fused_join_project_hits, 0);
}

#[test]
fn duplicate_heavy_join_project_keeps_admitted_transient_index_after_build() {
    let relation = sid(350);
    let equivalence = sid(351);
    let (context, registry) = two_i64_column_context(relation, equivalence, 45);
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::Scan(relation)),
            right: Box::new(RelExpr::Scan(relation)),
            left_column: 0,
            right_column: 0,
            equivalence,
        }),
        columns: vec![1, 3],
    };
    let layout = LayoutBinding {
        id: LayoutId(1_000),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let keys = vec![1_i64; 50];
    let payload = (0_i64..50).collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(keys.clone().into()),
                NativeColumn::I64(payload.clone().into()),
            ])
            .unwrap(),
        )
        .unwrap();
    let mut selection_stats = ExecutionStats::default();
    assert!(
        join_batch_selection_is_ephemeral_for_test(
            prepared.physical(),
            &store,
            &context,
            &registry,
            &mut selection_stats,
        )
        .unwrap()
    );
    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(native.rows().len(), 2_500);
    assert_eq!(stats.ephemeral_index_builds, 1);
    assert_eq!(stats.fused_join_project_hits, 1);
}

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

#[test]
fn non_contiguous_three_way_join_uses_order_preserving_gamma_quotients() {
    let (context, registry, [a, b, c], equivalence) = three_relation_i64_context();
    let bindings = [
        LayoutBinding {
            id: LayoutId(1_061),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1_062),
            family: LayoutFamily::Columnar,
        },
        LayoutBinding {
            id: LayoutId(1_063),
            family: LayoutFamily::Columnar,
        },
    ];
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

    let (native, stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(native, reference);
    assert_eq!(stats.multiway_join_apnf_executions, 1);
    assert_eq!(stats.scanned_rows, 140);
}

struct FourWaySubsetFixture {
    context: SemanticContext,
    registry: SemanticRegistry,
    relations: [SemanticId; 4],
    equivalence: SemanticId,
    bindings: [LayoutBinding; 4],
    logical: RelExpr,
    prepared: PreparedPlan,
}

fn four_way_subset_fixture() -> FourWaySubsetFixture {
    let relations = [sid(720), sid(721), sid(722), sid(723)];
    let equivalence = sid(724);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(720));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(720));
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
    let bindings = [1_081_u128, 1_082, 1_083, 1_084].map(|id| LayoutBinding {
        id: LayoutId(id),
        family: LayoutFamily::Columnar,
    });
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.into_iter().zip(bindings) {
        catalog.bind_relation(relation, binding);
    }
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::JoinEq {
            left: Box::new(RelExpr::JoinEq {
                left: Box::new(RelExpr::Scan(relations[0])),
                right: Box::new(RelExpr::Scan(relations[1])),
                left_column: 0,
                right_column: 0,
                equivalence,
            }),
            right: Box::new(RelExpr::Scan(relations[2])),
            left_column: 1,
            right_column: 1,
            equivalence,
        }),
        right: Box::new(RelExpr::Scan(relations[3])),
        left_column: 3,
        right_column: 1,
        equivalence,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    FourWaySubsetFixture {
        context,
        registry,
        relations,
        equivalence,
        bindings,
        logical,
        prepared,
    }
}

fn install_two_i64_fixture_relation(
    store: &mut PhysicalStore,
    model: &mut kernel_model::FiniteModel,
    relation: SemanticId,
    layout: LayoutBinding,
    first: Vec<i64>,
    second: Vec<i64>,
) {
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(first.clone().into()),
                NativeColumn::I64(second.clone().into()),
            ])
            .unwrap(),
        )
        .unwrap();
    model.relations.insert(
        relation,
        first
            .into_iter()
            .zip(second)
            .map(|(first, second)| vec![Value::I64(first), Value::I64(second)])
            .collect(),
    );
}


#[test]
fn gamma_quotient_execution_uses_nested_exact_and_coarser_semantic_coordinates() {
    let relations = [sid(730), sid(731), sid(732)];
    let exact = sid(733);
    let ascii_ci = sid(734);
    let mut registry = SemanticRegistry::default();
    let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(74));
    environment.pin_module(exact, exact_digest);
    environment.pin_module(ascii_ci, ci_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(74));
    for (relation, equivalence) in relations.into_iter().zip([exact, exact, ascii_ci]) {
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
    let layouts = [1_091_u128, 1_092, 1_093].map(|id| LayoutBinding {
        id: LayoutId(id),
        family: LayoutFamily::Columnar,
    });
    let mut catalog = PhysicalCatalog::default();
    for (relation, layout) in relations.into_iter().zip(layouts) {
        catalog.bind_relation(relation, layout);
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
        left_column: 1,
        right_column: 0,
        equivalence: ascii_ci,
    };
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let values = [vec!["X", "x"], vec!["X", "Y"], vec!["x", "y"]];
    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for ((relation, layout), values) in relations.into_iter().zip(layouts).zip(values) {
        let owned = values.into_iter().map(str::to_owned).collect::<Vec<_>>();
        store
            .install(
                relation,
                layout,
                NativeRelation::typed_columnar(vec![NativeColumn::Text(owned.clone().into())])
                    .unwrap(),
            )
            .unwrap();
        model.relations.insert(
            relation,
            owned
                .into_iter()
                .map(|value| vec![Value::Text(value)])
                .collect(),
        );
    }
    let mut stats = ExecutionStats::default();
    let native = execute_order_preserving_quotient_join_for_test(
        prepared.physical(),
        None,
        &store,
        &context,
        &registry,
        &mut stats,
    )
    .unwrap()
    .unwrap();
    let reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(kernel_query::RelationValue::Bag(native), reference);
    assert_eq!(stats.multiway_join_semantic_quotient_constraints, 2);
}


#[test]
fn execution_stats_merge_preserves_cyclic_qcn_telemetry() {
    let mut target = ExecutionStats {
        multiway_join_cyclic_budget_rejections: 2,
        multiway_join_semantic_quotient_candidate_visits: 3,
        ..ExecutionStats::default()
    };
    merge_execution_stats_for_test(
        &mut target,
        ExecutionStats {
            multiway_join_cyclic_budget_rejections: 5,
            multiway_join_semantic_quotient_candidate_visits: 7,
            ..ExecutionStats::default()
        },
    );
    assert_eq!(target.multiway_join_cyclic_budget_rejections, 7);
    assert_eq!(target.multiway_join_semantic_quotient_candidate_visits, 10);
}


fn n_way_i64_context(
    leaf_count: usize,
    relation_base: u64,
    equivalence: SemanticId,
) -> (SemanticContext, SemanticRegistry, Vec<SemanticId>) {
    let relations = (0..leaf_count)
        .map(|offset| sid(relation_base + offset as u64))
        .collect::<Vec<_>>();
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(relation_base));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(relation_base));
    for relation in &relations {
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
    (
        SemanticContext {
            schema,
            environment,
        },
        registry,
        relations,
    )
}

fn left_deep_equal_join(relations: &[SemanticId], equivalence: SemanticId) -> RelExpr {
    let mut logical = RelExpr::Scan(relations[0]);
    for relation in relations.iter().copied().skip(1) {
        logical = RelExpr::JoinEq {
            left: Box::new(logical),
            right: Box::new(RelExpr::Scan(relation)),
            left_column: 0,
            right_column: 0,
            equivalence,
        };
    }
    logical
}

fn bushy_equal_join(relations: &[SemanticId], equivalence: SemanticId) -> RelExpr {
    if relations.len() == 1 {
        return RelExpr::Scan(relations[0]);
    }
    let split = relations.len() / 2;
    RelExpr::JoinEq {
        left: Box::new(bushy_equal_join(&relations[..split], equivalence)),
        right: Box::new(bushy_equal_join(&relations[split..], equivalence)),
        left_column: 0,
        right_column: 0,
        equivalence,
    }
}

#[test]
fn nine_way_acyclic_quotient_join_preserves_bag_order_and_multiplicity() {
    const LEAVES: usize = 9;
    let equivalence = sid(780);
    let (context, registry, relations) = n_way_i64_context(LEAVES, 760, equivalence);
    let bindings = (0..LEAVES)
        .map(|offset| LayoutBinding {
            id: LayoutId(1_300 + offset as u128),
            family: LayoutFamily::Columnar,
        })
        .collect::<Vec<_>>();
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings.iter().copied()) {
        catalog.bind_relation(relation, binding);
    }
    let logical = left_deep_equal_join(&relations, equivalence);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let program = prepared.semantic_quotient_program.as_ref().unwrap();
    assert_eq!(program.hypergraph_order.as_ref().unwrap().len(), LEAVES);

    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for (index, (relation, binding)) in relations
        .iter()
        .copied()
        .zip(bindings.iter().copied())
        .enumerate()
    {
        let values = if index == 0 {
            vec![1_i64, 1, 2]
        } else {
            vec![1_i64, 2]
        };
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
    assert_eq!(native.rows().len(), 3);
    assert_eq!(stats.multiway_join_apnf_executions, 1);
    assert_eq!(stats.multiway_join_prepared_quotient_hits, 0);
    assert_eq!(stats.scanned_rows, 19);

    let (fallback, fallback_stats) = prepared
        .physical()
        .execute_native_with_prepared_programs(
            &store,
            prepared.result_type(),
            prepared.semantic_context(),
            &registry,
            None,
            None,
        )
        .unwrap();
    assert_eq!(fallback, native);
    assert_eq!(
        fallback_stats.multiway_join_order_preserving_enumerations,
        0
    );

    assert!(
        prepared
            .materialize_semantic_quotient_support(&mut store, &registry)
            .unwrap()
    );
    let (cached, cached_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    assert_eq!(cached, reference);
    assert_eq!(
        cached_stats.multiway_join_maintained_quotient_support_hits,
        1
    );

    let target = relations[LEAVES - 1];
    let delta = scan_delta(target, &[3], &[2], &context, &registry);
    store
        .apply_relation_delta(target, bindings[LEAVES - 1], &delta, &context, &registry)
        .unwrap();
    model
        .relations
        .insert(target, vec![vec![Value::I64(1)], vec![Value::I64(3)]]);
    let (after, after_stats) = prepared.execute_native_pinned(&store, &registry).unwrap();
    let after_reference = logical.evaluate(&model, &context, &registry).unwrap();
    assert_eq!(after, after_reference);
    assert_eq!(after.rows().len(), 2);
    assert_eq!(
        after_stats.multiway_join_maintained_quotient_support_hits,
        1
    );
}

#[test]
fn nine_way_bushy_quotient_join_preserves_logical_bag_order() {
    const LEAVES: usize = 9;
    let equivalence = sid(1_790);
    let (context, registry, relations) = n_way_i64_context(LEAVES, 1_760, equivalence);
    let bindings = (0..LEAVES)
        .map(|offset| LayoutBinding {
            id: LayoutId(1_760 + offset as u128),
            family: LayoutFamily::Columnar,
        })
        .collect::<Vec<_>>();
    let mut catalog = PhysicalCatalog::default();
    for (relation, binding) in relations.iter().copied().zip(bindings.iter().copied()) {
        catalog.bind_relation(relation, binding);
    }
    let logical = bushy_equal_join(&relations, equivalence);
    let prepared = prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    assert_eq!(
        prepared
            .semantic_quotient_program
            .as_ref()
            .and_then(|program| program.hypergraph_order.as_ref())
            .map(Vec::len),
        Some(LEAVES)
    );

    let mut store = PhysicalStore::default();
    let mut model = kernel_model::FiniteModel::default();
    for (index, (relation, binding)) in relations
        .iter()
        .copied()
        .zip(bindings.iter().copied())
        .enumerate()
    {
        let values = if index == 0 {
            vec![1_i64, 1, 2]
        } else if index % 2 == 0 {
            vec![2_i64, 1]
        } else {
            vec![1_i64, 2]
        };
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
    assert_eq!(native.rows().len(), 3);
    assert_eq!(stats.multiway_join_apnf_executions, 1);
    assert_eq!(stats.multiway_join_prepared_quotient_hits, 0);
}

