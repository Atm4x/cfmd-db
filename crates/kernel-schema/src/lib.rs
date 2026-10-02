mod context;
mod definitions;
mod rules;
mod schema;
mod subtype;
mod types;

pub use context::{ContextError, ModuleDigest, SemanticContext, SemanticEnvironment};
pub use definitions::{
    CapabilityDef, FieldDef, FieldRule, RelationDef, RelationSemantics, StructuralEquivalenceDef,
    StructuralOrderingDef,
};
pub use rules::{RuleValueExpr, SemanticRuleExpr, SemanticRuleTypeError, TextPattern};
pub use schema::{Schema, SchemaError};
pub use subtype::SubtypeClosure;
pub use types::{ScalarType, Symbol, SymbolKind, TypeError, TypeExpr, TypeVar};

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

    use super::*;

    #[test]
    fn rename_preserves_semantic_identity() {
        let id = SemanticId::new(41);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define(Symbol {
                id,
                kind: SymbolKind::Field,
                presentation_name: "name".into(),
            })
            .unwrap();

        schema.rename(id, "display_name").unwrap();

        let symbol = schema.symbol(id).unwrap();
        assert_eq!(symbol.id, id);
        assert_eq!(symbol.presentation_name, "display_name");
    }

    #[test]
    fn semantic_environment_is_explicitly_versioned() {
        let module = SemanticId::new(7);
        let digest = ModuleDigest([3; 32]);
        let mut env = SemanticEnvironment::new(SemanticEnvId::new(5));
        env.pin_module(module, digest);
        assert_eq!(env.module(module), Some(digest));
    }

    #[test]
    fn recursive_document_type_is_guarded_and_valid() {
        let x = TypeVar(0);
        let json = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Sum(BTreeMap::from([
                (SemanticId::new(1), TypeExpr::Scalar(ScalarType::I64)),
                (
                    SemanticId::new(2),
                    TypeExpr::Seq(Box::new(TypeExpr::Var(x))),
                ),
            ]))),
        };
        assert_eq!(json.validate(), Ok(()));
    }

    #[test]
    fn naked_recursive_variable_is_rejected() {
        let x = TypeVar(0);
        let invalid = TypeExpr::Mu {
            binder: x,
            body: Box::new(TypeExpr::Var(x)),
        };
        assert_eq!(invalid.validate(), Err(TypeError::UnguardedRecursion(x)));
    }

    #[test]
    fn free_type_variable_is_rejected() {
        let free = TypeExpr::Var(TypeVar(99));
        assert_eq!(free.validate(), Err(TypeError::FreeVariable(TypeVar(99))));
    }

    #[test]
    fn open_capability_is_not_closed_sum() {
        let capability_id = SemanticId::new(100);
        let field_id = SemanticId::new(101);
        let implementation = SemanticId::new(200);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_capability(CapabilityDef {
                id: capability_id,
                required_fields: BTreeMap::from([(field_id, TypeExpr::Scalar(ScalarType::Text))]),
            })
            .unwrap();
        schema.include(implementation, capability_id).unwrap();

        assert!(schema.capability(capability_id).is_some());
        assert!(schema.has_direct_inclusion(implementation, capability_id));
    }

    #[test]
    fn subtype_diamond_is_coherent_but_cycles_are_rejected() {
        let bottom = SemanticId::new(1);
        let left = SemanticId::new(2);
        let right = SemanticId::new(3);
        let top = SemanticId::new(4);
        let mut schema = Schema::new(SchemaRevisionId::new(1));

        schema.include(bottom, left).unwrap();
        schema.include(bottom, right).unwrap();
        schema.include(left, top).unwrap();
        schema.include(right, top).unwrap();

        assert!(schema.is_subtype(bottom, top));
        assert_eq!(
            schema.include(top, bottom),
            Err(SchemaError::SubtypeCycle {
                subtype: top,
                supertype: bottom,
            })
        );
    }

    #[test]
    fn subtype_bridge_updates_existing_descendants_and_ancestors() {
        let a = SemanticId::new(10);
        let b = SemanticId::new(11);
        let c = SemanticId::new(12);
        let d = SemanticId::new(13);
        let mut schema = Schema::new(SchemaRevisionId::new(1));

        schema.include(a, b).unwrap();
        schema.include(c, d).unwrap();
        schema.include(b, c).unwrap();

        assert!(schema.is_subtype(a, d));
        assert_eq!(
            schema
                .subtype_closure()
                .ancestors(a)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([a, b, c, d])
        );
    }

    #[test]
    fn context_requires_all_semantic_modules_referenced_by_schema() {
        let equality = SemanticId::new(700);
        let type_id = SemanticId::new(701);
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        schema
            .define_type(
                type_id,
                TypeExpr::Set {
                    element: Box::new(TypeExpr::Scalar(ScalarType::Text)),
                    equivalence: equality,
                },
            )
            .unwrap();
        let mut context = SemanticContext {
            schema,
            environment: SemanticEnvironment::new(SemanticEnvId::new(1)),
        };
        assert_eq!(
            context.validate(),
            Err(ContextError::MissingSemanticModule(equality))
        );
        context
            .environment
            .pin_module(equality, ModuleDigest([9; 32]));
        assert_eq!(context.validate(), Ok(()));
    }
    #[test]
    fn set_relation_requires_one_equivalence_per_column() {
        let mut schema = Schema::new(SchemaRevisionId::new(1));
        assert_eq!(
            schema.define_relation(RelationDef {
                id: SemanticId::new(10),
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![SemanticId::new(20)],
                },
            }),
            Err(SchemaError::RelationEquivalenceArityMismatch)
        );
    }

    #[test]
    fn subtype_adjacency_matches_direct_relation_scan_reference() {
        fn reference(schema: &Schema, subtype: SemanticId, supertype: SemanticId) -> bool {
            if subtype == supertype {
                return true;
            }
            let mut frontier = vec![subtype];
            let mut seen = BTreeSet::new();
            while let Some(current) = frontier.pop() {
                if !seen.insert(current) {
                    continue;
                }
                for (child, parent) in schema.inclusions() {
                    if child == current {
                        if parent == supertype {
                            return true;
                        }
                        frontier.push(parent);
                    }
                }
            }
            false
        }

        let ids = (0_u64..40)
            .map(|raw| SemanticId::new(u128::from(90_000_u64 + raw)))
            .collect::<Vec<_>>();
        let mut schema = Schema::new(SchemaRevisionId::new(90_000));
        for left in 0..ids.len() {
            for right in left + 1..ids.len() {
                if (left * 11 + right * 7) % 13 == 0 {
                    schema.include(ids[left], ids[right]).unwrap();
                    schema.include(ids[left], ids[right]).unwrap();
                }
            }
        }
        for &subtype in &ids {
            for &supertype in &ids {
                assert_eq!(
                    schema.is_subtype(subtype, supertype),
                    reference(&schema, subtype, supertype)
                );
            }
        }
    }
}

#[cfg(test)]
mod semantic_rule_persistence_tests {
    use super::*;
    use kernel_types::{SchemaRevisionId, SemanticId};
    use std::collections::BTreeSet;

    #[test]
    fn full_boolean_field_rule_and_entity_field_coordinates_share_type_law() {
        let person = SemanticId::new(901);
        let name = SemanticId::new(902);
        let age = SemanticId::new(903);
        let mut schema = Schema::new(SchemaRevisionId::new(9));
        schema
            .define_field(FieldDef {
                id: name,
                owner: person,
                value: TypeExpr::Scalar(ScalarType::Text),
            })
            .unwrap();
        schema
            .define_field(FieldDef {
                id: age,
                owner: person,
                value: TypeExpr::Scalar(ScalarType::I64),
            })
            .unwrap();

        schema
            .add_field_rule(
                name,
                FieldRule::Expr(SemanticRuleExpr::And(vec![
                    SemanticRuleExpr::TextLength {
                        value: RuleValueExpr::Input,
                        min: 2,
                        max: Some(32),
                    },
                    SemanticRuleExpr::Not(Box::new(SemanticRuleExpr::TextOneOf {
                        value: RuleValueExpr::Input,
                        allowed: BTreeSet::from(["forbidden".to_owned()]),
                    })),
                ])),
            )
            .unwrap();

        schema
            .add_entity_rule(
                person,
                SemanticRuleExpr::And(vec![
                    SemanticRuleExpr::TextLength {
                        value: RuleValueExpr::Field(name),
                        min: 2,
                        max: Some(32),
                    },
                    SemanticRuleExpr::I64Range {
                        value: RuleValueExpr::Field(age),
                        min: Some(0),
                        max: Some(150),
                    },
                ]),
            )
            .unwrap();

        assert_eq!(schema.entity_rules(person).len(), 1);
    }
}
