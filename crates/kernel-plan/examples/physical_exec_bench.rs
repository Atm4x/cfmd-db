use std::hint::black_box;
use std::time::{Duration, Instant};

use kernel_model::Value;
use kernel_plan::{
    LayoutBinding, LayoutFamily, LayoutId, NativeColumn, NativeRelation, PhysicalCatalog,
    PhysicalStore, PreparedPlan, prepare_with_catalog,
};
use kernel_query::RelExpr;
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

const ROWS: usize = 100_000;
const ROUNDS: usize = 9;
const ITERATIONS_PER_ROUND: u32 = 30;

struct Fixture {
    prepared: PreparedPlan,
    store: PhysicalStore,
    registry: SemanticRegistry,
    predicate: Vec<i64>,
    payload: Vec<i64>,
}

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn build_fixture(use_mixed_typed_batch: bool) -> Fixture {
    let relation = sid(100);
    let equivalence = sid(101);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(1));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::I64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence, equivalence],
            },
        })
        .expect("benchmark schema is valid");
    let context = SemanticContext {
        schema,
        environment,
    };
    let logical = RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Scan(relation)),
            column: 0,
            value: Value::I64(7),
            equivalence,
        }),
        columns: vec![1],
    };
    let binding = LayoutBinding {
        id: LayoutId(1),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical, &context, &registry, &catalog)
        .expect("benchmark query is valid");
    let predicate = (0..ROWS)
        .map(|index| i64::try_from(index % 16).unwrap())
        .collect::<Vec<_>>();
    let payload = (0..ROWS)
        .map(|index| i64::try_from(index).unwrap())
        .collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    let data = if use_mixed_typed_batch {
        NativeRelation::typed_columnar(vec![
            NativeColumn::I64(predicate.clone().into()),
            NativeColumn::I64(payload.clone().into()),
        ])
        .unwrap()
    } else {
        NativeRelation::i64_columnar(vec![predicate.clone(), payload.clone()]).unwrap()
    };
    store.install(relation, binding, data).unwrap();
    Fixture {
        prepared,
        store,
        registry,
        predicate,
        payload,
    }
}

fn native_round(fixture: &Fixture) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0_usize;
    for _ in 0..ITERATIONS_PER_ROUND {
        let (value, _) = fixture
            .prepared
            .execute_native_pinned(black_box(&fixture.store), black_box(&fixture.registry))
            .unwrap();
        rows = rows.wrapping_add(black_box(value.rows().len()));
    }
    (start.elapsed(), rows)
}

fn baseline_round(predicate: &[i64], payload: &[i64]) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0_usize;
    for _ in 0..ITERATIONS_PER_ROUND {
        let mut out = Vec::new();
        for row_index in 0..ROWS {
            if black_box(predicate[row_index]) == 7 {
                out.push(vec![Value::I64(payload[row_index])]);
            }
        }
        rows = rows.wrapping_add(black_box(out.len()));
        black_box(out);
    }
    (start.elapsed(), rows)
}

fn ns_per_iteration(duration: Duration) -> u128 {
    duration.as_nanos() / u128::from(ITERATIONS_PER_ROUND)
}

fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn print_measurements(label: &str, fixture: &Fixture) {
    let (warm, stats) = fixture
        .prepared
        .execute_native_pinned(&fixture.store, &fixture.registry)
        .unwrap();
    black_box(warm);
    println!("mode={label}");
    println!("rows={ROWS}");
    println!("matches={}", stats.output_rows);
    println!("native_values_read={}", stats.values_read);
    println!("rounds={ROUNDS}");
    println!("iterations_per_round={ITERATIONS_PER_ROUND}");

    let mut native_ns = Vec::with_capacity(ROUNDS);
    let mut baseline_ns = Vec::with_capacity(ROUNDS);
    let mut native_rows = 0_usize;
    let mut baseline_rows = 0_usize;
    for round in 0..ROUNDS {
        let native_first = round % 2 == 0;
        if native_first {
            let (duration, rows) = native_round(fixture);
            native_ns.push(ns_per_iteration(duration));
            native_rows = native_rows.wrapping_add(rows);
        }
        let (duration, rows) = baseline_round(&fixture.predicate, &fixture.payload);
        baseline_ns.push(ns_per_iteration(duration));
        baseline_rows = baseline_rows.wrapping_add(rows);
        if !native_first {
            let (duration, rows) = native_round(fixture);
            native_ns.push(ns_per_iteration(duration));
            native_rows = native_rows.wrapping_add(rows);
        }
    }
    assert_eq!(native_rows, baseline_rows);

    let native_min = *native_ns.iter().min().unwrap();
    let native_max = *native_ns.iter().max().unwrap();
    let baseline_min = *baseline_ns.iter().min().unwrap();
    let baseline_max = *baseline_ns.iter().max().unwrap();
    let native_median = median(&mut native_ns);
    let baseline_median = median(&mut baseline_ns);
    let ratio_milli = native_median.saturating_mul(1000) / baseline_median;
    println!("native_ns_per_iter_min={native_min}");
    println!("native_ns_per_iter_median={native_median}");
    println!("native_ns_per_iter_max={native_max}");
    println!("baseline_ns_per_iter_min={baseline_min}");
    println!("baseline_ns_per_iter_median={baseline_median}");
    println!("baseline_ns_per_iter_max={baseline_max}");
    println!("native_over_baseline_milli={ratio_milli}");
}

fn main() {
    print_measurements("i64_specialized", &build_fixture(false));
    print_measurements("mixed_typed_batch", &build_fixture(true));
}
