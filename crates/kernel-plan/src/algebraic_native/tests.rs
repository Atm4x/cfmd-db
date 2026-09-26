#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_non_recursive_algebraic_constructors_round_trip() {
        let a = SemanticId::new(1);
        let b = SemanticId::new(2);
        let left = SemanticId::new(3);
        let right = SemanticId::new(4);
        let eq = SemanticId::new(50);

        let cases = vec![
            (
                TypeExpr::Product(BTreeMap::from([
                    (a, TypeExpr::Scalar(ScalarType::I64)),
                    (
                        b,
                        TypeExpr::Option(Box::new(TypeExpr::Scalar(ScalarType::Text))),
                    ),
                ])),
                vec![Value::Product(BTreeMap::from([
                    (a, Value::I64(7)),
                    (b, Value::Option(Some(Box::new(Value::Text("x".into()))))),
                ]))],
            ),
            (
                TypeExpr::Sum(BTreeMap::from([
                    (left, TypeExpr::Scalar(ScalarType::I64)),
                    (right, TypeExpr::Scalar(ScalarType::Text)),
                ])),
                vec![
                    Value::Variant {
                        tag: left,
                        value: Box::new(Value::I64(9)),
                    },
                    Value::Variant {
                        tag: right,
                        value: Box::new(Value::Text("r".into())),
                    },
                ],
            ),
            (
                TypeExpr::Seq(Box::new(TypeExpr::Scalar(ScalarType::I64))),
                vec![Value::Seq(vec![Value::I64(1), Value::I64(2)])],
            ),
            (
                TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    equivalence: eq,
                },
                vec![Value::Set {
                    equivalence: eq,
                    elements: vec![Value::I64(1), Value::I64(2)],
                }],
            ),
            (
                TypeExpr::Bag {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: eq,
                },
                vec![Value::Bag {
                    equivalence: eq,
                    entries: vec![(Value::Text("a".into()), 3)],
                }],
            ),
            (
                TypeExpr::Map {
                    key: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    value: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    key_equivalence: eq,
                },
                vec![Value::Map {
                    key_equivalence: eq,
                    entries: vec![(Value::Text("a".into()), Value::I64(3))],
                }],
            ),
        ];

        for (ty, rows) in cases {
            let native = AlgebraicNativeColumn::from_values(&rows, &ty).unwrap();
            assert_eq!(native.len(), rows.len());
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(native.value_at(index).unwrap(), *row);
            }
        }
    }

    fn assert_variable_cardinality_mutation(ty: &TypeExpr, rows: &[Value]) {
        let native = AlgebraicNativeColumn::from_values(rows, ty).unwrap();
        let mut native = native.select_positions(&[2, 0]).unwrap();
        native.push_value(&rows[1]).unwrap();
        native.swap_remove_at(1).unwrap();
        assert_eq!(native.len(), 2);
        assert_eq!(native.value_at(0).unwrap(), rows[2]);
        assert_eq!(native.value_at(1).unwrap(), rows[1]);
    }

    #[test]
    fn option_sum_and_seq_mutation_stay_column_native() {
        let left = SemanticId::new(10);
        let right = SemanticId::new(11);
        let cases = vec![
            (
                TypeExpr::Option(Box::new(TypeExpr::Scalar(ScalarType::I64))),
                vec![
                    Value::Option(Some(Box::new(Value::I64(1)))),
                    Value::Option(None),
                    Value::Option(Some(Box::new(Value::I64(3)))),
                ],
            ),
            (
                TypeExpr::Sum(BTreeMap::from([
                    (left, TypeExpr::Scalar(ScalarType::I64)),
                    (right, TypeExpr::Scalar(ScalarType::Text)),
                ])),
                vec![
                    Value::Variant {
                        tag: left,
                        value: Box::new(Value::I64(1)),
                    },
                    Value::Variant {
                        tag: right,
                        value: Box::new(Value::Text("two".into())),
                    },
                    Value::Variant {
                        tag: left,
                        value: Box::new(Value::I64(3)),
                    },
                ],
            ),
            (
                TypeExpr::Seq(Box::new(TypeExpr::Scalar(ScalarType::I64))),
                vec![
                    Value::Seq(vec![Value::I64(1)]),
                    Value::Seq(vec![Value::I64(2), Value::I64(20)]),
                    Value::Seq(vec![Value::I64(3), Value::I64(30), Value::I64(300)]),
                ],
            ),
        ];
        for (ty, rows) in cases {
            assert_variable_cardinality_mutation(&ty, &rows);
        }
    }

    #[test]
    fn set_bag_and_map_mutation_stay_column_native() {
        let eq = SemanticId::new(12);
        let cases = vec![
            (
                TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    equivalence: eq,
                },
                vec![
                    Value::Set {
                        equivalence: eq,
                        elements: vec![Value::I64(1)],
                    },
                    Value::Set {
                        equivalence: eq,
                        elements: vec![Value::I64(2), Value::I64(20)],
                    },
                    Value::Set {
                        equivalence: eq,
                        elements: vec![Value::I64(3)],
                    },
                ],
            ),
            (
                TypeExpr::Bag {
                    element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    equivalence: eq,
                },
                vec![
                    Value::Bag {
                        equivalence: eq,
                        entries: vec![(Value::I64(1), 1)],
                    },
                    Value::Bag {
                        equivalence: eq,
                        entries: vec![(Value::I64(2), 2), (Value::I64(20), 1)],
                    },
                    Value::Bag {
                        equivalence: eq,
                        entries: vec![(Value::I64(3), 3)],
                    },
                ],
            ),
            (
                TypeExpr::Map {
                    key: Box::new(TypeExpr::Scalar(ScalarType::I64)),
                    value: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    key_equivalence: eq,
                },
                vec![
                    Value::Map {
                        key_equivalence: eq,
                        entries: vec![(Value::I64(1), Value::Text("one".into()))],
                    },
                    Value::Map {
                        key_equivalence: eq,
                        entries: vec![(Value::I64(2), Value::Text("two".into()))],
                    },
                    Value::Map {
                        key_equivalence: eq,
                        entries: vec![(Value::I64(3), Value::Text("three".into()))],
                    },
                ],
            ),
        ];

        for (ty, rows) in cases {
            assert_variable_cardinality_mutation(&ty, &rows);
        }
    }

    #[test]
    fn guarded_recursive_sum_round_trips() {
        let binder = TypeVar(1);
        let nil = SemanticId::new(1);
        let cons = SemanticId::new(2);
        let head = SemanticId::new(3);
        let tail = SemanticId::new(4);
        let ty = TypeExpr::Mu {
            binder,
            body: Box::new(TypeExpr::Sum(BTreeMap::from([
                (nil, TypeExpr::Scalar(ScalarType::Unit)),
                (
                    cons,
                    TypeExpr::Product(BTreeMap::from([
                        (head, TypeExpr::Scalar(ScalarType::I64)),
                        (tail, TypeExpr::Option(Box::new(TypeExpr::Var(binder)))),
                    ])),
                ),
            ]))),
        };
        let nil_value = Value::Variant {
            tag: nil,
            value: Box::new(Value::Unit),
        };
        let one = Value::Variant {
            tag: cons,
            value: Box::new(Value::Product(BTreeMap::from([
                (head, Value::I64(1)),
                (tail, Value::Option(Some(Box::new(nil_value.clone())))),
            ]))),
        };
        let native =
            AlgebraicNativeColumn::from_values(&[nil_value.clone(), one.clone()], &ty).unwrap();
        assert_eq!(native.value_at(0).unwrap(), nil_value);
        assert_eq!(native.value_at(1).unwrap(), one);
    }

    #[test]
    fn hostile_rejects_wrong_semantic_tags_and_empty_recursive_is_finite() {
        let eq = SemanticId::new(10);
        let wrong_eq = SemanticId::new(11);
        let set_ty = TypeExpr::Set {
            element: Box::new(TypeExpr::Scalar(ScalarType::I64)),
            equivalence: eq,
        };
        assert_eq!(
            AlgebraicNativeColumn::from_values(
                &[Value::Set {
                    equivalence: wrong_eq,
                    elements: vec![Value::I64(1)],
                }],
                &set_ty,
            ),
            Err(PhysicalExecutionError::PhysicalTypeMismatch)
        );

        let known = SemanticId::new(20);
        let unknown = SemanticId::new(21);
        let sum_ty = TypeExpr::Sum(BTreeMap::from([(known, TypeExpr::Scalar(ScalarType::I64))]));
        assert_eq!(
            AlgebraicNativeColumn::from_values(
                &[Value::Variant {
                    tag: unknown,
                    value: Box::new(Value::I64(1)),
                }],
                &sum_ty,
            ),
            Err(PhysicalExecutionError::PhysicalTypeMismatch)
        );

        let binder = TypeVar(9);
        let recursive = TypeExpr::Mu {
            binder,
            body: Box::new(TypeExpr::Seq(Box::new(TypeExpr::Var(binder)))),
        };
        let empty = AlgebraicNativeColumn::from_values(&[], &recursive).unwrap();
        assert!(empty.is_empty());

        let unguarded = TypeExpr::Mu {
            binder,
            body: Box::new(TypeExpr::Var(binder)),
        };
        assert_eq!(
            AlgebraicNativeColumn::from_values(&[Value::Seq(Vec::new())], &unguarded),
            Err(PhysicalExecutionError::PhysicalTypeMismatch)
        );
    }

    struct ComposedAlgebraFixture {
        context: kernel_schema::SemanticContext,
        registry: kernel_semantics::SemanticRegistry,
        product_equivalence: SemanticId,
        element_equivalence: SemanticId,
        fields: [SemanticId; 6],
        text_tag: SemanticId,
    }

    fn composed_algebra_fixture() -> ComposedAlgebraFixture {
        use kernel_schema::{Schema, SemanticEnvironment, StructuralEquivalenceDef};
        use kernel_semantics::{EquivalenceModule, SemanticRegistry};
        use kernel_types::{SchemaRevisionId, SemanticEnvId};

        let ci = SemanticId::new(90_000);
        let option_eq = SemanticId::new(90_001);
        let sequence_eq = SemanticId::new(90_002);
        let set_equivalence = SemanticId::new(90_003);
        let bag_eq = SemanticId::new(90_004);
        let map_eq = SemanticId::new(90_005);
        let sum_eq = SemanticId::new(90_006);
        let product_eq = SemanticId::new(90_007);
        let fields = std::array::from_fn(|index| SemanticId::new(90_010 + index as u128));
        let text_tag = SemanticId::new(90_020);
        let mut registry = SemanticRegistry::default();
        let digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
        let mut environment = SemanticEnvironment::new(SemanticEnvId::new(90_000));
        environment.pin_module(ci, digest);
        let mut schema = Schema::new(SchemaRevisionId::new(90_000));
        for (id, definition) in [
            (option_eq, StructuralEquivalenceDef::Option { inner: ci }),
            (sequence_eq, StructuralEquivalenceDef::Seq { element: ci }),
            (
                set_equivalence,
                StructuralEquivalenceDef::Set { element: ci },
            ),
            (bag_eq, StructuralEquivalenceDef::Bag { element: ci }),
            (map_eq, StructuralEquivalenceDef::Map { key: ci, value: ci }),
            (
                sum_eq,
                StructuralEquivalenceDef::Sum {
                    variants: BTreeMap::from([(text_tag, ci)]),
                },
            ),
        ] {
            schema
                .define_structural_equivalence(id, definition)
                .unwrap();
        }
        schema
            .define_structural_equivalence(
                product_eq,
                StructuralEquivalenceDef::Product {
                    fields: BTreeMap::from([
                        (fields[0], option_eq),
                        (fields[1], sequence_eq),
                        (fields[2], set_equivalence),
                        (fields[3], bag_eq),
                        (fields[4], map_eq),
                        (fields[5], sum_eq),
                    ]),
                },
            )
            .unwrap();
        ComposedAlgebraFixture {
            context: kernel_schema::SemanticContext {
                schema,
                environment,
            },
            registry,
            product_equivalence: product_eq,
            element_equivalence: ci,
            fields,
            text_tag,
        }
    }

    fn composed_algebra_type(fixture: &ComposedAlgebraFixture) -> TypeExpr {
        let text = TypeExpr::Scalar(ScalarType::Text);
        TypeExpr::Product(BTreeMap::from([
            (fixture.fields[0], TypeExpr::Option(Box::new(text.clone()))),
            (fixture.fields[1], TypeExpr::Seq(Box::new(text.clone()))),
            (
                fixture.fields[2],
                TypeExpr::Set {
                    element: Box::new(text.clone()),
                    equivalence: fixture.element_equivalence,
                },
            ),
            (
                fixture.fields[3],
                TypeExpr::Bag {
                    element: Box::new(text.clone()),
                    equivalence: fixture.element_equivalence,
                },
            ),
            (
                fixture.fields[4],
                TypeExpr::Map {
                    key: Box::new(text.clone()),
                    value: Box::new(text.clone()),
                    key_equivalence: fixture.element_equivalence,
                },
            ),
            (
                fixture.fields[5],
                TypeExpr::Sum(BTreeMap::from([(fixture.text_tag, text)])),
            ),
        ]))
    }

    fn composed_algebra_row(fixture: &ComposedAlgebraFixture, upper: bool) -> Value {
        let (a, b) = if upper {
            ("ALPHA", "BETA")
        } else {
            ("alpha", "beta")
        };
        let eq = fixture.element_equivalence;
        Value::Product(BTreeMap::from([
            (
                fixture.fields[0],
                Value::Option(Some(Box::new(Value::Text(a.into())))),
            ),
            (
                fixture.fields[1],
                Value::Seq(vec![Value::Text(a.into()), Value::Text(b.into())]),
            ),
            (
                fixture.fields[2],
                Value::Set {
                    equivalence: eq,
                    elements: vec![Value::Text(b.into()), Value::Text(a.into())],
                },
            ),
            (
                fixture.fields[3],
                Value::Bag {
                    equivalence: eq,
                    entries: vec![(Value::Text(b.into()), 1), (Value::Text(a.into()), 2)],
                },
            ),
            (
                fixture.fields[4],
                Value::Map {
                    key_equivalence: eq,
                    entries: vec![(Value::Text(a.into()), Value::Text(b.into()))],
                },
            ),
            (
                fixture.fields[5],
                Value::Variant {
                    tag: fixture.text_tag,
                    value: Box::new(Value::Text(a.into())),
                },
            ),
        ]))
    }

    #[test]
    fn native_structural_canonical_key_matches_registry_for_composed_algebra() {
        let fixture = composed_algebra_fixture();
        let rows = vec![
            composed_algebra_row(&fixture, false),
            composed_algebra_row(&fixture, true),
        ];
        let native =
            AlgebraicNativeColumn::from_values(&rows, &composed_algebra_type(&fixture)).unwrap();
        let compiled = fixture
            .registry
            .compile_equivalence(&fixture.context, fixture.product_equivalence)
            .unwrap();
        let mut keys = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let native_key = native.canonical_key_at_compiled(index, &compiled).unwrap();
            let registry_key = fixture
                .registry
                .canonical_equivalence_key(&fixture.context, fixture.product_equivalence, row)
                .unwrap();
            assert_eq!(native_key, registry_key);
            keys.push(native_key);
        }
        assert_eq!(keys[0], keys[1]);
    }

    #[test]
    fn selection_push_and_swap_remove_preserve_exact_values() {
        let field = SemanticId::new(1);
        let ty = TypeExpr::Product(BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::I64))]));
        let row = |value| Value::Product(BTreeMap::from([(field, Value::I64(value))]));
        let mut native =
            AlgebraicNativeColumn::from_values(&[row(1), row(2), row(3)], &ty).unwrap();
        let selected = native.select_positions(&[2, 0]).unwrap();
        assert_eq!(selected.value_at(0).unwrap(), row(3));
        assert_eq!(selected.value_at(1).unwrap(), row(1));
        native.swap_remove_at(1).unwrap();
        assert_eq!(native.value_at(0).unwrap(), row(1));
        assert_eq!(native.value_at(1).unwrap(), row(3));
        native.push_value(&row(4)).unwrap();
        assert_eq!(native.value_at(2).unwrap(), row(4));
    }

    #[test]
    fn typed_relation_compiler_keeps_scalars_native_and_structures_algebraic() {
        let field = SemanticId::new(1);
        let product_ty =
            TypeExpr::Product(BTreeMap::from([(field, TypeExpr::Scalar(ScalarType::I64))]));
        let column_types = vec![product_ty, TypeExpr::Scalar(ScalarType::I64)];
        let product = |value| Value::Product(BTreeMap::from([(field, Value::I64(value))]));
        let rows = vec![
            vec![product(1), Value::I64(10)],
            vec![product(2), Value::I64(20)],
        ];
        let mut relation = crate::NativeRelation::typed_from_rows(&rows, &column_types).unwrap();
        let crate::NativeRelation::TypedColumnar { columns, .. } = &relation else {
            panic!("typed compiler must produce TypedColumnar");
        };
        assert!(matches!(columns[0], crate::NativeColumn::Algebraic(_)));
        assert!(matches!(columns[1], crate::NativeColumn::I64(_)));
        assert_eq!(
            crate::native_relation::materialize_native_row(&relation, 0).unwrap(),
            rows[0]
        );
        crate::native_relation::remove_native_row(&mut relation, 0).unwrap();
        assert_eq!(
            crate::native_relation::materialize_native_row(&relation, 0).unwrap(),
            rows[1]
        );
        crate::native_relation::push_native_row(&mut relation, &vec![product(3), Value::I64(30)]).unwrap();
        assert_eq!(
            crate::native_relation::materialize_native_row(&relation, 1).unwrap(),
            vec![product(3), Value::I64(30)]
        );
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_sum_tag_filter_against_boxed_logical_variants() {
        use std::hint::black_box;
        use std::time::Instant;

        let left = SemanticId::new(1);
        let right = SemanticId::new(2);
        let ty = TypeExpr::Sum(BTreeMap::from([
            (left, TypeExpr::Scalar(ScalarType::I64)),
            (right, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let rows = (0_i64..100_000)
            .map(|value| Value::Variant {
                tag: if value % 3 == 0 { left } else { right },
                value: Box::new(Value::I64(value)),
            })
            .collect::<Vec<_>>();
        let native = AlgebraicNativeColumn::from_values(&rows, &ty).unwrap();
        let tags = native.sum_tags().unwrap();

        let start = Instant::now();
        let mut logical_hits = 0_usize;
        for _ in 0..10 {
            logical_hits += rows
                .iter()
                .filter(|row| matches!(row, Value::Variant { tag, .. } if *tag == left))
                .count();
        }
        black_box(logical_hits);
        let logical_ns = start.elapsed().as_nanos();

        let start = Instant::now();
        let mut native_hits = 0_usize;
        for _ in 0..10 {
            native_hits += tags.iter().filter(|&&tag| tag == left).count();
        }
        black_box(native_hits);
        let native_ns = start.elapsed().as_nanos();
        assert_eq!(logical_hits, native_hits);
        println!(
            "logical_ns={logical_ns} native_ns={native_ns} ratio_milli={}",
            logical_ns.saturating_mul(1_000) / native_ns.max(1)
        );
    }

    #[test]
    #[ignore = "diagnostic release benchmark"]
    fn benchmark_product_field_filter_against_logical_btree_rows() {
        use std::hint::black_box;
        use std::time::Instant;

        let a = SemanticId::new(1);
        let b = SemanticId::new(2);
        let c = SemanticId::new(3);
        let ty = TypeExpr::Product(BTreeMap::from([
            (a, TypeExpr::Scalar(ScalarType::I64)),
            (b, TypeExpr::Scalar(ScalarType::I64)),
            (c, TypeExpr::Scalar(ScalarType::I64)),
        ]));
        let rows = (0_i64..100_000)
            .map(|value| {
                Value::Product(BTreeMap::from([
                    (a, Value::I64(value)),
                    (b, Value::I64(value * 2)),
                    (c, Value::I64(value * 3)),
                ]))
            })
            .collect::<Vec<_>>();
        let native = AlgebraicNativeColumn::from_values(&rows, &ty).unwrap();
        let native_a = native.field(a).unwrap().as_i64().unwrap();

        let start = Instant::now();
        let mut logical_hits = 0_usize;
        for _ in 0..10 {
            for row in &rows {
                let Value::Product(fields) = row else {
                    unreachable!()
                };
                if matches!(fields.get(&a), Some(Value::I64(value)) if *value >= 50_000) {
                    logical_hits += 1;
                }
            }
        }
        black_box(logical_hits);
        let logical_ns = start.elapsed().as_nanos();

        let start = Instant::now();
        let mut native_hits = 0_usize;
        for _ in 0..10 {
            native_hits += native_a.iter().filter(|&&value| value >= 50_000).count();
        }
        black_box(native_hits);
        let native_ns = start.elapsed().as_nanos();
        assert_eq!(logical_hits, native_hits);
        println!(
            "logical_ns={logical_ns} native_ns={native_ns} ratio_milli={}",
            logical_ns.saturating_mul(1_000) / native_ns.max(1)
        );
    }
}
