use std::collections::{BTreeMap, BTreeSet};

use kernel_identity::IdentityTransport;
use kernel_model::{DatabaseState, FiniteModel};
use kernel_query::{ExactQuery, RelExpr, RelationDelta};
use kernel_schema::SemanticContext;

use kernel_schema::{
    FieldDef, ScalarType, Schema, SemanticEnvironment, Symbol, SymbolKind, TypeExpr,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry, TokenizerModule};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

use super::*;

fn set_relation_context(
    env_revision: u64,
    equality_digest: kernel_schema::ModuleDigest,
) -> SemanticContext {
    let relation = SemanticId::new(5000);
    let equality = SemanticId::new(5001);
    let mut schema = Schema::new(SchemaRevisionId::new(5000));
    schema
        .define_relation(kernel_schema::RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: vec![equality],
            },
        })
        .unwrap();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
    environment.pin_module(equality, equality_digest);
    SemanticContext {
        schema,
        environment,
    }
}

fn context(
    schema_revision: u64,
    env_revision: u64,
    name: &str,
) -> (SemanticContext, SemanticRegistry) {
    let entity = SemanticId::new(1);
    let field = SemanticId::new(2);
    let equality = SemanticId::new(3);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let mut schema = Schema::new(SchemaRevisionId::new(schema_revision));
    schema
        .define(Symbol {
            id: field,
            kind: SymbolKind::Field,
            presentation_name: name.into(),
        })
        .unwrap();
    schema
        .define_field(FieldDef {
            id: field,
            owner: entity,
            value: TypeExpr::Scalar(ScalarType::Text),
        })
        .unwrap();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
    environment.pin_module(equality, digest);
    (
        SemanticContext {
            schema,
            environment,
        },
        registry,
    )
}

#[test]
fn rename_and_revision_bump_transport_without_data_migration() {
    let (source, registry) = context(1, 1, "name");
    let (target, _) = context(2, 2, "display_name");
    let transport = DefinitionalTransport::verify(&source, &target, &registry).unwrap();
    assert_eq!(
        transport.transport_state(&DatabaseState::default()),
        DatabaseState::default()
    );
}

#[test]
fn semantic_type_change_is_not_definitional_transport() {
    let (source, registry) = context(1, 1, "name");
    let (mut target, _) = context(2, 2, "name");
    target
        .schema
        .define_field(FieldDef {
            id: SemanticId::new(9),
            owner: SemanticId::new(1),
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    assert_eq!(
        DefinitionalTransport::verify(&source, &target, &registry),
        Err(TransportError::NotDefinitionallyEquivalent)
    );
}
#[test]
fn typed_field_transport_reuses_exact_query_ir_and_rebuilds_trusted_revision() {
    let entity_type = SemanticId::new(20);
    let source_field = SemanticId::new(21);
    let target_field = SemanticId::new(22);
    let entity = kernel_types::EntityId::new(1);
    let registry = SemanticRegistry::default();

    let mut source_schema = Schema::new(SchemaRevisionId::new(10));
    source_schema
        .define_field(FieldDef {
            id: source_field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    let source_context = SemanticContext {
        schema: source_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(10)),
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(11));
    target_schema
        .define_field(FieldDef {
            id: target_field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    let target_context = SemanticContext {
        schema: target_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(11)),
    };

    let mut state = DatabaseState::default();
    state.lifecycle.entities.insert(entity);
    state.lifecycle.roots.insert(entity);
    state
        .model
        .carriers
        .insert(entity_type, BTreeSet::from([entity]));
    state
        .model
        .fields
        .insert((source_field, entity), kernel_model::Value::I64(41));
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source_context,
        &registry,
        state,
    )
    .unwrap();
    let transform = ExactQuery::new(kernel_query::Expr::AddI64(
        Box::new(kernel_query::Expr::Input),
        Box::new(kernel_query::Expr::Const(kernel_model::Value::I64(1))),
    ));
    let transport = TypedFieldTransport::verify(
        &source_context,
        &target_context,
        &registry,
        vec![FieldRewrite {
            source_field,
            target_field,
            transform,
        }],
    )
    .unwrap();
    let target_revision = transport
        .transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        )
        .unwrap();
    assert_eq!(
        target_revision
            .state()
            .model
            .fields
            .get(&(target_field, entity)),
        Some(&kernel_model::Value::I64(42))
    );
    assert!(
        !target_revision
            .state()
            .model
            .fields
            .contains_key(&(source_field, entity))
    );
}

#[test]
fn implementation_upgrade_with_same_contract_is_identity_transport() {
    let mut registry = SemanticRegistry::default();
    let old = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
    let new = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
    let source = set_relation_context(1, old);
    let target = set_relation_context(2, new);

    let transport =
        EquivalentSemanticEnvironmentTransport::verify(&source, &target, &registry).unwrap();
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source,
        &registry,
        DatabaseState::default(),
    )
    .unwrap();
    let target_revision = transport
        .transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        )
        .unwrap();

    assert_eq!(source_revision.state(), target_revision.state());
    assert_ne!(source.environment, target.environment);
}

#[test]
fn changed_law_is_not_misclassified_as_implementation_upgrade() {
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
    let ci = registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 1);
    let source = set_relation_context(1, exact);
    let target = set_relation_context(2, ci);

    assert_eq!(
        EquivalentSemanticEnvironmentTransport::verify(&source, &target, &registry),
        Err(TransportError::SemanticContractChanged(SemanticId::new(
            5001
        )))
    );
    assert!(SemanticLawMigration::verify(&source, &target, &registry).is_ok());
}

#[test]
fn environment_transport_compares_query_visible_modules_even_when_schema_does_not_use_them() {
    let query_equality = SemanticId::new(5050);
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let schema = Schema::new(SchemaRevisionId::new(5050));
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(1));
    source_environment.pin_module(query_equality, exact);
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(2));
    target_environment.pin_module(query_equality, ci);
    let source = SemanticContext {
        schema: schema.clone(),
        environment: source_environment,
    };
    let target = SemanticContext {
        schema,
        environment: target_environment,
    };

    assert_eq!(
        EquivalentSemanticEnvironmentTransport::verify(&source, &target, &registry),
        Err(TransportError::SemanticContractChanged(query_equality))
    );
    assert!(SemanticLawMigration::verify(&source, &target, &registry).is_ok());
}

#[test]
fn adding_query_visible_module_is_conservative_but_removing_it_is_not() {
    let extra = SemanticId::new(5060);
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence(EquivalenceModule::TextExact);
    let schema = Schema::new(SchemaRevisionId::new(5060));
    let source = SemanticContext {
        schema: schema.clone(),
        environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
    };
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(2));
    target_environment.pin_module(extra, exact);
    let target = SemanticContext {
        schema,
        environment: target_environment,
    };

    assert!(ConservativeSemanticEnvironmentExtension::verify(&source, &target, &registry).is_ok());
    assert_eq!(
        ConservativeSemanticEnvironmentExtension::verify(&target, &source, &registry),
        Err(TransportError::NotConservativeSemanticExtension)
    );
}

#[test]
fn relational_impact_is_preserved_by_identity_like_semantic_context_transports() {
    use kernel_change::Change;
    use kernel_model::Value;
    use kernel_query::RelExpr;

    let relation = SemanticId::new(5000);
    let extra_module = SemanticId::new(5070);
    let mut registry = SemanticRegistry::default();
    let exact_v1 = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
    let exact_v2 = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
    let tokenizer = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);

    let source = set_relation_context(1, exact_v1);
    let same_contract = set_relation_context(2, exact_v2);
    let equivalent =
        EquivalentSemanticEnvironmentTransport::verify(&source, &same_contract, &registry).unwrap();

    let mut definitionally_equivalent = source.clone();
    definitionally_equivalent.schema.revision = SchemaRevisionId::new(5001);
    let definitional =
        DefinitionalTransport::verify(&source, &definitionally_equivalent, &registry).unwrap();

    let mut extended = source.clone();
    extended.environment.revision = SemanticEnvId::new(3);
    extended.environment.pin_module(extra_module, tokenizer);
    let conservative =
        ConservativeSemanticEnvironmentExtension::verify(&source, &extended, &registry).unwrap();

    let mut old = FiniteModel::default();
    old.relations
        .insert(relation, vec![vec![Value::Text("A".into())]]);
    let mut next = FiniteModel::default();
    next.relations
        .insert(relation, vec![vec![Value::Text("a".into())]]);
    let change = Change::Replace(next);
    let query = RelExpr::Scan(relation);

    assert!(
        equivalent
            .check_rel_impact_law(&query, &old, &change, &registry)
            .unwrap()
    );
    assert!(
        definitional
            .check_rel_impact_law(&query, &old, &change, &registry)
            .unwrap()
    );
    assert!(
        conservative
            .check_rel_impact_law(&query, &old, &change, &registry)
            .unwrap()
    );
}

#[test]
fn genuine_semantic_law_migration_has_no_generic_relational_impact_invariance() {
    use kernel_change::Change;
    use kernel_model::Value;
    use kernel_query::{Impact, RelExpr, rel_impact_by_recompute};

    let relation = SemanticId::new(5000);
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let source = set_relation_context(1, exact);
    let target = set_relation_context(2, ci);
    SemanticLawMigration::verify(&source, &target, &registry).unwrap();

    let mut old = FiniteModel::default();
    old.relations
        .insert(relation, vec![vec![Value::Text("A".into())]]);
    let mut next = FiniteModel::default();
    next.relations
        .insert(relation, vec![vec![Value::Text("a".into())]]);
    let change = Change::Replace(next);
    let query = RelExpr::Scan(relation);

    assert_eq!(
        rel_impact_by_recompute(&query, &old, &change, &source, &registry),
        Impact::Changed
    );
    assert_eq!(
        rel_impact_by_recompute(&query, &old, &change, &target, &registry),
        Impact::Unaffected
    );
}

#[test]
fn law_migration_revalidates_target_instead_of_silently_collapsing_set_rows() {
    let relation = SemanticId::new(5000);
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
    let ci = registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 1);
    let source = set_relation_context(1, exact);
    let target = set_relation_context(2, ci);
    let migration = SemanticLawMigration::verify(&source, &target, &registry).unwrap();
    let mut state = DatabaseState::default();
    state.model.relations.insert(
        relation,
        vec![
            vec![kernel_model::Value::Text("A".into())],
            vec![kernel_model::Value::Text("a".into())],
        ],
    );
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source,
        &registry,
        state,
    )
    .unwrap();

    assert!(matches!(
        migration.transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        ),
        Err(TransportError::InvalidRevision(
            kernel_revision::RevisionError::InvalidTypedModel(
                kernel_validation::ValidationError::Semantic(
                    kernel_semantics::SemanticError::DuplicateRelationRow
                )
            )
        ))
    ));
}

#[test]
fn typed_relation_transport_reuses_relational_query_ir() {
    let source_relation = SemanticId::new(6000);
    let target_relation = SemanticId::new(6001);
    let i64_eq = SemanticId::new(6002);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(60));
    environment.pin_module(i64_eq, digest);

    let mut source_schema = Schema::new(SchemaRevisionId::new(60));
    source_schema
        .define_relation(kernel_schema::RelationDef {
            id: source_relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences: vec![i64_eq],
            },
        })
        .unwrap();
    let source = SemanticContext {
        schema: source_schema,
        environment: environment.clone(),
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(61));
    target_schema
        .define_relation(kernel_schema::RelationDef {
            id: target_relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences: vec![i64_eq],
            },
        })
        .unwrap();
    let target = SemanticContext {
        schema: target_schema,
        environment,
    };

    let transport = TypedRelationTransport::verify(
        &source,
        &target,
        &registry,
        vec![RelationRewrite {
            target_relation,
            transform: RelExpr::Project {
                input: Box::new(RelExpr::Scan(source_relation)),
                columns: vec![0],
            },
        }],
    )
    .unwrap();
    let mut state = DatabaseState::default();
    state.model.relations.insert(
        source_relation,
        vec![
            vec![kernel_model::Value::I64(1)],
            vec![kernel_model::Value::I64(2)],
        ],
    );
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source,
        &registry,
        state,
    )
    .unwrap();
    let target_revision = transport
        .transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        )
        .unwrap();

    assert_eq!(
        target_revision
            .state()
            .model
            .relations
            .materialize_owned(&target_relation),
        Some(vec![
            vec![kernel_model::Value::I64(1)],
            vec![kernel_model::Value::I64(2)],
        ])
    );
    assert!(
        !target_revision
            .state()
            .model
            .relations
            .contains_key(&source_relation)
    );
}

#[test]
fn typed_relation_transport_checks_full_set_bag_semantics() {
    let source_relation = SemanticId::new(6100);
    let target_relation = SemanticId::new(6101);
    let text_eq = SemanticId::new(6102);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::TextExact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(61));
    environment.pin_module(text_eq, digest);

    let mut source_schema = Schema::new(SchemaRevisionId::new(61));
    source_schema
        .define_relation(kernel_schema::RelationDef {
            id: source_relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences: vec![text_eq],
            },
        })
        .unwrap();
    let source = SemanticContext {
        schema: source_schema,
        environment: environment.clone(),
    };
    let mut target_schema = Schema::new(SchemaRevisionId::new(62));
    target_schema
        .define_relation(kernel_schema::RelationDef {
            id: target_relation,
            columns: vec![TypeExpr::Scalar(ScalarType::Text)],
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: vec![text_eq],
            },
        })
        .unwrap();
    let target = SemanticContext {
        schema: target_schema,
        environment,
    };

    assert_eq!(
        TypedRelationTransport::verify(
            &source,
            &target,
            &registry,
            vec![RelationRewrite {
                target_relation,
                transform: RelExpr::Scan(source_relation),
            }],
        ),
        Err(TransportError::RelationTransformType(
            RelQueryError::TypeMismatch
        ))
    );
}

fn identity_transport_context(
    person: SemanticId,
    friend: SemanticId,
    relation: SemanticId,
    env_revision: u64,
    registry: &mut SemanticRegistry,
) -> SemanticContext {
    let live_eq = SemanticId::new(person.raw() + 10_000);
    let historical_eq = SemanticId::new(person.raw() + 20_000);
    let live_digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
    let historical_digest =
        registry.install_equivalence(EquivalenceModule::HistoricalEntityIdExact(person));
    let mut schema = Schema::new(SchemaRevisionId::new(70));
    schema
        .define_field(FieldDef {
            id: friend,
            owner: person,
            value: TypeExpr::Scalar(ScalarType::LiveEntityRef(person)),
        })
        .unwrap();
    schema
        .define_relation(kernel_schema::RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::LiveEntityRef(person)),
                TypeExpr::Scalar(ScalarType::HistoricalEntityId(person)),
            ],
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences: vec![live_eq, historical_eq],
            },
        })
        .unwrap();
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(env_revision));
    environment.pin_module(live_eq, live_digest);
    environment.pin_module(historical_eq, historical_digest);
    SemanticContext {
        schema,
        environment,
    }
}

fn identity_transport_state(
    person: SemanticId,
    friend: SemanticId,
    relation: SemanticId,
) -> DatabaseState {
    let mut state = DatabaseState::default();
    state.lifecycle.entities = BTreeSet::from([
        kernel_types::EntityId::new(1),
        kernel_types::EntityId::new(2),
    ]);
    state.lifecycle.roots.insert(kernel_types::EntityId::new(1));
    state.lifecycle.keeps_alive.insert(
        kernel_types::EntityId::new(1),
        BTreeSet::from([kernel_types::EntityId::new(2)]),
    );
    state
        .model
        .carriers
        .insert(person, state.lifecycle.entities.clone());
    state.model.fields.insert(
        (friend, kernel_types::EntityId::new(1)),
        kernel_model::Value::LiveEntityRef {
            entity_type: person,
            id: kernel_types::EntityId::new(2),
        },
    );
    state.model.relations.insert(
        relation,
        vec![vec![
            kernel_model::Value::LiveEntityRef {
                entity_type: person,
                id: kernel_types::EntityId::new(1),
            },
            kernel_model::Value::HistoricalEntityId {
                entity_type: person,
                id: kernel_types::EntityId::new(2),
            },
        ]],
    );
    state
}

#[test]
fn bijective_identity_transport_rewrites_lifecycle_carriers_and_nested_references_coherently() {
    let person = SemanticId::new(7000);
    let friend = SemanticId::new(7001);
    let relation = SemanticId::new(7002);
    let mut registry = SemanticRegistry::default();
    let source = identity_transport_context(person, friend, relation, 70, &mut registry);
    let target = identity_transport_context(person, friend, relation, 71, &mut registry);
    let old = BTreeSet::from([
        kernel_types::EntityId::new(1),
        kernel_types::EntityId::new(2),
    ]);
    let new = BTreeSet::from([
        kernel_types::EntityId::new(11),
        kernel_types::EntityId::new(12),
    ]);
    let identity = kernel_identity::IdentityTransport::new(
        &old,
        &new,
        BTreeMap::from([
            (
                kernel_types::EntityId::new(1),
                kernel_types::EntityId::new(11),
            ),
            (
                kernel_types::EntityId::new(2),
                kernel_types::EntityId::new(12),
            ),
        ]),
    )
    .unwrap();
    let inverse = identity.inverse();
    let forward =
        BijectiveIdentityRevisionTransport::verify(&source, &target, &registry, identity).unwrap();
    let backward =
        BijectiveIdentityRevisionTransport::verify(&target, &source, &registry, inverse).unwrap();
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source,
        &registry,
        identity_transport_state(person, friend, relation),
    )
    .unwrap();
    let mapped = forward
        .transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        )
        .unwrap();
    assert_eq!(mapped.state().lifecycle.entities, new);
    assert_eq!(
        mapped
            .state()
            .model
            .fields
            .get(&(friend, kernel_types::EntityId::new(11))),
        Some(&kernel_model::Value::LiveEntityRef {
            entity_type: person,
            id: kernel_types::EntityId::new(12),
        })
    );
    assert!(matches!(
        &mapped.state().model.relations[&relation][0][1],
        kernel_model::Value::HistoricalEntityId { id, .. }
            if *id == kernel_types::EntityId::new(12)
    ));
    let round_trip = backward
        .transport_revision(&mapped, kernel_types::RevisionId::new(3), &registry)
        .unwrap();
    assert_eq!(round_trip.state(), source_revision.state());
}

#[test]
fn typed_field_transport_rejects_rewrite_for_passthrough_authority() {
    let entity_type = SemanticId::new(25);
    let field = SemanticId::new(26);
    let registry = SemanticRegistry::default();
    let mut schema = Schema::new(SchemaRevisionId::new(18));
    schema
        .define_field(FieldDef {
            id: field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(18)),
    };
    assert_eq!(
        TypedFieldTransport::verify(
            &context,
            &context,
            &registry,
            vec![FieldRewrite {
                source_field: field,
                target_field: field,
                transform: ExactQuery::new(kernel_query::Expr::Input),
            }],
        ),
        Err(TransportError::RewriteTargetsPassthroughField(field))
    );
}

#[test]
fn typed_relation_transport_rejects_rewrite_for_passthrough_authority() {
    let relation = SemanticId::new(27);
    let eq = SemanticId::new(28);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(19));
    environment.pin_module(eq, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(19));
    schema
        .define_relation(kernel_schema::RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: kernel_schema::RelationSemantics::Bag {
                column_equivalences: vec![eq],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    assert_eq!(
        TypedRelationTransport::verify(
            &context,
            &context,
            &registry,
            vec![RelationRewrite {
                target_relation: relation,
                transform: RelExpr::Scan(relation),
            }],
        ),
        Err(TransportError::RewriteTargetsPassthroughRelation(relation))
    );
}

#[test]
fn typed_transport_rejects_transform_whose_output_type_disagrees_with_target() {
    let entity_type = SemanticId::new(30);
    let source_field = SemanticId::new(31);
    let target_field = SemanticId::new(32);
    let registry = SemanticRegistry::default();
    let mut source_schema = Schema::new(SchemaRevisionId::new(20));
    source_schema
        .define_field(FieldDef {
            id: source_field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::I64),
        })
        .unwrap();
    let mut target_schema = Schema::new(SchemaRevisionId::new(21));
    target_schema
        .define_field(FieldDef {
            id: target_field,
            owner: entity_type,
            value: TypeExpr::Scalar(ScalarType::Text),
        })
        .unwrap();
    let source = SemanticContext {
        schema: source_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(20)),
    };
    let target = SemanticContext {
        schema: target_schema,
        environment: SemanticEnvironment::new(SemanticEnvId::new(21)),
    };
    let error = TypedFieldTransport::verify(
        &source,
        &target,
        &registry,
        vec![FieldRewrite {
            source_field,
            target_field,
            transform: ExactQuery::new(kernel_query::Expr::Input),
        }],
    )
    .unwrap_err();
    assert_eq!(
        error,
        TransportError::TransformType(QueryTypeError::TypeMismatch)
    );
}
#[test]
fn field_transport_cannot_silently_reinterpret_semantic_environment() {
    let entity_type = SemanticId::new(40);
    let field = SemanticId::new(41);
    let eq = SemanticId::new(42);
    let mut registry = SemanticRegistry::default();
    let exact = registry.install_equivalence(EquivalenceModule::TextExact);
    let ci = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let mut schema = Schema::new(SchemaRevisionId::new(30));
    schema
        .define_field(FieldDef {
            id: field,
            owner: entity_type,
            value: TypeExpr::Set {
                element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                equivalence: eq,
            },
        })
        .unwrap();
    let mut source_env = SemanticEnvironment::new(SemanticEnvId::new(30));
    source_env.pin_module(eq, exact);
    let mut target_env = SemanticEnvironment::new(SemanticEnvId::new(31));
    target_env.pin_module(eq, ci);
    let source = SemanticContext {
        schema: schema.clone(),
        environment: source_env,
    };
    let target = SemanticContext {
        schema,
        environment: target_env,
    };
    assert_eq!(
        TypedFieldTransport::verify(&source, &target, &registry, vec![]),
        Err(TransportError::SemanticEnvironmentChangeRequiresTransport)
    );
}
#[test]
fn impact_commutes_with_bijective_identity_transport() {
    use kernel_change::Change;
    use kernel_model::Value;
    use kernel_query::{ExactQuery, Expr};
    use std::collections::BTreeSet;

    let entity_type = SemanticId::new(9900);
    let source_ids = BTreeSet::from([
        kernel_types::EntityId::new(1),
        kernel_types::EntityId::new(2),
    ]);
    let target_ids = BTreeSet::from([
        kernel_types::EntityId::new(11),
        kernel_types::EntityId::new(12),
    ]);
    let identity = IdentityTransport::new(
        &source_ids,
        &target_ids,
        BTreeMap::from([
            (
                kernel_types::EntityId::new(1),
                kernel_types::EntityId::new(11),
            ),
            (
                kernel_types::EntityId::new(2),
                kernel_types::EntityId::new(12),
            ),
        ]),
    )
    .unwrap();
    let old = Value::LiveEntityRef {
        entity_type,
        id: kernel_types::EntityId::new(1),
    };
    let change = Change::Replace(Value::LiveEntityRef {
        entity_type,
        id: kernel_types::EntityId::new(2),
    });
    let query = ExactQuery::new(Expr::Input);

    assert!(check_impact_transport_law(&identity, &query, &old, &change).unwrap());
}

#[test]
fn relational_impact_commutes_with_bijective_identity_transport() {
    use kernel_change::Change;
    use kernel_model::Value;
    use kernel_query::RelExpr;
    use kernel_schema::{RelationDef, RelationSemantics};
    use std::collections::{BTreeMap, BTreeSet};

    let person = SemanticId::new(9910);
    let relation = SemanticId::new(9911);
    let equality = SemanticId::new(9912);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9910));
    environment.pin_module(equality, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(9910));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::LiveEntityRef(person))],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equality],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };

    let source_ids = BTreeSet::from([
        kernel_types::EntityId::new(1),
        kernel_types::EntityId::new(2),
    ]);
    let target_ids = BTreeSet::from([
        kernel_types::EntityId::new(11),
        kernel_types::EntityId::new(12),
    ]);
    let identity = IdentityTransport::new(
        &source_ids,
        &target_ids,
        BTreeMap::from([
            (
                kernel_types::EntityId::new(1),
                kernel_types::EntityId::new(11),
            ),
            (
                kernel_types::EntityId::new(2),
                kernel_types::EntityId::new(12),
            ),
        ]),
    )
    .unwrap();

    let mut old = FiniteModel::default();
    old.relations.insert(
        relation,
        vec![vec![Value::LiveEntityRef {
            entity_type: person,
            id: kernel_types::EntityId::new(1),
        }]],
    );
    let mut next = FiniteModel::default();
    next.relations.insert(
        relation,
        vec![vec![Value::LiveEntityRef {
            entity_type: person,
            id: kernel_types::EntityId::new(2),
        }]],
    );
    let change = Change::Replace(next);
    let query = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        value: Value::LiveEntityRef {
            entity_type: person,
            id: kernel_types::EntityId::new(1),
        },
        equivalence: equality,
    };

    assert!(
        check_rel_impact_transport_law(&identity, &query, &old, &change, &context, &registry)
            .unwrap()
    );
}

#[test]
fn retention_domain_mapping_does_not_resurrect_non_live_ids() {
    use std::collections::{BTreeMap, BTreeSet};

    let source_ids = BTreeSet::from([
        kernel_types::EntityId::new(1),
        kernel_types::EntityId::new(2),
    ]);
    let target_ids = BTreeSet::from([
        kernel_types::EntityId::new(11),
        kernel_types::EntityId::new(12),
    ]);
    let identity = IdentityTransport::new(
        &source_ids,
        &target_ids,
        BTreeMap::from([
            (
                kernel_types::EntityId::new(1),
                kernel_types::EntityId::new(11),
            ),
            (
                kernel_types::EntityId::new(2),
                kernel_types::EntityId::new(12),
            ),
        ]),
    )
    .unwrap();
    let mut state = DatabaseState::default();
    state
        .lifecycle
        .entities
        .insert(kernel_types::EntityId::new(1));
    state.lifecycle.roots.insert(kernel_types::EntityId::new(1));

    let transported = transport_database_state(&identity, &state).unwrap();
    assert_eq!(
        transported.lifecycle.entities,
        BTreeSet::from([kernel_types::EntityId::new(11)])
    );
    assert!(
        !transported
            .lifecycle
            .entities
            .contains(&kernel_types::EntityId::new(12))
    );
}
#[test]
fn historical_ids_must_be_covered_by_identity_retention_domain() {
    use std::collections::{BTreeMap, BTreeSet};

    let entity_type = SemanticId::new(9950);
    let identity = IdentityTransport::new(
        &BTreeSet::from([kernel_types::EntityId::new(1)]),
        &BTreeSet::from([kernel_types::EntityId::new(11)]),
        BTreeMap::from([(
            kernel_types::EntityId::new(1),
            kernel_types::EntityId::new(11),
        )]),
    )
    .unwrap();
    let historical = kernel_model::Value::HistoricalEntityId {
        entity_type,
        id: kernel_types::EntityId::new(2),
    };
    assert_eq!(
        transport_value(&identity, &historical),
        Err(TransportError::IdentitySourceCoverageMismatch)
    );
}
#[test]
fn semantic_environment_transport_is_generic_across_tokenizer_modules() {
    let tokenizer = SemanticId::new(9960);
    let mut registry = SemanticRegistry::default();
    let v1 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 1);
    let v2 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 2);
    let changed = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);
    let make_context = |revision, digest| {
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
        environment.pin_module(tokenizer, digest);
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(9960)),
            environment,
        }
    };
    let source = make_context(1, v1);
    let same_contract = make_context(2, v2);
    let new_contract = make_context(3, changed);

    assert!(
        EquivalentSemanticEnvironmentTransport::verify(&source, &same_contract, &registry).is_ok()
    );
    assert_eq!(
        EquivalentSemanticEnvironmentTransport::verify(&source, &new_contract, &registry),
        Err(TransportError::SemanticContractChanged(tokenizer))
    );
    assert!(SemanticLawMigration::verify(&source, &new_contract, &registry).is_ok());
}

#[test]
fn semantic_environment_transport_is_generic_across_ordering_modules() {
    let ordering = SemanticId::new(9970);
    let mut registry = SemanticRegistry::default();
    let v1 = registry.install_ordering_revision(OrderingModule::TextBinary, 1);
    let v2 = registry.install_ordering_revision(OrderingModule::TextBinary, 2);
    let changed = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitiveThenBinary);

    let context_with = |revision, digest| {
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
        environment.pin_module(ordering, digest);
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(9970)),
            environment,
        }
    };
    let source = context_with(1, v1);
    let upgraded = context_with(2, v2);
    let changed_law = context_with(3, changed);

    EquivalentSemanticEnvironmentTransport::verify(&source, &upgraded, &registry).unwrap();
    assert_eq!(
        EquivalentSemanticEnvironmentTransport::verify(&source, &changed_law, &registry),
        Err(TransportError::SemanticContractChanged(ordering))
    );
}

#[test]
fn schema_migration_transport_supports_merge_split_create_and_drop_in_one_verified_step() {
    use std::collections::BTreeSet;

    let entity_type = SemanticId::new(20_000);
    let old_left = SemanticId::new(20_001);
    let old_right = SemanticId::new(20_002);
    let old_dropped = SemanticId::new(20_006);
    let new_sum = SemanticId::new(20_003);
    let new_copy = SemanticId::new(20_004);
    let new_default = SemanticId::new(20_005);
    let entity = kernel_types::EntityId::new(7);
    let registry = SemanticRegistry::default();

    let mut source_schema = Schema::new(SchemaRevisionId::new(200));
    for field in [old_left, old_right, old_dropped] {
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
        environment: SemanticEnvironment::new(SemanticEnvId::new(200)),
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(201));
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
        environment: SemanticEnvironment::new(SemanticEnvId::new(201)),
    };

    let pick = |field| {
        ExactQuery::new(kernel_query::Expr::ProductField {
            input: Box::new(kernel_query::Expr::Input),
            field,
        })
    };
    let sum = ExactQuery::new(kernel_query::Expr::AddI64(
        Box::new(kernel_query::Expr::ProductField {
            input: Box::new(kernel_query::Expr::Input),
            field: old_left,
        }),
        Box::new(kernel_query::Expr::ProductField {
            input: Box::new(kernel_query::Expr::Input),
            field: old_right,
        }),
    ));
    let default = ExactQuery::new(kernel_query::Expr::TypedConst {
        value: kernel_model::Value::I64(99),
        ty: TypeExpr::Scalar(ScalarType::I64),
    });
    let migration = SchemaMigrationTransport::verify(
        &source,
        &target,
        &registry,
        vec![
            MigrationFieldRewrite {
                source_fields: vec![old_left, old_right],
                target_field: new_sum,
                transform: sum,
            },
            MigrationFieldRewrite {
                source_fields: vec![old_left],
                target_field: new_copy,
                transform: pick(old_left),
            },
            MigrationFieldRewrite {
                source_fields: vec![],
                target_field: new_default,
                transform: default,
            },
        ],
        vec![],
    )
    .unwrap();

    let mut state = DatabaseState::default();
    state.lifecycle.entities.insert(entity);
    state.lifecycle.roots.insert(entity);
    state
        .model
        .carriers
        .insert(entity_type, BTreeSet::from([entity]));
    state
        .model
        .fields
        .insert((old_left, entity), kernel_model::Value::I64(4));
    state
        .model
        .fields
        .insert((old_right, entity), kernel_model::Value::I64(6));
    state
        .model
        .fields
        .insert((old_dropped, entity), kernel_model::Value::I64(12));
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source,
        &registry,
        state,
    )
    .unwrap();
    assert_eq!(
        migration
            .transport_field_dependencies_exact(&BTreeSet::from([old_left]))
            .unwrap(),
        BTreeSet::from([new_sum, new_copy])
    );
    assert_eq!(
        migration
            .transport_field_coordinate_dependencies_exact(&BTreeSet::from([(old_left, entity)]))
            .unwrap(),
        BTreeSet::from([(new_sum, entity), (new_copy, entity)])
    );
    assert_eq!(
        migration.transport_field_dependencies_exact(&BTreeSet::from([old_dropped])),
        Err(TransportError::UnrepresentableSourceFieldDependency(
            old_dropped
        ))
    );
    let unknown_source_field = SemanticId::new(20_099);
    assert_eq!(
        migration.transport_field_dependencies_exact(&BTreeSet::from([unknown_source_field])),
        Err(TransportError::UnknownSourceField(unknown_source_field))
    );
    let (field_updates, implicit_dependencies) = migration
        .transport_field_updates_exact(
            source_revision.state(),
            &[(old_left, entity, Some(kernel_model::Value::I64(5)))],
        )
        .unwrap();
    assert!(field_updates.contains(&(new_sum, entity, Some(kernel_model::Value::I64(11)))));
    assert!(field_updates.contains(&(new_copy, entity, Some(kernel_model::Value::I64(5)))));
    assert_eq!(implicit_dependencies, BTreeSet::from([(old_right, entity)]));

    let migrated = migration
        .transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        )
        .unwrap();

    let fields = &migrated.state().model.fields;
    assert_eq!(
        fields.get(&(new_sum, entity)),
        Some(&kernel_model::Value::I64(10))
    );
    assert_eq!(
        fields.get(&(new_copy, entity)),
        Some(&kernel_model::Value::I64(4))
    );
    assert_eq!(
        fields.get(&(new_default, entity)),
        Some(&kernel_model::Value::I64(99))
    );
    assert!(!fields.contains_key(&(old_left, entity)));
    assert!(!fields.contains_key(&(old_right, entity)));
    assert_eq!(fields.len(), 3);
    assert_eq!(migrated.semantic_context(), &target);
}

#[test]
fn schema_migration_row_rewrite_changes_relation_column_type_without_host_callback() {
    let relation = SemanticId::new(21_000);
    let eq_i64 = SemanticId::new(21_001);
    let eq_f64 = SemanticId::new(21_002);
    let mut registry = SemanticRegistry::default();
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);

    let mut source_schema = Schema::new(SchemaRevisionId::new(210));
    source_schema
        .define_relation(kernel_schema::RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: vec![eq_i64],
            },
        })
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(210));
    source_environment.pin_module(eq_i64, i64_digest);
    source_environment.pin_module(eq_f64, f64_digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(211));
    target_schema
        .define_relation(kernel_schema::RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::F64)],
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: vec![eq_f64],
            },
        })
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(210));
    target_environment.pin_module(eq_i64, i64_digest);
    target_environment.pin_module(eq_f64, f64_digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let input_column = source.schema.relation_column_id(relation, 0).unwrap();
    let target_column = target.schema.relation_column_id(relation, 0).unwrap();
    let migration = SchemaMigrationTransport::verify(
        &source,
        &target,
        &registry,
        vec![],
        vec![MigrationRelationRewrite::Rows(MigrationRowRewrite {
            source_relation: relation,
            target_relation: relation,
            columns: vec![MigrationColumnRewrite {
                source_columns: vec![input_column],
                target_column,
                transform: ExactQuery::new(kernel_query::Expr::I64ToF64(Box::new(
                    kernel_query::Expr::ProductField {
                        input: Box::new(kernel_query::Expr::Input),
                        field: input_column,
                    },
                ))),
            }],
        })],
    )
    .unwrap();

    let source_type = RelExpr::Scan(relation)
        .typecheck(&source, &registry)
        .unwrap();
    let transported_delta = migration
        .transport_relation_delta_exact(
            relation,
            &RelationDelta {
                inserted: vec![vec![kernel_model::Value::I64(8)]],
                removed: vec![vec![kernel_model::Value::I64(7)]],
                result_type: source_type,
            },
            &registry,
        )
        .unwrap();
    assert_eq!(transported_delta.len(), 1);
    assert_eq!(
        transported_delta[0].1.inserted,
        vec![vec![kernel_model::Value::F64Bits(8.0_f64.to_bits())]]
    );
    assert_eq!(
        transported_delta[0].1.removed,
        vec![vec![kernel_model::Value::F64Bits(7.0_f64.to_bits())]]
    );

    let mut state = DatabaseState::default();
    state
        .model
        .relations
        .insert(relation, vec![vec![kernel_model::Value::I64(7)]]);
    let source_revision = kernel_revision::Revision::build(
        kernel_types::RevisionId::new(1),
        &source,
        &registry,
        state,
    )
    .unwrap();
    let migrated = migration
        .transport_revision(
            &source_revision,
            kernel_types::RevisionId::new(2),
            &registry,
        )
        .unwrap();
    assert_eq!(
        migrated
            .state()
            .model
            .relations
            .materialize_owned(&relation),
        Some(vec![vec![kernel_model::Value::F64Bits(7.0_f64.to_bits())]])
    );
}

#[test]
fn schema_migration_exposes_independent_row_local_physical_slice() {
    let relation = SemanticId::new(22_000);
    let eq_i64 = SemanticId::new(22_001);
    let eq_f64 = SemanticId::new(22_002);
    let mut registry = SemanticRegistry::default();
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let f64_digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);

    let mut source_schema = Schema::new(SchemaRevisionId::new(220));
    source_schema
        .define_relation(kernel_schema::RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: vec![eq_i64],
            },
        })
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(220));
    source_environment.pin_module(eq_i64, i64_digest);
    source_environment.pin_module(eq_f64, f64_digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(221));
    target_schema
        .define_relation(kernel_schema::RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::F64)],
            semantics: kernel_schema::RelationSemantics::Set {
                column_equivalences: vec![eq_f64],
            },
        })
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(220));
    target_environment.pin_module(eq_i64, i64_digest);
    target_environment.pin_module(eq_f64, f64_digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let input_column = source.schema.relation_column_id(relation, 0).unwrap();
    let target_column = target.schema.relation_column_id(relation, 0).unwrap();
    let migration = SchemaMigrationTransport::verify(
        &source,
        &target,
        &registry,
        vec![],
        vec![MigrationRelationRewrite::Rows(MigrationRowRewrite {
            source_relation: relation,
            target_relation: relation,
            columns: vec![MigrationColumnRewrite {
                source_columns: vec![input_column],
                target_column,
                transform: ExactQuery::new(kernel_query::Expr::I64ToF64(Box::new(
                    kernel_query::Expr::ProductField {
                        input: Box::new(kernel_query::Expr::Input),
                        field: input_column,
                    },
                ))),
            }],
        })],
    )
    .unwrap();

    assert_eq!(
        migration.relation_slice(relation),
        Some(MigrationRelationSlice::RowLocal {
            source_relation: relation,
            target_relation: relation,
        })
    );

    let mut state = DatabaseState::default();
    state.model.relations.insert(
        relation,
        vec![
            vec![kernel_model::Value::I64(7)],
            vec![kernel_model::Value::I64(11)],
        ],
    );
    assert_eq!(
        migration
            .materialize_relation_slice(&state, relation, &registry)
            .unwrap(),
        vec![
            vec![kernel_model::Value::F64Bits(7.0_f64.to_bits())],
            vec![kernel_model::Value::F64Bits(11.0_f64.to_bits())],
        ]
    );
    assert_eq!(
        migration
            .transform_row_local_slice(relation, &vec![kernel_model::Value::I64(13)])
            .unwrap(),
        vec![kernel_model::Value::F64Bits(13.0_f64.to_bits())]
    );
}

#[test]
fn schema_migration_query_slice_declares_exact_source_dependency_set() {
    let left = SemanticId::new(23_000);
    let right = SemanticId::new(23_001);
    let target_relation = SemanticId::new(23_002);
    let eq = SemanticId::new(23_003);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);

    let relation_def = |id| kernel_schema::RelationDef {
        id,
        columns: vec![TypeExpr::Scalar(ScalarType::I64)],
        semantics: kernel_schema::RelationSemantics::Set {
            column_equivalences: vec![eq],
        },
    };

    let mut source_schema = Schema::new(SchemaRevisionId::new(230));
    source_schema.define_relation(relation_def(left)).unwrap();
    source_schema.define_relation(relation_def(right)).unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(230));
    source_environment.pin_module(eq, digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(231));
    target_schema.define_relation(relation_def(left)).unwrap();
    target_schema.define_relation(relation_def(right)).unwrap();
    target_schema
        .define_relation(relation_def(target_relation))
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(230));
    target_environment.pin_module(eq, digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let migration = SchemaMigrationTransport::verify(
        &source,
        &target,
        &registry,
        vec![],
        vec![MigrationRelationRewrite::Query(RelationRewrite {
            target_relation,
            transform: RelExpr::Union {
                left: Box::new(RelExpr::Scan(left)),
                right: Box::new(RelExpr::Scan(right)),
            },
        })],
    )
    .unwrap();

    assert_eq!(
        migration.relation_slice(target_relation),
        Some(MigrationRelationSlice::Query {
            target_relation,
            source_relations: BTreeSet::from([left, right]),
        })
    );

    let mut state = DatabaseState::default();
    state
        .model
        .relations
        .insert(left, vec![vec![kernel_model::Value::I64(1)]]);
    state.model.relations.insert(
        right,
        vec![
            vec![kernel_model::Value::I64(2)],
            vec![kernel_model::Value::I64(3)],
        ],
    );
    assert_eq!(
        migration
            .materialize_relation_slice(&state, target_relation, &registry)
            .unwrap(),
        vec![
            vec![kernel_model::Value::I64(1)],
            vec![kernel_model::Value::I64(2)],
            vec![kernel_model::Value::I64(3)],
        ]
    );
    assert_eq!(
        migration.transform_row_local_slice(target_relation, &vec![kernel_model::Value::I64(1)]),
        Err(TransportError::MigrationSliceNotRowLocal(target_relation))
    );
}

#[test]
fn mixed_migration_source_retention_frontier_is_dependency_exact_and_monotone() {
    let source_a = SemanticId::new(24_000);
    let source_b = SemanticId::new(24_001);
    let target_ab = SemanticId::new(24_002);
    let target_b = SemanticId::new(24_003);
    let eq = SemanticId::new(24_004);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let relation_def = |id| kernel_schema::RelationDef {
        id,
        columns: vec![TypeExpr::Scalar(ScalarType::I64)],
        semantics: kernel_schema::RelationSemantics::Set {
            column_equivalences: vec![eq],
        },
    };

    let mut source_schema = Schema::new(SchemaRevisionId::new(240));
    source_schema
        .define_relation(relation_def(source_a))
        .unwrap();
    source_schema
        .define_relation(relation_def(source_b))
        .unwrap();
    let mut source_environment = SemanticEnvironment::new(SemanticEnvId::new(240));
    source_environment.pin_module(eq, digest);
    let source = SemanticContext {
        schema: source_schema,
        environment: source_environment,
    };

    let mut target_schema = Schema::new(SchemaRevisionId::new(241));
    target_schema
        .define_relation(relation_def(source_a))
        .unwrap();
    target_schema
        .define_relation(relation_def(source_b))
        .unwrap();
    target_schema
        .define_relation(relation_def(target_ab))
        .unwrap();
    target_schema
        .define_relation(relation_def(target_b))
        .unwrap();
    let mut target_environment = SemanticEnvironment::new(SemanticEnvId::new(240));
    target_environment.pin_module(eq, digest);
    let target = SemanticContext {
        schema: target_schema,
        environment: target_environment,
    };

    let migration = SchemaMigrationTransport::verify(
        &source,
        &target,
        &registry,
        vec![],
        vec![
            MigrationRelationRewrite::Query(RelationRewrite {
                target_relation: target_ab,
                transform: RelExpr::Union {
                    left: Box::new(RelExpr::Scan(source_a)),
                    right: Box::new(RelExpr::Scan(source_b)),
                },
            }),
            MigrationRelationRewrite::Query(RelationRewrite {
                target_relation: target_b,
                transform: RelExpr::Scan(source_b),
            }),
        ],
    )
    .unwrap();

    assert_eq!(
        migration
            .required_source_relations(&BTreeSet::new())
            .unwrap(),
        BTreeSet::from([source_a, source_b])
    );
    assert_eq!(
        migration
            .required_source_relations(&BTreeSet::from([source_a, source_b, target_ab]))
            .unwrap(),
        BTreeSet::from([source_b])
    );
    assert_eq!(
        migration
            .required_source_relations(&BTreeSet::from([source_a, source_b, target_ab, target_b,]))
            .unwrap(),
        BTreeSet::new()
    );
}
