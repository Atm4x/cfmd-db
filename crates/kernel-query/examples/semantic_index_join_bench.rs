use std::{hint::black_box, time::Instant};

use kernel_model::{FiniteModel, Value};
use kernel_query::{MaterializedJoinDeltaState, RelExpr, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

struct Fixture {
    context: SemanticContext,
    registry: SemanticRegistry,
    state: MaterializedJoinDeltaState,
    left_type: kernel_query::RelType,
    right_keys: Vec<String>,
}

fn fixture(rows: usize) -> Fixture {
    let left = SemanticId::new(39_100);
    let right = SemanticId::new(39_101);
    let text_eq = SemanticId::new(39_102);
    let i64_eq = SemanticId::new(39_103);

    let mut registry = SemanticRegistry::default();
    let text_digest = registry.install_equivalence(EquivalenceModule::TextAsciiCaseInsensitive);
    let i64_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(39_100));
    environment.pin_module(text_eq, text_digest);
    environment.pin_module(i64_eq, i64_digest);

    let mut schema = Schema::new(SchemaRevisionId::new(39_100));
    for relation in [left, right] {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::Text),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![text_eq, i64_eq],
                },
            })
            .unwrap();
    }
    let context = SemanticContext {
        schema,
        environment,
    };
    let query = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
        left_column: 0,
        right_column: 0,
        equivalence: text_eq,
    };

    let right_keys: Vec<String> = (0..rows).map(|i| format!("KEY-{i}")).collect();
    let mut model = FiniteModel::default();
    model
        .relations
        .insert(left, vec![vec![Value::Text("key-0".into()), Value::I64(0)]]);
    model.relations.insert(
        right,
        right_keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                vec![
                    Value::Text(key.clone()),
                    Value::I64(i64::try_from(i).expect("benchmark row count fits i64")),
                ]
            })
            .collect(),
    );
    let state = MaterializedJoinDeltaState::build(&query, &model, &context, &registry)
        .unwrap()
        .unwrap();
    let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();

    Fixture {
        context,
        registry,
        state,
        left_type,
        right_keys,
    }
}

fn delta(from_payload: i64, to_payload: i64, left_type: &kernel_query::RelType) -> RelationDelta {
    RelationDelta {
        removed: vec![vec![Value::Text("key-0".into()), Value::I64(from_payload)]],
        inserted: vec![vec![Value::Text("KEY-0".into()), Value::I64(to_payload)]],
        result_type: left_type.clone(),
    }
}

fn empty_delta(
    right: SemanticId,
    context: &SemanticContext,
    registry: &SemanticRegistry,
) -> RelationDelta {
    RelationDelta {
        inserted: Vec::new(),
        removed: Vec::new(),
        result_type: RelExpr::Scan(right).typecheck(context, registry).unwrap(),
    }
}

fn scan_probe(right_keys: &[String], needle: &str) -> usize {
    let mut matches = 0;
    for key in right_keys {
        if key.eq_ignore_ascii_case(needle) {
            matches += 1;
        }
    }
    black_box(matches)
}

fn run(rows: usize, rounds: usize, iterations: usize) {
    let right = SemanticId::new(39_101);
    let mut fixture = fixture(rows);
    let forward = delta(0, 1, &fixture.left_type);
    let backward = RelationDelta {
        removed: forward.inserted.clone(),
        inserted: forward.removed.clone(),
        result_type: fixture.left_type.clone(),
    };
    let empty_right = empty_delta(right, &fixture.context, &fixture.registry);

    let mut indexed_samples = Vec::with_capacity(rounds);
    let mut scan_samples = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let started = Instant::now();
        for _ in 0..iterations {
            black_box(
                fixture
                    .state
                    .apply_input_deltas(&forward, &empty_right, &fixture.context, &fixture.registry)
                    .unwrap(),
            );
            black_box(
                fixture
                    .state
                    .apply_input_deltas(
                        &backward,
                        &empty_right,
                        &fixture.context,
                        &fixture.registry,
                    )
                    .unwrap(),
            );
        }
        indexed_samples.push(started.elapsed().as_nanos() / (iterations as u128 * 2));

        let started = Instant::now();
        for _ in 0..iterations {
            black_box(scan_probe(&fixture.right_keys, "KEY-0"));
            black_box(scan_probe(&fixture.right_keys, "key-0"));
        }
        scan_samples.push(started.elapsed().as_nanos() / (iterations as u128 * 2));
    }

    indexed_samples.sort_unstable();
    scan_samples.sort_unstable();
    let indexed = indexed_samples[rounds / 2];
    let scan = scan_samples[rounds / 2];
    println!(
        "rows={rows},indexed_median_ns={indexed},legacy_scan_median_ns={scan},scan_over_indexed_x1000={}",
        scan.saturating_mul(1000) / indexed.max(1)
    );
}

fn main() {
    println!("workload=TextAsciiCaseInsensitive,left_rows=1,delta_rows=1,rounds=7");
    run(1_000, 7, 300);
    run(10_000, 7, 100);
    run(50_000, 7, 30);
}
