pub mod anchor_pullback;
mod canonical_key;
mod contracts;
mod deployment;
mod equivalence;
mod error;
mod implementation_descriptor;
mod module_digest;
pub mod observable;
mod ordering;
mod registry;
pub mod support_atom;
mod tokenizer;

pub use canonical_key::*;
pub use contracts::*;
pub use deployment::*;
pub use equivalence::*;
pub use error::*;
pub use implementation_descriptor::*;
pub use ordering::*;
pub use registry::*;
pub use tokenizer::TokenizerModule;

#[cfg(test)]
use canonical_key::{CANONICAL_EQ_KEY_MAGIC, MAX_CANONICAL_EQ_KEY_DEPTH};

#[cfg(test)]
mod tests {
    use std::cmp::Ordering as CmpOrdering;
    use std::collections::{BTreeMap, BTreeSet};

    use kernel_model::{FiniteModel, Value};
    use kernel_schema::{
        ModuleDigest, ScalarType, Schema, SemanticContext, SemanticEnvironment,
        StructuralEquivalenceDef, StructuralOrderingDef, TypeExpr,
    };
    use kernel_types::{EntityId, SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    fn context(eq: SemanticId, digest: ModuleDigest) -> SemanticContext {
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(eq, digest);
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(1)),
            environment,
        }
    }

    #[test]
    fn equality_is_resolved_through_pinned_digest_not_host_ord() {
        let eq = SemanticId::new(1);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let context = context(eq, digest);
        assert_eq!(
            registry.equivalent(
                &context,
                eq,
                &Value::Text("Alpha".into()),
                &Value::Text("alpha".into())
            ),
            Ok(true)
        );
    }

    #[test]
    fn bound_primitive_predicate_preserves_resolved_equivalence_contract() {
        let eq = SemanticId::new(11);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let context = context(eq, digest);
        let resolved = registry
            .resolve_primitive_equivalence(&context, eq)
            .unwrap()
            .expect("primitive equality resolves once");
        let right = Value::Text("Alpha".into());
        let bound = resolved.bind_right(&right).unwrap();
        for candidate in [
            Value::Text("ALPHA".into()),
            Value::Text("alpha".into()),
            Value::Text("Beta".into()),
        ] {
            assert_eq!(
                bound.matches(&candidate),
                resolved.equivalent(&candidate, &right).unwrap()
            );
        }
    }

    #[test]
    fn set_rejects_semantic_duplicates_even_when_rust_values_differ() {
        let eq = SemanticId::new(1);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let context = context(eq, digest);
        let owner = EntityId::new(1);
        let field = SemanticId::new(2);
        let mut model = FiniteModel::default();
        model
            .carriers
            .insert(SemanticId::new(3), BTreeSet::from([owner]));
        model.fields.insert(
            (field, owner),
            Value::Set {
                equivalence: eq,
                elements: vec![Value::Text("A".into()), Value::Text("a".into())],
            },
        );
        assert_eq!(
            registry.validate_model(&context, &model),
            Err(SemanticError::DuplicateSetElement { equivalence: eq })
        );
    }
    #[test]
    fn f64_key_equality_is_total_bitwise_not_ieee_partial_equality() {
        let eq = SemanticId::new(9);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::F64Bitwise);
        let context = context(eq, digest);
        let nan_a = Value::F64Bits(0x7ff8_0000_0000_0001);
        let nan_same = Value::F64Bits(0x7ff8_0000_0000_0001);
        let nan_other = Value::F64Bits(0x7ff8_0000_0000_0002);
        assert_eq!(
            registry.equivalent(&context, eq, &nan_a, &nan_same),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent(&context, eq, &nan_a, &nan_other),
            Ok(false)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                eq,
                &Value::F64Bits(0.0_f64.to_bits()),
                &Value::F64Bits((-0.0_f64).to_bits())
            ),
            Ok(false)
        );
    }
    #[test]
    fn builtin_equivalence_modules_satisfy_equivalence_laws_on_hostile_samples() {
        let cases = [
            (EquivalenceModule::UnitExact, vec![Value::Unit]),
            (
                EquivalenceModule::BoolExact,
                vec![Value::Bool(false), Value::Bool(true)],
            ),
            (
                EquivalenceModule::I64Exact,
                vec![Value::I64(-1), Value::I64(0), Value::I64(1)],
            ),
            (
                EquivalenceModule::F64Bitwise,
                vec![
                    Value::F64Bits(0.0_f64.to_bits()),
                    Value::F64Bits((-0.0_f64).to_bits()),
                    Value::F64Bits(0x7ff8_0000_0000_0001),
                    Value::F64Bits(0x7ff8_0000_0000_0002),
                ],
            ),
            (
                EquivalenceModule::TextExact,
                vec![Value::Text("A".into()), Value::Text("a".into())],
            ),
            (
                EquivalenceModule::TextAsciiCaseInsensitive,
                vec![
                    Value::Text("A".into()),
                    Value::Text("a".into()),
                    Value::Text("B".into()),
                ],
            ),
        ];
        for (module, values) in cases {
            let eq = SemanticId::new(77);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_equivalence(module);
            let context = context(eq, digest);
            for a in &values {
                assert_eq!(registry.equivalent(&context, eq, a, a), Ok(true));
                for b in &values {
                    let ab = registry.equivalent(&context, eq, a, b).unwrap();
                    let ba = registry.equivalent(&context, eq, b, a).unwrap();
                    assert_eq!(ab, ba);
                    for c in &values {
                        let bc = registry.equivalent(&context, eq, b, c).unwrap();
                        if ab && bc {
                            assert_eq!(registry.equivalent(&context, eq, a, c), Ok(true));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn structural_product_equivalence_is_schema_derived_and_compositional() {
        let product_eq = SemanticId::new(40);
        let name_eq = SemanticId::new(41);
        let age_eq = SemanticId::new(42);
        let name_field = SemanticId::new(1);
        let age_field = SemanticId::new(2);
        let mut registry = SemanticRegistry::default();
        let name_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let age_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                product_eq,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(name_field, name_eq), (age_field, age_eq)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(name_eq, name_digest);
        environment.pin_module(age_eq, age_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.equivalence_domain(&context, product_eq).unwrap(),
            EquivalenceDomain::Product(BTreeMap::from([
                (name_field, EquivalenceDomain::Text),
                (age_field, EquivalenceDomain::I64),
            ]))
        );
        let left = Value::Product(BTreeMap::from([
            (name_field, Value::Text("ALICE".into())),
            (age_field, Value::I64(30)),
        ]));
        let right = Value::Product(BTreeMap::from([
            (name_field, Value::Text("alice".into())),
            (age_field, Value::I64(30)),
        ]));
        assert_eq!(
            registry.equivalent(&context, product_eq, &left, &right),
            Ok(true)
        );
    }

    #[test]
    fn structural_equivalence_cycle_is_rejected_before_data_exists() {
        let a = SemanticId::new(50);
        let b = SemanticId::new(51);
        let field = SemanticId::new(1);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                a,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, b)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                b,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, a)]),
                },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        assert!(matches!(
            registry.validate_context(&context),
            Err(SemanticError::CyclicStructuralEquivalence(_))
        ));
    }

    #[test]
    fn guarded_recursive_equivalence_has_canonical_mu_domain_and_terminates_on_values() {
        let root = SemanticId::new(60);
        let sum = SemanticId::new(61);
        let product = SemanticId::new(62);
        let recursive_var = SemanticId::new(63);
        let unit_eq = SemanticId::new(64);
        let text_eq = SemanticId::new(65);
        let nil_tag = SemanticId::new(66);
        let cons_tag = SemanticId::new(67);
        let head_field = SemanticId::new(68);
        let tail_field = SemanticId::new(69);

        let mut registry = SemanticRegistry::default();
        let unit_digest = registry.install_equivalence(EquivalenceModule::UnitExact);
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(root, StructuralEquivalenceDef::Mu { body: sum })
            .unwrap();
        schema
            .define_structural_equivalence(
                sum,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(nil_tag, unit_eq), (cons_tag, product)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(head_field, text_eq), (tail_field, recursive_var)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                recursive_var,
                StructuralEquivalenceDef::Var { binder: root },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(unit_eq, unit_digest);
        environment.pin_module(text_eq, text_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        let x = kernel_schema::TypeVar(0);
        let ty = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Sum(BTreeMap::from([
                (nil_tag, TypeExpr::Scalar(ScalarType::Unit)),
                (
                    cons_tag,
                    TypeExpr::Product(BTreeMap::from([
                        (head_field, TypeExpr::Scalar(ScalarType::Text)),
                        (tail_field, TypeExpr::Var(x)),
                    ])),
                ),
            ]))),
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.equivalence_domain(&context, root).unwrap(),
            domain_for_type(&ty).unwrap()
        );

        let nil = || Value::Variant {
            tag: nil_tag,
            value: Box::new(Value::Unit),
        };
        let cons = |head: &str, tail: Value| Value::Variant {
            tag: cons_tag,
            value: Box::new(Value::Product(BTreeMap::from([
                (head_field, Value::Text(head.into())),
                (tail_field, tail),
            ]))),
        };
        let left = cons("A", cons("B", nil()));
        let right = cons("a", cons("b", nil()));
        let different = cons("a", nil());
        let compiled = registry.compile_equivalence(&context, root).unwrap();
        assert_eq!(registry.equivalent(&context, root, &left, &right), Ok(true));
        assert_eq!(
            registry.canonical_equivalence_key(&context, root, &left),
            registry.canonical_equivalence_key(&context, root, &right)
        );
        assert_eq!(compiled.equivalent(&left, &right), Ok(true));
        assert_eq!(
            compiled.canonical_key(&left),
            registry.canonical_equivalence_key(&context, root, &left)
        );
        assert_eq!(
            registry.equivalent(&context, root, &left, &different),
            Ok(false)
        );
        assert_eq!(compiled.equivalent(&left, &different), Ok(false));
        assert_ne!(
            registry.canonical_equivalence_key(&context, root, &left),
            registry.canonical_equivalence_key(&context, root, &different)
        );
    }

    #[test]
    fn recursive_equivalence_rejects_unguarded_and_free_variables() {
        let root = SemanticId::new(70);
        let recursive_var = SemanticId::new(71);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                root,
                StructuralEquivalenceDef::Mu {
                    body: recursive_var,
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                recursive_var,
                StructuralEquivalenceDef::Var { binder: root },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        let registry = SemanticRegistry::default();
        assert_eq!(
            registry.validate_context(&context),
            Err(SemanticError::UnguardedStructuralRecursion(root))
        );

        let free_var = SemanticId::new(72);
        let missing_binder = SemanticId::new(73);
        let mut schema = Schema::new(SchemaRevisionId::new(2));
        schema
            .define_structural_equivalence(
                free_var,
                StructuralEquivalenceDef::Var {
                    binder: missing_binder,
                },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(2)),
        };
        assert_eq!(
            registry.validate_context(&context),
            Err(SemanticError::FreeStructuralRecursion(missing_binder))
        );
    }

    #[test]
    fn recursive_equivalence_refinement_is_checked_coinductively_through_mu_binders() {
        let exact_root = SemanticId::new(80);
        let exact_option = SemanticId::new(81);
        let exact_product = SemanticId::new(82);
        let exact_var = SemanticId::new(83);
        let ci_root = SemanticId::new(84);
        let ci_option = SemanticId::new(85);
        let ci_product = SemanticId::new(86);
        let ci_var = SemanticId::new(87);
        let exact_text = SemanticId::new(88);
        let ci_text = SemanticId::new(89);
        let head_field = SemanticId::new(90);
        let tail_field = SemanticId::new(91);

        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(3));
        for (root, option, product, recursive_var, text_eq) in [
            (
                exact_root,
                exact_option,
                exact_product,
                exact_var,
                exact_text,
            ),
            (ci_root, ci_option, ci_product, ci_var, ci_text),
        ] {
            schema
                .define_structural_equivalence(root, StructuralEquivalenceDef::Mu { body: option })
                .unwrap();
            schema
                .define_structural_equivalence(
                    option,
                    StructuralEquivalenceDef::Option { inner: product },
                )
                .unwrap();
            schema
                .define_structural_equivalence(
                    product,
                    StructuralEquivalenceDef::Product {
                        fields: BTreeMap::from([
                            (head_field, text_eq),
                            (tail_field, recursive_var),
                        ]),
                    },
                )
                .unwrap();
            schema
                .define_structural_equivalence(
                    recursive_var,
                    StructuralEquivalenceDef::Var { binder: root },
                )
                .unwrap();
        }
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(3));
        environment.pin_module(exact_text, exact_digest);
        environment.pin_module(ci_text, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.equivalence_refines(&context, exact_root, ci_root),
            Ok(true)
        );
        assert_eq!(
            registry.equivalence_refines(&context, ci_root, exact_root),
            Ok(false)
        );
    }
    #[test]
    fn nominal_entity_equivalence_rejects_wrong_runtime_type() {
        let person = SemanticId::new(90);
        let order = SemanticId::new(91);
        let eq = SemanticId::new(92);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::LiveEntityIdExact(person));
        let context = context(eq, digest);
        let wrong = Value::LiveEntityRef {
            entity_type: order,
            id: EntityId::new(1),
        };
        assert_eq!(
            registry.equivalent(&context, eq, &wrong, &wrong),
            Err(SemanticError::TypeMismatch(eq))
        );
    }
    #[test]
    fn structural_option_and_sum_equivalence_are_compositional() {
        let text_eq = SemanticId::new(100);
        let option_eq = SemanticId::new(101);
        let sum_eq = SemanticId::new(102);
        let text_tag = SemanticId::new(103);
        let int_tag = SemanticId::new(104);
        let int_eq = SemanticId::new(105);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let int_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_structural_equivalence(
                option_eq,
                StructuralEquivalenceDef::Option { inner: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                sum_eq,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(text_tag, text_eq), (int_tag, int_eq)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(text_eq, text_digest);
        environment.pin_module(int_eq, int_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();

        assert_eq!(
            registry.equivalent(
                &context,
                option_eq,
                &Value::Option(Some(Box::new(Value::Text("A".into())))),
                &Value::Option(Some(Box::new(Value::Text("a".into())))),
            ),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                option_eq,
                &Value::Option(None),
                &Value::Option(Some(Box::new(Value::Text("a".into())))),
            ),
            Ok(false)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                sum_eq,
                &Value::Variant {
                    tag: text_tag,
                    value: Box::new(Value::Text("X".into())),
                },
                &Value::Variant {
                    tag: text_tag,
                    value: Box::new(Value::Text("x".into())),
                },
            ),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent(
                &context,
                sum_eq,
                &Value::Variant {
                    tag: text_tag,
                    value: Box::new(Value::Text("1".into())),
                },
                &Value::Variant {
                    tag: int_tag,
                    value: Box::new(Value::I64(1)),
                },
            ),
            Ok(false)
        );
    }

    #[test]
    fn structural_collection_equivalences_are_compositional() {
        let text_eq = SemanticId::new(1200);
        let set_eq = SemanticId::new(1201);
        let bag_eq = SemanticId::new(1202);
        let sequence_eq = SemanticId::new(1203);
        let map_eq = SemanticId::new(1204);
        let mut registry = SemanticRegistry::default();
        let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1200));
        schema
            .define_structural_equivalence(
                set_eq,
                StructuralEquivalenceDef::Set { element: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                bag_eq,
                StructuralEquivalenceDef::Bag { element: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                sequence_eq,
                StructuralEquivalenceDef::Seq { element: text_eq },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                map_eq,
                StructuralEquivalenceDef::Map {
                    key: text_eq,
                    value: text_eq,
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1200));
        environment.pin_module(text_eq, text_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        let set = |values: &[&str]| Value::Set {
            equivalence: text_eq,
            elements: values
                .iter()
                .map(|value| Value::Text((*value).into()))
                .collect(),
        };
        let left_set = set(&["A", "B"]);
        let right_set = set(&["b", "a"]);
        assert_eq!(
            registry.equivalent(&context, set_eq, &left_set, &right_set),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, set_eq, &left_set),
            registry.canonical_equivalence_key(&context, set_eq, &right_set)
        );
        let left_bag = Value::Bag {
            equivalence: text_eq,
            entries: vec![(Value::Text("A".into()), 2)],
        };
        let right_bag = Value::Bag {
            equivalence: text_eq,
            entries: vec![(Value::Text("a".into()), 2)],
        };
        assert_eq!(
            registry.equivalent(&context, bag_eq, &left_bag, &right_bag),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, bag_eq, &left_bag),
            registry.canonical_equivalence_key(&context, bag_eq, &right_bag)
        );
        let left_sequence = Value::Seq(vec![Value::Text("A".into()), Value::Text("B".into())]);
        let right_sequence = Value::Seq(vec![Value::Text("a".into()), Value::Text("b".into())]);
        assert_eq!(
            registry.equivalent(&context, sequence_eq, &left_sequence, &right_sequence),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, sequence_eq, &left_sequence),
            registry.canonical_equivalence_key(&context, sequence_eq, &right_sequence)
        );
        let map = |key: &str, value: &str| Value::Map {
            key_equivalence: text_eq,
            entries: vec![(Value::Text(key.into()), Value::Text(value.into()))],
        };
        let left_map = map("K", "V");
        let right_map = map("k", "v");
        assert_eq!(
            registry.equivalent(&context, map_eq, &left_map, &right_map),
            Ok(true)
        );
        assert_eq!(
            registry.canonical_equivalence_key(&context, map_eq, &left_map),
            registry.canonical_equivalence_key(&context, map_eq, &right_map)
        );
    }

    #[test]
    fn structural_equivalence_refinement_is_compositional_and_directional() {
        let exact = SemanticId::new(1210);
        let ci = SemanticId::new(1211);
        let exact_option = SemanticId::new(1212);
        let ci_option = SemanticId::new(1213);
        let field = SemanticId::new(1214);
        let exact_product = SemanticId::new(1215);
        let ci_product = SemanticId::new(1216);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(1210));
        schema
            .define_structural_equivalence(
                exact_option,
                StructuralEquivalenceDef::Option { inner: exact },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                ci_option,
                StructuralEquivalenceDef::Option { inner: ci },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                exact_product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, exact_option)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(
                ci_product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field, ci_option)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1210));
        environment.pin_module(exact, exact_digest);
        environment.pin_module(ci, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        assert_eq!(
            registry.equivalence_refines(&context, exact_product, ci_product),
            Ok(true)
        );
        assert_eq!(
            registry.equivalence_refines(&context, ci_product, exact_product),
            Ok(false)
        );
    }

    #[test]
    fn implementation_revision_changes_digest_without_changing_semantic_contract() {
        let mut registry = SemanticRegistry::default();
        let first = registry.install_equivalence_revision(EquivalenceModule::TextExact, 1);
        let second = registry.install_equivalence_revision(EquivalenceModule::TextExact, 2);
        let changed_law =
            registry.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 2);

        assert_ne!(first, second);
        assert_eq!(
            registry.equivalent_implementation_contract(first, second),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent_implementation_contract(first, changed_law),
            Ok(false)
        );
    }

    #[test]
    fn ordering_is_versioned_semantics_not_host_iteration_order() {
        let ordering = SemanticId::new(700);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitiveThenBinary);
        let context = context(ordering, digest);
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::Text("a".into()),
                &Value::Text("B".into()),
            ),
            Ok(CmpOrdering::Less)
        );
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::Text("A".into()),
                &Value::Text("a".into()),
            ),
            Ok(CmpOrdering::Less)
        );
    }

    #[test]
    fn structural_sum_order_uses_explicit_variant_rank_not_semantic_id_order() {
        let text_order = SemanticId::new(73_100);
        let sum_order = SemanticId::new(73_101);
        let high_id_first = SemanticId::new(90_000);
        let low_id_second = SemanticId::new(10);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::TextBinary);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(73_100));
        environment.pin_module(text_order, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(73_100));
        schema
            .define_structural_ordering(
                sum_order,
                StructuralOrderingDef::Sum {
                    variants: vec![(high_id_first, text_order), (low_id_second, text_order)],
                },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        registry.validate_context(&context).unwrap();

        let first = Value::Variant {
            tag: high_id_first,
            value: Box::new(Value::Text("z".into())),
        };
        let second = Value::Variant {
            tag: low_id_second,
            value: Box::new(Value::Text("a".into())),
        };
        assert_eq!(
            registry.compare(&context, sum_order, &first, &second),
            Ok(std::cmp::Ordering::Less)
        );
    }

    #[test]
    fn compiled_structural_ordering_reuses_sum_rank_index_across_sequence_elements() {
        let text_order = SemanticId::new(73_200);
        let sum_order = SemanticId::new(73_201);
        let seq_order = SemanticId::new(73_202);
        let first_tag = SemanticId::new(900_000);
        let second_tag = SemanticId::new(2);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::TextBinary);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(73_200));
        environment.pin_module(text_order, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(73_200));
        schema
            .define_structural_ordering(
                sum_order,
                StructuralOrderingDef::Sum {
                    variants: vec![(first_tag, text_order), (second_tag, text_order)],
                },
            )
            .unwrap();
        schema
            .define_structural_ordering(
                seq_order,
                StructuralOrderingDef::Seq { element: sum_order },
            )
            .unwrap();
        let context = SemanticContext {
            schema,
            environment,
        };
        let compiled = registry.compile_ordering(&context, seq_order).unwrap();

        let value = Value::Seq(vec![
            Value::Variant {
                tag: second_tag,
                value: Box::new(Value::Text("b".into())),
            },
            Value::Variant {
                tag: first_tag,
                value: Box::new(Value::Text("z".into())),
            },
            Value::Variant {
                tag: second_tag,
                value: Box::new(Value::Text("a".into())),
            },
        ]);
        let key = compiled.canonical_key(&value).unwrap();
        let CanonicalOrderKey::Seq(entries) = key else {
            panic!("compiled seq ordering must produce a sequence key");
        };
        assert_eq!(entries.len(), 3);
        assert!(matches!(
            entries[0],
            CanonicalOrderKey::Variant { rank: 1, .. }
        ));
        assert!(matches!(
            entries[1],
            CanonicalOrderKey::Variant { rank: 0, .. }
        ));
        assert!(matches!(
            entries[2],
            CanonicalOrderKey::Variant { rank: 1, .. }
        ));
    }

    #[test]
    fn f64_total_order_is_defined_for_signed_zero_and_nan() {
        let ordering = SemanticId::new(705);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::F64Total);
        let context = context(ordering, digest);
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::F64Bits((-0.0_f64).to_bits()),
                &Value::F64Bits(0.0_f64.to_bits()),
            ),
            Ok(CmpOrdering::Less)
        );
        assert_eq!(
            registry.compare(
                &context,
                ordering,
                &Value::F64Bits(f64::INFINITY.to_bits()),
                &Value::F64Bits(f64::NAN.to_bits()),
            ),
            Ok(CmpOrdering::Less)
        );
    }

    #[test]
    fn ordering_implementation_upgrade_preserves_contract_but_contract_change_does_not() {
        let mut registry = SemanticRegistry::default();
        let v1 = registry.install_ordering_revision(OrderingModule::TextBinary, 1);
        let v2 = registry.install_ordering_revision(OrderingModule::TextBinary, 2);
        let changed = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitiveThenBinary);
        assert_eq!(
            registry.equivalent_implementation_contract(v1, v2),
            Ok(true)
        );
        assert_eq!(
            registry.equivalent_implementation_contract(v1, changed),
            Ok(false)
        );
    }

    #[test]
    fn ordering_congruence_is_explicit_and_not_inferred_from_matching_scalar_type() {
        let text_eq = SemanticId::new(710);
        let binary_order = SemanticId::new(711);
        let ci_order = SemanticId::new(712);
        let mut registry = SemanticRegistry::default();
        let eq_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let binary_digest = registry.install_ordering(OrderingModule::TextBinary);
        let ci_digest = registry.install_ordering(OrderingModule::TextAsciiCaseInsensitive);
        let mut environment =
            kernel_schema::SemanticEnvironment::new(kernel_types::SemanticEnvId::new(710));
        environment.pin_module(text_eq, eq_digest);
        environment.pin_module(binary_order, binary_digest);
        environment.pin_module(ci_order, ci_digest);
        let context = SemanticContext {
            schema: kernel_schema::Schema::new(kernel_types::SchemaRevisionId::new(710)),
            environment,
        };

        assert_eq!(
            registry.ordering_congruent_with_equivalence(&context, binary_order, text_eq),
            Ok(false)
        );
        assert_eq!(
            registry.ordering_congruent_with_equivalence(&context, ci_order, text_eq),
            Ok(true)
        );
    }

    #[test]
    fn ordering_compatibility_requires_a_checked_law_certificate() {
        let valid = OrderingCompatibilitySpec {
            ordering: OrderingModule::TextAsciiCaseInsensitive,
            equivalence: EquivalenceModule::TextAsciiCaseInsensitive,
        };
        let checked =
            certify_ordering_compatibility(&valid, OrderingCompatibilityArtifact::BuiltinLaw)
                .unwrap();
        assert_eq!(checked.spec(), &valid);

        let invalid = OrderingCompatibilitySpec {
            ordering: OrderingModule::TextBinary,
            equivalence: EquivalenceModule::TextAsciiCaseInsensitive,
        };
        assert_eq!(
            certify_ordering_compatibility(&invalid, OrderingCompatibilityArtifact::BuiltinLaw,)
                .err(),
            Some(SemanticError::OrderingCompatibilityViolation)
        );
    }

    #[test]
    fn semantic_implementation_install_requires_contract_bound_checked_certificate() {
        let contract = SemanticContract::Equivalence(EquivalenceModule::TextExact);
        let checked = certify_implementation(
            &contract,
            SemanticImplementationArtifact::BuiltinEquivalence(EquivalenceModule::TextExact),
        )
        .unwrap();
        assert_eq!(checked.spec(), &contract);

        let mut registry = SemanticRegistry::default();
        let digest = registry.install_certified_implementation(checked, 7);
        let direct = registry.install_equivalence_revision(EquivalenceModule::TextExact, 7);
        assert_eq!(digest, direct);

        assert_eq!(
            certify_implementation(
                &SemanticContract::Equivalence(EquivalenceModule::TextExact),
                SemanticImplementationArtifact::BuiltinEquivalence(
                    EquivalenceModule::TextAsciiCaseInsensitive,
                ),
            )
            .err(),
            Some(SemanticError::ImplementationContractMismatch)
        );
    }

    #[test]
    fn builtin_module_spec_roundtrips_exact_implementation_revision() {
        let mut source = SemanticRegistry::default();
        let digest =
            source.install_equivalence_revision(EquivalenceModule::TextAsciiCaseInsensitive, 17);
        let spec = source.builtin_module_spec(digest).unwrap();
        assert_eq!(spec.digest(), digest);

        let mut restored = SemanticRegistry::default();
        assert_eq!(restored.install_builtin_module_spec(spec), digest);
        assert_eq!(restored.builtin_module_spec(digest), Some(spec));
    }

    #[test]
    fn deployment_authentication_does_not_replace_semantic_refinement() {
        let spec = BuiltinSemanticModuleSpec::Equivalence {
            module: EquivalenceModule::TextExact,
            implementation_revision: 1,
        };
        let mut package = SemanticImplementationPackage::builtin(spec);
        package.refinement = Some(SemanticRefinementCertificate::Builtin(
            SemanticImplementationArtifact::BuiltinEquivalence(
                EquivalenceModule::TextAsciiCaseInsensitive,
            ),
        ));
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(package.clone());
        let authentications = ArtifactAuthenticationSet::trusted_builtins(&[spec]);
        assert_eq!(
            deployment
                .authorize_artifact(
                    package.artifact_digest,
                    &SemanticExecutionPolicy::trusted_builtin_only(),
                    &authentications,
                )
                .err(),
            Some(SemanticDeploymentError::RefinementContractMismatch)
        );
    }

    #[test]
    fn deployment_refinement_does_not_replace_artifact_authentication() {
        let spec = BuiltinSemanticModuleSpec::Tokenizer {
            module: TokenizerModule::AsciiWhitespace,
            implementation_revision: 1,
        };
        let deployment = SemanticDeploymentRegistry::from_builtin_specs(&[spec]);
        assert_eq!(
            deployment
                .authorize_artifact(
                    ImplementationArtifactDigest(spec.digest().0),
                    &SemanticExecutionPolicy::trusted_builtin_only(),
                    &ArtifactAuthenticationSet::default(),
                )
                .err(),
            Some(SemanticDeploymentError::UnauthenticatedArtifact)
        );
    }

    #[test]
    fn deployment_revocation_can_select_certified_defined_contract_replacement() {
        let old = BuiltinSemanticModuleSpec::Ordering {
            module: OrderingModule::I64Ascending,
            implementation_revision: 1,
        };
        let replacement = BuiltinSemanticModuleSpec::Ordering {
            module: OrderingModule::I64Ascending,
            implementation_revision: 2,
        };
        let deployment = SemanticDeploymentRegistry::from_builtin_specs(&[old, replacement]);
        let authentications = ArtifactAuthenticationSet::trusted_builtins(&[old, replacement]);
        let mut policy = SemanticExecutionPolicy::trusted_builtin_only();
        policy
            .revoked_artifacts
            .insert(ImplementationArtifactDigest(old.digest().0));
        let authorization = deployment
            .authorize_contract(
                SemanticContractIdentity::Defined(old.contract()),
                &policy,
                &authentications,
            )
            .unwrap();
        assert_eq!(
            authorization.artifact_digest(),
            ImplementationArtifactDigest(replacement.digest().0)
        );
        assert_eq!(
            authorization.contract(),
            SemanticContractIdentity::Defined(old.contract())
        );
    }

    #[test]
    fn opaque_deployment_identity_requires_exact_artifact_and_runtime() {
        let artifact = ImplementationArtifactDigest([71; 32]);
        let runtime = RuntimeProfileDigest([72; 32]);
        let package = SemanticImplementationPackage {
            contract: SemanticContractIdentity::OpaqueArtifact { artifact, runtime },
            artifact_digest: artifact,
            runtime_profile: runtime,
            refinement: None,
            executable: None,
        };
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(package.clone());
        let mut authentications = ArtifactAuthenticationSet::default();
        authentications.mark_verified(artifact);
        let policy = SemanticExecutionPolicy {
            allowed_runtime_profiles: BTreeSet::from([runtime]),
            revoked_artifacts: BTreeSet::new(),
            require_authentication: true,
        };
        let authorization = deployment
            .authorize_contract(package.contract, &policy, &authentications)
            .unwrap();
        assert_eq!(authorization.artifact_digest(), artifact);
        assert_eq!(
            SemanticDeploymentRegistry::install_authorized_builtin(
                &authorization,
                &mut SemanticRegistry::default(),
            ),
            Err(SemanticDeploymentError::ExecutableUnavailable)
        );

        let mut wrong = package;
        wrong.runtime_profile = RuntimeProfileDigest([73; 32]);
        let mut deployment = SemanticDeploymentRegistry::default();
        deployment.register(wrong.clone());
        let mut wrong_auth = ArtifactAuthenticationSet::default();
        wrong_auth.mark_verified(artifact);
        assert_eq!(
            deployment
                .authorize_artifact(wrong.artifact_digest, &policy, &wrong_auth)
                .err(),
            Some(SemanticDeploymentError::RuntimeDoesNotMatchOpaqueContract)
        );
    }

    #[test]
    fn execution_capability_checks_only_the_requested_contract_closure() {
        let needed = BuiltinSemanticModuleSpec::Equivalence {
            module: EquivalenceModule::I64Exact,
            implementation_revision: 1,
        };
        let unrelated = BuiltinSemanticModuleSpec::Tokenizer {
            module: TokenizerModule::AsciiWhitespaceLowercase,
            implementation_revision: 1,
        };
        let deployment = SemanticDeploymentRegistry::from_builtin_specs(&[needed, unrelated]);
        let authentications = ArtifactAuthenticationSet::trusted_builtins(&[needed]);
        let policy = SemanticExecutionPolicy::trusted_builtin_only();
        assert!(
            deployment
                .authorize_required(
                    &[SemanticContractIdentity::Defined(needed.contract())],
                    &policy,
                    &authentications,
                )
                .is_ok()
        );
        assert_eq!(
            deployment
                .authorize_required(
                    &[
                        SemanticContractIdentity::Defined(needed.contract()),
                        SemanticContractIdentity::Defined(unrelated.contract()),
                    ],
                    &policy,
                    &authentications,
                )
                .err(),
            Some(SemanticDeploymentError::UnauthenticatedArtifact)
        );

        let capability = deployment.execution_capability(
            &[
                SemanticContractIdentity::Defined(needed.contract()),
                SemanticContractIdentity::Defined(unrelated.contract()),
            ],
            &policy,
            &authentications,
        );
        assert!(!capability.executable());
        assert_eq!(capability.authorized.len(), 1);
        assert_eq!(
            capability.unavailable,
            vec![(
                SemanticContractIdentity::Defined(unrelated.contract()),
                SemanticDeploymentError::UnauthenticatedArtifact,
            )]
        );
    }

    #[test]
    fn registry_validates_every_pinned_environment_module_not_only_schema_dependencies() {
        let extra = SemanticId::new(9900);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
        environment.pin_module(extra, ModuleDigest([99; 32]));
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(1)),
            environment,
        };
        assert_eq!(
            SemanticRegistry::default().validate_context(&context),
            Err(SemanticError::ModuleUnavailable(ModuleDigest([99; 32])))
        );
    }
    #[test]
    fn tokenizer_is_versioned_semantics_not_ambient_library_behavior() {
        let tokenizer = SemanticId::new(5000);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(50));
        environment.pin_module(tokenizer, digest);
        let context = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(50)),
            environment,
        };
        registry.validate_context(&context).unwrap();
        assert_eq!(
            registry.tokenize(&context, tokenizer, "Hello   WORLD"),
            Ok(vec!["hello".to_string(), "world".to_string()])
        );
    }

    #[test]
    fn tokenizer_implementation_revision_preserves_contract_but_contract_change_does_not() {
        let tokenizer = SemanticId::new(5001);
        let mut registry = SemanticRegistry::default();
        let v1 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 1);
        let v2 = registry.install_tokenizer_revision(TokenizerModule::AsciiWhitespace, 2);
        let lower = registry.install_tokenizer(TokenizerModule::AsciiWhitespaceLowercase);
        let make_context = |revision, digest| {
            let mut environment = SemanticEnvironment::new(SemanticEnvId::new(revision));
            environment.pin_module(tokenizer, digest);
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(51)),
                environment,
            }
        };
        let old = make_context(51, v1);
        let new_impl = make_context(52, v2);
        let changed = make_context(53, lower);
        assert_eq!(
            registry.contexts_semantically_equivalent(&old, &new_impl),
            Ok(true)
        );
        assert_eq!(
            registry.contexts_semantically_equivalent(&old, &changed),
            Ok(false)
        );
    }

    #[test]
    fn tokenizer_digest_cannot_masquerade_as_equality_module() {
        let symbol = SemanticId::new(5002);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_tokenizer(TokenizerModule::AsciiWhitespace);
        let context = context(symbol, digest);
        assert_eq!(
            registry.equivalence_domain(&context, symbol),
            Err(SemanticError::WrongModuleKind(symbol))
        );
    }

    #[test]
    fn canonical_equality_keys_match_every_builtin_primitive_contract() {
        let modules = [
            (
                EquivalenceModule::UnitExact,
                vec![(Value::Unit, Value::Unit)],
            ),
            (
                EquivalenceModule::BoolExact,
                vec![
                    (Value::Bool(false), Value::Bool(false)),
                    (Value::Bool(false), Value::Bool(true)),
                ],
            ),
            (
                EquivalenceModule::I64Exact,
                vec![
                    (Value::I64(-7), Value::I64(-7)),
                    (Value::I64(-7), Value::I64(7)),
                ],
            ),
            (
                EquivalenceModule::F64Bitwise,
                vec![
                    (
                        Value::F64Bits(0.0_f64.to_bits()),
                        Value::F64Bits((-0.0_f64).to_bits()),
                    ),
                    (
                        Value::F64Bits(0x7ff8_0000_0000_0001),
                        Value::F64Bits(0x7ff8_0000_0000_0001),
                    ),
                    (
                        Value::F64Bits(0x7ff8_0000_0000_0001),
                        Value::F64Bits(0x7ff8_0000_0000_0002),
                    ),
                ],
            ),
            (
                EquivalenceModule::TextExact,
                vec![
                    (Value::Text("A".into()), Value::Text("A".into())),
                    (Value::Text("A".into()), Value::Text("a".into())),
                ],
            ),
            (
                EquivalenceModule::TextAsciiCaseInsensitive,
                vec![
                    (Value::Text("AΩz".into()), Value::Text("aΩZ".into())),
                    (Value::Text("Ω".into()), Value::Text("ω".into())),
                ],
            ),
        ];

        for (offset, (module, pairs)) in modules.into_iter().enumerate() {
            let semantic = SemanticId::new(71_000 + offset as u128);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_equivalence(module);
            let context = context(semantic, digest);
            let resolved = registry
                .resolve_primitive_equivalence(&context, semantic)
                .unwrap()
                .unwrap();
            assert_eq!(resolved.module_digest(), digest);
            for (left, right) in pairs {
                assert_eq!(
                    resolved.canonical_key(&left).unwrap()
                        == resolved.canonical_key(&right).unwrap(),
                    resolved.equivalent(&left, &right).unwrap(),
                    "module={module:?}, left={left:?}, right={right:?}"
                );
            }
        }
    }

    #[test]
    fn canonical_entity_equality_keys_match_builtin_contracts() {
        let person = SemanticId::new(70_001);
        let modules = [
            EquivalenceModule::LiveEntityIdExact(person),
            EquivalenceModule::HistoricalEntityIdExact(person),
        ];
        for (offset, module) in modules.into_iter().enumerate() {
            let semantic = SemanticId::new(71_100 + offset as u128);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_equivalence(module);
            let context = context(semantic, digest);
            let resolved = registry
                .resolve_primitive_equivalence(&context, semantic)
                .unwrap()
                .unwrap();
            let values = match module {
                EquivalenceModule::LiveEntityIdExact(entity_type) => vec![
                    Value::LiveEntityRef {
                        entity_type,
                        id: EntityId::new(5),
                    },
                    Value::LiveEntityRef {
                        entity_type,
                        id: EntityId::new(6),
                    },
                ],
                EquivalenceModule::HistoricalEntityIdExact(entity_type) => vec![
                    Value::HistoricalEntityId {
                        entity_type,
                        id: EntityId::new(5),
                    },
                    Value::HistoricalEntityId {
                        entity_type,
                        id: EntityId::new(6),
                    },
                ],
                _ => unreachable!(),
            };
            for left in &values {
                for right in &values {
                    assert_eq!(
                        resolved.canonical_key(left).unwrap()
                            == resolved.canonical_key(right).unwrap(),
                        resolved.equivalent(left, right).unwrap(),
                        "module={module:?}, left={left:?}, right={right:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn canonical_order_keys_match_every_builtin_ordering_contract() {
        let entity_type = SemanticId::new(71_999);
        let modules = [
            OrderingModule::UnitExact,
            OrderingModule::BoolAscending,
            OrderingModule::I64Ascending,
            OrderingModule::F64Total,
            OrderingModule::TextBinary,
            OrderingModule::TextAsciiCaseInsensitive,
            OrderingModule::TextAsciiCaseInsensitiveThenBinary,
            OrderingModule::LiveEntityIdAscending(entity_type),
            OrderingModule::HistoricalEntityIdAscending(entity_type),
        ];
        for (offset, module) in modules.into_iter().enumerate() {
            let ordering = SemanticId::new(72_000 + offset as u128);
            let mut registry = SemanticRegistry::default();
            let digest = registry.install_ordering(module);
            let context = context(ordering, digest);
            let resolved = registry
                .resolve_primitive_ordering(&context, ordering)
                .unwrap();
            assert_eq!(resolved.module_digest(), digest);
            let values = match module {
                OrderingModule::UnitExact => vec![Value::Unit],
                OrderingModule::BoolAscending => vec![Value::Bool(false), Value::Bool(true)],
                OrderingModule::I64Ascending => {
                    vec![
                        Value::I64(i64::MIN),
                        Value::I64(-1),
                        Value::I64(0),
                        Value::I64(i64::MAX),
                    ]
                }
                OrderingModule::F64Total => vec![
                    Value::F64Bits(f64::NEG_INFINITY.to_bits()),
                    Value::F64Bits(0xfff8_0000_0000_0002),
                    Value::F64Bits((-0.0_f64).to_bits()),
                    Value::F64Bits(0.0_f64.to_bits()),
                    Value::F64Bits(f64::INFINITY.to_bits()),
                    Value::F64Bits(0x7ff8_0000_0000_0001),
                    Value::F64Bits(0x7ff8_0000_0000_0002),
                ],
                OrderingModule::TextBinary
                | OrderingModule::TextAsciiCaseInsensitive
                | OrderingModule::TextAsciiCaseInsensitiveThenBinary => vec![
                    Value::Text("A".into()),
                    Value::Text("a".into()),
                    Value::Text("B".into()),
                    Value::Text("aΩZ".into()),
                    Value::Text("AΩz".into()),
                    Value::Text("ω".into()),
                ],
                OrderingModule::LiveEntityIdAscending(entity_type) => vec![
                    Value::LiveEntityRef {
                        entity_type,
                        id: kernel_types::EntityId::new(1),
                    },
                    Value::LiveEntityRef {
                        entity_type,
                        id: kernel_types::EntityId::new(2),
                    },
                ],
                OrderingModule::HistoricalEntityIdAscending(entity_type) => vec![
                    Value::HistoricalEntityId {
                        entity_type,
                        id: kernel_types::EntityId::new(1),
                    },
                    Value::HistoricalEntityId {
                        entity_type,
                        id: kernel_types::EntityId::new(2),
                    },
                ],
            };
            for left in &values {
                for right in &values {
                    assert_eq!(
                        resolved
                            .canonical_key(left)
                            .unwrap()
                            .cmp(&resolved.canonical_key(right).unwrap()),
                        resolved.compare(left, right).unwrap(),
                        "module={module:?}, left={left:?}, right={right:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn f64_total_canonical_key_matches_total_cmp_on_many_bit_patterns() {
        let ordering = SemanticId::new(73_000);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_ordering(OrderingModule::F64Total);
        let context = context(ordering, digest);
        let resolved = registry
            .resolve_primitive_ordering(&context, ordering)
            .unwrap();
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for _ in 0..100_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let left = Value::F64Bits(state);
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let right = Value::F64Bits(state);
            assert_eq!(
                resolved
                    .canonical_key(&left)
                    .unwrap()
                    .cmp(&resolved.canonical_key(&right).unwrap()),
                resolved.compare(&left, &right).unwrap()
            );
        }
    }

    fn assert_structural_key_law(
        registry: &SemanticRegistry,
        context: &SemanticContext,
        equivalence: SemanticId,
        values: &[Value],
    ) {
        let compiled = registry.compile_equivalence(context, equivalence).unwrap();
        assert_eq!(compiled.equivalence(), equivalence);
        assert_eq!(
            compiled.domain(),
            &registry.equivalence_domain(context, equivalence).unwrap()
        );
        for left in values {
            for right in values {
                let oracle = registry
                    .equivalent(context, equivalence, left, right)
                    .unwrap();
                let left_key = registry
                    .canonical_equivalence_key(context, equivalence, left)
                    .unwrap();
                let right_key = registry
                    .canonical_equivalence_key(context, equivalence, right)
                    .unwrap();
                assert_eq!(left_key == right_key, oracle);
                assert_eq!(compiled.canonical_key(left).unwrap(), left_key);
                assert_eq!(compiled.canonical_key(right).unwrap(), right_key);
                assert_eq!(compiled.equivalent(left, right).unwrap(), oracle);
            }
        }
    }

    #[test]
    fn structural_product_option_and_sum_keys_match_equivalence_oracle() {
        let exact = SemanticId::new(80_000);
        let ci = SemanticId::new(80_001);
        let product = SemanticId::new(80_002);
        let option = SemanticId::new(80_003);
        let sum = SemanticId::new(80_004);
        let field_text = SemanticId::new(80_005);
        let field_int = SemanticId::new(80_006);
        let tag_text = SemanticId::new(80_007);
        let tag_int = SemanticId::new(80_008);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(80_000));
        schema
            .define_structural_equivalence(
                product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field_text, ci), (field_int, exact)]),
                },
            )
            .unwrap();
        schema
            .define_structural_equivalence(option, StructuralEquivalenceDef::Option { inner: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                sum,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(tag_text, ci), (tag_int, exact)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(80_000));
        environment.pin_module(exact, exact_digest);
        environment.pin_module(ci, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let product_value = |text: &str, int: i64| {
            Value::Product(BTreeMap::from([
                (field_text, Value::Text(text.into())),
                (field_int, Value::I64(int)),
            ]))
        };
        assert_structural_key_law(
            &registry,
            &context,
            product,
            &[
                product_value("A", 1),
                product_value("a", 1),
                product_value("a", 2),
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            option,
            &[
                Value::Option(None),
                Value::Option(Some(Box::new(Value::Text("A".into())))),
                Value::Option(Some(Box::new(Value::Text("a".into())))),
                Value::Option(Some(Box::new(Value::Text("B".into())))),
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            sum,
            &[
                Value::Variant {
                    tag: tag_text,
                    value: Box::new(Value::Text("A".into())),
                },
                Value::Variant {
                    tag: tag_text,
                    value: Box::new(Value::Text("a".into())),
                },
                Value::Variant {
                    tag: tag_int,
                    value: Box::new(Value::I64(1)),
                },
            ],
        );
    }

    #[test]
    fn structural_collection_keys_match_equivalence_oracle() {
        let ci = SemanticId::new(81_000);
        let set = SemanticId::new(81_001);
        let bag = SemanticId::new(81_002);
        let seq = SemanticId::new(81_003);
        let map = SemanticId::new(81_004);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(81_000));
        schema
            .define_structural_equivalence(set, StructuralEquivalenceDef::Set { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(bag, StructuralEquivalenceDef::Bag { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(seq, StructuralEquivalenceDef::Seq { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                map,
                StructuralEquivalenceDef::Map { key: ci, value: ci },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(81_000));
        environment.pin_module(ci, digest);
        let context = SemanticContext {
            schema,
            environment,
        };
        let text = |value: &str| Value::Text(value.into());
        assert_structural_key_law(
            &registry,
            &context,
            set,
            &[
                Value::Set {
                    equivalence: ci,
                    elements: vec![text("A"), text("B")],
                },
                Value::Set {
                    equivalence: ci,
                    elements: vec![text("b"), text("a")],
                },
                Value::Set {
                    equivalence: ci,
                    elements: vec![text("A"), text("C")],
                },
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            bag,
            &[
                Value::Bag {
                    equivalence: ci,
                    entries: vec![(text("A"), 2), (text("B"), 1)],
                },
                Value::Bag {
                    equivalence: ci,
                    entries: vec![(text("b"), 1), (text("a"), 2)],
                },
                Value::Bag {
                    equivalence: ci,
                    entries: vec![(text("a"), 1), (text("b"), 1)],
                },
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            seq,
            &[
                Value::Seq(vec![text("A"), text("B")]),
                Value::Seq(vec![text("a"), text("b")]),
                Value::Seq(vec![text("b"), text("a")]),
            ],
        );
        assert_structural_key_law(
            &registry,
            &context,
            map,
            &[
                Value::Map {
                    key_equivalence: ci,
                    entries: vec![(text("K1"), text("V1")), (text("K2"), text("V2"))],
                },
                Value::Map {
                    key_equivalence: ci,
                    entries: vec![(text("k2"), text("v2")), (text("k1"), text("v1"))],
                },
                Value::Map {
                    key_equivalence: ci,
                    entries: vec![(text("k1"), text("v2")), (text("k2"), text("v1"))],
                },
            ],
        );
    }

    #[test]
    fn coarse_unordered_structural_keys_use_canonical_finite_measures() {
        let exact = SemanticId::new(81_100);
        let ci = SemanticId::new(81_101);
        let set_ci = SemanticId::new(81_102);
        let bag_ci = SemanticId::new(81_103);
        let map_ci = SemanticId::new(81_104);
        let mut registry = SemanticRegistry::default();
        let exact_digest = registry.install_equivalence(EquivalenceModule::TextExact);
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut schema = Schema::new(SchemaRevisionId::new(81_100));
        schema
            .define_structural_equivalence(set_ci, StructuralEquivalenceDef::Set { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(bag_ci, StructuralEquivalenceDef::Bag { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                map_ci,
                StructuralEquivalenceDef::Map {
                    key: ci,
                    value: exact,
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(81_100));
        environment.pin_module(exact, exact_digest);
        environment.pin_module(ci, ci_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        let values = [
            (
                set_ci,
                Value::Set {
                    equivalence: exact,
                    elements: vec![Value::Text("A".into()), Value::Text("a".into())],
                },
            ),
            (
                bag_ci,
                Value::Bag {
                    equivalence: exact,
                    entries: vec![(Value::Text("A".into()), 2), (Value::Text("a".into()), 2)],
                },
            ),
            (
                map_ci,
                Value::Map {
                    key_equivalence: exact,
                    entries: vec![
                        (Value::Text("A".into()), Value::Text("same".into())),
                        (Value::Text("a".into()), Value::Text("same".into())),
                    ],
                },
            ),
        ];
        for (equivalence, value) in values {
            let key = registry
                .canonical_equivalence_key(&context, equivalence, &value)
                .unwrap();
            assert_eq!(
                decode_canonical_eq_key(&encode_canonical_eq_key(&key)),
                Ok(key.clone())
            );
            match key {
                CanonicalEqKey::Set(entries) => assert_eq!(entries[0].multiplicity, 2),
                CanonicalEqKey::Bag(entries) => assert_eq!(entries[0].multiplicity, 2),
                CanonicalEqKey::Map(entries) => assert_eq!(entries[0].multiplicity, 2),
                _ => panic!("fixture must lower to an unordered finite measure"),
            }
        }
    }

    #[test]
    fn canonical_eq_key_codec_roundtrips_recursive_structures_and_rejects_version_drift() {
        let key = CanonicalEqKey::Product(vec![
            (
                SemanticId::new(1),
                CanonicalEqKey::OptionSome(Box::new(CanonicalEqKey::Seq(vec![
                    CanonicalEqKey::TextAsciiCaseInsensitive("alpha".into()),
                    CanonicalEqKey::I64(-7),
                ]))),
            ),
            (
                SemanticId::new(2),
                CanonicalEqKey::Map(finite_measure_from_atoms([
                    CanonicalMapAtom {
                        key: CanonicalEqKey::TextExact("a".into()),
                        value: CanonicalEqKey::Bag(finite_measure_from_atoms([CanonicalBagAtom {
                            value: CanonicalEqKey::Bool(false),
                            stored_count: 2,
                        }])),
                    },
                    CanonicalMapAtom {
                        key: CanonicalEqKey::TextExact("b".into()),
                        value: CanonicalEqKey::Set(finite_measure_from_atoms([
                            CanonicalEqKey::I64(1),
                            CanonicalEqKey::I64(2),
                        ])),
                    },
                ])),
            ),
        ]);
        let encoded = encode_canonical_eq_key(&key);
        assert_eq!(decode_canonical_eq_key(&encoded), Ok(key));

        let mut previous = encoded.clone();
        previous[4..8].copy_from_slice(&(CANONICAL_EQ_KEY_ENCODING_VERSION - 1).to_be_bytes());
        assert_eq!(
            decode_canonical_eq_key(&previous),
            Err(CanonicalEqKeyCodecError::UnsupportedVersion(
                CANONICAL_EQ_KEY_ENCODING_VERSION - 1
            ))
        );

        let mut future = encoded.clone();
        future[4..8].copy_from_slice(&(CANONICAL_EQ_KEY_ENCODING_VERSION + 1).to_be_bytes());
        assert_eq!(
            decode_canonical_eq_key(&future),
            Err(CanonicalEqKeyCodecError::UnsupportedVersion(
                CANONICAL_EQ_KEY_ENCODING_VERSION + 1
            ))
        );

        let tuple = vec![
            CanonicalEqKey::I64(9),
            CanonicalEqKey::TextExact("x".into()),
        ];
        assert_eq!(
            decode_canonical_eq_key_tuple(&encode_canonical_eq_key_tuple(&tuple)),
            Ok(tuple)
        );
    }

    #[test]
    fn canonical_eq_key_v2_has_golden_bytes_independent_of_rust_layout() {
        let key = CanonicalEqKey::Product(vec![
            (
                SemanticId::new(1),
                CanonicalEqKey::TextAsciiCaseInsensitive("a".into()),
            ),
            (SemanticId::new(2), CanonicalEqKey::I64(-1)),
        ]);
        assert_eq!(
            encode_canonical_eq_key(&key),
            vec![
                67, 69, 75, 0, 0, 0, 0, 2, 8, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 1, 5, 0, 0, 0, 0, 0, 0, 0, 1, 97, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 2, 2, 255, 255, 255, 255, 255, 255, 255, 255,
            ]
        );
    }

    #[test]
    fn canonical_eq_key_codec_rejects_noncanonical_and_hostile_lengths() {
        let noncanonical = CanonicalEqKey::Set(vec![
            FiniteMeasureEntry {
                atom: CanonicalEqKey::I64(2),
                multiplicity: 1,
            },
            FiniteMeasureEntry {
                atom: CanonicalEqKey::I64(1),
                multiplicity: 1,
            },
        ]);
        assert_eq!(
            decode_canonical_eq_key(&encode_canonical_eq_key(&noncanonical)),
            Err(CanonicalEqKeyCodecError::NonCanonicalShape)
        );
        let noncanonical_ci = CanonicalEqKey::TextAsciiCaseInsensitive("Alpha".into());
        assert_eq!(
            decode_canonical_eq_key(&encode_canonical_eq_key(&noncanonical_ci)),
            Err(CanonicalEqKeyCodecError::NonCanonicalShape)
        );

        let mut hostile = Vec::from(CANONICAL_EQ_KEY_MAGIC);
        hostile.extend_from_slice(&CANONICAL_EQ_KEY_ENCODING_VERSION.to_be_bytes());
        hostile.push(13);
        hostile.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(
            decode_canonical_eq_key(&hostile),
            Err(CanonicalEqKeyCodecError::Truncated)
        );

        let mut too_deep = Vec::from(CANONICAL_EQ_KEY_MAGIC);
        too_deep.extend_from_slice(&CANONICAL_EQ_KEY_ENCODING_VERSION.to_be_bytes());
        too_deep.extend(std::iter::repeat_n(10_u8, MAX_CANONICAL_EQ_KEY_DEPTH + 1));
        too_deep.push(0);
        assert_eq!(
            decode_canonical_eq_key(&too_deep),
            Err(CanonicalEqKeyCodecError::DepthLimitExceeded)
        );
    }

    #[test]
    fn semantic_work_estimate_distinguishes_payload_and_nested_structure() {
        let scalar = semantic_value_work_estimate(&Value::I64(1));
        let text = semantic_value_work_estimate(&Value::Text("abcdefghij".into()));
        let nested = semantic_value_work_estimate(&Value::Seq(vec![
            Value::I64(1),
            Value::Seq(vec![Value::I64(2), Value::I64(3)]),
        ]));
        assert_eq!(scalar.total_units(), 1);
        assert_eq!(text.total_units(), 11);
        assert!(nested.total_units() > scalar.total_units());

        let key = CanonicalEqKey::Seq(vec![
            CanonicalEqKey::TextExact("abcd".into()),
            CanonicalEqKey::I64(1),
        ]);
        assert_eq!(canonical_eq_key_work_estimate(&key).total_units(), 7);
    }

    #[test]
    fn structural_canonical_dependency_closure_tracks_only_primitive_leaf_digests() {
        let ci = SemanticId::new(82_000);
        let exact = SemanticId::new(82_001);
        let product = SemanticId::new(82_002);
        let set = SemanticId::new(82_003);
        let field_a = SemanticId::new(82_004);
        let field_b = SemanticId::new(82_005);
        let mut registry = SemanticRegistry::default();
        let ci_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let exact_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
        let mut schema = Schema::new(SchemaRevisionId::new(82_000));
        schema
            .define_structural_equivalence(set, StructuralEquivalenceDef::Set { element: ci })
            .unwrap();
        schema
            .define_structural_equivalence(
                product,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([(field_a, set), (field_b, exact)]),
                },
            )
            .unwrap();
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(82_000));
        environment.pin_module(ci, ci_digest);
        environment.pin_module(exact, exact_digest);
        let context = SemanticContext {
            schema,
            environment,
        };

        assert_eq!(
            registry
                .canonical_equivalence_dependencies(&context, product)
                .unwrap(),
            vec![
                CanonicalEquivalenceDependency {
                    semantic_id: ci,
                    module_digest: ci_digest,
                },
                CanonicalEquivalenceDependency {
                    semantic_id: exact,
                    module_digest: exact_digest,
                },
            ]
        );
    }
}
