use std::{collections::BTreeMap, hint::black_box, time::Instant};

use kernel_model::{FiniteModel, Value};
use kernel_query::{MaterializedTopKDeltaState, OrderDirection, RelExpr, RelType, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

fn fixture(
    rows: i64,
) -> (
    SemanticContext,
    SemanticRegistry,
    MaterializedTopKDeltaState,
    RelationDelta,
    RelationDelta,
    RelType,
) {
    let relation = SemanticId::new(32_000);
    let i64_eq = SemanticId::new(32_001);
    let i64_order = SemanticId::new(32_002);
    let mut registry = SemanticRegistry::default();
    let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(32_000));
    environment.pin_module(i64_eq, eq_digest);
    environment.pin_module(i64_order, order_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(32_000));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![i64_eq],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let query = RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Scan(relation)),
        column: 0,
        ordering: i64_order,
        direction: OrderDirection::Ascending,
        k: 10,
    };
    let mut model = FiniteModel::default();
    model
        .relations
        .insert(relation, (0..rows).map(|i| vec![Value::I64(i)]).collect());
    let state = MaterializedTopKDeltaState::build(&query, &model, &context, &registry)
        .unwrap()
        .unwrap();
    let input_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let result_type = query.typecheck(&context, &registry).unwrap();
    let forward = RelationDelta {
        removed: vec![vec![Value::I64(0)]],
        inserted: vec![vec![Value::I64(rows)]],
        result_type: input_type.clone(),
    };
    let backward = RelationDelta {
        removed: forward.inserted.clone(),
        inserted: forward.removed.clone(),
        result_type: input_type,
    };
    (context, registry, state, forward, backward, result_type)
}

fn selected_with_ties(map: &BTreeMap<i64, usize>, k: usize) -> BTreeMap<i64, usize> {
    let mut selected = BTreeMap::new();
    let mut rows = 0usize;
    for (&key, &count) in map {
        if rows >= k {
            break;
        }
        selected.insert(key, count);
        rows = rows.saturating_add(count);
    }
    selected
}

fn hand_step(map: &mut BTreeMap<i64, usize>, from: i64, to: i64, result_type: &RelType) {
    let before = selected_with_ties(map, 10);
    let removed_count = map.remove(&from).unwrap();
    assert_eq!(removed_count, 1);
    assert!(map.insert(to, 1).is_none());
    let after = selected_with_ties(map, 10);

    let mut removed = Vec::new();
    let mut inserted = Vec::new();
    for (&key, &count) in &before {
        let next = after.get(&key).copied().unwrap_or(0);
        for _ in 0..count.saturating_sub(next) {
            removed.push(vec![Value::I64(key)]);
        }
    }
    for (&key, &count) in &after {
        let previous = before.get(&key).copied().unwrap_or(0);
        for _ in 0..count.saturating_sub(previous) {
            inserted.push(vec![Value::I64(key)]);
        }
    }
    black_box(RelationDelta {
        inserted,
        removed,
        result_type: result_type.clone(),
    });
}

fn full_replay_step(values: &mut Vec<i64>, from: i64, to: i64, k: usize) {
    let position = values.iter().position(|value| *value == from).unwrap();
    values.remove(position);
    values.push(to);
    let mut sorted = values.clone();
    sorted.sort_unstable();
    black_box(sorted[..k.min(sorted.len())].to_vec());
}

fn main() {
    let rows = 50_000_i64;
    let (context, registry, mut state, forward, backward, result_type) = fixture(rows);
    let mut baseline: BTreeMap<i64, usize> = (0..rows).map(|i| (i, 1)).collect();
    let rounds = 7;
    let iters = 1000;
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
            hand_step(&mut baseline, 0, rows, &result_type);
            hand_step(&mut baseline, rows, 0, &result_type);
        }
        hand.push(start.elapsed().as_nanos() / (iters * 2));
    }
    maintained.sort_unstable();
    hand.sort_unstable();
    let mid = rounds / 2;
    println!("rows={rows} k=10 rounds={rounds} iterations_per_round={iters}");
    println!("maintained_median_ns={}", maintained[mid]);
    println!("baseline_median_ns={}", hand[mid]);
    println!(
        "ratio_milli={}",
        maintained[mid].saturating_mul(1000) / hand[mid].max(1)
    );

    let mut replay_values: Vec<i64> = (0..rows).collect();
    let replay_iters = 20_u128;
    let start = Instant::now();
    for _ in 0..replay_iters {
        full_replay_step(&mut replay_values, 0, rows, 10);
        full_replay_step(&mut replay_values, rows, 0, 10);
    }
    let replay_ns = start.elapsed().as_nanos() / (replay_iters * 2);
    println!("full_replay_median_like_ns={replay_ns}");
    println!(
        "maintained_vs_full_replay_milli={}",
        maintained[mid].saturating_mul(1000) / replay_ns.max(1)
    );
}
