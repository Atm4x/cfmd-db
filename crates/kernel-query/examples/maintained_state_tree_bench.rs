use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

use kernel_model::{FiniteModel, Value};
use kernel_query::{
    AggregateSpec, MaterializedJoinGroupTopKState, MaterializedRelPlanState, OrderDirection,
    RelExpr, RelationDelta,
};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

#[allow(clippy::too_many_lines)]
fn main() {
    let left = SemanticId::new(1);
    let right = SemanticId::new(2);
    let eq = SemanticId::new(3);
    let order = SemanticId::new(4);
    let mut registry = SemanticRegistry::default();
    let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
    let mut env = SemanticEnvironment::new(SemanticEnvId::new(1));
    env.pin_module(eq, eq_digest);
    env.pin_module(order, order_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1));
    for relation in [left, right] {
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
    let context = SemanticContext {
        schema,
        environment: env,
    };
    let join = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(left)),
        right: Box::new(RelExpr::Scan(right)),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    let group = RelExpr::Group {
        input: Box::new(join),
        group_columns: vec![0],
        group_equivalences: vec![eq],
        aggregate: AggregateSpec::Count {
            result_equivalence: eq,
        },
    };
    let query = RelExpr::TopKWithTies {
        input: Box::new(group),
        column: 0,
        ordering: order,
        direction: OrderDirection::Descending,
        k: 10,
    };
    for n in [1_000usize, 10_000, 50_000] {
        let rows = (0..n)
            .map(|i| vec![Value::I64(i64::try_from(i).unwrap())])
            .collect::<Vec<_>>();
        let mut model = FiniteModel::default();
        model.relations.insert(left, rows.clone());
        model.relations.insert(right, rows);
        let build_start = Instant::now();
        let mut state = MaterializedJoinGroupTopKState::build(&query, &model, &context, &registry)
            .unwrap()
            .unwrap();
        let build_ns = build_start.elapsed().as_nanos();
        println!("n={n} compositional_build_ns={build_ns}");
        let recursive_build_start = Instant::now();
        let mut recursive =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let recursive_build_ns = recursive_build_start.elapsed().as_nanos();
        println!("n={n} recursive_build_ns={recursive_build_ns}");

        let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
        let right_type = RelExpr::Scan(right).typecheck(&context, &registry).unwrap();
        let forward = RelationDelta {
            inserted: vec![vec![Value::I64(i64::try_from(n).unwrap() + 1)]],
            removed: vec![vec![Value::I64(0)]],
            result_type: left_type,
        };
        let empty = RelationDelta {
            inserted: vec![],
            removed: vec![],
            result_type: right_type,
        };
        let backward = RelationDelta {
            inserted: forward.removed.clone(),
            removed: forward.inserted.clone(),
            result_type: forward.result_type.clone(),
        };
        let rounds = 10;
        let update_start = Instant::now();
        for _ in 0..rounds {
            black_box(
                state
                    .apply_join_input_deltas(&forward, &empty, &context, &registry)
                    .unwrap(),
            );
            black_box(
                state
                    .apply_join_input_deltas(&backward, &empty, &context, &registry)
                    .unwrap(),
            );
        }
        let update_ns = u64::try_from(update_start.elapsed().as_nanos()).unwrap() / (rounds * 2);
        println!("n={n} maintained_tree_ns={update_ns}");

        let mut leaf = BTreeMap::new();
        let recursive_start = Instant::now();
        for _ in 0..rounds {
            leaf.insert(left, forward.clone());
            black_box(
                recursive
                    .apply_relation_deltas(&leaf, &context, &registry)
                    .unwrap(),
            );
            leaf.insert(left, backward.clone());
            black_box(
                recursive
                    .apply_relation_deltas(&leaf, &context, &registry)
                    .unwrap(),
            );
        }
        let recursive_ns =
            u64::try_from(recursive_start.elapsed().as_nanos()).unwrap() / (rounds * 2);
        println!("n={n} recursive_tree_ns={recursive_ns}");
    }
}
