use super::*;
use kernel_model::FiniteModel;
use kernel_query::{ExactQuery, Expr, RelationDelta};
use kernel_transport::MigrationFieldRewrite;

fn sample_state() -> (DatabaseState, SemanticId, EntityId, SemanticId) {
    let entity_type = SemanticId::new(10);
    let age = SemanticId::new(11);
    let relation = SemanticId::new(12);
    let entity = EntityId::new(100);

    let mut lifecycle = LifecycleGraph::default();
    lifecycle.entities.insert(entity);
    lifecycle.roots.insert(entity);

    let mut model = FiniteModel::default();
    model.carriers.insert(entity_type, BTreeSet::from([entity]));
    model.fields.insert((age, entity), Value::I64(42));
    model.relations.insert(relation, vec![vec![Value::I64(42)]]);

    (
        DatabaseState {
            model,
            lifecycle: lifecycle.into(),
        },
        age,
        entity,
        relation,
    )
}

#[test]
fn compaction_decision_is_workload_weighted_and_never_uses_overlay_depth() {
    let workloads = [
        RelationCompactionWorkloadCost {
            expected_uses: 1_000,
            overlay_cost: 120,
            native_cost: 100,
        },
        RelationCompactionWorkloadCost {
            expected_uses: 10,
            overlay_cost: 80,
            native_cost: 100,
        },
    ];
    let decision = relation_compaction_decision(&workloads, 10_000, 500).unwrap();
    assert_eq!(decision.expected_savings, 20_000);
    assert_eq!(decision.expected_penalty, 200);
    assert!(decision.should_compact);

    let expensive = relation_compaction_decision(&workloads, 19_500, 500).unwrap();
    assert!(!expensive.should_compact);
}

#[test]
fn direct_realization_exactly_reconstructs_database_state() {
    let (state, _, _, _) = sample_state();
    let (atoms, root) = realize_database_state(&state);
    let certificate = certify_realization(&atoms, &root, &state).unwrap();
    assert_eq!(root.evaluate(&atoms).unwrap(), state);
    assert_eq!(certificate.dependencies(), &root.dependencies());
}

#[test]
fn scalar_transform_and_constant_are_semantic_realizations_not_atom_reinterpretation() {
    let (state, age, entity, _) = sample_state();
    let (atoms, mut root) = realize_database_state(&state);
    let original = root.fields().get(&(age, entity)).unwrap().clone();
    root.set_field_expr(
        age,
        entity,
        ValueRealizationExpr::Transform {
            source: Box::new(original),
            query: ExactQuery::new(Expr::I64ToF64(Box::new(Expr::Input))),
        },
    );
    let active = SemanticId::new(13);
    root.set_field_expr(
        active,
        entity,
        ValueRealizationExpr::Constant(Value::Bool(true)),
    );

    let realized = root.evaluate(&atoms).unwrap();
    assert_eq!(
        realized.model.fields.get(&(age, entity)),
        Some(&Value::F64Bits(42.0_f64.to_bits()))
    );
    assert_eq!(
        realized.model.fields.get(&(active, entity)),
        Some(&Value::Bool(true))
    );
    assert_eq!(root.dependencies().len(), atoms.len());
    let age_dependencies = root
        .dependency_graph()
        .dependencies(RealizationCoordinate::Field { field: age, entity })
        .cloned()
        .unwrap();
    assert_eq!(age_dependencies.len(), 1);
}

#[test]
fn materialization_changes_dependencies_but_not_semantics() {
    let (state, age, entity, _) = sample_state();
    let (mut atoms, mut before) = realize_database_state(&state);
    let source = before.fields().get(&(age, entity)).unwrap().clone();
    before.set_field_expr(
        age,
        entity,
        ValueRealizationExpr::Transform {
            source: Box::new(source),
            query: ExactQuery::new(Expr::I64ToF64(Box::new(Expr::Input))),
        },
    );

    let before_atoms = atoms.clone();
    let mut after = before.clone();
    let old_dependencies = before.dependencies();
    let native = after.materialize_field(&mut atoms, age, entity).unwrap();
    let certificate =
        certify_equivalent_realizations(&before_atoms, &before, &atoms, &after).unwrap();

    assert_eq!(&old_dependencies, certificate.before_dependencies());
    assert!(certificate.after_dependencies().contains(&native));
    assert_ne!(
        certificate.before_dependencies(),
        certificate.after_dependencies()
    );
    assert_eq!(
        before.evaluate(&before_atoms).unwrap(),
        after.evaluate(&atoms).unwrap()
    );
}

#[test]
fn gc_reachability_is_union_of_current_and_historical_roots() {
    let (state, age, entity, _) = sample_state();
    let (mut atoms, historical) = realize_database_state(&state);
    let mut current = historical.clone();
    let old_age_atom = match current.fields().get(&(age, entity)).unwrap() {
        ValueRealizationExpr::Direct(atom) => *atom,
        _ => unreachable!(),
    };
    current.materialize_field(&mut atoms, age, entity).unwrap();

    let current_only = reachable_atoms([&current]);
    assert!(!current_only.contains(&old_age_atom));
    let retained = reachable_atoms([&current, &historical]);
    assert!(retained.contains(&old_age_atom));

    atoms.retain(&retained);
    assert_eq!(historical.evaluate(&atoms).unwrap(), state);
}

fn migration_field_fixture() -> (
    SemanticContext,
    SemanticContext,
    SemanticRegistry,
    SchemaMigrationProgram,
    DatabaseState,
    SemanticId,
    SemanticId,
    SemanticId,
    EntityId,
) {
    use kernel_schema::{FieldDef, ScalarType, Schema, SemanticEnvironment, TypeExpr};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    let entity_type = SemanticId::new(30_000);
    let old_left = SemanticId::new(30_001);
    let old_right = SemanticId::new(30_002);
    let new_sum = SemanticId::new(30_003);
    let new_copy = SemanticId::new(30_004);
    let new_default = SemanticId::new(30_005);
    let entity = EntityId::new(77);
    let registry = SemanticRegistry::default();

    let mut source_schema = Schema::new(SchemaRevisionId::new(300));
    for field in [old_left, old_right] {
        source_schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
    }
    let source = SemanticContext {
        schema: source_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(300)),
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(301));
    for field in [new_sum, new_copy, new_default] {
        target_schema
            .define_field(FieldDef {
                id: field,
                owner: entity_type,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
    }
    let target = SemanticContext {
        schema: target_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(301)),
    };

    let product_field = |field| kernel_query::Expr::ProductField {
        input: Box::new(kernel_query::Expr::Input),
        field,
    };
    let program = SchemaMigrationProgram::new(
        target.clone(),
        vec![
            MigrationFieldRewrite {
                source_fields: vec![old_left, old_right],
                target_field: new_sum,
                transform: ExactQuery::new(kernel_query::Expr::AddI64(
                    Box::new(product_field(old_left)),
                    Box::new(product_field(old_right)),
                )),
            },
            MigrationFieldRewrite {
                source_fields: vec![old_left],
                target_field: new_copy,
                transform: ExactQuery::new(product_field(old_left)),
            },
            MigrationFieldRewrite {
                source_fields: vec![],
                target_field: new_default,
                transform: ExactQuery::new(kernel_query::Expr::TypedConst {
                    value: Value::I64(99),
                    ty: TypeExpr::Scalar(ScalarType::I64),
                }),
            },
        ],
        vec![],
    );

    let mut state = DatabaseState::default();
    state.lifecycle.entities.insert(entity);
    state.lifecycle.roots.insert(entity);
    state
        .model
        .carriers
        .insert(entity_type, BTreeSet::from([entity]));
    state.model.fields.insert((old_left, entity), Value::I64(4));
    state
        .model
        .fields
        .insert((old_right, entity), Value::I64(6));

    (
        source,
        target,
        registry,
        program,
        state,
        new_sum,
        new_copy,
        new_default,
        entity,
    )
}

#[test]
fn migration_program_composes_into_current_realization_without_old_semantic_world() {
    let (source, target, registry, program, state, new_sum, new_copy, new_default, entity) =
        migration_field_fixture();
    let (atoms, source_root) = realize_database_state(&state);
    let target_root =
        compose_schema_migration(&atoms, &source_root, &source, &registry, &program).unwrap();

    let transport = program.verify(&source, &registry).unwrap();
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source,
        &registry,
        state,
    )
    .unwrap();
    let expected = transport
        .transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        )
        .unwrap();
    let realized = target_root.evaluate(&atoms).unwrap();

    assert_eq!(&realized, expected.state());
    assert_eq!(
        realized.model.fields.get(&(new_sum, entity)),
        Some(&Value::I64(10))
    );
    assert_eq!(
        realized.model.fields.get(&(new_copy, entity)),
        Some(&Value::I64(4))
    );
    assert_eq!(
        realized.model.fields.get(&(new_default, entity)),
        Some(&Value::I64(99))
    );
    assert_eq!(expected.semantic_context(), &target);
    assert!(
        target_root
            .dependency_graph()
            .dependencies(RealizationCoordinate::Field {
                field: new_default,
                entity,
            })
            .unwrap()
            .is_empty()
    );
}

#[test]
fn row_local_migration_composes_as_relation_realization_over_old_atom() {
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticEnvironment, TypeExpr,
    };
    use kernel_semantics::EquivalenceModule;
    use kernel_transport::{MigrationColumnRewrite, MigrationRowRewrite};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    let relation = SemanticId::new(31_000);
    let eq_i64 = SemanticId::new(31_001);
    let eq_f64 = SemanticId::new(31_002);
    let source_column = SemanticId::new(31_003);
    let target_column = SemanticId::new(31_004);
    let mut registry = SemanticRegistry::default();
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);

    let mut source_schema = Schema::new(SchemaRevisionId::new(310));
    source_schema
        .define_relation_with_column_ids(
            RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq_i64],
                },
            },
            vec![source_column],
        )
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(310));
    source_environment.pin_module(eq_i64, i64_digest);
    source_environment.pin_module(eq_f64, f64_digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(311));
    target_schema
        .define_relation_with_column_ids(
            RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::F64)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq_f64],
                },
            },
            vec![target_column],
        )
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(310));
    target_environment.pin_module(eq_i64, i64_digest);
    target_environment.pin_module(eq_f64, f64_digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let program = SchemaMigrationProgram::new(
        target,
        vec![],
        vec![MigrationRelationRewrite::Rows(MigrationRowRewrite {
            source_relation: relation,
            target_relation: relation,
            columns: vec![MigrationColumnRewrite {
                source_columns: vec![source_column],
                target_column,
                transform: ExactQuery::new(kernel_query::Expr::I64ToF64(Box::new(
                    kernel_query::Expr::ProductField {
                        input: Box::new(kernel_query::Expr::Input),
                        field: source_column,
                    },
                ))),
            }],
        })],
    );
    let mut state = DatabaseState::default();
    state
        .model
        .relations
        .insert(relation, vec![vec![Value::I64(3)], vec![Value::I64(8)]]);
    let (atoms, source_root) = realize_database_state(&state);
    let old_atom = *source_root
        .dependency_graph()
        .dependencies(RealizationCoordinate::Relation(relation))
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let target_root =
        compose_schema_migration(&atoms, &source_root, &source, &registry, &program).unwrap();
    let realized = target_root.evaluate(&atoms).unwrap();

    assert_eq!(
        realized.model.relations.materialize_owned(&relation),
        Some(vec![
            vec![Value::F64Bits(3.0_f64.to_bits())],
            vec![Value::F64Bits(8.0_f64.to_bits())],
        ])
    );
    assert!(
        target_root
            .dependency_graph()
            .dependencies(RealizationCoordinate::Relation(relation))
            .unwrap()
            .contains(&old_atom)
    );
}

#[test]
#[ignore = "manual release-mode realization microbenchmark"]
fn realization_transform_hot_path_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    let (state, age, entity, _) = sample_state();
    let (mut atoms, direct_root) = realize_database_state(&state);
    let direct = direct_root.fields().get(&(age, entity)).unwrap().clone();
    let transformed = ValueRealizationExpr::Transform {
        source: Box::new(ValueRealizationExpr::Product(BTreeMap::from([(
            age,
            direct.clone(),
        )]))),
        query: ExactQuery::new(Expr::I64ToF64(Box::new(Expr::ProductField {
            input: Box::new(Expr::Input),
            field: age,
        }))),
    };
    let iterations = 1_000_000_u32;

    let start = Instant::now();
    for _ in 0..iterations {
        black_box(direct.evaluate(&atoms).unwrap());
    }
    let direct_elapsed = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        black_box(transformed.evaluate(&atoms).unwrap());
    }
    let transformed_elapsed = start.elapsed();

    let materialized_value = transformed.evaluate(&atoms).unwrap();
    let native = atoms.insert(PhysicalAtomPayload::Value(materialized_value));
    let materialized = ValueRealizationExpr::Direct(native);
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(materialized.evaluate(&atoms).unwrap());
    }
    let materialized_elapsed = start.elapsed();

    let ns = |elapsed: std::time::Duration| elapsed.as_nanos() as f64 / f64::from(iterations);
    let direct_ns = ns(direct_elapsed);
    let transformed_ns = ns(transformed_elapsed);
    let materialized_ns = ns(materialized_elapsed);
    eprintln!(
        "REALIZATION_PERF direct={direct_ns:.2}ns transformed={transformed_ns:.2}ns materialized={materialized_ns:.2}ns transform_ratio={:.2}x materialized_ratio={:.2}x",
        transformed_ns / direct_ns,
        materialized_ns / direct_ns,
    );
}

#[test]
#[ignore = "manual release-mode migration composition scale benchmark"]
fn migration_composition_scale_benchmark() {
    use kernel_schema::{FieldDef, ScalarType, Schema, SemanticEnvironment, TypeExpr};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};
    use std::time::Instant;

    let entity_type = SemanticId::new(32_000);
    let old_age = SemanticId::new(32_001);
    let new_age = SemanticId::new(32_002);
    let registry = SemanticRegistry::default();
    let mut source_schema = Schema::new(SchemaRevisionId::new(320));
    source_schema
        .define_field(FieldDef {
            id: old_age,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    let source = SemanticContext {
        schema: source_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(320)),
    };
    let mut target_schema = Schema::new(SchemaRevisionId::new(321));
    target_schema
        .define_field(FieldDef {
            id: new_age,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::F64),
        })
        .unwrap();
    let target = SemanticContext {
        schema: target_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(321)),
    };
    let program = SchemaMigrationProgram::new(
        target,
        vec![MigrationFieldRewrite {
            source_fields: vec![old_age],
            target_field: new_age,
            transform: ExactQuery::new(Expr::I64ToF64(Box::new(Expr::ProductField {
                input: Box::new(Expr::Input),
                field: old_age,
            }))),
        }],
        vec![],
    );

    let count = 100_000_u64;
    let mut state = DatabaseState::default();
    for raw in 0..count {
        let entity = EntityId::new(u128::from(raw + 1));
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
            .insert((old_age, entity), Value::I64(raw as i64));
    }
    let (atoms, root) = realize_database_state(&state);
    let start = Instant::now();
    let target_root =
        compose_schema_migration(&atoms, &root, &source, &registry, &program).unwrap();
    let elapsed = start.elapsed();
    eprintln!(
        "REALIZATION_COMPOSE_SCALE entities={count} elapsed_ms={:.3} target_deps={} source_atoms={}",
        elapsed.as_secs_f64() * 1_000.0,
        target_root.dependencies().len(),
        atoms.len(),
    );
}

#[test]
fn factorized_field_columns_compose_migration_without_per_entity_metadata() {
    let (source, _target, registry, program, state, new_sum, new_copy, new_default, entity) =
        migration_field_fixture();
    let (atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();

    let reference_atoms = atoms.clone();
    let (cell_atoms, cell_root) = realize_database_state(&state);
    let expected = compose_schema_migration(&cell_atoms, &cell_root, &source, &registry, &program)
        .unwrap()
        .evaluate(&cell_atoms)
        .unwrap();

    assert_eq!(target_root.evaluate(&atoms).unwrap(), expected);
    assert_eq!(
        target_root.read_field(&atoms, new_sum, entity).unwrap(),
        Value::I64(10)
    );
    assert_eq!(
        target_root.read_field(&atoms, new_copy, entity).unwrap(),
        Value::I64(4)
    );
    assert_eq!(
        target_root.read_field(&atoms, new_default, entity).unwrap(),
        Value::I64(99)
    );
    assert_eq!(reference_atoms.len(), 4);
    assert!(target_root.dependencies().len() <= reference_atoms.len());
    assert!(matches!(
        target_root.fields().get(&new_copy).unwrap().expr(),
        FactorizedFieldExpr::Direct(_)
    ));
    assert!(matches!(
        target_root.fields().get(&new_default).unwrap().expr(),
        FactorizedFieldExpr::Constant(Value::I64(99))
    ));
}

#[test]
fn factorized_materialization_replaces_transform_rule_with_one_native_column_atom() {
    let (source, _target, registry, program, state, new_sum, _, _, entity) =
        migration_field_fixture();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();
    let before = target_root.dependencies();

    let native = target_root
        .materialize_field_column(&mut atoms, new_sum, [entity])
        .unwrap();

    assert_eq!(
        target_root.read_field(&atoms, new_sum, entity).unwrap(),
        Value::I64(10)
    );
    assert!(target_root.dependencies().contains(&native));
    assert_ne!(target_root.dependencies(), before);
    assert!(matches!(
        target_root.fields().get(&new_sum).unwrap().expr(),
        FactorizedFieldExpr::Direct(atom) if *atom == native
    ));
}

#[test]
fn current_schema_field_write_installs_native_overlay_without_inverse_migration() {
    let (source, _target, registry, program, state, new_sum, _, _, entity) =
        migration_field_fixture();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let source_before = source_root.evaluate(&atoms).unwrap();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();

    let native = target_root
        .install_field_value_overlay(&mut atoms, new_sum, entity, Value::I64(777), 1)
        .unwrap();

    assert_eq!(
        target_root.read_field(&atoms, new_sum, entity).unwrap(),
        Value::I64(777)
    );
    assert_eq!(source_root.evaluate(&atoms).unwrap(), source_before);
    assert!(target_root.dependencies().contains(&native));
    assert!(matches!(
        target_root.fields().get(&new_sum).unwrap().expr(),
        FactorizedFieldExpr::ChunkOverlay { native_chunks, .. }
            if native_chunks.len() == 1 && native_chunks[0].atom() == native
    ));
}

#[test]
#[ignore = "manual release-mode factorized realization benchmark"]
fn factorized_realization_scale_and_hot_path_benchmark() {
    use kernel_schema::{FieldDef, ScalarType, Schema, SemanticEnvironment, TypeExpr};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};
    use std::hint::black_box;
    use std::time::Instant;

    let entity_type = SemanticId::new(50_000);
    let old_age = SemanticId::new(50_001);
    let new_age = SemanticId::new(50_002);
    let registry = SemanticRegistry::default();
    let mut source_schema = Schema::new(SchemaRevisionId::new(500));
    source_schema
        .define_field(FieldDef {
            id: old_age,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    let source = SemanticContext {
        schema: source_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(500)),
    };
    let mut target_schema = Schema::new(SchemaRevisionId::new(501));
    target_schema
        .define_field(FieldDef {
            id: new_age,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::F64),
        })
        .unwrap();
    let target = SemanticContext {
        schema: target_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(501)),
    };
    let program = SchemaMigrationProgram::new(
        target,
        vec![MigrationFieldRewrite {
            source_fields: vec![old_age],
            target_field: new_age,
            transform: ExactQuery::new(Expr::I64ToF64(Box::new(Expr::ProductField {
                input: Box::new(Expr::Input),
                field: old_age,
            }))),
        }],
        vec![],
    );

    let count = 100_000_u64;
    let mut state = DatabaseState::default();
    for raw in 0..count {
        let entity = EntityId::new(u128::from(raw + 1));
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
            .insert((old_age, entity), Value::I64(raw as i64));
    }
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let probe = EntityId::new(u128::from(count / 2));

    let compose_start = Instant::now();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();
    let compose_elapsed = compose_start.elapsed();
    let source_atom_count = atoms.len();
    let target_dependency_count = target_root.dependencies().len();

    let iterations = 500_000_u32;
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(source_root.read_field(&atoms, old_age, probe).unwrap());
    }
    let direct_elapsed = start.elapsed();
    let start = Instant::now();
    for _ in 0..iterations {
        black_box(target_root.read_field(&atoms, new_age, probe).unwrap());
    }
    let transformed_elapsed = start.elapsed();

    let entities = state
        .model
        .carriers
        .get(&entity_type)
        .unwrap()
        .iter()
        .copied();
    let materialize_start = Instant::now();
    target_root
        .materialize_field_column(&mut atoms, new_age, entities)
        .unwrap();
    let materialize_elapsed = materialize_start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        black_box(target_root.read_field(&atoms, new_age, probe).unwrap());
    }
    let materialized_elapsed = start.elapsed();

    let ns = |elapsed: std::time::Duration| elapsed.as_nanos() as f64 / f64::from(iterations);
    let direct_ns = ns(direct_elapsed);
    let transformed_ns = ns(transformed_elapsed);
    let materialized_ns = ns(materialized_elapsed);
    eprintln!(
        "FACTORIZED_REALIZATION_PERF rows={count} compose_us={:.2} source_atoms={source_atom_count} target_dependencies={target_dependency_count} direct={direct_ns:.2}ns transformed={transformed_ns:.2}ns transformed_ratio={:.2}x materialize_ms={:.2} materialized={materialized_ns:.2}ns materialized_ratio={:.2}x",
        compose_elapsed.as_secs_f64() * 1_000_000.0,
        transformed_ns / direct_ns,
        materialize_elapsed.as_secs_f64() * 1_000.0,
        materialized_ns / direct_ns,
    );
}

fn factorized_relation_migration_fixture(
    row_count: usize,
) -> (
    SemanticContext,
    SemanticRegistry,
    SchemaMigrationProgram,
    DatabaseState,
    SemanticId,
    SemanticId,
    SemanticId,
) {
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticEnvironment, TypeExpr,
    };
    use kernel_semantics::EquivalenceModule;
    use kernel_transport::{MigrationColumnRewrite, MigrationRowRewrite};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    let relation = SemanticId::new(61_000);
    let eq_i64 = SemanticId::new(61_001);
    let eq_f64 = SemanticId::new(61_002);
    let source_column = SemanticId::new(61_003);
    let target_column = SemanticId::new(61_004);
    let mut registry = SemanticRegistry::default();
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);

    let mut source_schema = Schema::new(SchemaRevisionId::new(610));
    source_schema
        .define_relation_with_column_ids(
            RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq_i64],
                },
            },
            vec![source_column],
        )
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(610));
    source_environment.pin_module(eq_i64, i64_digest);
    source_environment.pin_module(eq_f64, f64_digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(611));
    target_schema
        .define_relation_with_column_ids(
            RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::F64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq_f64],
                },
            },
            vec![target_column],
        )
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(610));
    target_environment.pin_module(eq_i64, i64_digest);
    target_environment.pin_module(eq_f64, f64_digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let program = SchemaMigrationProgram::new(
        target,
        vec![],
        vec![kernel_transport::MigrationRelationRewrite::Rows(
            MigrationRowRewrite {
                source_relation: relation,
                target_relation: relation,
                columns: vec![MigrationColumnRewrite {
                    source_columns: vec![source_column],
                    target_column,
                    transform: ExactQuery::new(Expr::I64ToF64(Box::new(Expr::ProductField {
                        input: Box::new(Expr::Input),
                        field: source_column,
                    }))),
                }],
            },
        )],
    );

    let mut state = DatabaseState::default();
    state.model.relations.insert(
        relation,
        (0..row_count)
            .map(|raw| vec![Value::I64(raw as i64)])
            .collect(),
    );
    (
        source,
        registry,
        program,
        state,
        relation,
        source_column,
        target_column,
    )
}

#[test]
fn exact_relation_delta_rewrites_only_touched_factorized_relation_authority() {
    let (source, registry, _program, state, relation, source_column, _target_column) =
        factorized_relation_migration_fixture(4);
    let (mut atoms, mut root) = realize_database_state_factorized(&state, &source).unwrap();
    let before_atoms = atoms.clone();
    let result_type = kernel_query::RelExpr::Scan(relation)
        .typecheck(&source, &registry)
        .unwrap();
    let delta = RelationDelta {
        removed: vec![vec![Value::I64(1)]],
        inserted: vec![vec![Value::I64(9)]],
        result_type,
    };

    root.apply_exact_relation_delta_endpoint(&mut atoms, relation, &delta, &source, &registry)
        .unwrap();

    assert_eq!(
        root.evaluate(&atoms).unwrap().model.relations[&relation],
        vec![
            vec![Value::I64(0)],
            vec![Value::I64(2)],
            vec![Value::I64(3)],
            vec![Value::I64(9)],
        ]
    );
    assert_eq!(
        source_column,
        *root
            .relations()
            .get(&relation)
            .unwrap()
            .column_order()
            .first()
            .unwrap()
    );
    assert_eq!(before_atoms.len() + 1, atoms.len());
}

#[test]
fn factorized_relation_columns_compile_row_local_migration_without_row_metadata() {
    let (source, registry, program, state, relation, _source_column, target_column) =
        factorized_relation_migration_fixture(4);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();

    assert_eq!(source_root.dependencies().len(), 2);
    assert_eq!(target_root.dependencies().len(), 2);
    assert!(matches!(
        target_root
            .relations()
            .get(&relation)
            .unwrap()
            .columns()
            .get(&target_column)
            .unwrap(),
        FactorizedRelationColumnExpr::I64ToF64Direct(_)
    ));
    assert_eq!(
        target_root
            .read_relation_column(&atoms, relation, target_column, 3)
            .unwrap(),
        Value::F64Bits(3.0_f64.to_bits())
    );
    assert_eq!(
        target_root.evaluate(&atoms).unwrap().model.relations[&relation],
        vec![
            vec![Value::F64Bits(0.0_f64.to_bits())],
            vec![Value::F64Bits(1.0_f64.to_bits())],
            vec![Value::F64Bits(2.0_f64.to_bits())],
            vec![Value::F64Bits(3.0_f64.to_bits())],
        ]
    );

    let native = target_root
        .materialize_relation_column(&mut atoms, relation, target_column)
        .unwrap();
    assert!(matches!(
        target_root
            .relations()
            .get(&relation)
            .unwrap()
            .columns()
            .get(&target_column)
            .unwrap(),
        FactorizedRelationColumnExpr::Direct(atom) if *atom == native
    ));
}

#[test]
fn current_schema_relation_write_installs_native_overlay_without_source_rewrite() {
    let (source, registry, program, state, relation, source_column, target_column) =
        factorized_relation_migration_fixture(8);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();

    let native = target_root
        .install_relation_cell_overlay(
            &mut atoms,
            relation,
            target_column,
            5,
            Value::F64Bits(777.0_f64.to_bits()),
            4,
        )
        .unwrap();

    assert_eq!(
        target_root
            .read_relation_column(&atoms, relation, target_column, 5)
            .unwrap(),
        Value::F64Bits(777.0_f64.to_bits())
    );
    assert_eq!(
        source_root
            .read_relation_column(&atoms, relation, source_column, 5)
            .unwrap(),
        Value::I64(5)
    );
    assert!(target_root.dependencies().contains(&native));
}

#[test]
#[ignore = "manual release-mode factorized relation benchmark"]
fn factorized_relation_scale_and_scan_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    let count = 100_000_usize;
    let (source, registry, program, state, relation, source_column, target_column) =
        factorized_relation_migration_fixture(count);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();

    let compose_start = Instant::now();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();
    let compose_elapsed = compose_start.elapsed();

    let scan = |root: &FactorizedRealizationRoot,
                atoms: &PhysicalAtomStore,
                column: SemanticId|
     -> std::time::Duration {
        let start = Instant::now();
        for row in 0..count {
            black_box(
                root.read_relation_column(atoms, relation, column, row)
                    .unwrap(),
            );
        }
        start.elapsed()
    };
    let direct_elapsed = scan(&source_root, &atoms, source_column);
    let derived_elapsed = scan(&target_root, &atoms, target_column);

    let materialize_start = Instant::now();
    target_root
        .materialize_relation_column(&mut atoms, relation, target_column)
        .unwrap();
    let materialize_elapsed = materialize_start.elapsed();
    let native_elapsed = scan(&target_root, &atoms, target_column);

    eprintln!(
        "FACTORIZED_RELATION_PERF rows={count} compose_us={:.2} source_atoms={} target_dependencies={} direct_scan_ms={:.3} derived_scan_ms={:.3} derived_ratio={:.2}x materialize_ms={:.3} native_scan_ms={:.3} native_ratio={:.2}x",
        compose_elapsed.as_secs_f64() * 1_000_000.0,
        source_root.dependencies().len(),
        target_root.dependencies().len(),
        direct_elapsed.as_secs_f64() * 1_000.0,
        derived_elapsed.as_secs_f64() * 1_000.0,
        derived_elapsed.as_secs_f64() / direct_elapsed.as_secs_f64(),
        materialize_elapsed.as_secs_f64() * 1_000.0,
        native_elapsed.as_secs_f64() * 1_000.0,
        native_elapsed.as_secs_f64() / direct_elapsed.as_secs_f64(),
    );
}

#[test]
fn factorized_relation_reorder_resolves_stable_column_ids_not_ordinals() {
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticEnvironment, TypeExpr,
    };
    use kernel_semantics::EquivalenceModule;
    use kernel_transport::{MigrationColumnRewrite, MigrationRelationRewrite, MigrationRowRewrite};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    let relation = SemanticId::new(62_000);
    let eq = SemanticId::new(62_001);
    let first = SemanticId::new(62_002);
    let second = SemanticId::new(62_003);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);

    let relation_def = |id| RelationDef {
        id,
        columns: vec![
            TypeExpr::Scalar(ScalarType::I64),
            TypeExpr::Scalar(ScalarType::I64),
        ],
        semantics: RelationSemantics::Bag {
            column_equivalences: vec![eq, eq],
        },
    };
    let mut source_schema = Schema::new(SchemaRevisionId::new(620));
    source_schema
        .define_relation_with_column_ids(relation_def(relation), vec![first, second])
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(620));
    source_environment.pin_module(eq, digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(621));
    target_schema
        .define_relation_with_column_ids(relation_def(relation), vec![second, first])
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(620));
    target_environment.pin_module(eq, digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };
    let identity = |column| {
        ExactQuery::new(Expr::ProductField {
            input: Box::new(Expr::Input),
            field: column,
        })
    };
    let program = SchemaMigrationProgram::new(
        target,
        vec![],
        vec![MigrationRelationRewrite::Rows(MigrationRowRewrite {
            source_relation: relation,
            target_relation: relation,
            columns: vec![
                MigrationColumnRewrite {
                    source_columns: vec![second],
                    target_column: second,
                    transform: identity(second),
                },
                MigrationColumnRewrite {
                    source_columns: vec![first],
                    target_column: first,
                    transform: identity(first),
                },
            ],
        })],
    );
    let mut state = DatabaseState::default();
    state
        .model
        .relations
        .insert(relation, vec![vec![Value::I64(7), Value::I64(9)]]);

    let (atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();

    assert_eq!(
        target_root.evaluate(&atoms).unwrap().model.relations[&relation],
        vec![vec![Value::I64(9), Value::I64(7)]]
    );
    assert!(
        target_root
            .relations()
            .get(&relation)
            .unwrap()
            .columns()
            .values()
            .all(|expr| matches!(expr, FactorizedRelationColumnExpr::Direct(_)))
    );
}

fn factorized_general_union_fixture(
    rows_per_side: usize,
) -> (
    SemanticContext,
    SemanticRegistry,
    SchemaMigrationProgram,
    DatabaseState,
    SemanticId,
    SemanticId,
    SemanticId,
) {
    use kernel_query::RelExpr;
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticEnvironment, TypeExpr,
    };
    use kernel_semantics::EquivalenceModule;
    use kernel_transport::{MigrationRelationRewrite, RelationRewrite};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    let left = SemanticId::new(63_000);
    let right = SemanticId::new(63_001);
    let target_relation = SemanticId::new(63_002);
    let eq = SemanticId::new(63_003);
    let column = SemanticId::new(63_004);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let relation_def = |id| RelationDef {
        id,
        columns: vec![TypeExpr::Scalar(ScalarType::I64)],
        semantics: RelationSemantics::Bag {
            column_equivalences: vec![eq],
        },
    };

    let mut source_schema = Schema::new(SchemaRevisionId::new(630));
    source_schema
        .define_relation_with_column_ids(relation_def(left), vec![column])
        .unwrap();
    source_schema
        .define_relation_with_column_ids(relation_def(right), vec![column])
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(630));
    source_environment.pin_module(eq, digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(631));
    target_schema
        .define_relation_with_column_ids(relation_def(left), vec![column])
        .unwrap();
    target_schema
        .define_relation_with_column_ids(relation_def(right), vec![column])
        .unwrap();
    target_schema
        .define_relation_with_column_ids(relation_def(target_relation), vec![column])
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(630));
    target_environment.pin_module(eq, digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };
    let program = SchemaMigrationProgram::new(
        target,
        vec![],
        vec![MigrationRelationRewrite::Query(RelationRewrite {
            target_relation,
            transform: RelExpr::Union {
                left: Box::new(RelExpr::Scan(left)),
                right: Box::new(RelExpr::Scan(right)),
            },
        })],
    );
    let mut state = DatabaseState::default();
    state.model.relations.insert(
        left,
        (0..rows_per_side)
            .map(|raw| vec![Value::I64(raw as i64)])
            .collect(),
    );
    state.model.relations.insert(
        right,
        (0..rows_per_side)
            .map(|raw| vec![Value::I64((rows_per_side + raw) as i64)])
            .collect(),
    );
    (
        source,
        registry,
        program,
        state,
        left,
        right,
        target_relation,
    )
}

#[test]
fn factorized_general_relation_requires_exact_preparation_then_cuts_over_native() {
    let (source, registry, program, state, left, right, target_relation) =
        factorized_general_union_fixture(1);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();

    assert_eq!(
        compose_schema_migration_factorized(&source_root, &source, &registry, &program),
        Err(RealizationError::GeneralRelationRewriteRequiresPreparedRealization(target_relation))
    );

    let prepared = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    assert_eq!(prepared.source_relations(), &BTreeSet::from([left, right]));
    assert_eq!(prepared.source_atoms().len(), 2);
    assert_eq!(prepared.prepared_atoms().len(), 1);
    assert_eq!(prepared.base_witness().relation(), target_relation);

    let target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared],
    )
    .unwrap();
    assert_eq!(
        target_root.evaluate(&atoms).unwrap().model.relations[&target_relation],
        vec![vec![Value::I64(0)], vec![Value::I64(1)]]
    );
    assert!(
        target_root
            .relations()
            .get(&target_relation)
            .unwrap()
            .columns()
            .values()
            .all(|expr| matches!(expr, FactorizedRelationColumnExpr::Direct(_)))
    );
}

#[test]
fn one_shot_bag_difference_keeps_only_inherent_blocker_counts() {
    use kernel_query::RelExpr;
    use kernel_transport::{MigrationRelationRewrite, RelationRewrite};

    let (source, registry, union_program, mut state, left, right, target_relation) =
        factorized_general_union_fixture(1);
    state.model.relations.insert(
        left,
        vec![
            vec![Value::I64(1)],
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(3)],
        ],
    );
    state.model.relations.insert(
        right,
        vec![
            vec![Value::I64(1)],
            vec![Value::I64(3)],
            vec![Value::I64(9)],
        ],
    );
    let program = SchemaMigrationProgram::new(
        union_program.target().clone(),
        vec![],
        vec![MigrationRelationRewrite::Query(RelationRewrite {
            target_relation,
            transform: RelExpr::Difference {
                left: Box::new(RelExpr::Scan(left)),
                right: Box::new(RelExpr::Scan(right)),
            },
        })],
    );
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared],
    )
    .unwrap();
    assert_eq!(
        target_root.evaluate(&atoms).unwrap().model.relations[&target_relation],
        vec![vec![Value::I64(1)], vec![Value::I64(2)]]
    );
}

#[test]
fn one_shot_bag_antijoin_retracts_complete_blocked_left_fiber() {
    use kernel_query::RelExpr;
    use kernel_transport::{MigrationRelationRewrite, RelationRewrite};

    let (source, registry, union_program, mut state, left, right, target_relation) =
        factorized_general_union_fixture(1);
    let equivalence = match &source.schema.relation(left).unwrap().semantics {
        kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => column_equivalences[0],
    };
    state.model.relations.insert(
        left,
        vec![
            vec![Value::I64(1)],
            vec![Value::I64(1)],
            vec![Value::I64(2)],
            vec![Value::I64(3)],
        ],
    );
    state
        .model
        .relations
        .insert(right, vec![vec![Value::I64(1)], vec![Value::I64(9)]]);
    let program = SchemaMigrationProgram::new(
        union_program.target().clone(),
        vec![],
        vec![MigrationRelationRewrite::Query(RelationRewrite {
            target_relation,
            transform: RelExpr::AntiJoin {
                left: Box::new(RelExpr::Scan(left)),
                right: Box::new(RelExpr::Scan(right)),
                left_column: 0,
                right_column: 0,
                equivalence,
            },
        })],
    );
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared],
    )
    .unwrap();
    assert_eq!(
        target_root.evaluate(&atoms).unwrap().model.relations[&target_relation],
        vec![vec![Value::I64(2)], vec![Value::I64(3)]]
    );
}

#[test]
fn one_shot_filter_eq_const_is_supported_without_whole_row_fallback() {
    use kernel_query::RelExpr;
    use kernel_transport::{MigrationRelationRewrite, RelationRewrite};

    let (source, registry, union_program, state, left, _right, target_relation) =
        factorized_general_union_fixture(1);
    let equivalence = match &source.schema.relation(left).unwrap().semantics {
        kernel_schema::RelationSemantics::Bag {
            column_equivalences,
        }
        | kernel_schema::RelationSemantics::Set {
            column_equivalences,
        } => column_equivalences[0],
    };
    let program = SchemaMigrationProgram::new(
        union_program.target().clone(),
        vec![],
        vec![MigrationRelationRewrite::Query(RelationRewrite {
            target_relation,
            transform: RelExpr::FilterEqConst {
                input: Box::new(RelExpr::Scan(left)),
                column: 0,
                value: Value::I64(0),
                equivalence,
            },
        })],
    );
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared],
    )
    .unwrap();
    assert_eq!(
        target_root.evaluate(&atoms).unwrap().model.relations[&target_relation],
        vec![vec![Value::I64(0)]]
    );
}

#[test]
fn general_relation_current_b_write_detaches_to_native_endpoint_without_inverse() {
    use kernel_change::{RewriteLawSetId, RewriteSpec, RewriteSpecId};
    use kernel_query::{RelExpr, RelationDelta, RelationValue};

    let (source, registry, program, state, _left, _right, target_relation) =
        factorized_general_union_fixture(1);
    let target_context = program.verify(&source, &registry).unwrap().target().clone();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared_migration = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let mut target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        std::slice::from_ref(&prepared_migration),
    )
    .unwrap();
    let old_target_atoms = target_root
        .relations()
        .get(&target_relation)
        .unwrap()
        .columns()
        .values()
        .filter_map(|expr| match expr {
            FactorizedRelationColumnExpr::Direct(atom) => Some(*atom),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let old_rows = target_root.evaluate(&atoms).unwrap().model.relations[&target_relation].clone();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&target_context, &registry)
        .unwrap();
    let delta = RelationDelta {
        removed: vec![vec![Value::I64(0)]],
        inserted: vec![vec![Value::I64(1)]],
        result_type: result_type.clone(),
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId::new(63_100)),
        law_set: RewriteLawSetId(SemanticId::new(63_101)),
        footprint: delta
            .rewrite_footprint(target_relation, &target_context, &registry)
            .unwrap(),
    };
    let old = RelationValue::Bag(old_rows.to_vec());
    let prepared_write = delta
        .prepare_relation_rewrite(
            target_relation,
            &old,
            &target_context,
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();

    let target_column = target_context
        .schema
        .relation_column_ids(target_relation)
        .unwrap()[0];
    let mut stale_root = target_root.clone();
    let mut stale_atoms = atoms.clone();
    stale_root
        .install_relation_cell_overlay(
            &mut stale_atoms,
            target_relation,
            target_column,
            0,
            Value::I64(9),
            1,
        )
        .unwrap();
    assert_eq!(
        stale_root.install_prepared_relation_endpoint(
            &mut stale_atoms,
            target_relation,
            &prepared_write,
            &registry,
        ),
        Err(RealizationError::RelationQuery(
            kernel_query::RelQueryError::StructuralRewriteBaseMismatch
        ))
    );

    let native = target_root
        .install_prepared_relation_endpoint(&mut atoms, target_relation, &prepared_write, &registry)
        .unwrap();

    assert_eq!(native.len(), 1);
    assert_eq!(
        target_root.evaluate(&atoms).unwrap().model.relations[&target_relation],
        vec![vec![Value::I64(1)], vec![Value::I64(1)]]
    );
    assert!(
        target_root
            .relations()
            .get(&target_relation)
            .unwrap()
            .columns()
            .values()
            .all(|expr| matches!(expr, FactorizedRelationColumnExpr::Direct(_)))
    );
    assert!(old_target_atoms.is_disjoint(&target_root.dependencies()));
}

#[test]
fn general_relation_current_b_write_uses_bounded_delta_overlay_then_compacts() {
    use kernel_change::{RewriteLawSetId, RewriteSpec, RewriteSpecId};
    use kernel_query::{RelExpr, RelationDelta};

    let (source, registry, program, state, _left, _right, target_relation) =
        factorized_general_union_fixture(2);
    let target_context = program.verify(&source, &registry).unwrap().target().clone();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared_migration = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let mut target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        std::slice::from_ref(&prepared_migration),
    )
    .unwrap();
    assert!(
        prepared_migration
            .base_witness()
            .shares_occurrence_root_with(
                target_root.relation_base_witness(target_relation).unwrap()
            )
    );
    let old_target_atoms = target_root
        .relations()
        .get(&target_relation)
        .unwrap()
        .columns()
        .values()
        .filter_map(|expr| match expr {
            FactorizedRelationColumnExpr::Direct(atom) => Some(*atom),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&target_context, &registry)
        .unwrap();
    let delta = RelationDelta {
        removed: vec![vec![Value::I64(0)]],
        inserted: vec![vec![Value::I64(99)]],
        result_type,
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId::new(63_120)),
        law_set: RewriteLawSetId(SemanticId::new(63_121)),
        footprint: delta
            .rewrite_footprint(target_relation, &target_context, &registry)
            .unwrap(),
    };
    let prepared = delta
        .prepare_relation_rewrite_on_base(
            target_root.relation_base_witness(target_relation).unwrap(),
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    let stale = prepared.clone();
    let delta_atom = target_root
        .install_prepared_relation_delta_overlay(
            &mut atoms,
            target_relation,
            &prepared,
            &target_context,
            &registry,
        )
        .unwrap()
        .expect("one inserted row gets one immutable delta atom");

    let mut values = target_root.evaluate(&atoms).unwrap().model.relations[&target_relation]
        .iter()
        .map(|row| match row.as_slice() {
            [Value::I64(value)] => *value,
            _ => panic!("unexpected row"),
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    assert_eq!(values, vec![1, 2, 3, 99]);

    assert!(target_root.dependencies().contains(&delta_atom));
    assert!(old_target_atoms.is_subset(&target_root.dependencies()));
    assert_eq!(
        target_root.install_prepared_relation_delta_overlay(
            &mut atoms,
            target_relation,
            &stale,
            &target_context,
            &registry,
        ),
        Err(RealizationError::RelationQuery(
            kernel_query::RelQueryError::StructuralRewriteBaseMismatch
        ))
    );

    let compacted = target_root
        .compact_relation_delta_overlay(&mut atoms, target_relation, &target_context, &registry)
        .unwrap();
    assert_eq!(compacted.len(), 1);
    assert!(!target_root.dependencies().contains(&delta_atom));
    assert!(old_target_atoms.is_disjoint(&target_root.dependencies()));
}

#[test]
fn bounded_relation_delta_overlay_routes_set_removal_by_gamma_class() {
    use kernel_change::{RewriteLawSetId, RewriteSpec, RewriteSpecId};
    use kernel_query::{RelExpr, RelationDelta};
    use kernel_schema::{
        RelationDef, RelationSemantics, ScalarType, Schema, SemanticEnvironment, TypeExpr,
    };
    use kernel_semantics::EquivalenceModule;
    use kernel_transport::{MigrationRelationRewrite, RelationRewrite};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    let left = SemanticId::new(63_200);
    let right = SemanticId::new(63_201);
    let target_relation = SemanticId::new(63_202);
    let eq = SemanticId::new(63_203);
    let column = SemanticId::new(63_204);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let relation_def = |id| RelationDef {
        id,
        columns: vec![TypeExpr::Scalar(ScalarType::Text)],
        semantics: RelationSemantics::Set {
            column_equivalences: vec![eq],
        },
    };
    let mut source_schema = Schema::new(SchemaRevisionId::new(632));
    source_schema
        .define_relation_with_column_ids(relation_def(left), vec![column])
        .unwrap();
    source_schema
        .define_relation_with_column_ids(relation_def(right), vec![column])
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(632));
    source_environment.pin_module(eq, digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };
    let mut target_schema = Schema::new(SchemaRevisionId::new(633));
    target_schema
        .define_relation_with_column_ids(relation_def(left), vec![column])
        .unwrap();
    target_schema
        .define_relation_with_column_ids(relation_def(right), vec![column])
        .unwrap();
    target_schema
        .define_relation_with_column_ids(relation_def(target_relation), vec![column])
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(632));
    target_environment.pin_module(eq, digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };
    let program = SchemaMigrationProgram::new(
        target,
        vec![],
        vec![MigrationRelationRewrite::Query(RelationRewrite {
            target_relation,
            transform: RelExpr::Union {
                left: Box::new(RelExpr::Scan(left)),
                right: Box::new(RelExpr::Scan(right)),
            },
        })],
    );
    let mut state = DatabaseState::default();
    state
        .model
        .relations
        .insert(left, vec![vec![Value::Text("Alpha".into())]]);
    state
        .model
        .relations
        .insert(right, vec![vec![Value::Text("Beta".into())]]);
    let target_context = program.verify(&source, &registry).unwrap().target().clone();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared_migration = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let mut target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared_migration],
    )
    .unwrap();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&target_context, &registry)
        .unwrap();
    let delta = RelationDelta {
        removed: vec![vec![Value::Text("ALPHA".into())]],
        inserted: vec![vec![Value::Text("Gamma".into())]],
        result_type,
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId::new(63_210)),
        law_set: RewriteLawSetId(SemanticId::new(63_211)),
        footprint: delta
            .rewrite_footprint(target_relation, &target_context, &registry)
            .unwrap(),
    };
    let prepared = delta
        .prepare_relation_rewrite_on_base(
            target_root.relation_base_witness(target_relation).unwrap(),
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    target_root
        .install_prepared_relation_delta_overlay(
            &mut atoms,
            target_relation,
            &prepared,
            &target_context,
            &registry,
        )
        .unwrap();
    let first_snapshot = target_root.clone();
    let mut rows = target_root.evaluate(&atoms).unwrap().model.relations[&target_relation]
        .iter()
        .map(|row| match row.as_slice() {
            [Value::Text(value)] => value.to_string(),
            _ => panic!("unexpected row"),
        })
        .collect::<Vec<_>>();
    rows.sort();
    assert_eq!(rows, vec!["Beta".to_owned(), "Gamma".to_owned()]);

    let scan_seed = target_root
        .relation_scan_occurrence_seed(target_relation)
        .unwrap();
    assert_eq!(scan_seed.row_count(), 2);
    let seeded_relations = BTreeSet::from([target_relation]);
    let scan_seeds = BTreeMap::from([(target_relation, scan_seed)]);
    let join = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(target_relation)),
        right: Box::new(RelExpr::Scan(target_relation)),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    }
    .prepare(&target_context, &registry)
    .unwrap();
    assert!(join.emits_occurrence_certificate_with_scan_seeds(&seeded_relations));
    let current_model = target_root.evaluate(&atoms).unwrap().model;
    let baseline_join = join
        .evaluate(&current_model, &target_context, &registry)
        .unwrap();
    let (seeded_join, _) = join
        .evaluate_with_occurrence_certificate_seeded(
            &current_model,
            &target_context,
            &registry,
            &scan_seeds,
        )
        .unwrap();
    assert_eq!(seeded_join, baseline_join);

    let second_delta = RelationDelta {
        removed: vec![vec![Value::Text("BETA".into())]],
        inserted: vec![vec![Value::Text("Delta".into())]],
        result_type: RelExpr::Scan(target_relation)
            .typecheck(&target_context, &registry)
            .unwrap(),
    };
    let second_spec = RewriteSpec {
        id: RewriteSpecId(SemanticId::new(63_212)),
        law_set: RewriteLawSetId(SemanticId::new(63_213)),
        footprint: second_delta
            .rewrite_footprint(target_relation, &target_context, &registry)
            .unwrap(),
    };
    let second_prepared = second_delta
        .prepare_relation_rewrite_on_base(
            target_root.relation_base_witness(target_relation).unwrap(),
            &registry,
            &second_spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    target_root
        .install_prepared_relation_delta_overlay(
            &mut atoms,
            target_relation,
            &second_prepared,
            &target_context,
            &registry,
        )
        .unwrap();
    let before_compaction = target_root
        .relation_delta_overlay_stats(target_relation)
        .unwrap()
        .unwrap();
    assert_eq!(before_compaction.live_rows, 2);
    assert_eq!(before_compaction.inserted_rows, 2);
    assert_eq!(before_compaction.delta_atoms, 2);
    target_root
        .compact_relation_delta_overlay(&mut atoms, target_relation, &target_context, &registry)
        .unwrap();
    let compacted = target_root
        .relation_delta_overlay_stats(target_relation)
        .unwrap()
        .unwrap();
    assert_eq!(compacted.inserted_rows, 0);
    assert_eq!(compacted.displaced_rows, 0);
    assert_eq!(compacted.delta_atoms, 0);
    let mut current_rows = target_root.evaluate(&atoms).unwrap().model.relations[&target_relation]
        .iter()
        .map(|row| match row.as_slice() {
            [Value::Text(value)] => value.to_string(),
            _ => panic!("unexpected row"),
        })
        .collect::<Vec<_>>();
    current_rows.sort();
    assert_eq!(current_rows, vec!["Delta".to_owned(), "Gamma".to_owned()]);
    let mut old_rows = first_snapshot.evaluate(&atoms).unwrap().model.relations[&target_relation]
        .iter()
        .map(|row| match row.as_slice() {
            [Value::Text(value)] => value.to_string(),
            _ => panic!("unexpected row"),
        })
        .collect::<Vec<_>>();
    old_rows.sort();
    assert_eq!(old_rows, vec!["Beta".to_owned(), "Gamma".to_owned()]);
}

#[test]
fn repeated_bag_overlay_lifecycle_preserves_old_roots_and_rejects_semantic_stale_rewrites() {
    use kernel_change::{RewriteLawSetId, RewriteSpec, RewriteSpecId};
    use kernel_query::{RelExpr, RelationDelta};

    let (source, registry, program, state, _left, _right, target_relation) =
        factorized_general_union_fixture(32);
    let target_context = program.verify(&source, &registry).unwrap().target().clone();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared_migration = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let mut root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared_migration],
    )
    .unwrap();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&target_context, &registry)
        .unwrap();

    let values = |root: &FactorizedRealizationRoot, atoms: &PhysicalAtomStore| {
        let mut values = root.evaluate(atoms).unwrap().model.relations[&target_relation]
            .iter()
            .map(|row| match row.as_slice() {
                [Value::I64(value)] => *value,
                _ => panic!("unexpected row"),
            })
            .collect::<Vec<_>>();
        values.sort_unstable();
        values
    };
    let mut expected = (0_i64..64).collect::<Vec<_>>();
    let mut retained = Vec::<(
        FactorizedRealizationRoot,
        Vec<i64>,
        BTreeSet<PhysicalAtomId>,
    )>::new();

    for step in 0..48usize {
        let remove = expected[step % expected.len()];
        let insert = 10_000_i64 + step as i64;
        let delta = RelationDelta {
            removed: vec![vec![Value::I64(remove)]],
            inserted: vec![vec![Value::I64(insert)]],
            result_type: result_type.clone(),
        };
        let spec = RewriteSpec {
            id: RewriteSpecId(SemanticId::new(640_000u128 + step as u128 * 2)),
            law_set: RewriteLawSetId(SemanticId::new(640_001u128 + step as u128 * 2)),
            footprint: delta
                .rewrite_footprint(target_relation, &target_context, &registry)
                .unwrap(),
        };
        let prepared = delta
            .prepare_relation_rewrite_on_base(
                root.relation_base_witness(target_relation).unwrap(),
                &registry,
                &spec,
                Vec::<Value>::new(),
            )
            .unwrap();
        let semantic_stale = prepared.clone();
        root.install_prepared_relation_delta_overlay(
            &mut atoms,
            target_relation,
            &prepared,
            &target_context,
            &registry,
        )
        .unwrap();
        let remove_at = expected.iter().position(|value| *value == remove).unwrap();
        expected.remove(remove_at);
        expected.push(insert);
        expected.sort_unstable();
        assert_eq!(values(&root, &atoms), expected);
        let stats = root
            .relation_delta_overlay_stats(target_relation)
            .unwrap()
            .unwrap();
        assert_eq!(stats.live_rows, expected.len());
        assert!(stats.inserted_rows <= step + 1);
        assert!(stats.delta_atoms <= stats.inserted_rows);
        assert_eq!(
            root.install_prepared_relation_delta_overlay(
                &mut atoms,
                target_relation,
                &semantic_stale,
                &target_context,
                &registry,
            ),
            Err(RealizationError::RelationQuery(
                kernel_query::RelQueryError::StructuralRewriteBaseMismatch
            ))
        );

        if step % 8 == 7 {
            retained.push((root.clone(), expected.clone(), root.dependencies()));
            root.compact_relation_delta_overlay(
                &mut atoms,
                target_relation,
                &target_context,
                &registry,
            )
            .unwrap();
            let stats = root
                .relation_delta_overlay_stats(target_relation)
                .unwrap()
                .unwrap();
            assert_eq!(stats.inserted_rows, 0);
            assert_eq!(stats.displaced_rows, 0);
            assert_eq!(stats.delta_atoms, 0);
            assert_eq!(values(&root, &atoms), expected);
            for (old_root, old_expected, old_dependencies) in &retained {
                assert_eq!(values(old_root, &atoms), *old_expected);
                assert_eq!(&old_root.dependencies(), old_dependencies);
            }
        }
    }
}

#[test]
#[ignore = "manual release-mode bounded current-B relation overlay benchmark"]
fn general_relation_current_b_bounded_overlay_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    use kernel_change::{RewriteLawSetId, RewriteSpec, RewriteSpecId};
    use kernel_query::{RelExpr, RelationDelta};

    let rows_per_side = std::env::var("CFMD_BENCH_ROWS_PER_SIDE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50_000_usize);
    let total = rows_per_side * 2;
    let (source, registry, program, state, _left, _right, target_relation) =
        factorized_general_union_fixture(rows_per_side);
    let target_context = program.verify(&source, &registry).unwrap().target().clone();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared_migration = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared_migration],
    )
    .unwrap();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&target_context, &registry)
        .unwrap();
    let delta = RelationDelta {
        removed: vec![],
        inserted: vec![vec![Value::I64(total as i64)]],
        result_type,
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId::new(63_130)),
        law_set: RewriteLawSetId(SemanticId::new(63_131)),
        footprint: delta
            .rewrite_footprint(target_relation, &target_context, &registry)
            .unwrap(),
    };
    let witness_start = Instant::now();
    let advanced = target_root
        .relation_base_witness(target_relation)
        .unwrap()
        .advance(kernel_types::RevisionId::new(0), &delta, &registry)
        .unwrap();
    let witness_elapsed = witness_start.elapsed();
    black_box(advanced);
    let prepare_start = Instant::now();
    let prepared = delta
        .prepare_relation_rewrite_on_base(
            target_root.relation_base_witness(target_relation).unwrap(),
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();
    let prepare_elapsed = prepare_start.elapsed();
    let clone_start = Instant::now();
    let mut sample = target_root.clone();
    let clone_elapsed = clone_start.elapsed();
    let install_start = Instant::now();
    let atom = sample
        .install_prepared_relation_delta_overlay(
            &mut atoms,
            target_relation,
            &prepared,
            &target_context,
            &registry,
        )
        .unwrap();
    let install_elapsed = install_start.elapsed();
    let target_column = target_context
        .schema
        .relation_column_ids(target_relation)
        .unwrap()[0];
    let scan_repetitions = 16;
    let direct_scan_start = Instant::now();
    for _ in 0..scan_repetitions {
        target_root
            .visit_relation_column_range(
                &atoms,
                target_relation,
                target_column,
                0,
                total,
                |value| {
                    black_box(value);
                },
            )
            .unwrap();
    }
    let direct_scan_elapsed = direct_scan_start.elapsed();
    let overlay_scan_start = Instant::now();
    for _ in 0..scan_repetitions {
        sample
            .visit_relation_column_range(
                &atoms,
                target_relation,
                target_column,
                0,
                total,
                |value| {
                    black_box(value);
                },
            )
            .unwrap();
    }
    let overlay_scan_elapsed = overlay_scan_start.elapsed();
    black_box(&sample);
    eprintln!(
        "GENERAL_REL_B_BOUNDED_WRITE_PERF rows={total} delta_rows=1 separate_witness_prepare_ms={:.3} witness_advance_us={:.3} prepare_us={:.3} root_clone_us={:.3} install_us={:.3} direct_scan_ms={:.3} overlay_scan_ms={:.3} scan_ratio={:.3}x delta_atom={}",
        0.0,
        witness_elapsed.as_secs_f64() * 1_000_000.0,
        prepare_elapsed.as_secs_f64() * 1_000_000.0,
        clone_elapsed.as_secs_f64() * 1_000_000.0,
        install_elapsed.as_secs_f64() * 1_000_000.0,
        direct_scan_elapsed.as_secs_f64() * 1_000.0,
        overlay_scan_elapsed.as_secs_f64() * 1_000.0,
        overlay_scan_elapsed.as_secs_f64() / direct_scan_elapsed.as_secs_f64(),
        atom.is_some(),
    );
}

#[test]
#[ignore = "manual release-mode repeated current-B overlay depth benchmark"]
fn repeated_relation_delta_overlay_depth_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    use kernel_change::{RewriteLawSetId, RewriteSpec, RewriteSpecId};
    use kernel_query::{RelExpr, RelationDelta};

    let rows_per_side = 50_000_usize;
    let total = rows_per_side * 2;
    let (source, registry, program, state, _left, _right, target_relation) =
        factorized_general_union_fixture(rows_per_side);
    let target_context = program.verify(&source, &registry).unwrap().target().clone();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared_migration = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let baseline = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared_migration],
    )
    .unwrap();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&target_context, &registry)
        .unwrap();
    let target_column = target_context
        .schema
        .relation_column_ids(target_relation)
        .unwrap()[0];
    let scan_repetitions = 8;
    let direct_start = Instant::now();
    for _ in 0..scan_repetitions {
        baseline
            .visit_relation_column_range(
                &atoms,
                target_relation,
                target_column,
                0,
                total,
                |value| {
                    black_box(value);
                },
            )
            .unwrap();
    }
    let direct_elapsed = direct_start.elapsed();
    let point_repetitions = 100_000usize;
    let direct_point_start = Instant::now();
    for index in 0..point_repetitions {
        let row = index.wrapping_mul(7_919) % total;
        black_box(
            baseline
                .read_relation_column(&atoms, target_relation, target_column, row)
                .unwrap(),
        );
    }
    let direct_point_elapsed = direct_point_start.elapsed();
    let range_width = 4_096usize;
    let range_repetitions = 64usize;
    let direct_range_start = Instant::now();
    for index in 0..range_repetitions {
        let start = index.wrapping_mul(1_543) % (total - range_width);
        baseline
            .visit_relation_column_range(
                &atoms,
                target_relation,
                target_column,
                start,
                start + range_width,
                |value| {
                    black_box(value);
                },
            )
            .unwrap();
    }
    let direct_range_elapsed = direct_range_start.elapsed();
    let mut sample = baseline.clone();
    let mut applied = 0usize;
    let mut last_scan_elapsed = direct_elapsed;
    for target_depth in [1usize, 16, 128, 1_024] {
        while applied < target_depth {
            let delta = RelationDelta {
                removed: vec![vec![Value::I64(applied as i64)]],
                inserted: vec![vec![Value::I64((total + applied) as i64)]],
                result_type: result_type.clone(),
            };
            let spec = RewriteSpec {
                id: RewriteSpecId(SemanticId::new(650_000u128 + applied as u128 * 2)),
                law_set: RewriteLawSetId(SemanticId::new(650_001u128 + applied as u128 * 2)),
                footprint: delta
                    .rewrite_footprint(target_relation, &target_context, &registry)
                    .unwrap(),
            };
            let prepared = delta
                .prepare_relation_rewrite_on_base(
                    sample.relation_base_witness(target_relation).unwrap(),
                    &registry,
                    &spec,
                    Vec::<Value>::new(),
                )
                .unwrap();
            sample
                .install_prepared_relation_delta_overlay(
                    &mut atoms,
                    target_relation,
                    &prepared,
                    &target_context,
                    &registry,
                )
                .unwrap();
            applied += 1;
        }
        let scan_start = Instant::now();
        for _ in 0..scan_repetitions {
            sample
                .visit_relation_column_range(
                    &atoms,
                    target_relation,
                    target_column,
                    0,
                    total,
                    |value| {
                        black_box(value);
                    },
                )
                .unwrap();
        }
        let scan_elapsed = scan_start.elapsed();
        let point_start = Instant::now();
        for index in 0..point_repetitions {
            let row = index.wrapping_mul(7_919) % total;
            black_box(
                sample
                    .read_relation_column(&atoms, target_relation, target_column, row)
                    .unwrap(),
            );
        }
        let point_elapsed = point_start.elapsed();
        let range_start = Instant::now();
        for index in 0..range_repetitions {
            let start = index.wrapping_mul(1_543) % (total - range_width);
            sample
                .visit_relation_column_range(
                    &atoms,
                    target_relation,
                    target_column,
                    start,
                    start + range_width,
                    |value| {
                        black_box(value);
                    },
                )
                .unwrap();
        }
        let range_elapsed = range_start.elapsed();
        last_scan_elapsed = scan_elapsed;
        let stats = sample
            .relation_delta_overlay_stats(target_relation)
            .unwrap()
            .unwrap();
        eprintln!(
            "REL_OVERLAY_DEPTH_PERF rows={total} depth={target_depth} inserted={} displaced={} atoms={} direct_ms={:.3} overlay_ms={:.3} scan_ratio={:.3}x point_ratio={:.3}x range4096_ratio={:.3}x",
            stats.inserted_rows,
            stats.displaced_rows,
            stats.delta_atoms,
            direct_elapsed.as_secs_f64() * 1_000.0,
            scan_elapsed.as_secs_f64() * 1_000.0,
            scan_elapsed.as_secs_f64() / direct_elapsed.as_secs_f64(),
            point_elapsed.as_secs_f64() / direct_point_elapsed.as_secs_f64(),
            range_elapsed.as_secs_f64() / direct_range_elapsed.as_secs_f64(),
        );
    }
    let compaction_rows = sample
        .evaluate(&atoms)
        .unwrap()
        .model
        .relations
        .materialize_owned(&target_relation)
        .unwrap();
    let result_type = sample
        .relation_base_witness(target_relation)
        .unwrap()
        .result_type()
        .clone();
    let legacy_witness_start = Instant::now();
    let legacy_witness = kernel_query::RelationBaseWitness::build(
        kernel_types::RevisionId::new(0),
        target_relation,
        &compaction_rows,
        result_type,
        &target_context,
        &registry,
    )
    .unwrap();
    black_box(legacy_witness);
    let legacy_witness_elapsed = legacy_witness_start.elapsed();
    let rebind_start = Instant::now();
    let scan_seed = sample
        .relation_scan_occurrence_seed(target_relation)
        .unwrap();
    let rebound = sample
        .relation_base_witness(target_relation)
        .unwrap()
        .rebind_dense_storage_identity_from_seed(kernel_types::RevisionId::new(0), &scan_seed)
        .unwrap();
    black_box(rebound);
    let rebind_elapsed = rebind_start.elapsed();
    eprintln!(
        "REL_OVERLAY_COMPACT_WITNESS rows={total} legacy_build_ms={:.3} certified_rebind_ms={:.3} ratio={:.3}x",
        legacy_witness_elapsed.as_secs_f64() * 1_000.0,
        rebind_elapsed.as_secs_f64() * 1_000.0,
        rebind_elapsed.as_secs_f64() / legacy_witness_elapsed.as_secs_f64(),
    );
    let compact_start = Instant::now();
    sample
        .compact_relation_delta_overlay(&mut atoms, target_relation, &target_context, &registry)
        .unwrap();
    let compact_elapsed = compact_start.elapsed();
    let incremental_scan = last_scan_elapsed.saturating_sub(direct_elapsed);
    let break_even_scans = if incremental_scan.is_zero() {
        f64::INFINITY
    } else {
        compact_elapsed.as_secs_f64() / (incremental_scan.as_secs_f64() / scan_repetitions as f64)
    };
    eprintln!(
        "REL_OVERLAY_DEPTH_COMPACT rows={total} depth={applied} compact_ms={:.3} break_even_future_full_scans={break_even_scans:.1}",
        compact_elapsed.as_secs_f64() * 1_000.0,
    );
}

#[test]
#[ignore = "manual release-mode current-B relation endpoint detachment benchmark"]
fn general_relation_current_b_write_detachment_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    use kernel_change::{RewriteLawSetId, RewriteSpec, RewriteSpecId};
    use kernel_query::{RelExpr, RelationDelta, RelationValue};

    let rows_per_side = 50_000_usize;
    let total = rows_per_side * 2;
    let (source, registry, program, state, _left, _right, target_relation) =
        factorized_general_union_fixture(rows_per_side);
    let target_context = program.verify(&source, &registry).unwrap().target().clone();
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let prepared_migration = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        &[prepared_migration],
    )
    .unwrap();
    let old_rows = target_root.evaluate(&atoms).unwrap().model.relations[&target_relation].to_vec();
    let result_type = RelExpr::Scan(target_relation)
        .typecheck(&target_context, &registry)
        .unwrap();
    let delta = RelationDelta {
        removed: vec![],
        inserted: vec![vec![Value::I64(total as i64)]],
        result_type,
    };
    let spec = RewriteSpec {
        id: RewriteSpecId(SemanticId::new(63_110)),
        law_set: RewriteLawSetId(SemanticId::new(63_111)),
        footprint: delta
            .rewrite_footprint(target_relation, &target_context, &registry)
            .unwrap(),
    };
    let prepared_write = delta
        .prepare_relation_rewrite(
            target_relation,
            &RelationValue::Bag(old_rows),
            &target_context,
            &registry,
            &spec,
            Vec::<Value>::new(),
        )
        .unwrap();

    let mut sample = target_root.clone();
    let start = Instant::now();
    let native = sample
        .install_prepared_relation_endpoint(&mut atoms, target_relation, &prepared_write, &registry)
        .unwrap();
    let elapsed = start.elapsed();
    black_box(sample);
    eprintln!(
        "GENERAL_REL_B_WRITE_DETACH_PERF rows={total} delta_rows=1 native_atoms={} detach_ms={:.3}",
        native.len(),
        elapsed.as_secs_f64() * 1_000.0,
    );
}

#[test]
#[ignore = "manual release-mode general relation preparation benchmark"]
fn factorized_general_relation_preparation_scale_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    let rows_per_side = 50_000_usize;
    let total = rows_per_side * 2;
    let (source, registry, program, state, _left, _right, target_relation) =
        factorized_general_union_fixture(rows_per_side);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();

    let prepare_start = Instant::now();
    let prepared = prepare_general_relation_factorized(
        &source_root,
        &mut atoms,
        &source,
        &registry,
        &program,
        target_relation,
    )
    .unwrap();
    let prepare_elapsed = prepare_start.elapsed();
    black_box(&prepared);

    let cutover_start = Instant::now();
    let target_root = compose_schema_migration_factorized_with_prepared(
        &source_root,
        &atoms,
        &source,
        &registry,
        &program,
        std::slice::from_ref(&prepared),
    )
    .unwrap();
    let cutover_elapsed = cutover_start.elapsed();

    eprintln!(
        "GENERAL_REL_PREP_PERF rows={total} source_relations={} source_atoms={} prepared_atoms={} prepare_ms={:.3} cutover_us={:.3} target_dependencies={}",
        prepared.source_relations().len(),
        prepared.source_atoms().len(),
        prepared.prepared_atoms().len(),
        prepare_elapsed.as_secs_f64() * 1_000.0,
        cutover_elapsed.as_secs_f64() * 1_000_000.0,
        target_root.dependencies().len(),
    );
}

#[test]
fn factorized_relation_chunk_materialization_is_bounded_and_releases_base_after_full_coverage() {
    let count = 10_usize;
    let chunk_rows = 4_usize;
    let (source, registry, program, state, relation, _source_column, target_column) =
        factorized_relation_migration_fixture(count);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let source_atom = match source_root
        .relations()
        .get(&relation)
        .unwrap()
        .columns()
        .values()
        .next()
        .unwrap()
    {
        FactorizedRelationColumnExpr::Direct(atom) => *atom,
        other => panic!("unexpected source realization: {other:?}"),
    };
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();
    let semantic_before = target_root.evaluate(&atoms).unwrap();

    assert_eq!(
        target_root
            .relation_column_native_chunk_count(relation, target_column)
            .unwrap(),
        0
    );
    let chunk = target_root
        .materialize_relation_column_chunk(&mut atoms, relation, target_column, 5, chunk_rows)
        .unwrap();
    let payload = atoms.get(chunk).unwrap().payload();
    let PhysicalAtomPayload::RelationColumnSegment(segment) = payload else {
        panic!("expected relation column segment");
    };
    assert_eq!(segment.start_row(), 4);
    assert_eq!(segment.len(), 4);
    assert_eq!(
        target_root
            .relation_column_native_chunk_count(relation, target_column)
            .unwrap(),
        1
    );
    for row in 0..count {
        assert_eq!(
            target_root
                .read_relation_column(&atoms, relation, target_column, row)
                .unwrap(),
            Value::F64Bits((row as f64).to_bits())
        );
    }
    assert!(target_root.dependencies().contains(&source_atom));

    target_root
        .materialize_relation_column_chunk(&mut atoms, relation, target_column, 0, chunk_rows)
        .unwrap();
    target_root
        .materialize_relation_column_chunk(&mut atoms, relation, target_column, 9, chunk_rows)
        .unwrap();
    assert_eq!(
        target_root
            .relation_column_native_chunk_count(relation, target_column)
            .unwrap(),
        3
    );
    assert!(!target_root.dependencies().contains(&source_atom));
    assert_eq!(target_root.evaluate(&atoms).unwrap(), semantic_before);
}

#[test]
#[ignore = "manual release-mode chunked relation realization benchmark"]
fn factorized_relation_chunked_access_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    let count = 100_000_usize;
    let chunk_rows = 4_096_usize;
    let (source, registry, program, state, relation, source_column, target_column) =
        factorized_relation_migration_fixture(count);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();

    let scan = |root: &FactorizedRealizationRoot, atoms: &PhysicalAtomStore, column: SemanticId| {
        let repeats = 8_u32;
        let start = Instant::now();
        for _ in 0..repeats {
            root.visit_relation_column_range(atoms, relation, column, 0, count, |value| {
                black_box(value);
            })
            .unwrap();
        }
        start.elapsed() / repeats
    };
    let random =
        |root: &FactorizedRealizationRoot, atoms: &PhysicalAtomStore, column: SemanticId| {
            let start = Instant::now();
            let mut row = 17_usize;
            for _ in 0..count {
                row = (row.wrapping_mul(65_537).wrapping_add(17)) % count;
                black_box(
                    root.read_relation_column(atoms, relation, column, row)
                        .unwrap(),
                );
            }
            start.elapsed()
        };

    let direct_scan = scan(&source_root, &atoms, source_column);
    let direct_random = random(&source_root, &atoms, source_column);
    let derived_scan = scan(&target_root, &atoms, target_column);
    let derived_random = random(&target_root, &atoms, target_column);

    let hot_row = count / 2;
    let materialize_repeats = 12_u32;
    let materialize_start = Instant::now();
    for _ in 0..materialize_repeats {
        let mut sample = target_root.clone();
        black_box(
            sample
                .materialize_relation_column_chunk(
                    &mut atoms,
                    relation,
                    target_column,
                    hot_row,
                    chunk_rows,
                )
                .unwrap(),
        );
    }
    let materialize_chunk = materialize_start.elapsed() / materialize_repeats;
    target_root
        .materialize_relation_column_chunk(&mut atoms, relation, target_column, hot_row, chunk_rows)
        .unwrap();
    let partial_scan = scan(&target_root, &atoms, target_column);
    let partial_random = random(&target_root, &atoms, target_column);

    let hot_start = (hot_row / chunk_rows) * chunk_rows;
    let hot_end = usize::min(hot_start + chunk_rows, count);
    let hot_repeats = 64_usize;
    let hot_direct_start = Instant::now();
    for _ in 0..hot_repeats {
        source_root
            .visit_relation_column_range(
                &atoms,
                relation,
                source_column,
                hot_start,
                hot_end,
                |value| {
                    black_box(value);
                },
            )
            .unwrap();
    }
    let hot_direct_scan = hot_direct_start.elapsed();
    let hot_start_time = Instant::now();
    for _ in 0..hot_repeats {
        target_root
            .visit_relation_column_range(
                &atoms,
                relation,
                target_column,
                hot_start,
                hot_end,
                |value| {
                    black_box(value);
                },
            )
            .unwrap();
    }
    let hot_chunk_scan = hot_start_time.elapsed();

    eprintln!(
        "CHUNKED_RELATION_PERF rows={count} chunk_rows={chunk_rows} direct_scan_ms={:.3} direct_random_ms={:.3} derived_scan_ms={:.3} derived_ratio={:.2}x derived_random_ms={:.3} derived_random_ratio={:.2}x materialize_chunk_us={:.2} partial_scan_ms={:.3} partial_scan_ratio={:.2}x partial_random_ms={:.3} partial_random_ratio={:.2}x hot_direct_us={:.2} hot_chunk_us={:.2} hot_ratio={:.2}x native_chunks={} deps={}",
        direct_scan.as_secs_f64() * 1_000.0,
        direct_random.as_secs_f64() * 1_000.0,
        derived_scan.as_secs_f64() * 1_000.0,
        derived_scan.as_secs_f64() / direct_scan.as_secs_f64(),
        derived_random.as_secs_f64() * 1_000.0,
        derived_random.as_secs_f64() / direct_random.as_secs_f64(),
        materialize_chunk.as_secs_f64() * 1_000_000.0,
        partial_scan.as_secs_f64() * 1_000.0,
        partial_scan.as_secs_f64() / direct_scan.as_secs_f64(),
        partial_random.as_secs_f64() * 1_000.0,
        partial_random.as_secs_f64() / direct_random.as_secs_f64(),
        hot_direct_scan.as_secs_f64() * 1_000_000.0,
        hot_chunk_scan.as_secs_f64() * 1_000_000.0,
        hot_chunk_scan.as_secs_f64() / hot_direct_scan.as_secs_f64(),
        target_root
            .relation_column_native_chunk_count(relation, target_column)
            .unwrap(),
        target_root.dependencies().len(),
    );
}

fn factorized_sparse_field_migration_fixture(
    count: usize,
) -> (
    SemanticContext,
    SemanticRegistry,
    SchemaMigrationProgram,
    DatabaseState,
    SemanticId,
    SemanticId,
    SemanticId,
    Vec<EntityId>,
) {
    use kernel_schema::{FieldDef, ScalarType, Schema, SemanticEnvironment, TypeExpr};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    let entity_type = SemanticId::new(70_000);
    let source_field = SemanticId::new(70_001);
    let target_field = SemanticId::new(70_002);
    let registry = SemanticRegistry::default();

    let mut source_schema = Schema::new(SchemaRevisionId::new(700));
    source_schema
        .define_field(FieldDef {
            id: source_field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    let source = SemanticContext {
        schema: source_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(700)),
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(701));
    target_schema
        .define_field(FieldDef {
            id: target_field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::F64),
        })
        .unwrap();
    let target = SemanticContext {
        schema: target_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(701)),
    };
    let program = SchemaMigrationProgram::new(
        target,
        vec![kernel_transport::MigrationFieldRewrite {
            source_fields: vec![source_field],
            target_field,
            transform: ExactQuery::new(Expr::I64ToF64(Box::new(Expr::ProductField {
                input: Box::new(Expr::Input),
                field: source_field,
            }))),
        }],
        vec![],
    );

    let entities = (0..count)
        .map(|index| EntityId::new((index as u128 + 1) * 10_003 + 97))
        .collect::<Vec<_>>();
    let mut state = DatabaseState::default();
    for (index, &entity) in entities.iter().enumerate() {
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
            .insert((source_field, entity), Value::I64(index as i64));
    }
    (
        source,
        registry,
        program,
        state,
        entity_type,
        source_field,
        target_field,
        entities,
    )
}

#[test]
fn factorized_field_chunk_uses_carrier_ordinal_segments_not_entity_id_ranges() {
    let count = 10_usize;
    let chunk_entities = 4_usize;
    let (source, registry, program, state, _owner, source_field, target_field, entities) =
        factorized_sparse_field_migration_fixture(count);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let source_atom = match source_root.fields().get(&source_field).unwrap().expr() {
        FactorizedFieldExpr::Direct(atom) => *atom,
        other => panic!("unexpected source realization: {other:?}"),
    };
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();
    let semantic_before = target_root.evaluate(&atoms).unwrap();

    let middle_entity = entities[5];
    let chunk = target_root
        .materialize_field_column_chunk(&mut atoms, target_field, middle_entity, chunk_entities)
        .unwrap();
    let FactorizedFieldExpr::ChunkOverlay {
        native_chunks,
        carrier,
        ..
    } = target_root.fields().get(&target_field).unwrap().expr()
    else {
        panic!("expected field chunk overlay");
    };
    assert_eq!(native_chunks.len(), 1);
    let native = native_chunks[0];
    assert_eq!(native.atom(), chunk);
    assert_eq!(native.segment().carrier(), *carrier);
    assert_eq!(native.segment().start_ordinal(), 4);
    assert_eq!(native.segment().len(), 4);
    assert!(middle_entity.raw() > 10_000, "fixture must use sparse ids");
    assert!(target_root.dependencies().contains(&source_atom));

    for (index, &entity) in entities.iter().enumerate() {
        assert_eq!(
            target_root
                .read_field(&atoms, target_field, entity)
                .unwrap(),
            Value::F64Bits((index as f64).to_bits())
        );
    }

    target_root
        .materialize_field_column_chunk(&mut atoms, target_field, entities[0], chunk_entities)
        .unwrap();
    target_root
        .materialize_field_column_chunk(&mut atoms, target_field, entities[9], chunk_entities)
        .unwrap();
    assert_eq!(
        target_root.field_native_chunk_count(target_field).unwrap(),
        3
    );
    assert!(!target_root.dependencies().contains(&source_atom));
    assert_eq!(target_root.evaluate(&atoms).unwrap(), semantic_before);
}

#[test]
#[ignore = "manual release-mode chunked field realization benchmark"]
fn factorized_field_chunked_access_benchmark() {
    use std::hint::black_box;
    use std::time::Instant;

    let count = 100_000_usize;
    let chunk_entities = 4_096_usize;
    let (source, registry, program, state, _owner, source_field, target_field, entities) =
        factorized_sparse_field_migration_fixture(count);
    let (mut atoms, source_root) = realize_database_state_factorized(&state, &source).unwrap();
    let mut target_root =
        compose_schema_migration_factorized(&source_root, &source, &registry, &program).unwrap();

    let scan = |root: &FactorizedRealizationRoot, atoms: &PhysicalAtomStore, field: SemanticId| {
        let repeats = 8_u32;
        let start = Instant::now();
        for _ in 0..repeats {
            root.visit_field_carrier_range(atoms, field, 0, count, |entity, value| {
                black_box((entity, value));
            })
            .unwrap();
        }
        start.elapsed() / repeats
    };
    let random =
        |root: &FactorizedRealizationRoot, atoms: &PhysicalAtomStore, field: SemanticId| {
            let start = Instant::now();
            let mut index = 17_usize;
            for _ in 0..count {
                index = (index.wrapping_mul(65_537).wrapping_add(17)) % count;
                black_box(root.read_field(atoms, field, entities[index]).unwrap());
            }
            start.elapsed()
        };

    let direct_scan = scan(&source_root, &atoms, source_field);
    let direct_random = random(&source_root, &atoms, source_field);
    let derived_scan = scan(&target_root, &atoms, target_field);
    let derived_random = random(&target_root, &atoms, target_field);

    let hot_index = count / 2;
    let materialize_repeats = 12_u32;
    let start = Instant::now();
    for _ in 0..materialize_repeats {
        let mut sample = target_root.clone();
        black_box(
            sample
                .materialize_field_column_chunk(
                    &mut atoms,
                    target_field,
                    entities[hot_index],
                    chunk_entities,
                )
                .unwrap(),
        );
    }
    let materialize_chunk = start.elapsed() / materialize_repeats;
    target_root
        .materialize_field_column_chunk(
            &mut atoms,
            target_field,
            entities[hot_index],
            chunk_entities,
        )
        .unwrap();
    let partial_scan = scan(&target_root, &atoms, target_field);
    let partial_random = random(&target_root, &atoms, target_field);

    let hot_start = (hot_index / chunk_entities) * chunk_entities;
    let hot_end = usize::min(hot_start + chunk_entities, count);
    let hot_repeats = 64_usize;
    let start = Instant::now();
    for _ in 0..hot_repeats {
        source_root
            .visit_field_carrier_range(&atoms, source_field, hot_start, hot_end, |entity, value| {
                black_box((entity, value));
            })
            .unwrap();
    }
    let hot_direct = start.elapsed();
    let start = Instant::now();
    for _ in 0..hot_repeats {
        target_root
            .visit_field_carrier_range(&atoms, target_field, hot_start, hot_end, |entity, value| {
                black_box((entity, value));
            })
            .unwrap();
    }
    let hot_chunk = start.elapsed();

    eprintln!(
        "CHUNKED_FIELD_PERF rows={count} chunk_entities={chunk_entities} direct_scan_ms={:.3} direct_random_ms={:.3} derived_scan_ms={:.3} derived_ratio={:.2}x derived_random_ms={:.3} derived_random_ratio={:.2}x materialize_chunk_us={:.2} partial_scan_ms={:.3} partial_scan_ratio={:.2}x partial_random_ms={:.3} partial_random_ratio={:.2}x hot_direct_us={:.2} hot_chunk_us={:.2} hot_ratio={:.2}x native_chunks={} deps={}",
        direct_scan.as_secs_f64() * 1_000.0,
        direct_random.as_secs_f64() * 1_000.0,
        derived_scan.as_secs_f64() * 1_000.0,
        derived_scan.as_secs_f64() / direct_scan.as_secs_f64(),
        derived_random.as_secs_f64() * 1_000.0,
        derived_random.as_secs_f64() / direct_random.as_secs_f64(),
        materialize_chunk.as_secs_f64() * 1_000_000.0,
        partial_scan.as_secs_f64() * 1_000.0,
        partial_scan.as_secs_f64() / direct_scan.as_secs_f64(),
        partial_random.as_secs_f64() * 1_000.0,
        partial_random.as_secs_f64() / direct_random.as_secs_f64(),
        hot_direct.as_secs_f64() * 1_000_000.0,
        hot_chunk.as_secs_f64() * 1_000_000.0,
        hot_chunk.as_secs_f64() / hot_direct.as_secs_f64(),
        target_root.field_native_chunk_count(target_field).unwrap(),
        target_root.dependencies().len(),
    );
}

#[test]
fn physical_atom_store_clones_share_payloads_and_reclaim_path_copies() {
    let mut base = PhysicalAtomStore::default();
    let mut ids = Vec::new();
    for value in 0_i64..4_096 {
        ids.push(base.insert(PhysicalAtomPayload::Value(Value::I64(value))));
    }
    let snapshot = base.clone();
    assert!(base.shares_storage_root_with(&snapshot));
    assert!(base.shares_atom_allocation_with(&snapshot, ids[2_048]));

    let mut successor = snapshot.clone();
    let inserted = successor.insert(PhysicalAtomPayload::Value(Value::I64(9_999)));
    assert!(!successor.shares_storage_root_with(&snapshot));
    assert!(successor.shares_atom_allocation_with(&snapshot, ids[2_048]));
    let probe = successor.unique_storage_probe_against(&snapshot);
    assert!(probe.map_nodes() < 128);
    assert_eq!(probe.atom_allocations(), 1);
    assert_eq!(
        successor.get(inserted).unwrap().payload(),
        &PhysicalAtomPayload::Value(Value::I64(9_999))
    );
    drop(successor);
    assert_eq!(probe.live_map_nodes(), 0);
    assert_eq!(probe.live_atom_allocations(), 0);
}

#[test]
fn physical_atom_store_exact_merge_reuses_existing_lineage_allocations() {
    let mut base = PhysicalAtomStore::default();
    let shared = base.insert(PhysicalAtomPayload::Value(Value::I64(7)));
    let mut branch = base.clone();
    let branch_only = branch.insert(PhysicalAtomPayload::Value(Value::I64(8)));
    let mut merged = base.clone();
    merged.merge_exact_from(&branch).unwrap();
    assert!(merged.shares_atom_allocation_with(&base, shared));
    assert!(merged.shares_atom_allocation_with(&branch, branch_only));
}

#[test]
#[ignore = "manual release-mode atom ownership hostile benchmark"]
fn physical_atom_store_ownership_hostile_benchmark() {
    use kernel_persistent::PersistentOrdMap;
    use std::{collections::BTreeMap, hint::black_box, sync::Arc, time::Instant};

    const N: u128 = 100_000;
    let payload = Arc::new(vec![0_u8; 64]);
    let base_entries = (0..N)
        .map(|id| (id, Arc::clone(&payload)))
        .collect::<Vec<_>>();
    let persistent_base = PersistentOrdMap::from_sorted_unique_owned(base_entries).unwrap();
    let cow_base = Arc::new(
        (0..N)
            .map(|id| (id, Arc::clone(&payload)))
            .collect::<BTreeMap<_, _>>(),
    );

    for updates in [1_u128, 16, 256, 1_024] {
        let persistent_snapshot = persistent_base.clone();
        let start = Instant::now();
        let mut persistent_next = persistent_base.clone();
        for id in N..N + updates {
            persistent_next.insert(id, Arc::clone(&payload));
        }
        let persistent_update = start.elapsed();
        black_box((&persistent_snapshot, &persistent_next));

        let cow_snapshot = Arc::clone(&cow_base);
        let start = Instant::now();
        let mut cow_next = Arc::clone(&cow_base);
        let map = Arc::make_mut(&mut cow_next);
        for id in N..N + updates {
            map.insert(id, Arc::clone(&payload));
        }
        let cow_update = start.elapsed();
        black_box((&cow_snapshot, &cow_next));

        eprintln!(
            "ATOM_OWNERSHIP_HOSTILE n={N} updates={updates} persistent_update_us={:.3} arc_cow_update_us={:.3} ratio={:.3}x",
            persistent_update.as_secs_f64() * 1_000_000.0,
            cow_update.as_secs_f64() * 1_000_000.0,
            persistent_update.as_secs_f64() / cow_update.as_secs_f64(),
        );
    }
}
