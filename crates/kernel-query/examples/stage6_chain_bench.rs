use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

use kernel_model::{FiniteModel, Value};
use kernel_query::{
    AggregateSpec, MaterializedRelPlanState, OrderDirection, RelExpr, RelationDelta,
};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

#[derive(Clone, Copy)]
struct Ids {
    left: SemanticId,
    right: SemanticId,
    blocker: SemanticId,
    eq: SemanticId,
    order: SemanticId,
}

fn context_and_registry() -> (Ids, SemanticContext, SemanticRegistry) {
    let ids = Ids {
        left: SemanticId::new(60_000),
        right: SemanticId::new(60_001),
        blocker: SemanticId::new(60_002),
        eq: SemanticId::new(60_003),
        order: SemanticId::new(60_004),
    };
    let mut registry = SemanticRegistry::default();
    let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(60_000));
    environment.pin_module(ids.eq, eq_digest);
    environment.pin_module(ids.order, order_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(60_000));
    schema
        .define_relation(RelationDef {
            id: ids.left,
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::I64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![ids.eq, ids.eq],
            },
        })
        .unwrap();
    for relation in [ids.right, ids.blocker] {
        schema
            .define_relation(RelationDef {
                id: relation,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![ids.eq],
                },
            })
            .unwrap();
    }
    (
        ids,
        SemanticContext {
            schema,
            environment,
        },
        registry,
    )
}

fn join_chain(ids: Ids) -> RelExpr {
    let filtered = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(ids.left)),
        column: 1,
        value: Value::I64(1),
        equivalence: ids.eq,
    };
    let projected = RelExpr::Project {
        input: Box::new(filtered),
        columns: vec![0],
    };
    let joined = RelExpr::JoinEq {
        left: Box::new(projected),
        right: Box::new(RelExpr::Scan(ids.right)),
        left_column: 0,
        right_column: 0,
        equivalence: ids.eq,
    };
    let grouped = RelExpr::Group {
        input: Box::new(joined),
        group_columns: vec![0],
        group_equivalences: vec![ids.eq],
        aggregate: AggregateSpec::Count {
            result_equivalence: ids.eq,
        },
    };
    RelExpr::TopKWithTies {
        input: Box::new(grouped),
        column: 0,
        ordering: ids.order,
        direction: OrderDirection::Ascending,
        k: 10,
    }
}

fn blocker_chain(ids: Ids) -> RelExpr {
    let anti = RelExpr::AntiJoin {
        left: Box::new(RelExpr::Scan(ids.left)),
        right: Box::new(RelExpr::Scan(ids.blocker)),
        left_column: 0,
        right_column: 0,
        equivalence: ids.eq,
    };
    let projected = RelExpr::Project {
        input: Box::new(anti),
        columns: vec![0],
    };
    let grouped = RelExpr::Group {
        input: Box::new(projected),
        group_columns: vec![0],
        group_equivalences: vec![ids.eq],
        aggregate: AggregateSpec::Count {
            result_equivalence: ids.eq,
        },
    };
    RelExpr::TopKWithTies {
        input: Box::new(grouped),
        column: 0,
        ordering: ids.order,
        direction: OrderDirection::Ascending,
        k: 10,
    }
}

fn model(ids: Ids, rows: i64) -> FiniteModel {
    let mut model = FiniteModel::default();
    model.relations.insert(
        ids.left,
        (0..rows)
            .map(|i| vec![Value::I64(i), Value::I64(1)])
            .collect(),
    );
    model
        .relations
        .insert(ids.right, (0..rows).map(|i| vec![Value::I64(i)]).collect());
    model.relations.insert(ids.blocker, Vec::new());
    model
}

fn leaf_delta(
    query: &RelExpr,
    relation: SemanticId,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    from: Vec<Value>,
    to: Vec<Value>,
) -> RelationDelta {
    let scan = RelExpr::Scan(relation);
    assert!(query.scan_relations().contains(&relation));
    RelationDelta {
        removed: vec![from],
        inserted: vec![to],
        result_type: scan.typecheck(context, registry).unwrap(),
    }
}

fn blocker_delta(
    relation: SemanticId,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    inserted: Vec<Vec<Value>>,
    removed: Vec<Vec<Value>>,
) -> RelationDelta {
    RelationDelta {
        inserted,
        removed,
        result_type: RelExpr::Scan(relation)
            .typecheck(context, registry)
            .unwrap(),
    }
}

fn run_pair(
    label: &str,
    mut state: MaterializedRelPlanState,
    forward: &BTreeMap<SemanticId, RelationDelta>,
    backward: &BTreeMap<SemanticId, RelationDelta>,
    context: &SemanticContext,
    registry: &SemanticRegistry,
    rounds: u64,
) {
    let mut times = Vec::with_capacity(usize::try_from(rounds * 2).unwrap());
    for _ in 0..rounds {
        for deltas in [forward, backward] {
            let start = Instant::now();
            let result = state
                .apply_relation_deltas(deltas, context, registry)
                .unwrap();
            let ns = u64::try_from(start.elapsed().as_nanos()).unwrap();
            black_box(result);
            times.push(ns);
        }
    }
    times.sort_unstable();
    let mid = times.len() / 2;
    println!(
        "stage6 label={label} samples={} median_ns={} p90_ns={}",
        times.len(),
        times[mid],
        times[(times.len() * 9 / 10).min(times.len() - 1)]
    );
}

#[allow(clippy::too_many_lines)]
fn main() {
    let (ids, context, registry) = context_and_registry();
    let rows = 50_000_i64;
    let old = model(ids, rows);

    let join_query = join_chain(ids);
    let join_state =
        MaterializedRelPlanState::build(&join_query, &old, &context, &registry).unwrap();
    let forward_left = leaf_delta(
        &join_query,
        ids.left,
        &context,
        &registry,
        vec![Value::I64(0), Value::I64(1)],
        vec![Value::I64(rows + 1), Value::I64(1)],
    );
    let backward_left = RelationDelta {
        inserted: forward_left.removed.clone(),
        removed: forward_left.inserted.clone(),
        result_type: forward_left.result_type.clone(),
    };
    let join_forward = BTreeMap::from([(ids.left, forward_left)]);
    let join_backward = BTreeMap::from([(ids.left, backward_left)]);
    run_pair(
        "linear_join_group_topk",
        join_state,
        &join_forward,
        &join_backward,
        &context,
        &registry,
        200,
    );

    let blocker_query = blocker_chain(ids);
    let blocker_state =
        MaterializedRelPlanState::build(&blocker_query, &old, &context, &registry).unwrap();
    let block = blocker_delta(
        ids.blocker,
        &context,
        &registry,
        vec![vec![Value::I64(0)]],
        Vec::new(),
    );
    let unblock = blocker_delta(
        ids.blocker,
        &context,
        &registry,
        Vec::new(),
        vec![vec![Value::I64(0)]],
    );
    let blocker_forward = BTreeMap::from([(ids.blocker, block)]);
    let blocker_backward = BTreeMap::from([(ids.blocker, unblock)]);
    run_pair(
        "blocker_group_topk",
        blocker_state,
        &blocker_forward,
        &blocker_backward,
        &context,
        &registry,
        200,
    );
}
