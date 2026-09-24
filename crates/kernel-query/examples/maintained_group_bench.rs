use std::{collections::BTreeMap, hint::black_box, time::Instant};

use kernel_model::{FiniteModel, Value};
use kernel_query::{AggregateSpec, MaterializedGroupDeltaState, RelExpr, RelType, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

type Fixture = (
    SemanticContext,
    SemanticRegistry,
    RelExpr,
    MaterializedGroupDeltaState,
    RelationDelta,
    RelationDelta,
    RelType,
);

fn fixture(rows: i64) -> Fixture {
    let relation = SemanticId::new(31_000);
    let i64_eq = SemanticId::new(31_001);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(31_000));
    environment.pin_module(i64_eq, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(31_000));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::I64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![i64_eq, i64_eq],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let query = RelExpr::Group {
        input: Box::new(RelExpr::Scan(relation)),
        group_columns: vec![0],
        group_equivalences: vec![i64_eq],
        aggregate: AggregateSpec::Count {
            result_equivalence: i64_eq,
        },
    };
    let mut model = FiniteModel::default();
    model.relations.insert(
        relation,
        (0..rows)
            .map(|i| vec![Value::I64(i), Value::I64(i)])
            .collect(),
    );
    let state = MaterializedGroupDeltaState::build(&query, &model, &context, &registry)
        .unwrap()
        .unwrap();
    let input_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let result_type = query.typecheck(&context, &registry).unwrap();
    let forward = RelationDelta {
        removed: vec![vec![Value::I64(rows - 1), Value::I64(rows - 1)]],
        inserted: vec![vec![Value::I64(rows), Value::I64(rows)]],
        result_type: input_type.clone(),
    };
    let backward = RelationDelta {
        removed: forward.inserted.clone(),
        inserted: forward.removed.clone(),
        result_type: input_type,
    };
    (
        context,
        registry,
        query,
        state,
        forward,
        backward,
        result_type,
    )
}

fn hand_step(baseline: &mut BTreeMap<i64, u64>, from: i64, to: i64, result_type: &RelType) {
    let old = baseline.remove(&from).unwrap();
    baseline.insert(to, old);
    black_box(RelationDelta {
        removed: vec![vec![Value::I64(from), Value::I64(1)]],
        inserted: vec![vec![Value::I64(to), Value::I64(1)]],
        result_type: result_type.clone(),
    });
}

fn main() {
    let rows = 50_000_i64;
    let (context, registry, _query, mut state, forward, backward, result_type) = fixture(rows);
    let mut baseline: BTreeMap<i64, u64> = (0..rows).map(|i| (i, 1)).collect();
    let rounds = 7;
    let iters = 100;
    let mut maintained = Vec::new();
    let mut hand = Vec::new();
    for _ in 0..rounds {
        let start = Instant::now();
        for _ in 0..iters {
            black_box(
                state
                    .apply_input_delta(&forward, &context, &registry)
                    .unwrap(),
            );
            black_box(
                state
                    .apply_input_delta(&backward, &context, &registry)
                    .unwrap(),
            );
        }
        maintained.push(start.elapsed().as_nanos() / (iters * 2));

        let start = Instant::now();
        for _ in 0..iters {
            hand_step(&mut baseline, rows - 1, rows, &result_type);
            hand_step(&mut baseline, rows, rows - 1, &result_type);
        }
        hand.push(start.elapsed().as_nanos() / (iters * 2));
    }
    maintained.sort_unstable();
    hand.sort_unstable();
    let mid = rounds / 2;
    println!("groups={rows} rounds={rounds} iterations_per_round={iters}");
    println!("maintained_median_ns={}", maintained[mid]);
    println!("baseline_median_ns={}", hand[mid]);
    println!(
        "ratio_milli={}",
        maintained[mid].saturating_mul(1000) / hand[mid].max(1)
    );
}
