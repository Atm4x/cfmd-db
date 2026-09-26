/// Prepared physical boundary for a grounded positive recursive query.
///
/// APNF/SAMF (or another certified finite grounding) owns carrier/rule
/// construction. The physical plan pins Γ and delegates least-support plus
/// exact N∞ proof-tree multiplicity to `kernel-fixpoint`; it never expands
/// multiplicities into repeated physical rows.
#[derive(Debug, Clone, PartialEq, Eq)]
// HOSTILE[P165][ACTIVE][CLEAN]: grounded positive recursion delegates compact least-support/N∞ semantics to kernel-fixpoint; no executor-side recursive row expansion.
pub struct PreparedPositiveRecursivePlan {
    call: kernel_query::FixpointCall,
    semantic_context: kernel_schema::SemanticContext,
}

impl PreparedPositiveRecursivePlan {
    pub fn compile(
        call: &kernel_query::FixpointCall,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<Self, kernel_query::RelQueryError> {
        call.typecheck(context, registry)?;
        Ok(Self {
            call: call.clone(),
            semantic_context: context.clone(),
        })
    }

    pub fn execute(
        &self,
        context: &kernel_schema::SemanticContext,
        registry: &kernel_semantics::SemanticRegistry,
    ) -> Result<kernel_query::CompactRecursiveBag, kernel_query::RelQueryError> {
        if context != &self.semantic_context {
            return Err(kernel_query::RelQueryError::SemanticRevisionMismatch);
        }
        self.call.evaluate_compact(context, registry)
    }
}

#[cfg(test)]
mod positive_recursive_plan_tests {
    use super::*;
    use kernel_model::Value;
    use kernel_query::{FixpointCall, PositiveRecursiveRowAtom, PositiveRecursiveRowRule, RelType};
    use kernel_schema::{
        RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment, TypeExpr,
    };
    use kernel_types::{SchemaRevisionId, SemanticEnvId};

    fn context() -> (SemanticContext, kernel_semantics::SemanticRegistry) {
        (
            SemanticContext {
                schema: Schema::new(SchemaRevisionId::new(90_001)),
                environment: SemanticEnvironment::new(SemanticEnvId::new(90_001)),
            },
            kernel_semantics::SemanticRegistry::default(),
        )
    }

    fn bag_type() -> RelType {
        RelType {
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: Vec::new(),
            },
        }
    }

    #[test]
    fn finite_recursive_plan_matches_independent_dag_oracle() {
        // Independent oracle equations:
        // a=2; b=3*a=6; c=5 + 2*a*b=29.
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(10)],
                    seed_multiplicity: 2,
                },
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(20)],
                    seed_multiplicity: 0,
                },
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(30)],
                    seed_multiplicity: 5,
                },
            ],
            rules: vec![
                PositiveRecursiveRowRule {
                    body: vec![0],
                    head: 1,
                    coefficient: 3,
                },
                PositiveRecursiveRowRule {
                    body: vec![0, 1],
                    head: 2,
                    coefficient: 2,
                },
            ],
        };
        let (context, registry) = context();
        let plan = PreparedPositiveRecursivePlan::compile(&call, &context, &registry).unwrap();
        let result = plan.execute(&context, &registry).unwrap();
        let expected = [2_u64, 6, 29];
        for ((_, actual), expected) in result.entries().iter().zip(expected) {
            assert_eq!(
                *actual,
                kernel_fixpoint::NaturalInfinity::finite_u64(expected)
            );
        }
    }

    #[test]
    fn hostile_productive_cycle_is_compact_infinity_not_executor_loop() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(1)],
                    seed_multiplicity: 1,
                },
                PositiveRecursiveRowAtom {
                    row: vec![Value::I64(2)],
                    seed_multiplicity: 0,
                },
            ],
            rules: vec![
                PositiveRecursiveRowRule {
                    body: vec![0],
                    head: 1,
                    coefficient: 1,
                },
                PositiveRecursiveRowRule {
                    body: vec![1],
                    head: 0,
                    coefficient: 1,
                },
            ],
        };
        let (context, registry) = context();
        let plan = PreparedPositiveRecursivePlan::compile(&call, &context, &registry).unwrap();
        let result = plan.execute(&context, &registry).unwrap();
        assert!(
            result
                .entries()
                .iter()
                .all(|(_, weight)| weight.is_infinite())
        );
        assert_eq!(
            result.require_finite(),
            Err(kernel_query::RelQueryError::NonFiniteRecursiveMultiplicity)
        );
    }

    #[test]
    fn prepared_recursive_plan_rejects_gamma_drift() {
        let call = FixpointCall {
            result_type: bag_type(),
            atoms: vec![PositiveRecursiveRowAtom {
                row: vec![Value::I64(1)],
                seed_multiplicity: 1,
            }],
            rules: Vec::new(),
        };
        let (context, registry) = context();
        let plan = PreparedPositiveRecursivePlan::compile(&call, &context, &registry).unwrap();
        let changed = SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(90_002)),
            environment: context.environment.clone(),
        };
        assert_eq!(
            plan.execute(&changed, &registry),
            Err(kernel_query::RelQueryError::SemanticRevisionMismatch)
        );
    }
}
