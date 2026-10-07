#[test]
fn stale_observable_core_falls_back_to_exact_rebuild() {
    let dir = durable_test_dir("observable-core-stale-fallback-pass84");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_474, 9_474, &[1, 2, 3]);
    let binding = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    root.physical_store_mut_for_test()
        .install_observable_atom_state(binding, &context, &registry)
        .unwrap();
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    runtime.checkpoint().unwrap();
    drop(runtime);

    let (durability, scan) = DurableRevisionStore::open(&dir).unwrap();
    let mut stale_cores = durability.artifact_cores().to_vec();
    for core in &mut stale_cores {
        let DurableArtifactCore::ObservableAtom {
            source_revision, ..
        } = core;
        *source_revision = RevisionId::new(1);
    }
    let materializations = durability
        .materialization_specs()
        .iter()
        .map(|spec| RuntimeMaterializationSpec {
            id: spec.id,
            query: spec.query.clone(),
        })
        .collect::<Vec<_>>();
    let (_, report) = recover_runtime_bundle_with_policy_and_cores(
        durability.checkpoint_revision(),
        &scan,
        &materializations,
        durability.physical_artifact_specs(),
        &stale_cores,
        PhysicalRecoveryPolicy::default(),
        durability.semantic_registry(),
    )
    .unwrap();
    assert!(report.rehydrated.is_empty());
    assert!(matches!(
        report.rebuilt.as_slice(),
        [DurablePhysicalArtifactSpec::ObservableAtom { .. }]
    ));
    drop(durability);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn wal_core_replay_uses_relation_semantics_for_coarse_first_match_removal() {
    let dir = durable_test_dir("observable-core-coarse-removal-pass84");
    let (context, registry, relation, layout, binding, runtime) =
        coarse_observable_core_fixture(&dir);
    runtime.checkpoint().unwrap();

    let delta = RelationDelta {
        inserted: vec![vec![Value::Text("Gamma".into())]],
        removed: vec![vec![Value::Text("alpha".into())]],
        result_type: RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap(),
    };
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    runtime
        .commit_derived_relation_data(
            ClientTransactionId::new(0x5372),
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(5_372),
                target_revision: RevisionId::new(5_373),
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
    assert_eq!(report.rehydrated.len(), 1);
    assert_eq!(report.attempted_rebuild_key_evaluations, 0);
    let snapshot = reopened.snapshot().unwrap();
    let atom = snapshot
        .physical_store()
        .observable_atom_states_for_test()
        .get(&binding)
        .unwrap();
    assert_eq!(atom.row_count(), 3);
    assert_eq!(atom.distinct_key_count(), 3);
    let installed = snapshot
        .physical_store()
        .installed(relation, layout)
        .unwrap();
    let surviving_rows = installed
        .scan_positions()
        .map(|position| materialize_native_row(&installed.data, position).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        surviving_rows,
        vec![
            vec![Value::Text("ALPHA".into())],
            vec![Value::Text("Beta".into())],
            vec![Value::Text("Gamma".into())],
        ]
    );
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bounded_physical_recovery_prioritizes_manual_pin_over_advisor_recipe() {
    let dir = durable_test_dir("bounded-physical-recovery-priority");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_175, 9_175, &(1_i64..=16).collect::<Vec<_>>());
    let semantic = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    root.physical_store_mut_for_test()
        .install_observable_atom_state(semantic.clone(), &context, &registry)
        .unwrap();
    let i64 = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    root.physical_store_mut_for_test()
        .install_i64_index(i64, &context, &registry)
        .unwrap();
    root.physical_store_mut_for_test()
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::I64Index(i64));
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());

    let (reopened, report) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_key_evaluations: 0,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert!(
        snapshot
            .physical_store()
            .observable_atom_state(&semantic)
            .is_some()
    );
    assert!(snapshot.physical_store().i64_index(i64).is_none());
    assert!(report.rebuilt.is_empty());
    assert!(matches!(
        report.rehydrated.as_slice(),
        [DurablePhysicalArtifactSpec::ObservableAtom {
            advisor_managed: false,
            ..
        }]
    ));
    assert_eq!(report.skipped_key_evaluation_budget.len(), 1);
    assert!(matches!(
        report.skipped_key_evaluation_budget.as_slice(),
        [DurablePhysicalArtifactSpec::I64Index {
            advisor_managed: true,
            ..
        }]
    ));
    assert_eq!(report.attempted_rebuild_key_evaluations, 0);
    assert_eq!(report.advisor_rebuild_key_evaluations, 0);
    assert_eq!(snapshot.revision_id(), RevisionId::new(5_175));
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn physical_recovery_byte_budget_drops_optional_artifact_but_keeps_revision() {
    let dir = durable_test_dir("bounded-physical-recovery-bytes");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_176, 9_176, &[1, 2, 3, 4]);
    let semantic = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    root.physical_store_mut_for_test()
        .install_observable_atom_state(semantic.clone(), &context, &registry)
        .unwrap();
    root.physical_store_mut_for_test()
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::semantic_observable(semantic.clone()));
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());

    let (reopened, report) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_total_estimated_bytes: 0,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision_id(), RevisionId::new(5_176));
    assert!(
        snapshot
            .physical_store()
            .observable_atom_state(&semantic)
            .is_none()
    );
    assert!(report.rebuilt.is_empty());
    assert_eq!(report.skipped_estimated_byte_budget.len(), 1);
    assert_eq!(report.attempted_rebuild_key_evaluations, 0);
    assert!(report.total_estimated_bytes_after > 0);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn supervisor_reuses_bounded_physical_recovery_policy_on_reopen() {
    let dir = durable_test_dir("bounded-supervisor-recovery-policy");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_177, 9_177, &[1, 2, 3, 4]);
    let i64 = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    root.physical_store_mut_for_test()
        .install_i64_index(i64, &context, &registry)
        .unwrap();
    root.physical_store_mut_for_test()
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::I64Index(i64));
    let policy = PhysicalRecoveryPolicy {
        max_advisor_rebuild_key_evaluations: 0,
        ..PhysicalRecoveryPolicy::default()
    };
    let supervisor =
        DurableRuntimeSupervisor::create_with_recovery_policy(root, &dir, policy, &registry)
            .unwrap();

    let first = supervisor.recover_with_report().unwrap();
    assert_eq!(first.skipped_key_evaluation_budget.len(), 1);
    assert!(
        supervisor
            .snapshot()
            .unwrap()
            .physical_store()
            .i64_index(i64)
            .is_none()
    );
    let second = supervisor.recover_with_report().unwrap();
    assert_eq!(second.skipped_key_evaluation_budget.len(), 1);
    assert!(
        supervisor
            .snapshot()
            .unwrap()
            .physical_store()
            .i64_index(i64)
            .is_none()
    );
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deferred_advisor_recovery_can_resume_after_runtime_starts_serving() {
    let dir = durable_test_dir("deferred-physical-recovery-resume");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_178, 9_178, &[1, 2, 3, 4]);
    let i64 = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    root.physical_store_mut_for_test()
        .install_i64_index(i64, &context, &registry)
        .unwrap();
    root.physical_store_mut_for_test()
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::I64Index(i64));
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());

    let (runtime, initial) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_key_evaluations: 0,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(initial.deferred_advisor_artifacts().len(), 1);
    let before = runtime.snapshot().unwrap();
    let source_version = before.root_version();
    assert_eq!(before.revision_id(), RevisionId::new(5_178));
    assert!(before.physical_store().i64_index(i64).is_none());
    drop(before);

    let resumed = runtime
        .resume_deferred_physical_recovery(&initial, PhysicalRecoveryPolicy::default())
        .unwrap();
    assert_eq!(resumed.rebuilt.len(), 1);
    assert!(resumed.deferred_advisor_artifacts().is_empty());
    let after = runtime.snapshot().unwrap();
    assert_eq!(after.revision_id(), RevisionId::new(5_178));
    assert_eq!(after.root_version().raw(), source_version.raw() + 1);
    assert!(after.physical_store().i64_index(i64).is_some());
    assert!(
        after
            .physical_store()
            .advisor_managed_artifacts_for_test()
            .contains(&UnifiedArtifactId::I64Index(i64))
    );
    drop(after);

    runtime.checkpoint().unwrap();
    drop(runtime);
    let (reopened, second) =
        DurableRuntime::open_with_recovery_policy(&dir, PhysicalRecoveryPolicy::default()).unwrap();
    assert!(second.deferred_advisor_artifacts().is_empty());
    let reopened_snapshot = reopened.snapshot().unwrap();
    assert!(reopened_snapshot.physical_store().i64_index(i64).is_some());
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn deferred_recovery_uses_live_telemetry_to_rank_optional_rebuilds() {
    let dir = durable_test_dir("deferred-recovery-live-benefit-ranking");
    let values = (1_i64..=16).collect::<Vec<_>>();
    let (registry, preferred, other, root) = advisor_i64_recovery_bundle(5_179, &values, &values);
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());

    let (runtime, initial) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_key_evaluations: 0,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    assert_eq!(initial.deferred_advisor_artifacts().len(), 2);

    let mut controller = UnifiedAdvisorController::new(
        TelemetryDecayPolicy::default(),
        UnifiedAdvisorPolicy::default(),
        PhysicalPressurePolicy::default(),
    );
    controller.observe(
        PhysicalArtifactTelemetryTarget::I64Index(preferred),
        ArtifactTelemetry {
            read_work_saved: 10_000,
            maintenance_work: 0,
            rebuild_work: 0,
        },
    );
    let resumed = controller
        .resume_deferred_recovery(
            &runtime,
            &initial,
            PhysicalRecoveryPolicy {
                max_advisor_rebuild_key_evaluations: 16,
                ..PhysicalRecoveryPolicy::default()
            },
        )
        .unwrap();

    assert!(matches!(
        resumed.rebuilt.as_slice(),
        [DurablePhysicalArtifactSpec::I64Index { relation, .. }] if *relation == preferred.relation
    ));
    assert!(matches!(
        resumed.skipped_key_evaluation_budget.as_slice(),
        [DurablePhysicalArtifactSpec::I64Index { relation, .. }] if *relation == other.relation
    ));
    let snapshot = runtime.snapshot().unwrap();
    assert!(snapshot.physical_store().i64_index(preferred).is_some());
    assert!(snapshot.physical_store().i64_index(other).is_none());
    drop(snapshot);
    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bounded_recovery_schedules_cheaper_advisor_rebuild_before_semantic_id_order() {
    let dir = durable_test_dir("bounded-physical-recovery-work-order");
    let expensive_values = (1_i64..=10).collect::<Vec<_>>();
    let cheap_values = vec![1_i64, 2];
    let (registry, expensive, cheap, root) =
        advisor_i64_recovery_bundle(5_181, &expensive_values, &cheap_values);
    assert!(expensive.relation < cheap.relation);
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());

    let (reopened, report) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_key_evaluations: 2,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert!(snapshot.physical_store().i64_index(cheap).is_some());
    assert!(snapshot.physical_store().i64_index(expensive).is_none());
    assert_eq!(report.advisor_rebuild_key_evaluations, 2);
    assert!(matches!(
        report.rebuilt.as_slice(),
        [DurablePhysicalArtifactSpec::I64Index { relation, .. }] if *relation == cheap.relation
    ));
    assert!(matches!(
        report.skipped_key_evaluation_budget.as_slice(),
        [DurablePhysicalArtifactSpec::I64Index { relation, .. }] if *relation == expensive.relation
    ));
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn recovery_semantic_work_budget_distinguishes_same_cell_count_by_payload_size() {
    let dir = durable_test_dir("bounded-physical-recovery-semantic-work");
    let (_context, registry, short, long, root) = two_text_recovery_bundle(
        5_182,
        &["a", "b"],
        &[
            "this-is-a-deliberately-long-semantic-key-payload",
            "this-is-another-deliberately-long-semantic-key-payload",
        ],
    );
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());

    let (reopened, report) = DurableRuntime::open_with_recovery_policy(
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_semantic_work_units: 4,
            ..PhysicalRecoveryPolicy::default()
        },
    )
    .unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert!(
        snapshot
            .physical_store()
            .has_semantic_statistics_for_test(&short)
    );
    assert!(
        !snapshot
            .physical_store()
            .has_semantic_statistics_for_test(&long)
    );
    assert_eq!(report.advisor_rebuild_key_evaluations, 2);
    assert_eq!(report.advisor_rebuild_semantic_work_units, 4);
    assert_eq!(report.skipped_semantic_work_budget.len(), 1);
    assert!(matches!(
        report.skipped_semantic_work_budget.as_slice(),
        [DurablePhysicalArtifactSpec::SemanticStatistics { key_parts, .. }]
            if key_parts.as_slice() == [DurableSemanticKeyPart {
                column: 1,
                equivalence: long.key_parts[0].equivalence,
            }]
    ));
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn typed_algebraic_work_estimate_matches_logical_value_shape_without_row_materialization() {
    let value = Value::Seq(vec![Value::Text("abc".into()), Value::Text("d".into())]);
    let data = NativeRelation::typed_from_rows(
        &[vec![value.clone()]],
        &[TypeExpr::Seq(Box::new(TypeExpr::Scalar(ScalarType::Text)))],
    )
    .unwrap();
    assert_eq!(
        native_semantic_column_work_units(&data, 0).unwrap(),
        kernel_semantics::semantic_value_work_estimate(&value).total_units()
    );
}

#[test]
fn deferred_recovery_never_replays_manual_or_incompatible_recipes() {
    let dir = durable_test_dir("deferred-physical-recovery-safety");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_179, 9_179, &[1, 2, 3]);
    let manual = SemanticIndexBinding::single(relation, layout, 0, sid(101));
    root.physical_store_mut_for_test()
        .install_observable_atom_state(manual.clone(), &context, &registry)
        .unwrap();
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let before = runtime.snapshot().unwrap().root_version();
    let fabricated = PhysicalRecoveryReport {
        skipped_key_evaluation_budget: vec![DurablePhysicalArtifactSpec::ObservableAtom {
            relation,
            key_parts: vec![DurableSemanticKeyPart {
                column: 0,
                equivalence: sid(101),
            }],
            advisor_managed: false,
        }],
        skipped_estimated_byte_budget: vec![DurablePhysicalArtifactSpec::ObservableAtom {
            relation: sid(0xdead),
            key_parts: vec![DurableSemanticKeyPart {
                column: 0,
                equivalence: sid(101),
            }],
            advisor_managed: true,
        }],
        ..PhysicalRecoveryReport::default()
    };

    assert_eq!(fabricated.deferred_advisor_artifacts().len(), 1);
    let report = runtime
        .resume_deferred_physical_recovery(&fabricated, PhysicalRecoveryPolicy::default())
        .unwrap();
    assert!(report.rebuilt.is_empty());
    assert_eq!(report.dropped_incompatible.len(), 1);
    let after = runtime.snapshot().unwrap();
    assert_eq!(after.root_version(), before);
    assert!(
        after
            .physical_store()
            .observable_atom_state(&manual)
            .is_some()
    );
    drop(after);
    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn supervisor_can_resume_deferred_recovery_without_reopen() {
    let dir = durable_test_dir("deferred-supervisor-recovery-resume");
    let (context, registry, relation, layout, mut root) =
        scan_runtime_bundle(5_180, 9_180, &[1, 2, 3, 4]);
    let i64 = I64IndexBinding {
        relation,
        layout,
        key_column: 0,
        equivalence: sid(101),
    };
    root.physical_store_mut_for_test()
        .install_i64_index(i64, &context, &registry)
        .unwrap();
    root.physical_store_mut_for_test()
        .advisor_managed_artifacts_mut()
        .insert(UnifiedArtifactId::I64Index(i64));
    let supervisor = DurableRuntimeSupervisor::create_with_recovery_policy(
        root,
        &dir,
        PhysicalRecoveryPolicy {
            max_advisor_rebuild_key_evaluations: 0,
            ..PhysicalRecoveryPolicy::default()
        },
        &registry,
    )
    .unwrap();
    let initial = supervisor.recover_with_report().unwrap();
    assert_eq!(initial.deferred_advisor_artifacts().len(), 1);
    let resumed = supervisor
        .resume_deferred_physical_recovery(&initial, PhysicalRecoveryPolicy::default())
        .unwrap();
    assert_eq!(resumed.rebuilt.len(), 1);
    let snapshot = supervisor.snapshot().unwrap();
    assert!(snapshot.physical_store().i64_index(i64).is_some());
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stale_durable_physical_recipe_never_blocks_logical_recovery() {
    let dir = durable_test_dir("stale-physical-recipe");
    let (_, registry, _, _, root) = scan_runtime_bundle(517, 1217, &[1, 2]);
    let materializations = root.durable_materialization_specs();
    let stale = DurablePhysicalArtifactSpec::ObservableAtom {
        relation: sid(0xdead),
        key_parts: vec![DurableSemanticKeyPart {
            column: 0,
            equivalence: sid(101),
        }],
        advisor_managed: true,
    };
    drop(
        DurableRevisionStore::create_with_materializations_and_physical_artifacts(
            &dir,
            root.revision(),
            &materializations,
            std::slice::from_ref(&stale),
            &registry,
        )
        .unwrap(),
    );

    let (reopened, report) =
        DurableRuntime::open_with_recovery_policy(&dir, PhysicalRecoveryPolicy::default()).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision_id(), RevisionId::new(517));
    assert!(
        snapshot
            .physical_store()
            .observable_atom_states_for_test()
            .is_empty()
    );
    assert_eq!(report.dropped_incompatible, vec![stale]);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn stale_durable_layout_recipe_falls_back_without_blocking_logical_recovery() {
    let dir = durable_test_dir("stale-layout-recipe");
    let relation = sid(5_190);
    let equivalence = sid(5_191);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(5_190));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(5_190));
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
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(
        relation,
        vec![
            vec![Value::Text("alpha".into())],
            vec![Value::Text("beta".into())],
        ],
    );
    let revision = kernel_revision::Revision::build(
        RevisionId::new(5_190),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    drop(
        DurableRevisionStore::create_with_materializations_and_physical_artifacts(
            &dir,
            &revision,
            &[],
            &[DurablePhysicalArtifactSpec::RelationLayout {
                relation,
                layout_id: 9_190,
                kind: DurableRelationLayoutKind::I64Columnar,
            }],
            &registry,
        )
        .unwrap(),
    );

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision(), &revision);
    assert_eq!(
        snapshot.root().relation_layout(relation),
        Some(LayoutBinding::RECOVERY_ROW_STORE)
    );
    assert_eq!(
        snapshot
            .physical_store()
            .installed(relation, LayoutBinding::RECOVERY_ROW_STORE)
            .unwrap()
            .data
            .clone(),
        NativeRelation::row_store(vec![
            vec![Value::Text("alpha".into())],
            vec![Value::Text("beta".into())]
        ])
    );
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn conflicting_durable_layout_recipes_fall_back_deterministically() {
    let dir = durable_test_dir("conflicting-layout-recipes");
    let (_, registry, relation, _, root) = scan_runtime_bundle(5_191, 9_191, &[1, 2]);
    let revision = root.revision().clone();
    let materializations = root.durable_materialization_specs();
    let specs = [
        DurablePhysicalArtifactSpec::RelationLayout {
            relation,
            layout_id: 9_191,
            kind: DurableRelationLayoutKind::TypedColumnar,
        },
        DurablePhysicalArtifactSpec::RelationLayout {
            relation,
            layout_id: 9_192,
            kind: DurableRelationLayoutKind::I64Columnar,
        },
    ];
    drop(
        DurableRevisionStore::create_with_materializations_and_physical_artifacts(
            &dir,
            &revision,
            &materializations,
            &specs,
            &registry,
        )
        .unwrap(),
    );
    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(
        snapshot.root().relation_layout(relation),
        Some(LayoutBinding::RECOVERY_ROW_STORE)
    );
    assert_eq!(snapshot.revision(), &revision);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

fn structural_durable_index_fixture() -> (
    SemanticRegistry,
    SemanticContext,
    RuntimeRevisionBundle,
    kernel_types::SemanticId,
    kernel_types::SemanticId,
) {
    let relation = sid(1_320);
    let structural = sid(1_321);
    let ci = sid(1_322);
    let mut registry = SemanticRegistry::default();
    let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1_320));
    environment.pin_module(ci, ci_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1_320));
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
    let option_text = |value: &str| Value::Option(Some(Box::new(Value::Text(value.into()))));
    let mut rows = vec![vec![option_text("Alpha")], vec![option_text("alpha")]];
    for index in 0..64 {
        rows.push(vec![option_text(&format!("other-{index}"))]);
    }
    let mut model = kernel_model::FiniteModel::default();
    model.relations.insert(relation, rows.clone());
    let revision = kernel_revision::Revision::build(
        RevisionId::new(1_320),
        &context,
        &registry,
        kernel_model::DatabaseState {
            model,
            ..kernel_model::DatabaseState::default()
        },
    )
    .unwrap();
    let layout = LayoutBinding {
        id: LayoutId(1_320),
        family: LayoutFamily::RowStore,
    };
    let mut physical = PhysicalStore::default();
    physical
        .install(relation, layout, NativeRelation::row_store(rows))
        .unwrap();
    physical
        .install_observable_atom_state(
            SemanticIndexBinding::single(relation, layout, 0, structural),
            &context,
            &registry,
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
    (registry, context, root, relation, structural)
}

#[test]
fn durable_structural_observable_atom_rebuilds_and_serves_after_reopen() {
    let dir = durable_test_dir("structural-physical-recipe");
    let (registry, context, root, relation, structural) = structural_durable_index_fixture();
    let layout = root.relation_layout(relation).unwrap();
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    let recovered_binding = SemanticIndexBinding::single(relation, layout, 0, structural);
    assert!(
        snapshot
            .physical_store()
            .observable_atom_state(&recovered_binding)
            .is_some()
    );
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::Option(Some(Box::new(Value::Text("ALPHA".into())))),
        equivalence: structural,
    };
    let prepared = prepare_with_catalog(query, &context, &registry, &catalog).unwrap();
    let (result, stats) = prepared
        .execute_native_pinned(snapshot.physical_store(), &registry)
        .unwrap();
    assert_eq!(result.rows().len(), 2);
    assert_eq!(stats.persisted_index_hits, 1);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

struct ExitAfterDurableCommit {
    inner: kernel_durability::DurableRevisionStore,
    crash_marker: std::path::PathBuf,
}

impl kernel_durability::RevisionDurability for ExitAfterDurableCommit {
    fn durably_prepare(
        &mut self,
        descriptor: &kernel_durability::DurableRevisionDescriptor,
    ) -> Result<kernel_durability::DurablePrepareToken, kernel_durability::DurabilityError> {
        self.inner.durably_prepare(descriptor)
    }

    fn durably_commit(
        &mut self,
        prepared: kernel_durability::DurablePrepareToken,
    ) -> Result<kernel_durability::DurableCommitReceipt, kernel_durability::DurabilityError> {
        self.inner.durably_commit(prepared)?;
        let marker = std::fs::File::create(&self.crash_marker)?;
        marker.sync_all()?;
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    }
}

#[test]
fn subprocess_commit_durable_before_publish_recovers_target_revision() {
    const WORKER_ENV: &str = "CFMD_PLAN_COMMIT_CRASH_WORKER";
    const DIR_ENV: &str = "CFMD_PLAN_COMMIT_CRASH_DIR";
    let dir = durable_test_dir("commit-before-publish");
    let (context, registry, relation, layout, root) = scan_runtime_bundle(530, 1203, &[1, 2]);
    let retry_delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let retry_mutations = [RevisionRelationMutation {
        relation,
        delta: &retry_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let retry_target = target_revision_for(&root, 531, &retry_mutations, &registry);
    drop(DurableRuntime::create(root, &dir, &registry).unwrap());
    let marker = dir.join(".cfmd-plan-crash-ready");
    let _ = std::fs::remove_file(&marker);

    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("tests::crash_worker_commit_durable_before_publish")
        .arg("--nocapture")
        .env(WORKER_ENV, "1")
        .env(DIR_ENV, &dir)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.is_file() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("commit crash worker exited before killpoint: {status}");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "commit crash worker did not reach durable-COMMIT killpoint"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    child.kill().unwrap();
    let _ = child.wait().unwrap();
    let _ = std::fs::remove_file(&marker);

    let reopened = DurableRuntimeSupervisor::open(&dir).unwrap();
    assert!(matches!(
        reopened
            .commit_revision(
                ClientTransactionId::new(1006),
                &RevisionTransitionRequest {
                    target_revision: &retry_target,
                    mutations: &retry_mutations,
                    registry: &registry,
                },
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: RevisionId(531)
        }
    ));
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision_id(), RevisionId::new(531));
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
fn crash_worker_commit_durable_before_publish() {
    const WORKER_ENV: &str = "CFMD_PLAN_COMMIT_CRASH_WORKER";
    const DIR_ENV: &str = "CFMD_PLAN_COMMIT_CRASH_DIR";
    if std::env::var_os(WORKER_ENV).is_none() {
        return;
    }
    let dir = std::path::PathBuf::from(std::env::var_os(DIR_ENV).unwrap());
    let (context, registry, relation, _, root) = scan_runtime_bundle(530, 1203, &[1, 2]);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let target = target_revision_for(&root, 531, &mutations, &registry);
    let (durability, scan) = kernel_durability::DurableRevisionStore::open(&dir).unwrap();
    assert!(scan.committed().is_empty());
    let cell = RuntimeRevisionCell::new(root);
    let mut durability = ExitAfterDurableCommit {
        inner: durability,
        crash_marker: dir.join(".cfmd-plan-crash-ready"),
    };
    let _ = cell.commit_revision_durable(
        ClientTransactionId::new(1006),
        &RevisionTransitionRequest {
            target_revision: &target,
            mutations: &mutations,
            registry: &registry,
        },
        &mut durability,
    );
    panic!("worker must exit after durable COMMIT and before publish");
}

#[test]
fn client_transaction_identity_survives_checkpoint_and_makes_retry_idempotent() {
    let dir = durable_test_dir("transaction-idempotency");
    let (context, registry, relation, _, root) = scan_runtime_bundle(540, 1204, &[1, 2]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let transaction_id = ClientTransactionId::new(0xabc);
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let snapshot = runtime.snapshot().unwrap();
    let target = target_revision_for(snapshot.root(), 541, &mutations, &registry);
    drop(snapshot);

    assert!(matches!(
        runtime
            .commit_revision(
                transaction_id,
                &RevisionTransitionRequest {
                    target_revision: &target,
                    mutations: &mutations,
                    registry: &registry,
                },
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::Committed(_)
    ));
    assert_eq!(
        runtime.transaction_outcome(transaction_id).unwrap(),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(541)
        }
    );
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(
        reopened.transaction_outcome(transaction_id).unwrap(),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(541)
        }
    );
    assert!(matches!(
        reopened
            .commit_revision(
                transaction_id,
                &RevisionTransitionRequest {
                    target_revision: &target,
                    mutations: &mutations,
                    registry: &registry,
                },
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: RevisionId(541)
        }
    ));
    assert!(
        reopened
            .snapshot()
            .unwrap()
            .materialization(test_materialization_id())
            .is_some()
    );

    let next_delta = scan_delta(relation, &[4], &[2], &context, &registry);
    let next_mutations = [RevisionRelationMutation {
        relation,
        delta: &next_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let live = reopened.snapshot().unwrap();
    let next_target = target_revision_for(live.root(), 542, &next_mutations, &registry);
    drop(live);
    assert!(matches!(
        reopened.commit_revision(
            transaction_id,
            &RevisionTransitionRequest {
                target_revision: &next_target,
                mutations: &next_mutations,
                registry: &registry,
            },
        ),
        Err(DurableRuntimeCommitError::TransactionIdConflict {
            committed_target: RevisionId(541),
            requested_target: RevisionId(542),
            ..
        })
    ));
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn supervisor_recovers_fail_stopped_runtime_and_retries_same_transaction() {
    let dir = durable_test_dir("supervisor-recovery");
    let (context, registry, relation, _, root) = scan_runtime_bundle(550, 1205, &[1, 2]);
    let supervisor = DurableRuntimeSupervisor::create(root, &dir, &registry).unwrap();
    let delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let snapshot = supervisor.snapshot().unwrap();
    let target = target_revision_for(snapshot.root(), 551, &mutations, &registry);
    drop(snapshot);

    supervisor.force_live_runtime_recovery_required_for_test();

    let transaction_id = ClientTransactionId::new(0xdef);
    assert!(matches!(
        supervisor
            .commit_revision(
                transaction_id,
                &RevisionTransitionRequest {
                    target_revision: &target,
                    mutations: &mutations,
                    registry: &registry,
                },
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::Committed(_)
    ));
    assert_eq!(
        supervisor.snapshot().unwrap().revision_id(),
        RevisionId::new(551)
    );
    assert_eq!(
        supervisor.transaction_outcome(transaction_id).unwrap(),
        DurableTransactionOutcome::Committed {
            target_revision: RevisionId::new(551)
        }
    );
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn supervisor_recovers_poisoned_reconstructible_runtime_from_durable_authority() {
    let dir = durable_test_dir("supervisor-poison-recovery-pass87");
    let (_, registry, _, _, root) = scan_runtime_bundle(560, 1215, &[1, 2]);
    let supervisor = Arc::new(DurableRuntimeSupervisor::create(root, &dir, &registry).unwrap());
    DurableRuntimeSupervisor::poison_runtime_slot_for_test(Arc::clone(&supervisor));
    assert!(supervisor.runtime_slot_is_poisoned_for_test());

    let snapshot = supervisor.snapshot().unwrap();
    assert_eq!(snapshot.revision_id(), RevisionId::new(560));
    assert!(!supervisor.runtime_slot_is_poisoned_for_test());

    drop(snapshot);
    drop(supervisor);
    std::fs::remove_dir_all(dir).unwrap();
}

fn migration_context(
    schema_revision: u64,
    environment_revision: u64,
    relation: SemanticId,
    equivalence: SemanticId,
    entity_type: SemanticId,
    field: SemanticId,
    digest: kernel_schema::ModuleDigest,
) -> SemanticContext {
    let mut schema = Schema::new(SchemaRevisionId::new(schema_revision));
    schema
        .define_field(FieldDef {
            id: field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(environment_revision));
    environment.pin_module(equivalence, digest);
    SemanticContext {
        schema,
        environment,
    }
}

fn migration_state(
    entity_type: SemanticId,
    field: SemanticId,
    relation: SemanticId,
    entities: &[(EntityId, i64)],
    rows: &[&str],
) -> kernel_model::DatabaseState {
    let mut state = kernel_model::DatabaseState::default();
    for &(entity, value) in entities {
        state.lifecycle.entities.insert(entity);
        state.lifecycle.roots.insert(entity);
        state
            .model
            .carriers
            .entry(entity_type)
            .or_default()
            .insert(entity);
        state
            .model
            .fields
            .insert((field, entity), Value::I64(value));
    }
    state.model.relations.insert(
        relation,
        rows.iter()
            .map(|value| vec![Value::Text((*value).into())])
            .collect(),
    );
    state
}

struct FullRevisionReopenAssertions {
    field: SemanticId,
    second_entity: EntityId,
    equivalence: SemanticId,
    equivalence_digest: kernel_schema::ModuleDigest,
    transaction_id: ClientTransactionId,
}

fn assert_reopened_full_revision(
    dir: &std::path::Path,
    _registry: &SemanticRegistry,
    target: &kernel_revision::Revision,
    assertions: &FullRevisionReopenAssertions,
) {
    let reopened = DurableRuntime::open(dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision(), target);
    assert_eq!(
        snapshot.revision().state().model.fields[&(assertions.field, assertions.second_entity)],
        Value::I64(20)
    );
    assert_eq!(
        snapshot
            .revision()
            .semantic_context()
            .environment
            .module(assertions.equivalence),
        Some(assertions.equivalence_digest)
    );
    assert!(
        snapshot
            .materialization(test_materialization_id())
            .is_some()
    );
    assert_eq!(
        reopened
            .transaction_outcome(assertions.transaction_id)
            .unwrap(),
        DurableTransactionOutcome::Committed {
            target_revision: target.id()
        }
    );
}

fn create_migration_runtime(
    dir: &std::path::Path,
    relation: SemanticId,
    source: kernel_revision::Revision,
    initial_text: &str,
    registry: &SemanticRegistry,
) -> DurableRuntime {
    let mut physical = PhysicalStore::default();
    physical
        .install(
            relation,
            LayoutBinding::RECOVERY_ROW_STORE,
            NativeRelation::row_store(vec![vec![Value::Text(initial_text.into())]]),
        )
        .unwrap();
    let root = RuntimeRevisionBundle::build(
        source,
        physical,
        BTreeMap::from([(relation, LayoutBinding::RECOVERY_ROW_STORE)]),
        &[RuntimeMaterializationSpec {
            id: test_materialization_id(),
            query: RelExpr::Scan(relation),
        }],
        registry,
    )
    .unwrap();
    DurableRuntime::create(root, dir, registry).unwrap()
}

fn create_volatile_migration_runtime(
    relation: SemanticId,
    source: kernel_revision::Revision,
    initial_text: &str,
    registry: &SemanticRegistry,
) -> DurableRuntime {
    let mut physical = PhysicalStore::default();
    physical
        .install(
            relation,
            LayoutBinding::RECOVERY_ROW_STORE,
            NativeRelation::row_store(vec![vec![Value::Text(initial_text.into())]]),
        )
        .unwrap();
    let root = RuntimeRevisionBundle::build(
        source,
        physical,
        BTreeMap::from([(relation, LayoutBinding::RECOVERY_ROW_STORE)]),
        &[RuntimeMaterializationSpec {
            id: test_materialization_id(),
            query: RelExpr::Scan(relation),
        }],
        registry,
    )
    .unwrap();
    DurableRuntime::create_volatile(root, registry).unwrap()
}

#[test]
#[allow(clippy::too_many_lines)] // End-to-end mixed durability scenario kept contiguous as one protocol regression.
fn durable_mixed_revision_updates_lifecycle_and_relations_incrementally() {
    let dir = durable_test_dir("mixed-revision-incremental-publication");
    let relation = sid(7_951);
    let equivalence = sid(7_952);
    let entity_type = sid(7_953);
    let field = sid(7_954);
    let first = EntityId::new(7951);
    let second = EntityId::new(7952);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence_revision(EquivalenceModule::TextExact, 11);
    let context = migration_context(
        7951,
        7951,
        relation,
        equivalence,
        entity_type,
        field,
        digest,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(7951),
        &context,
        &registry,
        migration_state(entity_type, field, relation, &[(first, 1)], &["A"]),
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source.clone(), "A", &registry);
    let target = kernel_revision::Revision::build(
        RevisionId::new(7952),
        &context,
        &registry,
        migration_state(
            entity_type,
            field,
            relation,
            &[(first, 1), (second, 2)],
            &["A", "B"],
        ),
    )
    .unwrap();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let delta = RelationDelta {
        inserted: vec![vec![Value::Text("B".into())]],
        removed: vec![],
        result_type,
    };
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let model_delta = DurableModelDelta::between(source.state(), target.state());
    let model_complement = DurableModelDelta::between(target.state(), source.state());
    let transaction_id = ClientTransactionId::new(0x7952);
    let outcome = runtime
        .commit_mixed_revision(
            transaction_id,
            &MixedRevisionTransitionRequest {
                source_revision: source.id(),
                target_revision: &target,
                mutations: &mutations,
                model_delta: &model_delta,
                model_complement: &model_complement,
                registry: &registry,
            },
        )
        .unwrap();
    assert!(matches!(
        outcome,
        DurableRuntimeCommitOutcome::Committed(DurableRuntimeCommitReceipt {
            publication: RuntimePublicationEffect::Incremental(_),
            ..
        })
    ));
    let retry = runtime
        .commit_mixed_revision(
            transaction_id,
            &MixedRevisionTransitionRequest {
                source_revision: source.id(),
                target_revision: &target,
                mutations: &mutations,
                model_delta: &model_delta,
                model_complement: &model_complement,
                registry: &registry,
            },
        )
        .unwrap();
    assert_eq!(
        retry,
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: target.id()
        }
    );
    assert_eq!(runtime.snapshot().unwrap().revision(), &target);
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(reopened.snapshot().unwrap().revision(), &target);
    assert_eq!(
        reopened.transaction_outcome(transaction_id).unwrap(),
        DurableTransactionOutcome::Committed {
            target_revision: target.id()
        }
    );
    let reopened_retry = reopened
        .commit_mixed_revision(
            transaction_id,
            &MixedRevisionTransitionRequest {
                source_revision: source.id(),
                target_revision: &target,
                mutations: &mutations,
                model_delta: &model_delta,
                model_complement: &model_complement,
                registry: &registry,
            },
        )
        .unwrap();
    assert_eq!(
        reopened_retry,
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: target.id()
        }
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn durable_full_revision_replacement_covers_schema_gamma_lifecycle_and_fields() {
    let dir = durable_test_dir("full-revision-replacement");
    let relation = sid(8_001);
    let equivalence = sid(8_002);
    let entity_type = sid(8_003);
    let field = sid(8_004);
    let first = EntityId::new(81);
    let second = EntityId::new(82);
    let mut registry = SemanticRegistry::default();
    let exact_digest = registry.install_equivalence_revision(EquivalenceModule::TextExact, 11);
    let ci_digest =
        registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 17);
    let source_context = migration_context(
        801,
        801,
        relation,
        equivalence,
        entity_type,
        field,
        exact_digest,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(801),
        &source_context,
        &registry,
        migration_state(entity_type, field, relation, &[(first, 1)], &["A"]),
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source, "A", &registry);
    let target_context = migration_context(
        802,
        802,
        relation,
        equivalence,
        entity_type,
        field,
        ci_digest,
    );
    let target = kernel_revision::Revision::build(
        RevisionId::new(802),
        &target_context,
        &registry,
        migration_state(
            entity_type,
            field,
            relation,
            &[(first, 10), (second, 20)],
            &["A", "a"],
        ),
    )
    .unwrap();
    let transaction_id = ClientTransactionId::new(0x802);
    let outcome = runtime
        .replace_revision(
            transaction_id,
            &FullRevisionTransitionRequest {
                target_revision: &target,
                registry: &registry,
            },
        )
        .unwrap();
    assert!(matches!(
        outcome,
        DurableRuntimeCommitOutcome::Committed(DurableRuntimeCommitReceipt {
            publication: RuntimePublicationEffect::Rebuilt,
            ..
        })
    ));
    assert_eq!(runtime.snapshot().unwrap().revision(), &target);
    drop(runtime);
    assert_reopened_full_revision(
        &dir,
        &registry,
        &target,
        &FullRevisionReopenAssertions {
            field,
            second_entity: second,
            equivalence,
            equivalence_digest: ci_digest,
            transaction_id,
        },
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn volatile_historical_release_removes_durable_and_runtime_retention_together() {
    let relation = sid(8_041);
    let equivalence = sid(8_042);
    let entity_type = sid(8_043);
    let field = sid(8_044);
    let entity = EntityId::new(841);
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence_revision(EquivalenceModule::TextExact, 11);
    let ci = registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 17);
    let source_context =
        migration_context(841, 841, relation, equivalence, entity_type, field, exact);
    let source = kernel_revision::Revision::build(
        RevisionId::new(841),
        &source_context,
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 1)], &["A"]),
    )
    .unwrap();
    let runtime = create_volatile_migration_runtime(relation, source, "A", &registry);
    let target_context = migration_context(842, 842, relation, equivalence, entity_type, field, ci);
    let target = kernel_revision::Revision::build(
        RevisionId::new(842),
        &target_context,
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 1)], &["A"]),
    )
    .unwrap();
    let program = kernel_transport::SchemaMigrationProgram::new(
        target_context.clone(),
        Vec::new(),
        Vec::new(),
    );
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: target_context.schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(8_045)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(8_046)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x842),
            &FullRevisionTransitionRequest {
                target_revision: &target,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let pins = runtime.retained_historical_epochs().unwrap();
    assert_eq!(pins.len(), 1);
    let effect_id = pins[0].effect_id();
    assert_eq!(runtime.snapshot().unwrap().retained_schema_epoch_count_for_test(), 1);
    assert!(runtime
        .release_historical_epoch_authority(effect_id)
        .unwrap()
        .is_none());
    assert!(runtime.retained_historical_epochs().unwrap().is_empty());
    assert_eq!(runtime.snapshot().unwrap().retained_schema_epoch_count_for_test(), 0);
}

#[test]
fn durable_schema_migration_publishes_target_and_complement_atomically() {
    let dir = durable_test_dir("schema-migration-complement-runtime");
    let relation = sid(8_051);
    let equivalence = sid(8_052);
    let entity_type = sid(8_053);
    let field = sid(8_054);
    let entity = EntityId::new(851);
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence_revision(EquivalenceModule::TextExact, 11);
    let ci = registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 17);
    let source_context =
        migration_context(851, 851, relation, equivalence, entity_type, field, exact);
    let source = kernel_revision::Revision::build(
        RevisionId::new(851),
        &source_context,
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 1)], &["A"]),
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source, "A", &registry);
    let target_context = migration_context(852, 852, relation, equivalence, entity_type, field, ci);
    let target = kernel_revision::Revision::build(
        RevisionId::new(852),
        &target_context,
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 1)], &["A"]),
    )
    .unwrap();
    let migration_program = kernel_transport::SchemaMigrationProgram::new(
        target_context.clone(),
        Vec::new(),
        Vec::new(),
    );
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: target_context.schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(8_055)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(8_056)),
            encoding_version: 1,
            complement: Value::Product(BTreeMap::new()),
        },
        kernel_lens::ComplementRetention::Forever,
    );
    let outcome = runtime
        .migrate_schema(
            ClientTransactionId::new(0x852),
            &FullRevisionTransitionRequest {
                target_revision: &target,
                registry: &registry,
            },
            &migration_program,
            &complement,
        )
        .unwrap();
    assert!(matches!(
        outcome,
        DurableRuntimeCommitOutcome::Committed(DurableRuntimeCommitReceipt {
            publication: RuntimePublicationEffect::Rebuilt,
            ..
        })
    ));
    assert_eq!(runtime.snapshot().unwrap().revision(), &target);
    let local_chain = runtime
        .local_historical_complement_chain(
            source_context.schema.revision,
            target_context.schema.revision,
        )
        .unwrap();
    assert_eq!(local_chain.steps().len(), 1);
    assert_eq!(
        local_chain.steps()[0].local_capsule().unwrap().complement,
        Value::Product(BTreeMap::new())
    );
    assert_runtime_historical_product_restore(
        &runtime,
        source_context.schema.revision,
        target_context.schema.revision,
    );
    drop(runtime);

    let (store, scan) = DurableRevisionStore::open(&dir).unwrap();
    assert_eq!(scan.durable_revision(), target.id());
    assert_eq!(
        store.migration_complements(),
        std::slice::from_ref(&complement)
    );
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

fn assert_runtime_historical_product_restore(
    runtime: &DurableRuntime,
    source: SchemaRevisionId,
    target: SchemaRevisionId,
) {
    let restored_field = sid(8_057);
    let mut registry = kernel_durability::HistoricalLensRegistry::default();
    registry
        .register(
            kernel_durability::HistoricalLensImplementationKey {
                lens_spec: kernel_lens::LensSpecId(sid(8_055)),
                semantic_pins: kernel_lens::SemanticManifestId(sid(8_056)),
                encoding_version: 1,
            },
            kernel_durability::HistoricalLensImplementation::ProductField {
                field: restored_field,
            },
        )
        .unwrap();
    assert_eq!(
        runtime
            .restore_historical_value(source, target, &Value::I64(2), &registry)
            .unwrap(),
        Value::Product(BTreeMap::from([(restored_field, Value::I64(2))]))
    );
}

#[test]
fn historical_semantic_implementation_manifest_survives_checkpoint_and_retry() {
    let dir = durable_test_dir("historical-semantic-deployment");
    let relation = sid(8_101);
    let equivalence = sid(8_102);
    let entity_type = sid(8_103);
    let field = sid(8_104);
    let entity = EntityId::new(811);
    let mut registry = SemanticRegistry::default();
    let exact_v11 = registry.install_equivalence_revision(EquivalenceModule::TextExact, 11);
    let ci_v17 =
        registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 17);
    let exact_v19 = registry.install_equivalence_revision(EquivalenceModule::TextExact, 19);

    let source_context = migration_context(
        811,
        811,
        relation,
        equivalence,
        entity_type,
        field,
        exact_v11,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(811),
        &source_context,
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 1)], &["A"]),
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source, "A", &registry);

    let first_target = kernel_revision::Revision::build(
        RevisionId::new(812),
        &migration_context(812, 812, relation, equivalence, entity_type, field, ci_v17),
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 2)], &["a"]),
    )
    .unwrap();
    let first_transaction = ClientTransactionId::new(0x812);
    runtime
        .replace_revision(
            first_transaction,
            &FullRevisionTransitionRequest {
                target_revision: &first_target,
                registry: &registry,
            },
        )
        .unwrap();

    let second_target = kernel_revision::Revision::build(
        RevisionId::new(813),
        &migration_context(
            813,
            813,
            relation,
            equivalence,
            entity_type,
            field,
            exact_v19,
        ),
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 3)], &["B"]),
    )
    .unwrap();
    runtime
        .replace_revision(
            ClientTransactionId::new(0x813),
            &FullRevisionTransitionRequest {
                target_revision: &second_target,
                registry: &registry,
            },
        )
        .unwrap();
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(reopened.snapshot().unwrap().revision(), &second_target);
    assert_eq!(
        reopened
            .semantic_registry()
            .builtin_module_spec(ci_v17)
            .unwrap()
            .digest(),
        ci_v17
    );
    assert_eq!(
        reopened
            .replace_revision(
                first_transaction,
                &FullRevisionTransitionRequest {
                    target_revision: &first_target,
                    registry: &registry,
                },
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: RevisionId::new(812),
        }
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn committed_transaction_id_rejects_same_target_id_with_different_derived_delta() {
    let dir = durable_test_dir("same-target-id-derived-intent-conflict");
    let (context, registry, relation, _, root) = scan_runtime_bundle(860, 1280, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let transaction_id = ClientTransactionId::new(0x860);
    let delta = scan_delta(relation, &[2], &[], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let request = DerivedRelationTransitionRequest {
        source_revision: RevisionId::new(860),
        target_revision: RevisionId::new(861),
        mutations: &mutations,
    };
    runtime
        .commit_derived_relation_data(transaction_id, &request)
        .unwrap();

    let conflicting_delta = scan_delta(relation, &[3], &[], &context, &registry);
    let conflicting_mutations = [RevisionRelationMutation {
        relation,
        delta: &conflicting_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    assert!(matches!(
        runtime.commit_derived_relation_data(
            transaction_id,
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(860),
                target_revision: RevisionId::new(861),
                mutations: &conflicting_mutations,
            },
        ),
        Err(DurableRuntimeCommitError::TransactionIdConflict {
            committed_target: RevisionId(861),
            requested_target: RevisionId(861),
            ..
        })
    ));
    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn exact_derived_transaction_intent_survives_later_heads_checkpoint_compaction_and_reopen() {
    let dir = durable_test_dir("historical-exact-derived-transaction-intent");
    let (context, registry, relation, _, root) = scan_runtime_bundle(865, 1285, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();

    let first_transaction = ClientTransactionId::new(0x8651);
    let first_delta = scan_delta(relation, &[2], &[], &context, &registry);
    let first_mutations = [RevisionRelationMutation {
        relation,
        delta: &first_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let first_request = DerivedRelationTransitionRequest {
        source_revision: RevisionId::new(865),
        target_revision: RevisionId::new(866),
        mutations: &first_mutations,
    };
    runtime
        .commit_derived_relation_data(first_transaction, &first_request)
        .unwrap();

    let second_delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let second_mutations = [RevisionRelationMutation {
        relation,
        delta: &second_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    runtime
        .commit_derived_relation_data(
            ClientTransactionId::new(0x8652),
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(866),
                target_revision: RevisionId::new(867),
                mutations: &second_mutations,
            },
        )
        .unwrap();
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(
        reopened
            .commit_derived_relation_data(first_transaction, &first_request)
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: RevisionId::new(866),
        }
    );

    let conflicting_delta = scan_delta(relation, &[4], &[], &context, &registry);
    let conflicting_mutations = [RevisionRelationMutation {
        relation,
        delta: &conflicting_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    assert!(matches!(
        reopened.commit_derived_relation_data(
            first_transaction,
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(865),
                target_revision: RevisionId::new(866),
                mutations: &conflicting_mutations,
            },
        ),
        Err(DurableRuntimeCommitError::TransactionIdConflict {
            committed_target: RevisionId(866),
            requested_target: RevisionId(866),
            ..
        })
    ));
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn derived_relation_commit_is_compact_exact_and_survives_compaction_retry() {
    let dir = durable_test_dir("derived-relation-compact-exact");
    let (context, registry, relation, _, root) = scan_runtime_bundle(875, 1295, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();

    let first_transaction = ClientTransactionId::new(0x8751);
    let first_delta = scan_delta(relation, &[2], &[], &context, &registry);
    let first_mutations = [RevisionRelationMutation {
        relation,
        delta: &first_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let first_request = DerivedRelationTransitionRequest {
        source_revision: RevisionId::new(875),
        target_revision: RevisionId::new(876),
        mutations: &first_mutations,
    };
    assert!(matches!(
        runtime
            .commit_derived_relation_data(first_transaction, &first_request)
            .unwrap(),
        DurableRuntimeCommitOutcome::Committed(_)
    ));

    let second_delta = scan_delta(relation, &[3], &[1], &context, &registry);
    let second_mutations = [RevisionRelationMutation {
        relation,
        delta: &second_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    runtime
        .commit_derived_relation_data(
            ClientTransactionId::new(0x8752),
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(876),
                target_revision: RevisionId::new(877),
                mutations: &second_mutations,
            },
        )
        .unwrap();
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(
        reopened
            .commit_derived_relation_data(first_transaction, &first_request)
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: RevisionId::new(876),
        }
    );

    let conflicting_delta = scan_delta(relation, &[99], &[], &context, &registry);
    let conflicting_mutations = [RevisionRelationMutation {
        relation,
        delta: &conflicting_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    assert!(matches!(
        reopened.commit_derived_relation_data(
            first_transaction,
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(875),
                target_revision: RevisionId::new(876),
                mutations: &conflicting_mutations,
            },
        ),
        Err(DurableRuntimeCommitError::TransactionIdConflict { .. })
    ));
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

fn assert_relation_rewrite_causal_identity(
    runtime: &DurableRuntime,
    _transaction_id: ClientTransactionId,
    relation: SemanticId,
    spec: &kernel_change::RewriteSpec,
    source: RevisionId,
    target: RevisionId,
) {
    assert_eq!(runtime.causal_coverage_root().unwrap(), source);
    let causal = runtime.revision_effect_ideal(target).unwrap().unwrap();
    assert_eq!(causal.events().len(), 1);
    let payload = &causal.events().values().next().unwrap().payload;
    assert!(matches!(
        payload,
        DurableTransactionIntent::RelationRewrite {
            source_revision,
            target_revision,
            rewrite_intents,
            ..
        } if *source_revision == source
            && *target_revision == target
            && rewrite_intents == &[DurableRelationRewriteIntent {
                relation,
                rewrite_spec: spec.id.0,
                law_set: spec.law_set.0,
            }]
    ));
}

#[test]
fn derived_relation_rewrite_persists_intent_and_idempotency_distinguishes_spec() {
    let dir = durable_test_dir("derived-relation-rewrite-intent");
    let (context, registry, relation, _, root) = scan_runtime_bundle(885, 1305, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let transaction_id = ClientTransactionId::new(0x8851);
    let delta = scan_delta(relation, &[2], &[], &context, &registry);
    let source = runtime.snapshot().unwrap();
    let old = RelExpr::Scan(relation)
        .evaluate(&source.revision().state().model, &context, &registry)
        .unwrap();
    drop(source);
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(88_501)),
        law_set: RewriteLawSetId(SemanticId::new(88_502)),
        footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
    };
    let rewrite = delta
        .prepare_relation_rewrite(
            relation,
            &old,
            &context,
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &rewrite,
    }];
    let request = DerivedRelationRewriteTransitionRequest {
        source_revision: RevisionId::new(885),
        target_revision: RevisionId::new(886),
        rewrites: &rewrites,
    };
    assert!(matches!(
        runtime
            .commit_derived_relation_rewrites(transaction_id, &request)
            .unwrap(),
        DurableRuntimeCommitOutcome::Committed(_)
    ));
    runtime.checkpoint().unwrap();
    runtime.compact_obsolete_generations().unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_relation_rewrite_causal_identity(
        &reopened,
        transaction_id,
        relation,
        &spec,
        RevisionId::new(885),
        RevisionId::new(886),
    );
    assert_eq!(
        reopened
            .commit_derived_relation_rewrites(transaction_id, &request)
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: RevisionId::new(886),
        }
    );

    let conflicting_spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(88_503)),
        law_set: RewriteLawSetId(SemanticId::new(88_504)),
        footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
    };
    let conflicting_rewrite = delta
        .prepare_relation_rewrite(
            relation,
            &old,
            &context,
            &registry,
            &conflicting_spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    let conflicting_rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &conflicting_rewrite,
    }];
    assert!(matches!(
        reopened.commit_derived_relation_rewrites(
            transaction_id,
            &DerivedRelationRewriteTransitionRequest {
                source_revision: RevisionId::new(885),
                target_revision: RevisionId::new(886),
                rewrites: &conflicting_rewrites,
            },
        ),
        Err(DurableRuntimeCommitError::TransactionIdConflict { .. })
    ));
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn coherent_resolution_gate_precedes_durable_rewrite_publication() {
    let dir = durable_test_dir("coherent-resolution-publication");
    let (context, registry, relation, _, root) = scan_runtime_bundle(887, 1307, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let transaction_id = ClientTransactionId::new(0x8871);
    let delta = scan_delta(relation, &[2], &[1], &context, &registry);
    let old = RelExpr::Scan(relation)
        .evaluate(
            &runtime.snapshot().unwrap().revision().state().model,
            &context,
            &registry,
        )
        .unwrap();
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(88_701)),
        law_set: RewriteLawSetId(SemanticId::new(88_702)),
        footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
    };
    let rewrite = delta
        .prepare_relation_rewrite(
            relation,
            &old,
            &context,
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &rewrite,
    }];
    let request = DerivedRelationRewriteTransitionRequest {
        source_revision: RevisionId::new(887),
        target_revision: RevisionId::new(888),
        rewrites: &rewrites,
    };

    assert!(matches!(
        runtime.commit_derived_coherent_resolution(
            transaction_id,
            &request,
            relation_cube_certificate(&old, &old),
        ),
        Err(DurableRuntimeCommitError::Runtime(
            PhysicalExecutionError::ResolutionCoherenceEndpointMismatch
        ))
    ));
    assert_eq!(
        runtime.snapshot().unwrap().revision().id(),
        RevisionId::new(887)
    );

    let endpoint = rewrite.apply_structural(&old, &registry).unwrap();
    assert!(matches!(
        runtime
            .commit_derived_coherent_resolution(
                transaction_id,
                &request,
                relation_cube_certificate(&old, &endpoint),
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::Committed(_)
    ));
    assert_eq!(
        runtime.snapshot().unwrap().revision().id(),
        RevisionId::new(888)
    );
    assert_relation_rewrite_causal_identity(
        &runtime,
        transaction_id,
        relation,
        &spec,
        RevisionId::new(887),
        RevisionId::new(888),
    );
    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

fn assert_reopened_effect_prerequisites(
    dir: &std::path::Path,
    effect: kernel_change::RevisionEffectId,
    expected: &BTreeSet<kernel_change::RevisionEffectId>,
) {
    let (reopened, _) = DurableRevisionStore::open(dir).unwrap();
    assert_eq!(
        reopened
            .revision_effect_record(effect)
            .unwrap()
            .prerequisites,
        *expected
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn multi_parent_coherent_resolution_persists_exact_parent_cut() {
    let dir = durable_test_dir("multi-parent-coherent-resolution");
    let (context, registry, relation, _, root) = scan_runtime_bundle(900, 1320, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();

    for (source, target, value, transaction) in [
        (900, 901, 2, ClientTransactionId::new(0x9001)),
        (901, 902, 3, ClientTransactionId::new(0x9002)),
    ] {
        let delta = scan_delta(relation, &[value], &[], &context, &registry);
        let mutations = [RevisionRelationMutation {
            relation,
            delta: &delta,
            object_field_writes: &[],
            authorization: kernel_durability::DurableRelationAuthorization::default(),
        }];
        runtime
            .commit_derived_relation_data(
                transaction,
                &DerivedRelationTransitionRequest {
                    source_revision: RevisionId::new(source),
                    target_revision: RevisionId::new(target),
                    mutations: &mutations,
                },
            )
            .unwrap();
    }

    let old = RelExpr::Scan(relation)
        .evaluate(
            &runtime.snapshot().unwrap().revision().state().model,
            &context,
            &registry,
        )
        .unwrap();
    let delta = scan_delta(relation, &[4], &[], &context, &registry);
    let spec = kernel_change::RewriteSpec {
        id: RewriteSpecId(SemanticId::new(90_301)),
        law_set: RewriteLawSetId(SemanticId::new(90_302)),
        footprint: kernel_change::RewriteFootprint::opaque_relation(relation),
    };
    let rewrite = delta
        .prepare_relation_rewrite(
            relation,
            &old,
            &context,
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    let rewrites = [RevisionRelationRewrite {
        relation,
        rewrite: &rewrite,
    }];
    let request = DerivedRelationRewriteTransitionRequest {
        source_revision: RevisionId::new(902),
        target_revision: RevisionId::new(903),
        rewrites: &rewrites,
    };
    let transaction_id = ClientTransactionId::new(0x9003);
    let endpoint = rewrite.apply_structural(&old, &registry).unwrap();
    runtime
        .commit_derived_multi_parent_residual_chain_resolution(
            transaction_id,
            &request,
            &relation_residual_chain_certificate(&old, &endpoint),
            &[RevisionId::new(901), RevisionId::new(902)],
        )
        .unwrap();

    let (parent_901, parent_902, resolution_effect) =
        runtime.with_durability_for_test(|durability| {
            assert!(matches!(
                durability.transaction_intent(transaction_id),
                Some(kernel_durability::DurableCommittedTransaction {
                    intent: kernel_durability::DurableClientIntent::RelationResolution { causal_parents, .. },
                    ..
                }) if causal_parents == &vec![RevisionId::new(901), RevisionId::new(902)]
            ));
            let parent_901 = *durability
                .revision_effect_frontier(RevisionId::new(901))
                .unwrap()
                .iter()
                .next()
                .unwrap();
            let parent_902 = *durability
                .revision_effect_frontier(RevisionId::new(902))
                .unwrap()
                .iter()
                .next()
                .unwrap();
            let resolution_effect = *durability
                .revision_effect_frontier(RevisionId::new(903))
                .unwrap()
                .iter()
                .next()
                .unwrap();
            assert_eq!(
                durability
                    .revision_effect_record(resolution_effect)
                    .unwrap()
                    .prerequisites,
                BTreeSet::from([parent_901, parent_902])
            );
            (parent_901, parent_902, resolution_effect)
        });
    drop(runtime);

    assert_reopened_effect_prerequisites(
        &dir,
        resolution_effect,
        &BTreeSet::from([parent_901, parent_902]),
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn derived_relation_commit_matches_authoritative_target_and_rejects_stale_source() {
    let dir = durable_test_dir("derived-relation-authority-and-stale-source");
    let (context, registry, relation, _, root) = scan_runtime_bundle(878, 1298, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();

    let first_delta = scan_delta(relation, &[2], &[], &context, &registry);
    let first_mutations = [RevisionRelationMutation {
        relation,
        delta: &first_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let source = runtime.snapshot().unwrap();
    let expected = target_revision_for(source.root(), 879, &first_mutations, &registry);
    drop(source);
    runtime
        .commit_derived_relation_data(
            ClientTransactionId::new(0x8781),
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(878),
                target_revision: RevisionId::new(879),
                mutations: &first_mutations,
            },
        )
        .unwrap();
    let committed = runtime.snapshot().unwrap();
    assert_eq!(committed.revision().id(), expected.id());
    assert_eq!(
        committed.revision().semantic_context(),
        expected.semantic_context()
    );
    assert_eq!(committed.revision().state(), expected.state());
    assert_eq!(
        committed.revision().model_rule_witnesses(),
        expected.model_rule_witnesses()
    );

    let stale_delta = scan_delta(relation, &[3], &[], &context, &registry);
    let stale_mutations = [RevisionRelationMutation {
        relation,
        delta: &stale_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let stale_transaction = ClientTransactionId::new(0x8782);
    let stale_request = DerivedRelationTransitionRequest {
        source_revision: RevisionId::new(878),
        target_revision: RevisionId::new(880),
        mutations: &stale_mutations,
    };
    assert!(matches!(
        runtime.commit_derived_relation_data(stale_transaction, &stale_request),
        Err(DurableRuntimeCommitError::Runtime(
            PhysicalExecutionError::InvalidRevisionTransition
        ))
    ));
    assert_eq!(
        runtime.snapshot().unwrap().revision().id(),
        RevisionId::new(879)
    );
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(reopened.snapshot().unwrap().revision(), &expected);
    assert!(matches!(
        reopened.commit_derived_relation_data(stale_transaction, &stale_request),
        Err(DurableRuntimeCommitError::Runtime(
            PhysicalExecutionError::InvalidRevisionTransition
        ))
    ));
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn revision_and_materialization_registry_commit_and_recover_atomically() {
    let dir = durable_test_dir("revision-and-materializations-atomic");
    let (context, registry, relation, _, root) = scan_runtime_bundle(868, 1286, &[1, 2, 3]);
    let equivalence = match &context.schema.relation(relation).unwrap().semantics {
        RelationSemantics::Bag {
            column_equivalences,
        }
        | RelationSemantics::Set {
            column_equivalences,
        } => column_equivalences[0],
    };
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let delta = scan_delta(relation, &[4], &[1], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let snapshot = runtime.snapshot().unwrap();
    let target = target_revision_for(snapshot.root(), 869, &mutations, &registry);
    drop(snapshot);
    let filtered_id = MaterializationId::new(2);
    let desired = [RuntimeMaterializationSpec {
        id: filtered_id,
        query: RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(2),
            equivalence,
        },
    }];
    let transaction_id = ClientTransactionId::new(0x868);
    assert!(matches!(
        runtime
            .replace_revision_and_materializations(
                transaction_id,
                &RevisionAndMaterializationsTransitionRequest {
                    target_revision: &target,
                    materializations: &desired,
                    registry: &registry,
                },
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::Committed(_)
    ));

    let snapshot = runtime.snapshot().unwrap();
    assert_eq!(snapshot.revision(), &target);
    assert!(
        snapshot
            .materialization(test_materialization_id())
            .is_none()
    );
    assert_eq!(
        snapshot
            .materialization(filtered_id)
            .unwrap()
            .output_value(&context, &registry)
            .unwrap()
            .rows(),
        &[vec![Value::I64(2)]]
    );
    drop(snapshot);
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert_eq!(snapshot.revision(), &target);
    assert!(
        snapshot
            .materialization(test_materialization_id())
            .is_none()
    );
    assert_eq!(
        snapshot
            .materialization(filtered_id)
            .unwrap()
            .output_value(&context, reopened.semantic_registry())
            .unwrap()
            .rows(),
        &[vec![Value::I64(2)]]
    );
    drop(snapshot);
    assert_eq!(
        reopened
            .replace_revision_and_materializations(
                transaction_id,
                &RevisionAndMaterializationsTransitionRequest {
                    target_revision: &target,
                    materializations: &desired,
                    registry: &registry,
                },
            )
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted {
            target_revision: RevisionId::new(869),
        }
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn materialization_configuration_evolves_durably_and_reopens_without_caller_specs() {
    let dir = durable_test_dir("materialization-reconfigure");
    let (context, registry, relation, _, root) = scan_runtime_bundle(870, 1281, &[1, 2, 3]);
    let equivalence = match &context.schema.relation(relation).unwrap().semantics {
        RelationSemantics::Bag {
            column_equivalences,
        }
        | RelationSemantics::Set {
            column_equivalences,
        } => column_equivalences[0],
    };
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let filtered_id = MaterializationId::new(2);
    let desired = [RuntimeMaterializationSpec {
        id: filtered_id,
        query: RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(2),
            equivalence,
        },
    }];

    assert!(matches!(
        runtime.reconfigure_materializations(&desired).unwrap(),
        DurableMaterializationConfigOutcome::Applied(_)
    ));
    let snapshot = runtime.snapshot().unwrap();
    assert!(
        snapshot
            .materialization(test_materialization_id())
            .is_none()
    );
    assert!(snapshot.materialization(filtered_id).is_some());
    drop(snapshot);
    assert_eq!(
        runtime.reconfigure_materializations(&desired).unwrap(),
        DurableMaterializationConfigOutcome::AlreadyApplied
    );
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    assert!(
        snapshot
            .materialization(test_materialization_id())
            .is_none()
    );
    let maintained = snapshot.materialization(filtered_id).unwrap();
    assert_eq!(
        maintained.output_value(&context, &registry).unwrap().rows(),
        &[vec![Value::I64(2)]]
    );
    drop(snapshot);
    assert_eq!(
        reopened.reconfigure_materializations(&desired).unwrap(),
        DurableMaterializationConfigOutcome::AlreadyApplied
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn invalid_materialization_reconfiguration_is_rejected_before_durable_publication() {
    let dir = durable_test_dir("invalid-materialization-reconfigure");
    let (_context, registry, _relation, _, root) = scan_runtime_bundle(880, 1282, &[1, 2]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let before = runtime.snapshot().unwrap();
    let bad = [RuntimeMaterializationSpec {
        id: MaterializationId::new(99),
        query: RelExpr::Scan(sid(999_999)),
    }];
    assert!(matches!(
        runtime.reconfigure_materializations(&bad),
        Err(DurableRuntimeCheckpointError::Runtime(_))
    ));
    let after = runtime.snapshot().unwrap();
    assert_eq!(after.revision(), before.revision());
    assert!(after.materialization(test_materialization_id()).is_some());
    drop(before);
    drop(after);
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert!(
        reopened
            .snapshot()
            .unwrap()
            .materialization(test_materialization_id())
            .is_some()
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn causal_history_release_reclaims_p462_derived_timeline_and_reopens_at_new_floor() {
    let dir = durable_test_dir("causal-history-release-p462-derived");
    let (context, registry, relation, _, root) = scan_runtime_bundle(9_100, 13_100, &[1]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let delta = scan_delta(relation, &[2], &[], &context, &registry);
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    runtime
        .commit_derived_relation_data(
            ClientTransactionId::new(0x4631),
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(9_100),
                target_revision: RevisionId::new(9_101),
                mutations: &mutations,
            },
        )
        .unwrap();

    let retained = runtime.snapshot().unwrap();
    assert_eq!(retained.historical_exact_effect_count_for_test(), 1);
    let probe = retained.historical_exact_effect_storage_probe_for_test();
    assert!(probe.total_nodes() > 0);

    assert!(
        runtime
            .release_causal_history_before_head()
            .unwrap()
            .is_some()
    );
    let current = runtime.snapshot().unwrap();
    assert_eq!(
        current.historical_lineage_floor_for_test(),
        Some(RevisionId::new(9_101))
    );
    assert_eq!(current.historical_exact_effect_count_for_test(), 0);
    assert!(matches!(
        runtime.certify_transition_rebase(RevisionId::new(9_100), &mutations, None, None),
        Err(RuntimeHistoricalSnapshotError::Unavailable { revision }) if revision == RevisionId::new(9_100)
    ));
    assert!(!probe.is_fully_reclaimed());
    drop(retained);
    assert!(probe.is_fully_reclaimed());
    drop(current);
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(
        reopened.causal_coverage_root().unwrap(),
        RevisionId::new(9_101)
    );
    assert!(
        reopened
            .revision_effect_ideal(RevisionId::new(9_101))
            .unwrap()
            .unwrap()
            .events()
            .is_empty()
    );
    assert!(matches!(
        reopened.certify_transition_rebase(RevisionId::new(9_100), &mutations, None, None),
        Err(RuntimeHistoricalSnapshotError::Unavailable { revision }) if revision == RevisionId::new(9_100)
    ));
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn security_guard_dependency_is_sealed_with_residual_publication() {
    let dir = durable_test_dir("security-guard-dependency-seal");
    let relation = sid(9_801);
    let equivalence = sid(9_802);
    let entity_type = sid(9_803);
    let field = sid(9_804);
    let entity = EntityId::new(9801);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence_revision(EquivalenceModule::TextExact, 11);
    let context = migration_context(
        9801,
        9801,
        relation,
        equivalence,
        entity_type,
        field,
        digest,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9801),
        &context,
        &registry,
        migration_state(entity_type, field, relation, &[(entity, 7)], &["A"]),
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source.clone(), "A", &registry);

    let mut guard_changed_state = source.state().clone();
    guard_changed_state.lifecycle.roots.remove(&entity);
    let guard_changed = kernel_revision::Revision::build(
        RevisionId::new(9802),
        &context,
        &registry,
        guard_changed_state,
    )
    .unwrap();
    let guard_delta = DurableModelDelta::between(source.state(), guard_changed.state());
    let guard_complement = DurableModelDelta::between(guard_changed.state(), source.state());
    runtime
        .commit_mixed_revision(
            ClientTransactionId::new(0x9802),
            &MixedRevisionTransitionRequest {
                source_revision: source.id(),
                target_revision: &guard_changed,
                mutations: &[],
                model_delta: &guard_delta,
                model_complement: &guard_complement,
                registry: &registry,
            },
        )
        .unwrap();

    let mut effect_state = guard_changed.state().clone();
    effect_state
        .model
        .fields
        .insert((field, entity), Value::I64(8));
    let effect_target =
        kernel_revision::Revision::build(RevisionId::new(9803), &context, &registry, effect_state)
            .unwrap();
    let effect_delta = DurableModelDelta::between(guard_changed.state(), effect_target.state());
    let effect_complement =
        DurableModelDelta::between(effect_target.state(), guard_changed.state());
    let request = MixedRevisionTransitionRequest {
        source_revision: guard_changed.id(),
        target_revision: &effect_target,
        mutations: &[],
        model_delta: &effect_delta,
        model_complement: &effect_complement,
        registry: &registry,
    };
    let guard_digest = kernel_durability::ClientIntentGuardDigest::canonical(b"root-present");
    let guard_observation = RuntimeGuardObservationFootprint::new(
        source.id(),
        [
            RuntimeHistoryCoordinate::LifecycleRoot { entity },
            RuntimeHistoryCoordinate::LifecycleRoot { entity },
        ],
    )
    .unwrap();
    assert_eq!(guard_observation.source_revision(), source.id());
    assert_eq!(guard_observation.coordinates().len(), 1);
    let result = runtime.commit_mixed_revision_residual_guarded_with_dependencies(
        ClientTransactionId::new(0x9803),
        &request,
        &[],
        &effect_delta,
        Some(guard_digest),
        Some(&guard_observation),
    );
    assert!(matches!(
        result,
        Err(DurableRuntimeCommitError::GuardDependencyConflict(_))
    ));
    assert_eq!(
        runtime.snapshot().unwrap().revision().id(),
        guard_changed.id()
    );

    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep explicit semantic and durability inputs at this boundary."
)]
fn p465_security_contexts(
    source_schema_revision: u64,
    target_schema_revision: u64,
    relation: SemanticId,
    equivalence: SemanticId,
    entity_type: SemanticId,
    source_fields: [SemanticId; 3],
    target_fields: [SemanticId; 3],
    registry: &mut SemanticRegistry,
) -> (
    SemanticContext,
    SemanticContext,
    kernel_transport::SchemaMigrationProgram,
) {
    let digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::TextExact);
    let mut source_schema = Schema::new(SchemaRevisionId::new(source_schema_revision));
    for field in source_fields {
        source_schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
    }
    source_schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let mut source_environment =
        SemanticEnvironment::new(SemanticEnvId::new(source_schema_revision));
    source_environment.pin_module(equivalence, digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(target_schema_revision));
    for field in target_fields {
        target_schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
    }
    target_schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let mut target_environment =
        SemanticEnvironment::new(SemanticEnvId::new(target_schema_revision));
    target_environment.pin_module(equivalence, digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let rewrites = source_fields
        .into_iter()
        .zip(target_fields)
        .map(
            |(source_field, target_field)| kernel_transport::MigrationFieldRewrite {
                source_fields: vec![source_field],
                target_field,
                transform: kernel_query::ExactQuery::new(kernel_query::Expr::ProductField {
                    input: Box::new(kernel_query::Expr::Input),
                    field: source_field,
                }),
            },
        )
        .collect();
    let program =
        kernel_transport::SchemaMigrationProgram::new(target.clone(), rewrites, Vec::new());
    (source, target, program)
}

fn p465_security_state(
    entity_type: SemanticId,
    fields: [SemanticId; 3],
    relation: SemanticId,
    entity: EntityId,
    values: [i64; 3],
) -> kernel_model::DatabaseState {
    let mut state = kernel_model::DatabaseState::default();
    state.lifecycle.entities.insert(entity);
    state.lifecycle.roots.insert(entity);
    state
        .model
        .carriers
        .entry(entity_type)
        .or_default()
        .insert(entity);
    for (field, value) in fields.into_iter().zip(values) {
        state
            .model
            .fields
            .insert((field, entity), Value::I64(value));
    }
    state
        .model
        .relations
        .insert(relation, vec![vec![Value::Text("security-anchor".into())]]);
    state
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p465_schema_aware_field_walker_transports_guarded_intent_atomically() {
    let dir = durable_test_dir("p465-schema-aware-field-walker");
    let relation = sid(9_801);
    let equivalence = sid(9_802);
    let entity_type = sid(9_803);
    let source_fields = [sid(9_804), sid(9_805), sid(9_806)];
    let target_fields = [sid(9_814), sid(9_815), sid(9_816)];
    let final_fields = [sid(9_824), sid(9_825), sid(9_826)];
    let entity = EntityId::new(9800);
    let mut registry = SemanticRegistry::default();
    let (source_context, target_context, program) = p465_security_contexts(
        9800,
        9801,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let (second_source_context, _, second_program) = p465_security_contexts(
        9801,
        9802,
        relation,
        equivalence,
        entity_type,
        target_fields,
        final_fields,
        &mut registry,
    );
    assert_eq!(second_source_context, target_context);
    let source = kernel_revision::Revision::build(
        RevisionId::new(9800),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime =
        create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let mut source_after_state = source.state().clone();
    source_after_state
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(200));
    source_after_state
        .model
        .fields
        .insert((source_fields[1], entity), Value::I64(1));
    let source_after = kernel_revision::Revision::build(
        RevisionId::new(98_099),
        &source_context,
        &registry,
        source_after_state,
    )
    .unwrap();
    let client_delta = DurableModelDelta::between(source.state(), source_after.state());
    let guard = RuntimeGuardObservationFootprint::new(
        source.id(),
        [RuntimeHistoryCoordinate::Field {
            field: source_fields[2],
            owner: entity,
        }],
    )
    .unwrap();

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source, RevisionId::new(9801), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_817)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_818)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9800),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let digest = kernel_durability::ClientIntentGuardDigest::canonical(b"security-generation==7");
    let tx = ClientTransactionId::new(0x9801);
    let request = SchemaAwareFieldTransitionRequest {
        formation_revision: source.id(),
        formation_semantic_revision: source_context.revision(),
        client_model_delta: &client_delta,
        guard_observation: Some(&guard),
        client_guard_digest: Some(digest),
    };
    let prepared = runtime
        .prepare_schema_aware_field_publication(&request, &BTreeSet::new())
        .unwrap();
    assert_eq!(prepared.authorized_head_revision, migrated.id());
    assert!(!prepared.current_model_delta().fields.is_empty());
    let outcome = runtime
        .commit_prepared_schema_aware_publication(tx, &prepared)
        .unwrap();
    assert!(matches!(outcome, DurableRuntimeCommitOutcome::Committed(_)));
    let head = runtime.snapshot().unwrap();
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[0], entity)],
        Value::I64(200)
    );
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[1], entity)],
        Value::I64(1)
    );
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[2], entity)],
        Value::I64(7)
    );
    drop(head);

    // Advance the realized world through another schema epoch. Retry identity must
    // remain the original A intent and must not be rebound to B or C semantics.
    let current = runtime.snapshot().unwrap().revision().clone();
    let second_transport = second_program
        .verify(&second_source_context, &registry)
        .unwrap();
    let migrated_again = second_transport
        .transport_revision(&current, RevisionId::new(9803), &registry)
        .unwrap();
    let second_complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: second_source_context.schema.revision,
            target_schema: second_program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_827)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_828)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9802),
            &FullRevisionTransitionRequest {
                target_revision: &migrated_again,
                registry: &registry,
            },
            &second_program,
            &second_complement,
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();

    // Fresh A intent after restart must use retained per-epoch action roots, not
    // replay the A/B causal segments. It changes a coordinate untouched by the
    // earlier transported intent, so the exact transport is admissible.
    let mut late_source_state = source.state().clone();
    late_source_state
        .model
        .fields
        .insert((source_fields[2], entity), Value::I64(9));
    let late_delta = DurableModelDelta::between(source.state(), &late_source_state);
    let late = reopened
        .commit_schema_aware_field_intent(
            ClientTransactionId::new(0x9803),
            &SchemaAwareFieldTransitionRequest {
                formation_revision: source.id(),
                formation_semantic_revision: source_context.revision(),
                client_model_delta: &late_delta,
                guard_observation: None,
                client_guard_digest: None,
            },
        )
        .unwrap();
    assert!(matches!(late, DurableRuntimeCommitOutcome::Committed(_)));
    assert_eq!(
        reopened.snapshot().unwrap().revision().state().model.fields[&(final_fields[2], entity)],
        Value::I64(9),
    );

    let retry = reopened
        .commit_schema_aware_field_intent(tx, &request)
        .unwrap();
    assert!(matches!(
        retry,
        DurableRuntimeCommitOutcome::AlreadyCommitted { .. }
    ));
    assert_eq!(
        reopened.snapshot().unwrap().revision().id(),
        RevisionId::new(migrated_again.id().raw() + 1),
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete mixed schema-aware publication case together."
)]
fn p508_prepared_schema_aware_publication_unifies_relation_and_field_effects() {
    let dir = durable_test_dir("p508-mixed-prepared-publication");
    let relation = sid(9_861);
    let equivalence = sid(9_862);
    let entity_type = sid(9_863);
    let source_fields = [sid(9_864), sid(9_865), sid(9_866)];
    let target_fields = [sid(9_874), sid(9_875), sid(9_876)];
    let entity = EntityId::new(9_860);
    let mut registry = SemanticRegistry::default();
    let (source_context, _target_context, program) = p465_security_contexts(
        9_860,
        9_861,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9_860),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let added_entity = EntityId::new(9_861);
    let mut changed = source.state().clone();
    changed
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(200));
    changed.lifecycle.entities.insert(added_entity);
    changed.lifecycle.roots.insert(added_entity);
    changed
        .lifecycle
        .keeps_alive
        .entry(entity)
        .or_default()
        .insert(added_entity);
    changed
        .model
        .carriers
        .entry(entity_type)
        .or_default()
        .insert(added_entity);
    for (field, value) in source_fields.into_iter().zip([300, 0, 9]) {
        changed
            .model
            .fields
            .insert((field, added_entity), Value::I64(value));
    }
    let mixed_model_delta = DurableModelDelta::between(source.state(), &changed);
    let source_type = RelExpr::Scan(relation)
        .typecheck(&source_context, &registry)
        .unwrap();
    let relation_delta = RelationDelta {
        inserted: vec![vec![Value::Text("security-next".into())]],
        removed: vec![vec![Value::Text("security-anchor".into())]],
        result_type: source_type,
    };
    let mutations = [RevisionRelationMutation {
        relation,
        delta: &relation_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source, RevisionId::new(9_861), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_877)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_878)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9860),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let current_bridge = runtime
        .current_schema_bridge(source_context.schema.revision)
        .unwrap()
        .expect("retained migration lineage must compile current-world bridge");
    assert_eq!(current_bridge.source_context(), &source_context);
    assert_eq!(current_bridge.target_context(), program.target());
    assert_eq!(current_bridge.step_count(), 1);
    assert_eq!(
        current_bridge
            .compile_read_exact(&RelExpr::Scan(relation), &registry)
            .unwrap(),
        RelExpr::Scan(relation)
    );
    assert_eq!(
        current_bridge
            .transport_relation_delta_exact(relation, &relation_delta, &registry)
            .unwrap(),
        vec![(relation, relation_delta.clone())]
    );
    let relation_column = source_context
        .schema
        .relation_column_id(relation, 0)
        .unwrap();
    assert_eq!(
        current_bridge
            .transport_relation_write_footprint_exact(
                relation,
                &BTreeSet::from([relation_column]),
            )
            .unwrap(),
        vec![kernel_transport::MigrationRelationWriteFootprint {
            target_relation: relation,
            target_columns: BTreeSet::from([relation_column]),
        }]
    );

    let witness = runtime
        .schema_aware_formation_context_witness(source.id(), source_context.revision())
        .unwrap();
    assert_eq!(witness.formation_revision(), source.id());
    assert_eq!(witness.semantic_context(), &source_context);
    assert_eq!(witness.authorized_head_revision(), migrated.id());

    let prepared = runtime
        .prepare_schema_aware_publication_with_witness(
            &witness,
            &mutations,
            &mixed_model_delta,
            &BTreeSet::new(),
            &BTreeSet::new(),
            None,
        )
        .unwrap();
    assert!(!prepared.current_model_delta().fields.is_empty());
    assert_eq!(prepared.current_model_delta().carriers.len(), 1);
    assert_eq!(
        prepared.current_model_delta().lifecycle_entities_inserted,
        vec![added_entity]
    );
    assert_eq!(
        prepared.current_model_delta().lifecycle_roots_inserted,
        vec![added_entity]
    );
    assert_eq!(prepared.current_model_delta().lifecycle_keeps_alive.len(), 1);
    let authority = prepared.current_model_authority_footprint();
    assert!(authority.carrier_presence.is_empty());
    assert_eq!(
        authority.carrier_members,
        BTreeSet::from([(entity_type, added_entity)])
    );
    assert_eq!(authority.lifecycle_entities, BTreeSet::from([added_entity]));
    assert_eq!(authority.lifecycle_roots, BTreeSet::from([added_entity]));
    assert_eq!(authority.keeps_alive_presence, BTreeSet::from([entity]));
    assert_eq!(
        authority.keeps_alive_edges,
        BTreeSet::from([(entity, added_entity)])
    );
    assert_eq!(prepared.current_relation_deltas().count(), 1);
    let tx = ClientTransactionId::new(0x9861);
    assert!(matches!(
        runtime
            .commit_prepared_schema_aware_publication(tx, &prepared)
            .unwrap(),
        DurableRuntimeCommitOutcome::Committed(_)
    ));
    let head = runtime.snapshot().unwrap();
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[0], entity)],
        Value::I64(200)
    );
    assert!(head
        .revision()
        .state()
        .lifecycle
        .roots
        .contains(&added_entity));
    assert!(head
        .revision()
        .state()
        .lifecycle
        .keeps_alive
        .get(&entity)
        .is_some_and(|children| children.contains(&added_entity)));
    assert_eq!(
        head.revision()
            .state()
            .model
            .relations
            .materialize_owned(&relation),
        Some(vec![vec![Value::Text("security-next".into())]])
    );
    drop(head);
    assert!(matches!(
        runtime
            .commit_prepared_schema_aware_publication(tx, &prepared)
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted { .. }
    ));
    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "manual release-mode retained relational formation-lineage benchmark"]
fn p512_relational_formation_lineage_cost_tracks_relevant_deltas_not_total_history() {
    let iterations = 2_000;
    let (short, short_selected) =
        crate::runtime_impl::benchmark_relational_formation_lineage_for_test(1_000, 100, iterations);
    let (long, long_selected) =
        crate::runtime_impl::benchmark_relational_formation_lineage_for_test(100_000, 100, iterations);
    eprintln!(
        "PASS512 relational formation lineage: 1k={short:?} 100k={long:?} selected={short_selected}/{long_selected} iterations={iterations}"
    );
    assert!(short_selected <= 100);
    assert!(long_selected <= 100);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete durable no-op schema-aware publication case together."
)]
fn p510_same_idempotent_model_overlap_seals_retry_identity_without_fake_revision() {
    let dir = durable_test_dir("p510-schema-aware-noop-intent-seal");
    let relation = sid(9_891);
    let equivalence = sid(9_892);
    let entity_type = sid(9_893);
    let source_fields = [sid(9_894), sid(9_895), sid(9_896)];
    let target_fields = [sid(9_904), sid(9_905), sid(9_906)];
    let entity = EntityId::new(9_890);
    let mut registry = SemanticRegistry::default();
    let (source_context, _target_context, program) = p465_security_contexts(
        9_890,
        9_892,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let child = EntityId::new(9_891);
    let mut source_state =
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]);
    source_state.lifecycle.entities.insert(child);
    source_state.lifecycle.roots.insert(child);
    source_state
        .lifecycle
        .keeps_alive
        .entry(entity)
        .or_default()
        .insert(child);
    source_state
        .model
        .carriers
        .entry(entity_type)
        .or_default()
        .insert(child);
    for (field, value) in source_fields.into_iter().zip([101, 0, 8]) {
        source_state
            .model
            .fields
            .insert((field, child), Value::I64(value));
    }
    let source = kernel_revision::Revision::build(
        RevisionId::new(9_890),
        &source_context,
        &registry,
        source_state,
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let mut satisfied_state = source.state().clone();
    satisfied_state.lifecycle.roots.remove(&child);
    let stale_client_delta = DurableModelDelta::between(source.state(), &satisfied_state);
    assert_eq!(stale_client_delta.lifecycle_roots_removed, vec![child]);
    let satisfied_revision = kernel_revision::Revision::build(
        RevisionId::new(9_891),
        &source_context,
        &registry,
        satisfied_state,
    )
    .unwrap();
    let satisfied_complement =
        DurableModelDelta::between(satisfied_revision.state(), source.state());
    runtime
        .commit_mixed_revision(
            ClientTransactionId::new(0x9890),
            &MixedRevisionTransitionRequest {
                source_revision: source.id(),
                target_revision: &satisfied_revision,
                mutations: &[],
                model_delta: &stale_client_delta,
                model_complement: &satisfied_complement,
                registry: &registry,
            },
        )
        .unwrap();

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&satisfied_revision, RevisionId::new(9_892), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_907)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_908)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9891),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let prepared = runtime
        .prepare_schema_aware_publication(
            source.id(),
            source_context.revision(),
            &[],
            &stale_client_delta,
            &BTreeSet::new(),
            &BTreeSet::new(),
            None,
        )
        .unwrap();
    let head_before = runtime.snapshot().unwrap().revision().id();
    let tx = ClientTransactionId::new(0x9892);
    assert!(matches!(
        runtime
            .commit_prepared_schema_aware_publication(tx, &prepared)
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadySatisfied { target_revision }
            if target_revision == head_before
    ));
    assert_eq!(runtime.snapshot().unwrap().revision().id(), head_before);
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(reopened.snapshot().unwrap().revision().id(), head_before);
    assert!(matches!(
        reopened
            .commit_prepared_schema_aware_publication(tx, &prepared)
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted { target_revision }
            if target_revision == head_before
    ));
    assert_eq!(reopened.snapshot().unwrap().revision().id(), head_before);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p479_formation_world_seal_does_not_revalidate_a_guard_in_target_epoch() {
    let dir = durable_test_dir("p465-schema-aware-field-guard-conflict");
    let relation = sid(9_821);
    let equivalence = sid(9_822);
    let entity_type = sid(9_823);
    let source_fields = [sid(9_824), sid(9_825), sid(9_826)];
    let target_fields = [sid(9_834), sid(9_835), sid(9_836)];
    let entity = EntityId::new(9820);
    let mut registry = SemanticRegistry::default();
    let (source_context, _, program) = p465_security_contexts(
        9820,
        9821,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9820),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime =
        create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);
    let mut source_after_state = source.state().clone();
    source_after_state
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(200));
    source_after_state
        .model
        .fields
        .insert((source_fields[1], entity), Value::I64(1));
    let source_after = kernel_revision::Revision::build(
        RevisionId::new(98_299),
        &source_context,
        &registry,
        source_after_state,
    )
    .unwrap();
    let client_delta = DurableModelDelta::between(source.state(), source_after.state());
    let guard_coordinate = RuntimeHistoryCoordinate::Field {
        field: source_fields[2],
        owner: entity,
    };
    let guard = RuntimeGuardObservationFootprint::with_exact_values_and_rules(
        source.id(),
        [(guard_coordinate, Some(Value::I64(7)), Some(
            kernel_schema::SemanticRuleExpr::I64Range {
                value: kernel_schema::RuleValueExpr::Input,
                min: Some(0),
                max: Some(10),
            },
        ))],
    )
    .unwrap();

    let mut source_boundary_state = source.state().clone();
    source_boundary_state
        .model
        .fields
        .insert((source_fields[2], entity), Value::I64(8));
    let source_boundary = kernel_revision::Revision::build(
        RevisionId::new(9821),
        &source_context,
        &registry,
        source_boundary_state,
    )
    .unwrap();
    let source_boundary_delta = DurableModelDelta::between(source.state(), source_boundary.state());
    let source_boundary_complement = DurableModelDelta::between(source_boundary.state(), source.state());
    runtime
        .commit_mixed_revision(
            ClientTransactionId::new(0x981f),
            &MixedRevisionTransitionRequest {
                source_revision: source.id(),
                target_revision: &source_boundary,
                mutations: &[],
                model_delta: &source_boundary_delta,
                model_complement: &source_boundary_complement,
                registry: &registry,
            },
        )
        .unwrap();
    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source_boundary, RevisionId::new(9822), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_837)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_838)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9820),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let current = runtime.snapshot().unwrap().revision().clone();
    let mut changed_state = current.state().clone();
    changed_state
        .model
        .fields
        .insert((target_fields[2], entity), Value::I64(99));
    let changed = kernel_revision::Revision::build(
        RevisionId::new(9823),
        current.semantic_context(),
        &registry,
        changed_state,
    )
    .unwrap();
    let changed_delta = DurableModelDelta::between(current.state(), changed.state());
    let changed_complement = DurableModelDelta::between(changed.state(), current.state());
    runtime
        .commit_mixed_revision(
            ClientTransactionId::new(0x9821),
            &MixedRevisionTransitionRequest {
                source_revision: current.id(),
                target_revision: &changed,
                mutations: &[],
                model_delta: &changed_delta,
                model_complement: &changed_complement,
                registry: &registry,
            },
        )
        .unwrap();

    let prepared = runtime
        .prepare_schema_aware_publication_with_formation_proof(
            source.id(),
            source_context.revision(),
            &[],
            &client_delta,
            &BTreeSet::new(),
            &BTreeSet::new(),
            Some(kernel_durability::ClientIntentGuardDigest::canonical(
                b"security-generation==7",
            )),
            Some(&guard),
            &[],
        )
        .unwrap();
    let result = runtime.commit_prepared_schema_aware_publication(
        ClientTransactionId::new(0x9822),
        &prepared,
    );
    assert!(matches!(
        result,
        Ok(DurableRuntimeCommitOutcome::Committed(_))
    ));
    let head = runtime.snapshot().unwrap();
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[0], entity)],
        Value::I64(200)
    );
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[1], entity)],
        Value::I64(1)
    );
    // The B-native write happened after the logical A serialization point. It remains
    // authoritative; the old A guard is not transported forward and re-evaluated in B.
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[2], entity)],
        Value::I64(99)
    );
    drop(head);
    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p479_formation_world_seal_rejects_guard_change_before_migration_boundary() {
    let dir = durable_test_dir("p479-formation-guard-source-conflict");
    let relation = sid(9_841);
    let equivalence = sid(9_842);
    let entity_type = sid(9_843);
    let source_fields = [sid(9_844), sid(9_845), sid(9_846)];
    let target_fields = [sid(9_854), sid(9_855), sid(9_856)];
    let entity = EntityId::new(9840);
    let mut registry = SemanticRegistry::default();
    let (source_context, _, program) = p465_security_contexts(
        9840,
        9841,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9840),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime =
        create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let mut client_after_state = source.state().clone();
    client_after_state
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(200));
    client_after_state
        .model
        .fields
        .insert((source_fields[1], entity), Value::I64(1));
    let client_after = kernel_revision::Revision::build(
        RevisionId::new(98_499),
        &source_context,
        &registry,
        client_after_state,
    )
    .unwrap();
    let client_delta = DurableModelDelta::between(source.state(), client_after.state());
    let guard = RuntimeGuardObservationFootprint::new(
        source.id(),
        [RuntimeHistoryCoordinate::Field {
            field: source_fields[2],
            owner: entity,
        }],
    )
    .unwrap();

    let mut changed_a_state = source.state().clone();
    changed_a_state
        .model
        .fields
        .insert((source_fields[2], entity), Value::I64(8));
    let changed_a = kernel_revision::Revision::build(
        RevisionId::new(9841),
        &source_context,
        &registry,
        changed_a_state,
    )
    .unwrap();
    let changed_a_delta = DurableModelDelta::between(source.state(), changed_a.state());
    let changed_a_complement = DurableModelDelta::between(changed_a.state(), source.state());
    runtime
        .commit_mixed_revision(
            ClientTransactionId::new(0x9840),
            &MixedRevisionTransitionRequest {
                source_revision: source.id(),
                target_revision: &changed_a,
                mutations: &[],
                model_delta: &changed_a_delta,
                model_complement: &changed_a_complement,
                registry: &registry,
            },
        )
        .unwrap();

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&changed_a, RevisionId::new(9842), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_857)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_858)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9841),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let result = runtime.prepare_schema_aware_publication_with_formation_proof(
        source.id(),
        source_context.revision(),
        &[],
        &client_delta,
        &BTreeSet::new(),
        &BTreeSet::new(),
        Some(kernel_durability::ClientIntentGuardDigest::canonical(
            b"security-generation==7",
        )),
        Some(&guard),
        &[],
    );
    assert!(matches!(
        result,
        Err(DurableRuntimeCommitError::GuardDependencyConflict(_))
    ));
    let head = runtime.snapshot().unwrap();
    assert_eq!(head.revision().id(), migrated.id());
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[2], entity)],
        Value::I64(8)
    );
    drop(head);
    drop(runtime);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete formation relational seal case together."
)]
fn p511_relational_formation_capsule_seals_only_when_a_native_history_preserves_observation() {
    let dir = durable_test_dir("p511-relational-formation-seal");
    let relation = sid(9_851);
    let equivalence = sid(9_852);
    let entity_type = sid(9_853);
    let source_fields = [sid(9_854), sid(9_855), sid(9_856)];
    let target_fields = [sid(9_864), sid(9_865), sid(9_866)];
    let entity = EntityId::new(9_850);
    let mut registry = SemanticRegistry::default();
    let (source_context, _, program) = p465_security_contexts(
        9_850, 9_852, relation, equivalence, entity_type, source_fields, target_fields, &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9_850),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime = create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let observation = |value: &str, observation_id| {
        let query = RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text(value.into()),
            equivalence,
        };
        let mut state = MaterializedRelPlanState::build(
            &query,
            &source.state().model,
            &source_context,
            &registry,
        )
        .unwrap();
        state.bind_revision(source.id()).unwrap();
        RuntimeRelationalCausalObservation {
            observation_id,
            observed_revision: source.id(),
            intent_prefix: kernel_durability::DurableIntentPrefix::empty(),
            capsule: kernel_query::RelCausalCapsule::capture(&state).unwrap(),
        }
    };
    let preserved = observation("security-anchor", 0);
    let changed = observation("other", 1);

    let result_type = RelExpr::Scan(relation)
        .typecheck(&source_context, &registry)
        .unwrap();
    let a_delta = RelationDelta {
        inserted: vec![vec![Value::Text("other".into())]],
        removed: Vec::new(),
        result_type,
    };
    let a_mutations = [RevisionRelationMutation {
        relation,
        delta: &a_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    runtime
        .commit_derived_relation_data(
            ClientTransactionId::new(0x9850),
            &DerivedRelationTransitionRequest {
                source_revision: source.id(),
                target_revision: RevisionId::new(9_851),
                mutations: &a_mutations,
            },
        )
        .unwrap();
    let changed_a = runtime.snapshot().unwrap().revision().clone();
    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&changed_a, RevisionId::new(9_852), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_867)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_868)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forever,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9851),
            &FullRevisionTransitionRequest { target_revision: &migrated, registry: &registry },
            &program,
            &complement,
        )
        .unwrap();
    drop(runtime);
    let runtime = DurableRuntime::open(&dir).unwrap();

    let mut target_state = source.state().clone();
    target_state
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(200));
    let client_delta = DurableModelDelta::between(source.state(), &target_state);
    assert!(runtime
        .prepare_schema_aware_publication_with_formation_proof(
            source.id(), source_context.revision(), &[], &client_delta,
            &BTreeSet::new(), &BTreeSet::new(), None, None, &[preserved],
        )
        .is_ok());
    assert!(matches!(
        runtime.prepare_schema_aware_publication_with_formation_proof(
            source.id(), source_context.revision(), &[], &client_delta,
            &BTreeSet::new(), &BTreeSet::new(), None, None, &[changed],
        ),
        Err(DurableRuntimeCommitError::GuardDependencyConflict(_))
    ));

    let retained = runtime.snapshot().unwrap();
    assert_eq!(retained.retained_schema_epoch_count_for_test(), 1);
    assert_eq!(retained.retained_relation_delta_count_for_test(), 1);
    let effect_id = retained.retained_schema_epoch_effect_ids_for_test()[0];
    assert!(runtime
        .release_historical_epoch_authority(effect_id)
        .unwrap()
        .is_some());
    let current = runtime.snapshot().unwrap();
    assert_eq!(current.retained_schema_epoch_count_for_test(), 0);
    assert_eq!(current.retained_relation_delta_count_for_test(), 0);
    assert_eq!(retained.retained_schema_epoch_count_for_test(), 1);
    assert!(runtime.release_causal_history_before_head().unwrap().is_some());
    runtime.compact_obsolete_generations().unwrap();
    drop(retained);
    drop(current);

    drop(runtime);
    let reopened = DurableRuntime::open(&dir).unwrap();
    assert_eq!(
        reopened
            .snapshot()
            .unwrap()
            .retained_schema_epoch_count_for_test(),
        0
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p480_sealed_old_effect_cannot_reorder_before_later_b_causal_observation_after_reopen() {
    let dir = durable_test_dir("p480-b-causal-observation-reorder");
    let relation = sid(9_861);
    let equivalence = sid(9_862);
    let entity_type = sid(9_863);
    let source_fields = [sid(9_864), sid(9_865), sid(9_866)];
    let target_fields = [sid(9_874), sid(9_875), sid(9_876)];
    let entity = EntityId::new(9860);
    let mut registry = SemanticRegistry::default();
    let (source_context, _, program) = p465_security_contexts(
        9860,
        9861,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9860),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime =
        create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let mut client_after_state = source.state().clone();
    client_after_state
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(200));
    let client_after = kernel_revision::Revision::build(
        RevisionId::new(98_699),
        &source_context,
        &registry,
        client_after_state,
    )
    .unwrap();
    let client_delta = DurableModelDelta::between(source.state(), client_after.state());

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source, RevisionId::new(9861), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_877)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_878)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9860),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let current = runtime.snapshot().unwrap().revision().clone();
    let mut b_after_state = current.state().clone();
    b_after_state
        .model
        .fields
        .insert((target_fields[2], entity), Value::I64(8));
    let b_after = kernel_revision::Revision::build(
        RevisionId::new(9862),
        current.semantic_context(),
        &registry,
        b_after_state,
    )
    .unwrap();
    let b_delta = DurableModelDelta::between(current.state(), b_after.state());
    let b_complement = DurableModelDelta::between(b_after.state(), current.state());
    let b_observation = RuntimeGuardObservationFootprint::new(
        current.id(),
        [RuntimeHistoryCoordinate::Field {
            field: target_fields[0],
            owner: entity,
        }],
    )
    .unwrap();
    runtime
        .commit_mixed_revision_residual_guarded_with_dependencies(
            ClientTransactionId::new(0x9861),
            &MixedRevisionTransitionRequest {
                source_revision: current.id(),
                target_revision: &b_after,
                mutations: &[],
                model_delta: &b_delta,
                model_complement: &b_complement,
                registry: &registry,
            },
            &[],
            &b_delta,
            Some(kernel_durability::ClientIntentGuardDigest::canonical(
                b"password==OLD",
            )),
            Some(&b_observation),
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let result = reopened.commit_schema_aware_field_intent(
        ClientTransactionId::new(0x9862),
        &SchemaAwareFieldTransitionRequest {
            formation_revision: source.id(),
            formation_semantic_revision: source_context.revision(),
            client_model_delta: &client_delta,
            guard_observation: None,
            client_guard_digest: None,
        },
    );
    let Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(conflict)) = result else {
        panic!("retroactive sealed effect must not cross a later causal observation");
    };
    assert_eq!(conflict.conflicting_effects, Vec::<u128>::new());
    assert_eq!(conflict.coordination_effects.len(), 1);
    assert_eq!(
        conflict.coordinates,
        vec![RuntimeHistoryCoordinate::Field {
            field: target_fields[0],
            owner: entity
        }],
    );
    let head = reopened.snapshot().unwrap();
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[0], entity)],
        Value::I64(100)
    );
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[2], entity)],
        Value::I64(8)
    );
    drop(head);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p482_predicate_certificate_allows_retroactive_value_change_that_preserves_later_requirement_after_reopen()
 {
    let dir = durable_test_dir("p482-predicate-preservation");
    let relation = sid(9_861);
    let equivalence = sid(9_862);
    let entity_type = sid(9_863);
    let source_fields = [sid(9_864), sid(9_865), sid(9_866)];
    let target_fields = [sid(9_874), sid(9_875), sid(9_876)];
    let entity = EntityId::new(9860);
    let mut registry = SemanticRegistry::default();
    let (source_context, _, program) = p465_security_contexts(
        9860,
        9861,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9860),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime =
        create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let mut client_after_state = source.state().clone();
    client_after_state
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(200));
    let client_after = kernel_revision::Revision::build(
        RevisionId::new(98_699),
        &source_context,
        &registry,
        client_after_state,
    )
    .unwrap();
    let client_delta = DurableModelDelta::between(source.state(), client_after.state());

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source, RevisionId::new(9861), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_877)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_878)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9860),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let current = runtime.snapshot().unwrap().revision().clone();
    let mut b_after_state = current.state().clone();
    b_after_state
        .model
        .fields
        .insert((target_fields[2], entity), Value::I64(8));
    let b_after = kernel_revision::Revision::build(
        RevisionId::new(9862),
        current.semantic_context(),
        &registry,
        b_after_state,
    )
    .unwrap();
    let b_delta = DurableModelDelta::between(current.state(), b_after.state());
    let b_complement = DurableModelDelta::between(b_after.state(), current.state());
    let b_observation = RuntimeGuardObservationFootprint::with_exact_values_and_rules(
        current.id(),
        [(
            RuntimeHistoryCoordinate::Field {
                field: target_fields[0],
                owner: entity,
            },
            Some(Value::I64(100)),
            Some(kernel_schema::SemanticRuleExpr::I64Range {
                value: kernel_schema::RuleValueExpr::Input,
                min: Some(50),
                max: Some(250),
            }),
        )],
    )
    .unwrap();
    runtime
        .commit_mixed_revision_residual_guarded_with_dependencies(
            ClientTransactionId::new(0x9861),
            &MixedRevisionTransitionRequest {
                source_revision: current.id(),
                target_revision: &b_after,
                mutations: &[],
                model_delta: &b_delta,
                model_complement: &b_complement,
                registry: &registry,
            },
            &[],
            &b_delta,
            Some(kernel_durability::ClientIntentGuardDigest::canonical(
                b"password==OLD",
            )),
            Some(&b_observation),
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let result = reopened.commit_schema_aware_field_intent(
        ClientTransactionId::new(0x9862),
        &SchemaAwareFieldTransitionRequest {
            formation_revision: source.id(),
            formation_semantic_revision: source_context.revision(),
            client_model_delta: &client_delta,
            guard_observation: None,
            client_guard_digest: None,
        },
    );
    result.expect("age/password-like value change inside the later unary requirement must commute");
    let head = reopened.snapshot().unwrap();
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[0], entity)],
        Value::I64(200)
    );
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[2], entity)],
        Value::I64(8)
    );
    drop(head);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p482_predicate_certificate_rejects_retroactive_value_change_that_breaks_later_requirement_after_reopen()
 {
    let dir = durable_test_dir("p482-predicate-rejection");
    let relation = sid(9_861);
    let equivalence = sid(9_862);
    let entity_type = sid(9_863);
    let source_fields = [sid(9_864), sid(9_865), sid(9_866)];
    let target_fields = [sid(9_874), sid(9_875), sid(9_876)];
    let entity = EntityId::new(9860);
    let mut registry = SemanticRegistry::default();
    let (source_context, _, program) = p465_security_contexts(
        9860,
        9861,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9860),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [100, 0, 7]),
    )
    .unwrap();
    let runtime =
        create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);

    let mut client_after_state = source.state().clone();
    client_after_state
        .model
        .fields
        .insert((source_fields[0], entity), Value::I64(300));
    let client_after = kernel_revision::Revision::build(
        RevisionId::new(98_699),
        &source_context,
        &registry,
        client_after_state,
    )
    .unwrap();
    let client_delta = DurableModelDelta::between(source.state(), client_after.state());

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source, RevisionId::new(9861), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_877)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_878)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9860),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let current = runtime.snapshot().unwrap().revision().clone();
    let mut b_after_state = current.state().clone();
    b_after_state
        .model
        .fields
        .insert((target_fields[2], entity), Value::I64(8));
    let b_after = kernel_revision::Revision::build(
        RevisionId::new(9862),
        current.semantic_context(),
        &registry,
        b_after_state,
    )
    .unwrap();
    let b_delta = DurableModelDelta::between(current.state(), b_after.state());
    let b_complement = DurableModelDelta::between(b_after.state(), current.state());
    let b_observation = RuntimeGuardObservationFootprint::with_exact_values_and_rules(
        current.id(),
        [(
            RuntimeHistoryCoordinate::Field {
                field: target_fields[0],
                owner: entity,
            },
            Some(Value::I64(100)),
            Some(kernel_schema::SemanticRuleExpr::I64Range {
                value: kernel_schema::RuleValueExpr::Input,
                min: Some(50),
                max: Some(250),
            }),
        )],
    )
    .unwrap();
    runtime
        .commit_mixed_revision_residual_guarded_with_dependencies(
            ClientTransactionId::new(0x9861),
            &MixedRevisionTransitionRequest {
                source_revision: current.id(),
                target_revision: &b_after,
                mutations: &[],
                model_delta: &b_delta,
                model_complement: &b_complement,
                registry: &registry,
            },
            &[],
            &b_delta,
            Some(kernel_durability::ClientIntentGuardDigest::canonical(
                b"password==OLD",
            )),
            Some(&b_observation),
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let result = reopened.commit_schema_aware_field_intent(
        ClientTransactionId::new(0x9862),
        &SchemaAwareFieldTransitionRequest {
            formation_revision: source.id(),
            formation_semantic_revision: source_context.revision(),
            client_model_delta: &client_delta,
            guard_observation: None,
            client_guard_digest: None,
        },
    );
    let Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(conflict)) = result else {
        panic!("retroactive value outside the later unary requirement must require coordination");
    };
    assert_eq!(conflict.conflicting_effects, Vec::<u128>::new());
    assert_eq!(conflict.coordination_effects.len(), 1);
    let head = reopened.snapshot().unwrap();
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[0], entity)],
        Value::I64(100)
    );
    assert_eq!(
        head.revision().state().model.fields[&(target_fields[2], entity)],
        Value::I64(8)
    );
    drop(head);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "diagnostic retained-epoch index benchmark"]
fn benchmark_p473_retained_epoch_first_conflict_is_history_depth_flat() {
    let iterations = 50_000_u64;
    let shallow = benchmark_retained_epoch_first_conflict_for_test(1_000, iterations);
    let deep = benchmark_retained_epoch_first_conflict_for_test(100_000, iterations);
    eprintln!(
        "P473 retained epoch first-conflict lookup: depth=1k {shallow:?}, depth=100k {deep:?}, iterations={iterations}",
    );
}

#[allow(
    clippy::similar_names,
    reason = "Names distinguish related before and after states."
)]
fn p474_row_local_relation_contexts(
    source_schema_revision: u64,
    target_schema_revision: u64,
    relation: SemanticId,
    eq_i64: SemanticId,
    eq_f64: SemanticId,
    registry: &mut SemanticRegistry,
) -> (SemanticContext, kernel_transport::SchemaMigrationProgram) {
    let i64_digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
    let f64_digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::F64Bitwise);

    let mut source_schema = Schema::new(SchemaRevisionId::new(source_schema_revision));
    source_schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![eq_i64],
            },
        })
        .unwrap();
    let source_column = source_schema.relation_column_ids(relation).unwrap()[0];
    let mut source_environment =
        SemanticEnvironment::new(SemanticEnvId::new(source_schema_revision));
    source_environment.pin_module(eq_i64, i64_digest);
    source_environment.pin_module(eq_f64, f64_digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(target_schema_revision));
    target_schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::F64)],
            semantics: RelationSemantics::Set {
                column_equivalences: vec![eq_f64],
            },
        })
        .unwrap();
    let target_column = target_schema.relation_column_ids(relation).unwrap()[0];
    let mut target_environment =
        SemanticEnvironment::new(SemanticEnvId::new(target_schema_revision));
    target_environment.pin_module(eq_i64, i64_digest);
    target_environment.pin_module(eq_f64, f64_digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let program = kernel_transport::SchemaMigrationProgram::new(
        target,
        Vec::new(),
        vec![kernel_transport::MigrationRelationRewrite::Rows(
            kernel_transport::MigrationRowRewrite {
                source_relation: relation,
                target_relation: relation,
                columns: vec![kernel_transport::MigrationColumnRewrite {
                    source_columns: vec![source_column],
                    target_column,
                    transform: kernel_query::ExactQuery::new(kernel_query::Expr::I64ToF64(
                        Box::new(kernel_query::Expr::ProductField {
                            input: Box::new(kernel_query::Expr::Input),
                            field: source_column,
                        }),
                    )),
                }],
            },
        )],
    );
    (source, program)
}

#[test]
#[allow(
    clippy::similar_names,
    reason = "Names distinguish related before and after states."
)]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p474_schema_aware_relation_walker_transports_row_local_delta_after_reopen() {
    let dir = durable_test_dir("p474-schema-aware-row-local-relation");
    let relation = sid(9_840);
    let eq_i64 = sid(9_841);
    let eq_f64 = sid(9_842);
    let mut registry = SemanticRegistry::default();
    let (source_context, program) =
        p474_row_local_relation_contexts(9_840, 9_841, relation, eq_i64, eq_f64, &mut registry);
    let mut state = kernel_model::DatabaseState::default();
    state
        .model
        .relations
        .insert(relation, vec![vec![Value::I64(7)]]);
    let source =
        kernel_revision::Revision::build(RevisionId::new(9_840), &source_context, &registry, state)
            .unwrap();
    let mut physical = PhysicalStore::default();
    physical
        .install(
            relation,
            LayoutBinding::RECOVERY_ROW_STORE,
            NativeRelation::row_store(vec![vec![Value::I64(7)]]),
        )
        .unwrap();
    let root = RuntimeRevisionBundle::build(
        source.clone(),
        physical,
        BTreeMap::from([(relation, LayoutBinding::RECOVERY_ROW_STORE)]),
        &[],
        &registry,
    )
    .unwrap();
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();

    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source, RevisionId::new(9_841), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_843)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_844)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9840),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let source_type = RelExpr::Scan(relation)
        .typecheck(&source_context, &registry)
        .unwrap();
    let client_delta = RelationDelta {
        inserted: vec![vec![Value::I64(8)]],
        removed: vec![vec![Value::I64(7)]],
        result_type: source_type,
    };
    let client_mutations = [RevisionRelationMutation {
        relation,
        delta: &client_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    assert_eq!(
        reopened
            .snapshot()
            .unwrap()
            .revision()
            .state()
            .model
            .relations
            .materialize_owned(&relation),
        Some(vec![vec![Value::F64Bits(7.0_f64.to_bits())]]),
    );
    let transported_probe = program
        .verify(&source_context, &registry)
        .unwrap()
        .transport_relation_delta_exact(relation, &client_delta, &registry)
        .unwrap()
        .pop()
        .unwrap()
        .1;
    reopened
        .validate_relation_delta_at(RevisionId::new(9_841), relation, &transported_probe)
        .unwrap();
    let action_mutations = [RevisionRelationMutation {
        relation,
        delta: &client_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization {
            relationship_attach: true,
            ..Default::default()
        },
    }];
    let action_transport = reopened.prepare_schema_aware_publication(
        source.id(),
        source_context.revision(),
        &action_mutations,
        &DurableModelDelta::default(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        None,
    );
    assert!(matches!(
        action_transport,
        Err(DurableRuntimeCommitError::MigrationTransport(
            kernel_transport::TransportError::UnrepresentableSourceRelationEffect(candidate)
        )) if candidate == relation
    ));

    let prepared = reopened
        .prepare_schema_aware_publication(
            source.id(),
            source_context.revision(),
            &client_mutations,
            &DurableModelDelta::default(),
            &BTreeSet::new(),
            &BTreeSet::new(),
            None,
        )
        .unwrap();
    let tx = ClientTransactionId::new(0x9841);
    let outcome = reopened
        .commit_prepared_schema_aware_publication(tx, &prepared)
        .unwrap();
    assert!(matches!(outcome, DurableRuntimeCommitOutcome::Committed(_)));
    assert_eq!(
        reopened
            .snapshot()
            .unwrap()
            .revision()
            .state()
            .model
            .relations
            .materialize_owned(&relation),
        Some(vec![vec![Value::F64Bits(8.0_f64.to_bits())]]),
    );
    assert!(matches!(
        reopened
            .commit_prepared_schema_aware_publication(tx, &prepared)
            .unwrap(),
        DurableRuntimeCommitOutcome::AlreadyCommitted { .. }
    ));

    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::similar_names,
    reason = "Names distinguish related before and after states."
)]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p498_retained_epoch_prefix_qualified_join_observation_is_restart_equivalent() {
    let dir = durable_test_dir("p488-retained-epoch-join-observation-restart");
    let relation = sid(9_860);
    let eq_i64 = sid(9_861);
    let eq_f64 = sid(9_862);
    let mut registry = SemanticRegistry::default();
    let (source_context, program) =
        p474_row_local_relation_contexts(9_860, 9_862, relation, eq_i64, eq_f64, &mut registry);
    let formation_revision = RevisionId::new(9_860);
    let later_revision = RevisionId::new(9_861);
    let migrated_revision = RevisionId::new(9_862);

    let mut state = kernel_model::DatabaseState::default();
    state
        .model
        .relations
        .insert(relation, vec![vec![Value::I64(7)]]);
    let source =
        kernel_revision::Revision::build(formation_revision, &source_context, &registry, state)
            .unwrap();
    let mut physical = PhysicalStore::default();
    physical
        .install(
            relation,
            LayoutBinding::RECOVERY_ROW_STORE,
            NativeRelation::row_store(vec![vec![Value::I64(7)]]),
        )
        .unwrap();
    let root = RuntimeRevisionBundle::build(
        source,
        physical,
        BTreeMap::from([(relation, LayoutBinding::RECOVERY_ROW_STORE)]),
        &[],
        &registry,
    )
    .unwrap();
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();

    let source_type = RelExpr::Scan(relation)
        .typecheck(&source_context, &registry)
        .unwrap();
    let join = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence: eq_i64,
    };
    let snapshot = runtime.snapshot().unwrap();
    let mut candidate_model = snapshot.revision().state().model.clone();
    candidate_model
        .relations
        .insert(relation, vec![vec![Value::I64(7)], vec![Value::I64(2)]]);
    let mut maintained =
        MaterializedRelPlanState::build(&join, &candidate_model, &source_context, &registry)
            .unwrap();
    maintained.bind_revision(formation_revision).unwrap();
    let capsule = kernel_query::RelCausalCapsule::capture(&maintained).unwrap();
    let observation = RuntimeRelationalCausalObservation {
        observation_id: 0,
        observed_revision: formation_revision,
        intent_prefix: kernel_durability::DurableIntentPrefix::empty().append(vec![
            kernel_durability::DurableRelationMutation {
                relation,
                inserted: vec![vec![Value::I64(2)]],
                removed: Vec::new(),
                object_field_writes: Vec::new(),
                authorization: kernel_durability::DurableRelationAuthorization::default(),
            },
        ]),
        capsule,
    };
    drop(snapshot);

    let later_delta = RelationDelta {
        inserted: vec![vec![Value::I64(2)]],
        removed: Vec::new(),
        result_type: source_type.clone(),
    };
    let later_mutations = [RevisionRelationMutation {
        relation,
        delta: &later_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    runtime
        .commit_derived_relation_data_guarded_with_dependencies_and_relational_observations(
            ClientTransactionId::new(0x9860),
            &DerivedRelationTransitionRequest {
                source_revision: formation_revision,
                target_revision: later_revision,
                mutations: &later_mutations,
            },
            None,
            None,
            &[observation],
        )
        .unwrap();

    let pre_migration = runtime.snapshot().unwrap().revision().clone();
    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&pre_migration, migrated_revision, &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_863)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_864)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9861),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let old_delta = RelationDelta {
        inserted: vec![vec![Value::I64(8)]],
        removed: vec![vec![Value::I64(7)]],
        result_type: source_type,
    };
    let old_mutations = [RevisionRelationMutation {
        relation,
        delta: &old_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    let before = runtime.prepare_schema_aware_publication(
        formation_revision,
        source_context.revision(),
        &old_mutations,
        &DurableModelDelta::default(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        None,
    );
    let Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(before_conflict)) = before
    else {
        panic!("retained A-native join observation must reject the retroactive A effect");
    };
    assert!(before_conflict.conflicting_effects.is_empty());
    assert_eq!(before_conflict.coordination_effects.len(), 1);
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let after = reopened.prepare_schema_aware_publication(
        formation_revision,
        source_context.revision(),
        &old_mutations,
        &DurableModelDelta::default(),
        &BTreeSet::new(),
        &BTreeSet::new(),
        None,
    );
    let Err(DurableRuntimeCommitError::SchemaAwareTransitionConflict(after_conflict)) = after
    else {
        panic!("reopen must reconstruct the retained relational capsule authority");
    };
    assert_eq!(before_conflict, after_conflict);
    assert_eq!(
        reopened
            .snapshot()
            .unwrap()
            .revision()
            .state()
            .model
            .relations
            .materialize_owned(&relation),
        Some(vec![
            vec![Value::F64Bits(7.0_f64.to_bits())],
            vec![Value::F64Bits(2.0_f64.to_bits())],
        ]),
    );
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p483_joint_multi_field_causal_predicate_is_evaluated_atomically_after_reopen() {
    let dir = durable_test_dir("p483-joint-causal-predicate");
    let relation = sid(9_881);
    let equivalence = sid(9_882);
    let entity_type = sid(9_883);
    let source_fields = [sid(9_884), sid(9_885), sid(9_886)];
    let target_fields = [sid(9_894), sid(9_895), sid(9_896)];
    let entity = EntityId::new(9880);
    let mut registry = SemanticRegistry::default();
    let (source_context, _, program) = p465_security_contexts(
        9880,
        9881,
        relation,
        equivalence,
        entity_type,
        source_fields,
        target_fields,
        &mut registry,
    );
    let source = kernel_revision::Revision::build(
        RevisionId::new(9880),
        &source_context,
        &registry,
        p465_security_state(entity_type, source_fields, relation, entity, [0, 0, 7]),
    )
    .unwrap();
    let runtime =
        create_migration_runtime(&dir, relation, source.clone(), "security-anchor", &registry);
    let transport = program.verify(&source_context, &registry).unwrap();
    let migrated = transport
        .transport_revision(&source, RevisionId::new(9881), &registry)
        .unwrap();
    let complement = DurableMigrationComplement::from_capsule(
        kernel_lens::ComplementCapsule {
            source_schema: source_context.schema.revision,
            target_schema: program.target().schema.revision,
            lens_spec: kernel_lens::LensSpecId(sid(9_897)),
            semantic_pins: kernel_lens::SemanticManifestId(sid(9_898)),
            encoding_version: 1,
            complement: Value::Unit,
        },
        kernel_lens::ComplementRetention::Forget,
    );
    runtime
        .migrate_schema(
            ClientTransactionId::new(0x9880),
            &FullRevisionTransitionRequest {
                target_revision: &migrated,
                registry: &registry,
            },
            &program,
            &complement,
        )
        .unwrap();

    let current = runtime.snapshot().unwrap().revision().clone();
    let mut later_state = current.state().clone();
    later_state
        .model
        .fields
        .insert((target_fields[2], entity), Value::I64(8));
    let later = kernel_revision::Revision::build(
        RevisionId::new(9882),
        current.semantic_context(),
        &registry,
        later_state,
    )
    .unwrap();
    let later_delta = DurableModelDelta::between(current.state(), later.state());
    let later_complement = DurableModelDelta::between(later.state(), current.state());
    let predicate = kernel_schema::SemanticRuleExpr::Or(vec![
        kernel_schema::SemanticRuleExpr::I64Range {
            value: kernel_schema::RuleValueExpr::Field(target_fields[0]),
            min: Some(0),
            max: Some(0),
        },
        kernel_schema::SemanticRuleExpr::I64Range {
            value: kernel_schema::RuleValueExpr::Field(target_fields[1]),
            min: Some(0),
            max: Some(0),
        },
    ]);
    let observation = RuntimeGuardObservationFootprint::with_exact_values_rules_and_groups(
        current.id(),
        [],
        [RuntimeJointCausalObservationGroup {
            group_id: 0,
            relation,
            owner: entity,
            observed_fields: BTreeMap::from([
                (target_fields[0], Value::I64(0)),
                (target_fields[1], Value::I64(0)),
            ]),
            predicate,
        }],
    )
    .unwrap();
    runtime
        .commit_mixed_revision_residual_guarded_with_dependencies(
            ClientTransactionId::new(0x9881),
            &MixedRevisionTransitionRequest {
                source_revision: current.id(),
                target_revision: &later,
                mutations: &[],
                model_delta: &later_delta,
                model_complement: &later_complement,
                registry: &registry,
            },
            &[],
            &later_delta,
            Some(kernel_durability::ClientIntentGuardDigest::canonical(
                b"x==0||y==0",
            )),
            Some(&observation),
        )
        .unwrap();
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let snapshot = reopened.snapshot().unwrap();
    let one_change = snapshot
        .certify_retroactive_exact_writes_against_causal_observations(
            current.id(),
            [
                (
                    RuntimeHistoryCoordinate::Field {
                        field: target_fields[0],
                        owner: entity,
                    },
                    Some(Value::I64(1)),
                ),
                (
                    RuntimeHistoryCoordinate::Field {
                        field: target_fields[1],
                        owner: entity,
                    },
                    Some(Value::I64(0)),
                ),
            ],
        )
        .unwrap();
    assert!(matches!(
        one_change,
        RuntimeTransitionRebaseOutcome::Certified(_)
    ));

    let both_change = snapshot
        .certify_retroactive_exact_writes_against_causal_observations(
            current.id(),
            [
                (
                    RuntimeHistoryCoordinate::Field {
                        field: target_fields[0],
                        owner: entity,
                    },
                    Some(Value::I64(1)),
                ),
                (
                    RuntimeHistoryCoordinate::Field {
                        field: target_fields[1],
                        owner: entity,
                    },
                    Some(Value::I64(1)),
                ),
            ],
        )
        .unwrap();
    let RuntimeTransitionRebaseOutcome::Conflict(conflict) = both_change else {
        panic!(
            "joint predicate must reject simultaneous writes that falsify the later observation"
        );
    };
    assert_eq!(conflict.coordination_effects.len(), 1);
    drop(snapshot);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operator or protocol case analysis together."
)]
fn p487_relational_causal_braid_verdict_is_restart_equivalent() {
    let dir = durable_test_dir("p487-relational-causal-restart-equivalence");
    let (context, registry, relation, _, root) = scan_runtime_bundle(9_900, 9_900, &[0]);
    let runtime = DurableRuntime::create(root, &dir, &registry).unwrap();
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::I64(1),
        equivalence: sid(101),
    };
    let snapshot = runtime.snapshot().unwrap();
    let mut maintained = MaterializedRelPlanState::build(
        &query,
        &snapshot.revision().state().model,
        &context,
        &registry,
    )
    .unwrap();
    maintained.bind_revision(snapshot.revision().id()).unwrap();
    let capsule = kernel_query::RelCausalCapsule::capture(&maintained).unwrap();
    let observation = RuntimeRelationalCausalObservation {
        observation_id: 0,
        observed_revision: capsule.revision(),
        intent_prefix: kernel_durability::DurableIntentPrefix::empty(),
        capsule,
    };
    drop(snapshot);

    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let later_delta = RelationDelta {
        inserted: vec![vec![Value::I64(2)]],
        removed: Vec::new(),
        result_type: result_type.clone(),
    };
    let later_mutations = [RevisionRelationMutation {
        relation,
        delta: &later_delta,
        object_field_writes: &[],
        authorization: kernel_durability::DurableRelationAuthorization::default(),
    }];
    runtime
        .commit_derived_relation_data_guarded_with_dependencies_and_relational_observations(
            ClientTransactionId::new(0x9901),
            &DerivedRelationTransitionRequest {
                source_revision: RevisionId::new(9_900),
                target_revision: RevisionId::new(9_901),
                mutations: &later_mutations,
            },
            None,
            None,
            &[observation],
        )
        .unwrap();

    let changed_delta = RelationDelta {
        inserted: vec![vec![Value::I64(1)]],
        removed: Vec::new(),
        result_type: result_type.clone(),
    };
    let preserved_delta = RelationDelta {
        inserted: vec![vec![Value::I64(3)]],
        removed: Vec::new(),
        result_type,
    };
    let certify = |runtime: &DurableRuntime, delta: &RelationDelta| {
        runtime
            .certify_transition_rebase(
                RevisionId::new(9_900),
                &[RevisionRelationMutation {
                    relation,
                    delta,
                    object_field_writes: &[],
                    authorization: kernel_durability::DurableRelationAuthorization::default(),
                }],
                None,
                None,
            )
            .unwrap()
    };

    let before_changed = certify(&runtime, &changed_delta);
    let before_preserved = certify(&runtime, &preserved_delta);
    assert!(matches!(
        before_changed,
        RuntimeTransitionRebaseOutcome::Conflict(_)
    ));
    assert!(matches!(
        before_preserved,
        RuntimeTransitionRebaseOutcome::Certified(_)
    ));
    drop(runtime);

    let reopened = DurableRuntime::open(&dir).unwrap();
    let after_changed = certify(&reopened, &changed_delta);
    let after_preserved = certify(&reopened, &preserved_delta);
    assert_eq!(before_changed, after_changed);
    assert_eq!(before_preserved, after_preserved);
    let RuntimeTransitionRebaseOutcome::Conflict(conflict) = after_changed else {
        unreachable!();
    };
    assert_eq!(conflict.conflicting_effects, Vec::<u128>::new());
    assert_eq!(conflict.coordination_effects.len(), 1);
    drop(reopened);
    std::fs::remove_dir_all(dir).unwrap();
}
