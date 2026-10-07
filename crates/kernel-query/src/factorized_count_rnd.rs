use std::collections::{BTreeMap, BTreeSet};

use kernel_exact::ExactNatural;
use kernel_model::{FiniteModel, Value};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{CanonicalEqKey, EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

use crate::{AggregateSpec, RelExpr, RelationDelta};

#[derive(Debug, Clone)]
struct Binding {
    relation: SemanticId,
    column: usize,
    counts: BTreeMap<CanonicalEqKey, ExactNatural>,
}

#[derive(Debug, Clone)]
struct GammaFactorizedJoinCount {
    equivalence: SemanticId,
    bindings: Vec<Binding>,
    total: ExactNatural,
}

impl GammaFactorizedJoinCount {
    fn build(
        specs: &[(SemanticId, usize)],
        equivalence: SemanticId,
        model: &FiniteModel,
        context: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Self {
        let bindings = specs
            .iter()
            .map(|&(relation, column)| {
                let mut counts = BTreeMap::<CanonicalEqKey, ExactNatural>::new();
                for row in model.relations.get(&relation).into_iter().flatten() {
                    let key = registry
                        .canonical_equivalence_key(context, equivalence, &row[column])
                        .unwrap();
                    counts.entry(key).or_default().add_u128(1);
                }
                Binding {
                    relation,
                    column,
                    counts,
                }
            })
            .collect::<Vec<_>>();
        let total = Self::recompute_total(&bindings);
        Self {
            equivalence,
            bindings,
            total,
        }
    }

    fn contribution(bindings: &[Binding], key: &CanonicalEqKey) -> ExactNatural {
        let mut product = ExactNatural::one();
        for binding in bindings {
            let Some(count) = binding.counts.get(key) else {
                return ExactNatural::zero();
            };
            product.multiply_assign(count);
        }
        product
    }

    fn recompute_total(bindings: &[Binding]) -> ExactNatural {
        let Some(first) = bindings.first() else {
            return ExactNatural::zero();
        };
        let mut total = ExactNatural::zero();
        for key in first.counts.keys() {
            total.add_assign(&Self::contribution(bindings, key));
        }
        total
    }

    fn apply_delta(
        &mut self,
        relation: SemanticId,
        delta: &RelationDelta,
        context: &SemanticContext,
        registry: &SemanticRegistry,
    ) {
        let index = self
            .bindings
            .iter()
            .position(|b| b.relation == relation)
            .unwrap();
        let column = self.bindings[index].column;
        let mut removals = BTreeMap::<CanonicalEqKey, ExactNatural>::new();
        let mut insertions = BTreeMap::<CanonicalEqKey, ExactNatural>::new();
        for row in &delta.removed {
            let key = registry
                .canonical_equivalence_key(context, self.equivalence, &row[column])
                .unwrap();
            removals.entry(key).or_default().add_u128(1);
        }
        for row in &delta.inserted {
            let key = registry
                .canonical_equivalence_key(context, self.equivalence, &row[column])
                .unwrap();
            insertions.entry(key).or_default().add_u128(1);
        }
        let keys = removals
            .keys()
            .chain(insertions.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in keys {
            let old = Self::contribution(&self.bindings, &key);
            {
                let counts = &mut self.bindings[index].counts;
                let mut count = counts.get(&key).cloned().unwrap_or_default();
                if let Some(remove) = removals.get(&key) {
                    assert!(count.checked_sub_assign(remove));
                }
                if let Some(insert) = insertions.get(&key) {
                    count.add_assign(insert);
                }
                if count.is_zero() {
                    counts.remove(&key);
                } else {
                    counts.insert(key.clone(), count);
                }
            }
            let new = Self::contribution(&self.bindings, &key);
            assert!(self.total.checked_sub_assign(&old));
            self.total.add_assign(&new);
        }
    }
}

fn fixture() -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    [SemanticId; 3],
) {
    let eq = SemanticId::new(1_290_000);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1_290_000));
    environment.pin_module(eq, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1_290_000));
    let relations = [
        SemanticId::new(1_291_000),
        SemanticId::new(1_291_001),
        SemanticId::new(1_291_002),
    ];
    for relation in relations {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq],
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
        eq,
        relations,
    )
}

fn count_query(relations: [SemanticId; 3], eq: SemanticId) -> RelExpr {
    let first = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relations[0])),
        right: Box::new(RelExpr::Scan(relations[1])),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    let second = RelExpr::JoinEq {
        left: Box::new(first),
        right: Box::new(RelExpr::Scan(relations[2])),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    RelExpr::Group {
        input: Box::new(second),
        group_columns: vec![],
        group_equivalences: vec![],
        aggregate: AggregateSpec::Count {
            result_equivalence: eq,
        },
    }
}

fn model_from_counts(relations: [SemanticId; 3], counts: [[u8; 2]; 3]) -> FiniteModel {
    let mut model = FiniteModel::default();
    for (relation_index, relation) in relations.into_iter().enumerate() {
        let mut rows = Vec::new();
        for key in 0_i64..2 {
            for _ in 0..counts[relation_index][usize::try_from(key).unwrap()] {
                rows.push(vec![Value::I64(key)]);
            }
        }
        model.relations.insert(relation, rows);
    }
    model
}

fn evaluated_count(
    query: &RelExpr,
    model: &FiniteModel,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> u64 {
    let value = query.evaluate(model, context, registry).unwrap();
    let rows = value.rows();
    assert_eq!(rows.len(), 1);
    match rows[0].as_slice() {
        [Value::I64(value)] => u64::try_from(*value).unwrap(),
        _ => panic!("unexpected count row"),
    }
}

#[test]
fn gamma_factorized_count_matches_full_join_over_all_small_bag_worlds() {
    let (context, registry, eq, relations) = fixture();
    let query = count_query(relations, eq);
    for code in 0_u32..729 {
        let mut n = code;
        let mut counts = [[0_u8; 2]; 3];
        for relation in &mut counts {
            for key in relation {
                *key = u8::try_from(n % 3).unwrap();
                n /= 3;
            }
        }
        let model = model_from_counts(relations, counts);
        let state = GammaFactorizedJoinCount::build(
            &relations.map(|r| (r, 0)),
            eq,
            &model,
            &context,
            &registry,
        );
        assert_eq!(
            state.total.to_u64().unwrap(),
            evaluated_count(&query, &model, &context, &registry)
        );
    }
}

#[test]
fn gamma_factorized_count_delta_matches_rebuild() {
    let (context, registry, eq, relations) = fixture();
    let query = count_query(relations, eq);
    let mut model = model_from_counts(relations, [[2, 1], [1, 2], [2, 2]]);
    let mut state = GammaFactorizedJoinCount::build(
        &relations.map(|r| (r, 0)),
        eq,
        &model,
        &context,
        &registry,
    );
    let relation = relations[0];
    let delta = RelationDelta {
        inserted: vec![vec![Value::I64(1)]],
        removed: vec![vec![Value::I64(0)]],
        result_type: RelExpr::Scan(relation)
            .typecheck(&context, &registry)
            .unwrap(),
    };
    state.apply_delta(relation, &delta, &context, &registry);
    let rows = model.relations.get(&relation).unwrap().to_vec();
    let mut next = rows;
    let pos = next
        .iter()
        .position(|row| row == &vec![Value::I64(0)])
        .unwrap();
    next.remove(pos);
    next.push(vec![Value::I64(1)]);
    model.relations.insert(relation, next);
    assert_eq!(
        state.total.to_u64().unwrap(),
        evaluated_count(&query, &model, &context, &registry)
    );
}

fn grouped_count_query(relations: [SemanticId; 3], eq: SemanticId) -> RelExpr {
    let first = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relations[0])),
        right: Box::new(RelExpr::Scan(relations[1])),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    let second = RelExpr::JoinEq {
        left: Box::new(first),
        right: Box::new(RelExpr::Scan(relations[2])),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    RelExpr::Group {
        input: Box::new(second),
        group_columns: vec![0],
        group_equivalences: vec![eq],
        aggregate: AggregateSpec::Count {
            result_equivalence: eq,
        },
    }
}

impl GammaFactorizedJoinCount {
    fn grouped_counts(&self) -> BTreeMap<CanonicalEqKey, ExactNatural> {
        let Some(first) = self.bindings.first() else {
            return BTreeMap::new();
        };
        first
            .counts
            .keys()
            .filter_map(|key| {
                let contribution = Self::contribution(&self.bindings, key);
                (!contribution.is_zero()).then_some((key.clone(), contribution))
            })
            .collect()
    }
}

fn evaluated_grouped_counts(
    query: &RelExpr,
    model: &FiniteModel,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    eq: SemanticId,
) -> BTreeMap<CanonicalEqKey, ExactNatural> {
    query
        .evaluate(model, context, registry)
        .unwrap()
        .into_rows()
        .into_iter()
        .map(|row| {
            let key = registry
                .canonical_equivalence_key(context, eq, &row[0])
                .unwrap();
            let Value::I64(count) = row[1] else {
                panic!("unexpected grouped count payload")
            };
            let mut exact = ExactNatural::zero();
            exact.add_u128(u128::try_from(count).unwrap());
            (key, exact)
        })
        .collect()
}

#[test]
fn gamma_factorized_grouped_count_matches_full_join_over_all_small_bag_worlds() {
    let (context, registry, eq, relations) = fixture();
    let query = grouped_count_query(relations, eq);
    for code in 0_u32..729 {
        let mut n = code;
        let mut counts = [[0_u8; 2]; 3];
        for relation in &mut counts {
            for key in relation {
                *key = u8::try_from(n % 3).unwrap();
                n /= 3;
            }
        }
        let model = model_from_counts(relations, counts);
        let state = GammaFactorizedJoinCount::build(
            &relations.map(|r| (r, 0)),
            eq,
            &model,
            &context,
            &registry,
        );
        assert_eq!(
            state.grouped_counts(),
            evaluated_grouped_counts(&query, &model, &context, &registry, eq)
        );
    }
}
