mod error;
mod extents;
mod rules;
mod state;
mod violation;

pub use error::*;
pub use extents::*;
pub use rules::*;
pub use state::*;
pub use violation::*;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kernel_model::{DatabaseState, FiniteModel, Value};
    use kernel_schema::{RelationSemantics, ScalarType, SemanticContext, TypeExpr};
    use kernel_semantics::SemanticError;
    use kernel_types::{EntityId, SemanticId};

    use crate::state::validate_value;
    use kernel_schema::{CapabilityDef, FieldDef, RelationDef, Schema, SemanticEnvironment};
    use kernel_semantics::{EquivalenceModule, SemanticRegistry};
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    use super::*;

    fn id(raw: u128) -> EntityId {
        EntityId::new(raw)
    }

    pub(super) fn fixture() -> (SemanticContext, SemanticRegistry, DatabaseState) {
        let person = SemanticId::new(1);
        let name = SemanticId::new(2);
        let relation = SemanticId::new(3);
        let text_eq = SemanticId::new(4);
        let entity_eq = SemanticId::new(5);
        let set_eq = SemanticId::new(6);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let entity_digest =
            registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                set_eq,
                kernel_schema::StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        schema
            .define_field(FieldDef {
                id: name,
                owner: person,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::LiveEntityRef(person)),
                    TypeExpr::Set {
                        element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                        equivalence: text_eq,
                    },
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![entity_eq, set_eq],
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        environment.pin_module(entity_eq, entity_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(10));
        state.lifecycle.roots.insert(id(10));
        state
            .model
            .carriers
            .insert(person, BTreeSet::from([id(10)]));
        state
            .model
            .fields
            .insert((name, id(10)), Value::Text("Ada".into()));
        state.model.relations.insert(
            relation,
            vec![vec![
                Value::LiveEntityRef {
                    entity_type: person,
                    id: id(10),
                },
                Value::Set {
                    equivalence: text_eq,
                    elements: vec![Value::Text("db".into())],
                },
            ]],
        );
        (context, registry, state)
    }

    #[test]
    fn well_typed_model_validates() {
        let (context, registry, state) = fixture();
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        assert!(
            dynamic_violation_measure(&context, &registry, &state, &extents)
                .unwrap()
                .is_zero()
        );
    }

    #[test]
    fn relation_cardinality_model_rule_lowers_through_query_and_has_exact_vmf_mass() {
        let (mut context, registry, mut state) = fixture();
        let relation = SemanticId::new(3);
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationCardinality {
                relation,
                min: 2,
                max: Some(3),
            })
            .unwrap();

        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::ModelRuleViolation { rule_index: 0 }),
        );
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        let measure = dynamic_violation_measure(&context, &registry, &state, &extents).unwrap();
        assert_eq!(
            measure.mass(&DynamicViolationWitness::ModelRule { rule_index: 0 }),
            1,
        );
        let oracle = kernel_query::RelExpr::Scan(relation)
            .evaluate(&state.model, &context, &registry)
            .unwrap();
        assert_eq!(
            oracle.rows().len(),
            1,
            "specialized cardinality must agree with query oracle"
        );
        assert_eq!(
            validate_relations_with_extents(
                &context,
                &registry,
                &state,
                &extents,
                &BTreeSet::from([relation]),
            ),
            Err(ValidationError::ModelRuleViolation { rule_index: 0 }),
        );
        assert_eq!(
            relation_dynamic_violation_measure(&context, &registry, &state, &extents, relation)
                .unwrap()
                .mass(&DynamicViolationWitness::ModelRule { rule_index: 0 }),
            1,
        );

        let second = state.model.relations.get(&relation).unwrap()[0].clone();
        state
            .model
            .relations
            .get_mut(&relation)
            .unwrap()
            .push(second);
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        assert!(
            dynamic_violation_measure(&context, &registry, &state, &extents)
                .unwrap()
                .is_zero()
        );
    }

    #[test]
    fn quantified_model_rules_have_exact_row_predicates_and_dependency_columns() {
        let (mut context, registry, mut state) = fixture();
        let relation = SemanticId::new(700);
        let text_eq = SemanticId::new(4);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        state
            .model
            .relations
            .insert(relation, vec![vec![Value::Text("db".into())]]);
        let text_column = context.schema.relation_column_id(relation, 0).unwrap();
        let predicate = kernel_schema::SemanticRuleExpr::TextLength {
            value: kernel_schema::RuleValueExpr::Field(text_column),
            min: 2,
            max: None,
        };
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationExists {
                relation,
                predicate: predicate.clone(),
            })
            .unwrap();
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationAll {
                relation,
                predicate,
            })
            .unwrap();

        let plan = crate::CompiledRulePlan::compile(&context);
        assert_eq!(plan.model_rules()[0].dependency().relation(), relation);
        assert_eq!(
            plan.model_rules()[0].dependency().columns(),
            &BTreeSet::from([text_column])
        );
        assert_eq!(
            plan.model_rules()[1].dependency().columns(),
            &BTreeSet::from([text_column])
        );
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));

        state.model.relations.get_mut(&relation).unwrap()[0][0] = Value::Text("x".into());
        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::ModelRuleViolation { rule_index: 0 }),
        );
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        let measure = dynamic_violation_measure(&context, &registry, &state, &extents).unwrap();
        assert_eq!(
            measure.mass(&DynamicViolationWitness::ModelRule { rule_index: 0 }),
            1
        );
        assert_eq!(
            measure.mass(&DynamicViolationWitness::ModelRule { rule_index: 1 }),
            1
        );
        assert_eq!(
            validate_relations_with_extents(
                &context,
                &registry,
                &state,
                &extents,
                &BTreeSet::from([SemanticId::new(3)]),
            ),
            Ok(()),
            "unrelated relation changes must not evaluate the quantified rule",
        );
        let relation_measure =
            relation_dynamic_violation_measure(&context, &registry, &state, &extents, relation)
                .unwrap();
        assert_eq!(
            relation_measure.mass(&DynamicViolationWitness::ModelRule { rule_index: 0 }),
            1
        );
        assert_eq!(
            relation_measure.mass(&DynamicViolationWitness::ModelRule { rule_index: 1 }),
            1
        );
    }

    #[test]
    fn same_relation_field_footprint_skips_unrelated_model_rules() {
        let (mut context, registry, mut state) = fixture();
        let relation = SemanticId::new(704);
        let text_eq = SemanticId::new(4);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::Text),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, text_eq],
                },
            })
            .unwrap();
        state.model.relations.insert(
            relation,
            vec![vec![Value::Text("ok".into()), Value::Text("a".into())]],
        );
        let guarded = context.schema.relation_column_id(relation, 0).unwrap();
        let unrelated = context.schema.relation_column_id(relation, 1).unwrap();
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationAll {
                relation,
                predicate: kernel_schema::SemanticRuleExpr::TextLength {
                    value: kernel_schema::RuleValueExpr::Field(guarded),
                    min: 2,
                    max: None,
                },
            })
            .unwrap();
        let plan = crate::CompiledRulePlan::compile(&context);
        assert_eq!(
            plan.model_rules_for_mutation(
                relation,
                &crate::RelationMutationFootprint::fields([unrelated]),
            )
            .count(),
            0,
        );
        assert_eq!(
            plan.model_rules_for_mutation(
                relation,
                &crate::RelationMutationFootprint::fields([guarded]),
            )
            .count(),
            1,
        );
        state.model.relations.get_mut(&relation).unwrap()[0][0] = Value::Text("x".into());
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        assert_eq!(
            validate_relations_with_extents_selective(
                &context,
                &registry,
                &state,
                &extents,
                &BTreeSet::from([relation]),
                &BTreeMap::from([(
                    relation,
                    crate::RelationMutationFootprint::fields([unrelated])
                )]),
            ),
            Ok(()),
        );
        assert_eq!(
            validate_relations_with_extents_selective(
                &context,
                &registry,
                &state,
                &extents,
                &BTreeSet::from([relation]),
                &BTreeMap::from([(
                    relation,
                    crate::RelationMutationFootprint::fields([guarded])
                )]),
            ),
            Err(ValidationError::ModelRuleViolation { rule_index: 0 }),
        );
    }

    #[test]
    #[ignore = "performance benchmark"]
    fn benchmark_relevant_model_rule_scan_scaling() {
        use std::time::Instant;

        for rows in [1_000usize, 100_000, 1_000_000] {
            let (mut context, _registry, mut state) = fixture();
            let relation = SemanticId::new(705);
            let text_eq = SemanticId::new(4);
            context
                .schema
                .define_relation(kernel_schema::RelationDef {
                    id: relation,
                    columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                    semantics: RelationSemantics::Bag {
                        column_equivalences: vec![text_eq],
                    },
                })
                .unwrap();
            state.model.relations.insert(
                relation,
                (0..rows).map(|_| vec![Value::Text("ok".into())]).collect(),
            );
            let column = context.schema.relation_column_id(relation, 0).unwrap();
            context
                .schema
                .add_model_rule(kernel_schema::ModelRuleExpr::RelationAll {
                    relation,
                    predicate: kernel_schema::SemanticRuleExpr::TextLength {
                        value: kernel_schema::RuleValueExpr::Field(column),
                        min: 2,
                        max: Some(2),
                    },
                })
                .unwrap();
            let plan = crate::CompiledRulePlan::compile(&context);
            let rule = &plan.model_rules()[0];
            let started = Instant::now();
            assert!(rule.is_satisfied(&state).unwrap());
            let relevant = started.elapsed();
            let started = Instant::now();
            assert_eq!(
                plan.model_rules_for_mutation(
                    relation,
                    &crate::RelationMutationFootprint::fields([SemanticId::new(999_999)]),
                )
                .count(),
                0,
            );
            let skipped = started.elapsed();
            eprintln!(
                "MODEL_RULE_SCAN rows={rows} relevant_ns={} skipped_ns={}",
                relevant.as_nanos(),
                skipped.as_nanos()
            );
        }
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_maintained_model_rule_witness_delta_against_full_scan() {
        use std::time::Instant;

        let rows = 1_000_000usize;
        let (mut context, _registry, mut state) = fixture();
        let relation = SemanticId::new(706);
        let text_eq = SemanticId::new(4);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        state.model.relations.insert(
            relation,
            (0..rows).map(|_| vec![Value::Text("ok".into())]).collect(),
        );
        let column = context.schema.relation_column_id(relation, 0).unwrap();
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationAll {
                relation,
                predicate: kernel_schema::SemanticRuleExpr::TextLength {
                    value: kernel_schema::RuleValueExpr::Field(column),
                    min: 2,
                    max: Some(2),
                },
            })
            .unwrap();
        let witnesses = crate::ModelRuleWitnessState::build(&context, &state).unwrap();
        let removed = vec![Value::Text("ok".into())];
        let inserted = vec![Value::Text("x".into())];
        let footprint = crate::RelationMutationFootprint::fields([column]);

        let mut target = state.clone();
        let target_rows = target.model.relations.get_mut(&relation).unwrap();
        target_rows.pop();
        target_rows.push(inserted.clone());
        let plan = crate::CompiledRulePlan::compile(&context);
        let started = Instant::now();
        assert_eq!(plan.model_rules()[0].violation_mass(&target).unwrap(), 1);
        let full_scan = started.elapsed();

        let started = Instant::now();
        let next = witnesses
            .apply_relation_delta(&context, relation, &footprint, &[removed], &[inserted])
            .unwrap();
        assert_eq!(next.violation_mass(&context, 0).unwrap(), 1);
        let maintained = started.elapsed();
        eprintln!(
            "MODEL_RULE_WITNESS rows={rows} full_scan_ns={} maintained_delta_ns={} ratio={:.2}",
            full_scan.as_nanos(),
            maintained.as_nanos(),
            full_scan.as_secs_f64() / maintained.as_secs_f64(),
        );
    }

    #[test]
    fn exists_all_specialization_agrees_with_relational_filter_oracle() {
        let (mut context, registry, mut state) = fixture();
        let relation = SemanticId::new(701);
        let text_eq = SemanticId::new(4);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        state.model.relations.insert(
            relation,
            vec![
                vec![Value::Text("ok".into())],
                vec![Value::Text("bad".into())],
            ],
        );
        let column = context.schema.relation_column_id(relation, 0).unwrap();
        let predicate = kernel_schema::SemanticRuleExpr::TextOneOf {
            value: kernel_schema::RuleValueExpr::Field(column),
            allowed: BTreeSet::from(["ok".to_owned()]),
        };
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationExists {
                relation,
                predicate: predicate.clone(),
            })
            .unwrap();
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationAll {
                relation,
                predicate,
            })
            .unwrap();

        let plan = crate::CompiledRulePlan::compile(&context);
        assert!(plan.model_rules()[0].is_satisfied(&state).unwrap());
        assert!(!plan.model_rules()[1].is_satisfied(&state).unwrap());
        assert_eq!(plan.model_rules()[1].violation_mass(&state).unwrap(), 1);

        let oracle = kernel_query::RelExpr::FilterEqConst {
            input: Box::new(kernel_query::RelExpr::Scan(relation)),
            column: 0,
            value: Value::Text("ok".into()),
            equivalence: text_eq,
        }
        .evaluate(&state.model, &context, &registry)
        .unwrap();
        assert_eq!(oracle.rows().len(), 1);
        assert_eq!(
            oracle.rows().len(),
            state
                .model
                .relations
                .get(&relation)
                .unwrap()
                .iter()
                .filter(|row| row[0] == Value::Text("ok".into()))
                .count()
        );
    }

    #[test]
    fn exact_f64_sum_model_rule_uses_exact_aggregate_and_column_dependency() {
        let (mut context, mut registry, mut state) = fixture();
        let relation = SemanticId::new(702);
        let eq_f64 = SemanticId::new(703);
        let digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::F64Bitwise);
        context.environment.pin_module(eq_f64, digest);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::F64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq_f64],
                },
            })
            .unwrap();
        state.model.relations.insert(
            relation,
            vec![
                vec![Value::F64Bits(1.0e16_f64.to_bits())],
                vec![Value::F64Bits(1.0_f64.to_bits())],
                vec![Value::F64Bits((-1.0e16_f64).to_bits())],
            ],
        );
        let column = context.schema.relation_column_id(relation, 0).unwrap();
        let one = kernel_schema::FiniteF64::new(1.0).unwrap();
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationExactF64SumRange {
                relation,
                column,
                min: Some(one),
                max: Some(one),
            })
            .unwrap();
        let plan = crate::CompiledRulePlan::compile(&context);
        let rule = &plan.model_rules()[0];
        assert_eq!(rule.dependency().columns(), &BTreeSet::from([column]));
        assert!(rule.is_satisfied(&state).unwrap());
        assert_eq!(rule.violation_mass(&state).unwrap(), 0);
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
        state
            .model
            .relations
            .get_mut(&relation)
            .unwrap()
            .push(vec![Value::F64Bits(0.5_f64.to_bits())]);
        assert!(!rule.is_satisfied(&state).unwrap());
        assert_eq!(rule.violation_mass(&state).unwrap(), 1);
    }

    #[test]
    fn maintained_model_rule_witnesses_update_exactly_from_relation_delta() {
        let (mut context, mut registry, mut state) = fixture();
        let relation = SemanticId::new(710);
        let text_eq = SemanticId::new(4);
        let f64_eq = SemanticId::new(711);
        let f64_digest =
            registry.install_equivalence(kernel_semantics::EquivalenceModule::F64Bitwise);
        context.environment.pin_module(f64_eq, f64_digest);
        context
            .schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::F64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, f64_eq],
                },
            })
            .unwrap();
        let text_column = context.schema.relation_column_id(relation, 0).unwrap();
        let sum_column = context.schema.relation_column_id(relation, 1).unwrap();
        let predicate = kernel_schema::SemanticRuleExpr::TextOneOf {
            value: kernel_schema::RuleValueExpr::Field(text_column),
            allowed: BTreeSet::from(["ok".to_owned()]),
        };
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationExists {
                relation,
                predicate: predicate.clone(),
            })
            .unwrap();
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationAll {
                relation,
                predicate,
            })
            .unwrap();
        context
            .schema
            .add_model_rule(kernel_schema::ModelRuleExpr::RelationExactF64SumRange {
                relation,
                column: sum_column,
                min: Some(kernel_schema::FiniteF64::new(3.0).unwrap()),
                max: Some(kernel_schema::FiniteF64::new(3.0).unwrap()),
            })
            .unwrap();
        state.model.relations.insert(
            relation,
            vec![
                vec![Value::Text("ok".into()), Value::F64Bits(1.0_f64.to_bits())],
                vec![Value::Text("bad".into()), Value::F64Bits(2.0_f64.to_bits())],
            ],
        );

        let witnesses = crate::ModelRuleWitnessState::build(&context, &state).unwrap();
        assert_eq!(witnesses.violation_mass(&context, 0).unwrap(), 0);
        assert_eq!(witnesses.violation_mass(&context, 1).unwrap(), 1);
        assert_eq!(witnesses.violation_mass(&context, 2).unwrap(), 0);

        let removed = vec![vec![
            Value::Text("bad".into()),
            Value::F64Bits(2.0_f64.to_bits()),
        ]];
        let inserted = vec![vec![
            Value::Text("ok".into()),
            Value::F64Bits(2.0_f64.to_bits()),
        ]];
        let footprint = crate::RelationMutationFootprint::fields([text_column]);
        let next = witnesses
            .apply_relation_delta(&context, relation, &footprint, &removed, &inserted)
            .unwrap();
        assert_eq!(next.violation_mass(&context, 0).unwrap(), 0);
        assert_eq!(next.violation_mass(&context, 1).unwrap(), 0);
        assert_eq!(next.violation_mass(&context, 2).unwrap(), 0);

        let mut target = state.clone();
        target.model.relations.insert(
            relation,
            vec![
                vec![Value::Text("ok".into()), Value::F64Bits(1.0_f64.to_bits())],
                vec![Value::Text("ok".into()), Value::F64Bits(2.0_f64.to_bits())],
            ],
        );
        let rebuilt = crate::ModelRuleWitnessState::build(&context, &target).unwrap();
        assert_eq!(next, rebuilt);
    }

    #[test]
    fn dynamic_violation_measure_exposes_missing_live_reference_witness() {
        let (context, registry, mut state) = fixture();
        let relation = SemanticId::new(3);
        let person = SemanticId::new(1);
        let missing = id(999);
        state.model.relations.get_mut(&relation).unwrap()[0][0] = Value::LiveEntityRef {
            entity_type: person,
            id: missing,
        };
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        let measure = dynamic_violation_measure(&context, &registry, &state, &extents).unwrap();
        let relation_measure =
            relation_dynamic_violation_measure(&context, &registry, &state, &extents, relation)
                .unwrap();
        assert_eq!(relation_measure, measure);
        assert_eq!(measure.witness_count(), 1);
        assert_eq!(
            measure.mass(&DynamicViolationWitness::MissingLiveReference {
                location: DynamicViolationLocation::RelationCell {
                    relation,
                    row: 0,
                    column: 0,
                },
                target_type: person,
                target: missing,
            }),
            1
        );
    }

    #[test]
    fn wrong_field_type_is_rejected() {
        let (context, registry, mut state) = fixture();
        state
            .model
            .fields
            .insert((SemanticId::new(2), id(10)), Value::I64(99));
        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::TypeMismatch)
        );
    }

    #[test]
    fn product_shape_is_keyed_by_semantic_field_id() {
        let product_type = TypeExpr::Product(BTreeMap::from([
            (SemanticId::new(20), TypeExpr::Scalar(ScalarType::I64)),
            (SemanticId::new(21), TypeExpr::Scalar(ScalarType::Text)),
        ]));
        let value = Value::Product(BTreeMap::from([
            (SemanticId::new(20), Value::I64(1)),
            (SemanticId::new(22), Value::Text("wrong field".into())),
        ]));
        let (context, registry, state) = fixture();
        let entity_types = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        assert_eq!(
            validate_value(
                &value,
                &product_type,
                &state.model,
                &context,
                &registry,
                &entity_types,
                &BTreeMap::new()
            ),
            Err(ValidationError::ProductShapeMismatch)
        );
    }
    #[test]
    fn equality_domain_is_checked_even_for_singleton_collection() {
        let set_field = SemanticId::new(50);
        let text_eq = SemanticId::new(51);
        let mut registry = SemanticRegistry::default();
        let wrong_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_field(FieldDef {
                id: set_field,
                owner: SemanticId::new(1),
                value: TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: text_eq,
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, wrong_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(10));
        state.lifecycle.roots.insert(id(10));
        state
            .model
            .carriers
            .insert(SemanticId::new(1), BTreeSet::from([id(10)]));
        state.model.fields.insert(
            (set_field, id(10)),
            Value::Set {
                equivalence: text_eq,
                elements: vec![Value::Text("only".into())],
            },
        );
        assert!(matches!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::Semantic(
                SemanticError::EquivalenceDomainMismatch { .. }
            ))
        ));
    }

    #[test]
    fn set_relation_rejects_semantically_duplicate_rows() {
        let relation = SemanticId::new(60);
        let text_eq = SemanticId::new(61);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![text_eq],
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.model.relations.insert(
            relation,
            vec![
                vec![Value::Text("Alpha".into())],
                vec![Value::Text("alpha".into())],
            ],
        );
        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::Semantic(
                SemanticError::DuplicateRelationRow
            ))
        );

        let rows = state.model.relations.get(&relation).unwrap();
        let measure =
            relation_uniqueness_violation_measure(&rows.to_vec(), &[text_eq], &context, &registry)
                .unwrap();
        assert_eq!(measure.witness_count(), 1);
        assert_eq!(measure.iter().next().map(|(_, mass)| mass), Some(1));
    }

    #[test]
    fn relation_uniqueness_violation_measure_is_zero_for_distinct_gamma_classes() {
        let text_eq = SemanticId::new(62);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, digest);
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(1)),
            environment,
        };
        let rows = vec![
            vec![Value::Text("Alpha".into())],
            vec![Value::Text("Beta".into())],
        ];
        assert!(
            relation_uniqueness_violation_measure(&rows, &[text_eq], &context, &registry)
                .unwrap()
                .is_zero()
        );
    }
    #[test]
    fn unit_is_a_real_schema_type_not_an_untyped_runtime_sentinel() {
        let field = SemanticId::new(70);
        let owner_type = SemanticId::new(71);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_field(FieldDef {
                id: field,
                owner: owner_type,
                value: TypeExpr::Scalar(ScalarType::Unit),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        state
            .model
            .carriers
            .insert(owner_type, BTreeSet::from([id(1)]));
        state.model.fields.insert((field, id(1)), Value::Unit);
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
    }
    #[test]
    fn structural_product_equivalence_validates_composite_set_keys() {
        let owner_type = SemanticId::new(80);
        let field = SemanticId::new(81);
        let product_eq = SemanticId::new(82);
        let text_eq = SemanticId::new(83);
        let i64_eq = SemanticId::new(84);
        let name_field = SemanticId::new(85);
        let age_field = SemanticId::new(86);
        let product_type = TypeExpr::Product(BTreeMap::from([
            (name_field, TypeExpr::Scalar(ScalarType::Text)),
            (age_field, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                product_eq,
                kernel_schema::StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(name_field, text_eq), (age_field, i64_eq)]),
                },
            )
            .unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: owner_type,
                value: TypeExpr::Set {
                    element: Box::new(product_type),
                    equivalence: product_eq,
                },
            })
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(i64_eq, i64_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let mut state = DatabaseState::default();
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        state
            .model
            .carriers
            .insert(owner_type, BTreeSet::from([id(1)]));
        let key = |name: &str| {
            Value::Product(BTreeMap::from([
                (name_field, Value::Text(name.into())),
                (age_field, Value::I64(30)),
            ]))
        };
        state.model.fields.insert(
            (field, id(1)),
            Value::Set {
                equivalence: product_eq,
                elements: vec![key("ALICE"), key("alice")],
            },
        );
        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::Semantic(
                SemanticError::DuplicateSetElement {
                    equivalence: product_eq
                }
            ))
        );
    }
    #[test]
    fn empty_set_relation_still_rejects_wrong_column_equivalence_domain() {
        let relation = SemanticId::new(950);
        let wrong_eq = SemanticId::new(951);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(kernel_semantics::EquivalenceModule::I64Exact);
        let mut environment = kernel_schema::SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(wrong_eq, digest);
        let mut schema = kernel_schema::Schema::new(SchemaRevisionId::new(1));
        schema
            .define_relation(kernel_schema::RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::Text)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![wrong_eq],
                },
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        assert!(matches!(
            registry.validate_context(&context),
            Err(SemanticError::EquivalenceDomainMismatch { .. })
        ));
    }

    #[test]
    fn dense_type_extents_match_subtype_membership_and_overlap() {
        let concrete = SemanticId::new(980);
        let secondary = SemanticId::new(981);
        let parent = SemanticId::new(982);
        let unrelated = SemanticId::new(983);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema.include(concrete, parent).unwrap();

        let mut model = FiniteModel::default();
        model
            .carriers
            .insert(concrete, BTreeSet::from([id(1), id(2)]));
        model
            .carriers
            .insert(secondary, BTreeSet::from([id(2), id(3)]));
        model.carriers.insert(unrelated, BTreeSet::from([id(4)]));

        let extents = DenseTypeExtents::compile(&model, &schema).unwrap();
        assert_eq!(extents.entity_count(), 4);
        assert!(extents.contains(id(1), concrete));
        assert!(extents.contains(id(1), parent));
        assert!(extents.contains(id(2), concrete));
        assert!(extents.contains(id(2), secondary));
        assert!(extents.contains(id(3), secondary));
        assert!(!extents.contains(id(3), parent));
        assert!(extents.contains(id(4), unrelated));
        assert!(!extents.contains(id(4), parent));
    }

    #[test]
    fn validation_accepts_subtype_owner_through_dense_extent() {
        let concrete = SemanticId::new(990);
        let parent = SemanticId::new(991);
        let field = SemanticId::new(992);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema.include(concrete, parent).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: parent,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1)]));
        state.model.fields.insert((field, id(1)), Value::I64(7));
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
    }

    #[test]
    fn capability_required_fields_are_enforced_for_every_member() {
        let concrete = SemanticId::new(1_100);
        let capability = SemanticId::new(1_101);
        let field = SemanticId::new(1_102);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability,
                required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema.include(concrete, capability).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: capability,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1), id(2)]));
        state.lifecycle.entities.extend([id(1), id(2)]);
        state.lifecycle.roots.extend([id(1), id(2)]);
        state
            .model
            .fields
            .insert((field, id(1)), Value::Text("present".into()));

        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::MissingCapabilityRequiredField {
                capability,
                field,
                entity: id(2),
            })
        );

        state
            .model
            .fields
            .insert((field, id(2)), Value::Text("present too".into()));
        assert_eq!(validate_state(&context, &registry, &state), Ok(()));
    }

    #[test]
    fn capability_violation_measure_counts_overlapping_member_once() {
        let concrete = SemanticId::new(1_105);
        let secondary = SemanticId::new(1_106);
        let capability = SemanticId::new(1_107);
        let field = SemanticId::new(1_108);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability,
                required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema.include(concrete, capability).unwrap();
        schema.include(secondary, capability).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: capability,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1)]));
        state
            .model
            .carriers
            .insert(secondary, BTreeSet::from([id(1)]));
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();

        let measure = dynamic_violation_measure(&context, &registry, &state, &extents).unwrap();
        let witness = DynamicViolationWitness::MissingCapabilityRequiredField {
            capability,
            field,
            entity: id(1),
        };
        assert_eq!(measure.mass(&witness), 1);
    }

    #[test]
    fn capability_required_field_contract_must_match_schema_field() {
        let concrete = SemanticId::new(1_110);
        let capability = SemanticId::new(1_111);
        let field = SemanticId::new(1_112);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability,
                required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema.include(concrete, capability).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner: capability,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1)]));
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        state.model.fields.insert((field, id(1)), Value::I64(7));

        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::CapabilityRequiredFieldContractMismatch { capability, field })
        );
    }

    #[test]
    fn capability_required_field_owner_must_follow_from_capability_itself() {
        let concrete = SemanticId::new(1_120);
        let capability = SemanticId::new(1_121);
        let owner = SemanticId::new(1_122);
        let field = SemanticId::new(1_123);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability,
                required_fields: BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::I64))]),
            })
            .unwrap();
        schema.include(concrete, capability).unwrap();
        schema.include(concrete, owner).unwrap();
        schema
            .define_field(FieldDef {
                id: field,
                owner,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        let mut state = DatabaseState::default();
        state
            .model
            .carriers
            .insert(concrete, BTreeSet::from([id(1)]));
        state.lifecycle.entities.insert(id(1));
        state.lifecycle.roots.insert(id(1));
        state.model.fields.insert((field, id(1)), Value::I64(7));

        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::CapabilityRequiredFieldContractMismatch { capability, field })
        );
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_dense_type_extent_membership_against_carrier_scan() {
        use std::hint::black_box;
        use std::time::Instant;

        let parent = SemanticId::new(10_000);
        let carrier_count = 128_u128;
        let per_carrier = 256_u128;
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        let mut model = FiniteModel::default();
        let mut probes = Vec::new();
        for carrier_index in 0..carrier_count {
            let actual = SemanticId::new(20_000 + carrier_index);
            schema.include(actual, parent).unwrap();
            let start = carrier_index * per_carrier + 1;
            let entities = (start..start + per_carrier)
                .map(id)
                .collect::<BTreeSet<_>>();
            probes.extend(entities.iter().copied());
            model.carriers.insert(actual, entities);
        }
        let compile_start = Instant::now();
        let dense = DenseTypeExtents::compile(&model, &schema).unwrap();
        let compile_ns = compile_start.elapsed().as_nanos();

        let start = Instant::now();
        let mut baseline_hits = 0_usize;
        for _ in 0..4 {
            for &entity in &probes {
                let hit = model.carriers.iter().any(|(&actual, entities)| {
                    entities.contains(&entity) && schema.is_subtype(actual, parent)
                });
                baseline_hits += usize::from(black_box(hit));
            }
        }
        let baseline_ns = start.elapsed().as_nanos();

        let start = Instant::now();
        let mut dense_hits = 0_usize;
        for _ in 0..4 {
            for &entity in &probes {
                dense_hits += usize::from(black_box(dense.contains(entity, parent)));
            }
        }
        let dense_ns = start.elapsed().as_nanos();
        assert_eq!(baseline_hits, dense_hits);
        let ratio_milli = baseline_ns.saturating_mul(1_000) / dense_ns.max(1);
        println!(
            "compile_ns={compile_ns} baseline_ns={baseline_ns} dense_ns={dense_ns} ratio_milli={ratio_milli} probes={}",
            probes.len()
        );
    }
}

#[cfg(test)]
mod entity_rule_hostile_tests {
    use super::*;
    use kernel_schema::{RuleValueExpr, SemanticRuleExpr, TextPattern};
    use kernel_types::SemanticId;

    #[test]
    fn entity_rule_uses_stable_field_coordinate_in_validation_and_vmf() {
        let (mut context, registry, state) = super::tests::fixture();
        let person = SemanticId::new(1);
        let name = SemanticId::new(2);
        context
            .schema
            .add_entity_rule(
                person,
                SemanticRuleExpr::TextMatches {
                    value: RuleValueExpr::Field(name),
                    pattern: TextPattern::literal("Grace"),
                },
            )
            .unwrap();

        assert_eq!(
            validate_state(&context, &registry, &state),
            Err(ValidationError::EntityRuleViolation {
                owner: person,
                entity: kernel_types::EntityId::new(10),
                rule_index: 0,
            })
        );

        let extents = DenseTypeExtents::compile(&state.model, &context.schema).unwrap();
        let measure = dynamic_violation_measure(&context, &registry, &state, &extents).unwrap();
        assert_eq!(
            measure.mass(&DynamicViolationWitness::EntityRule {
                owner: person,
                entity: kernel_types::EntityId::new(10),
                rule_index: 0,
            }),
            1
        );
    }
}
