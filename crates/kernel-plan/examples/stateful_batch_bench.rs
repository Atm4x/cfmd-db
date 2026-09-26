use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::{Duration, Instant};

use kernel_plan::{
    LayoutBinding, LayoutFamily, LayoutId, NativeColumn, NativeRelation, PhysicalCatalog,
    PhysicalStore, PreparedPlan, prepare_with_catalog,
};
use kernel_query::{AggregateSpec, OrderDirection, RelExpr};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

const ROWS: usize = 100_000;
const ROUNDS: usize = 5;
const ITERATIONS: u32 = 10;

struct Fixture {
    group: PreparedPlan,
    top: PreparedPlan,
    store: PhysicalStore,
    registry: SemanticRegistry,
    keys: Vec<i64>,
    payload: Vec<i64>,
}

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn fixture() -> Fixture {
    let relation = sid(600);
    let equality = sid(601);
    let ordering = sid(602);
    let mut registry = SemanticRegistry::default();
    let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let ord_digest = registry.install_ordering(OrderingModule::I64Ascending);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(60));
    environment.pin_module(equality, eq_digest);
    environment.pin_module(ordering, ord_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(60));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::I64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equality, equality],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let group_expr = RelExpr::Group {
        input: Box::new(RelExpr::Scan(relation)),
        group_columns: vec![0],
        group_equivalences: vec![equality],
        aggregate: AggregateSpec::Count {
            result_equivalence: equality,
        },
    };
    let top_expr = RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Scan(relation)),
        column: 1,
        ordering,
        direction: OrderDirection::Ascending,
        k: 100,
    };
    let layout = LayoutBinding {
        id: LayoutId(600),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let group = prepare_with_catalog(group_expr, &context, &registry, &catalog).unwrap();
    let top = prepare_with_catalog(top_expr, &context, &registry, &catalog).unwrap();
    let keys = (0..ROWS)
        .map(|i| i64::try_from(i % 1024).unwrap())
        .collect::<Vec<_>>();
    let payload = (0..ROWS)
        .map(|i| {
            let x = u64::try_from(i)
                .unwrap()
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            i64::try_from(x % 50_000).unwrap()
        })
        .collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(keys.clone().into()),
                NativeColumn::I64(payload.clone().into()),
            ])
            .unwrap(),
        )
        .unwrap();
    Fixture {
        group,
        top,
        store,
        registry,
        keys,
        payload,
    }
}

fn run_native(f: &Fixture, plan: &PreparedPlan) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0usize;
    for _ in 0..ITERATIONS {
        let (result, stats) = plan
            .execute_native_pinned(black_box(&f.store), black_box(&f.registry))
            .unwrap();
        rows = rows.wrapping_add(black_box(result.rows().len()));
        black_box(stats);
    }
    (start.elapsed(), rows)
}

fn run_group_baseline(f: &Fixture) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0usize;
    for _ in 0..ITERATIONS {
        let mut slot = BTreeMap::<i64, usize>::new();
        let mut out = Vec::<(i64, i64)>::new();
        for &key in &f.keys {
            if let Some(&index) = slot.get(&key) {
                out[index].1 += 1;
            } else {
                let index = out.len();
                slot.insert(key, index);
                out.push((key, 1));
            }
        }
        rows = rows.wrapping_add(black_box(out.len()));
        black_box(out);
    }
    (start.elapsed(), rows)
}

fn run_top_baseline(f: &Fixture) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0usize;
    for _ in 0..ITERATIONS {
        let mut positions = (0..ROWS).collect::<Vec<_>>();
        let mut keys = f.payload.clone();
        let (_, threshold, _) = keys.select_nth_unstable(99);
        let threshold = *threshold;
        positions.retain(|&position| f.payload[position] <= threshold);
        positions.sort_by_key(|&position| f.payload[position]);
        rows = rows.wrapping_add(black_box(positions.len()));
        black_box(positions);
    }
    (start.elapsed(), rows)
}

fn per_iter(duration: Duration) -> u128 {
    duration.as_nanos() / u128::from(ITERATIONS)
}

fn median(mut values: Vec<u128>) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn measure(
    f: &Fixture,
    plan: &PreparedPlan,
    baseline: fn(&Fixture) -> (Duration, usize),
) -> (u128, u128) {
    let mut native = Vec::new();
    let mut hand = Vec::new();
    for round in 0..ROUNDS {
        let (a, na) = if round % 2 == 0 {
            run_native(f, plan)
        } else {
            baseline(f)
        };
        let (b, nb) = if round % 2 == 0 {
            baseline(f)
        } else {
            run_native(f, plan)
        };
        assert_eq!(na, nb);
        if round % 2 == 0 {
            native.push(per_iter(a));
            hand.push(per_iter(b));
        } else {
            hand.push(per_iter(a));
            native.push(per_iter(b));
        }
    }
    (median(native), median(hand))
}

fn ratio_milli(numerator: u128, denominator: u128) -> u128 {
    numerator.saturating_mul(1_000) / denominator.max(1)
}

fn main() {
    let fixture = fixture();
    let (_, group_stats) = fixture
        .group
        .execute_native_pinned(&fixture.store, &fixture.registry)
        .unwrap();
    let (_, top_stats) = fixture
        .top
        .execute_native_pinned(&fixture.store, &fixture.registry)
        .unwrap();
    println!("rows={ROWS}");
    println!(
        "group_typed_stateful_hits={}",
        group_stats.typed_stateful_batch_hits
    );
    println!(
        "top_typed_stateful_hits={}",
        top_stats.typed_stateful_batch_hits
    );
    let (group_native, group_base) = measure(&fixture, &fixture.group, run_group_baseline);
    println!("group_typed_ns={group_native}");
    println!("group_baseline_ns={group_base}");
    let group_ratio = ratio_milli(group_native, group_base);
    println!(
        "group_typed_over_baseline={}.{:03}",
        group_ratio / 1_000,
        group_ratio % 1_000
    );
    let (top_native, top_base) = measure(&fixture, &fixture.top, run_top_baseline);
    println!("top_typed_ns={top_native}");
    println!("top_baseline_ns={top_base}");
    let top_ratio = ratio_milli(top_native, top_base);
    println!(
        "top_typed_over_baseline={}.{:03}",
        top_ratio / 1_000,
        top_ratio % 1_000
    );
}
