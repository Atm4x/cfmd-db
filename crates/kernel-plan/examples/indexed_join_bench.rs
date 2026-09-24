use std::collections::BTreeMap;
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

const ROWS: usize = 20_000;
const ROUNDS: usize = 7;
const ITERATIONS: u32 = 8;

struct Fixture {
    prepared: PreparedPlan,
    store: PhysicalStore,
    registry: SemanticRegistry,
    keys: Vec<i64>,
    payloads: Vec<i64>,
}

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn fixture() -> Fixture {
    let relation = sid(500);
    let equivalence = sid(501);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(50));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(50));
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
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let logical = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(relation)),
        right: Box::new(RelExpr::Scan(relation)),
        left_column: 0,
        right_column: 0,
        equivalence,
    };
    let binding = LayoutBinding {
        id: LayoutId(50),
        family: LayoutFamily::Columnar,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, binding);
    let prepared = prepare_with_catalog(logical, &context, &registry, &catalog).unwrap();
    let keys = (0..ROWS)
        .map(|i| i64::try_from(i).unwrap())
        .collect::<Vec<_>>();
    let payloads = (0..ROWS)
        .map(|i| i64::try_from(i * 3).unwrap())
        .collect::<Vec<_>>();
    let mut store = PhysicalStore::default();
    store
        .install(
            relation,
            binding,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(keys.clone()),
                NativeColumn::I64(payloads.clone()),
            ])
            .unwrap(),
        )
        .unwrap();
    Fixture {
        prepared,
        store,
        registry,
        keys,
        payloads,
    }
}

fn cfmd_round(fixture: &Fixture) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0usize;
    for _ in 0..ITERATIONS {
        let (value, _) = fixture
            .prepared
            .execute_native_pinned(black_box(&fixture.store), black_box(&fixture.registry))
            .unwrap();
        rows = rows.wrapping_add(black_box(value.rows().len()));
    }
    (start.elapsed(), rows)
}

fn baseline_round(keys: &[i64], payloads: &[i64]) -> (Duration, usize) {
    let start = Instant::now();
    let mut total = 0usize;
    for _ in 0..ITERATIONS {
        let mut index = BTreeMap::<i64, Vec<usize>>::new();
        for (row, key) in keys.iter().copied().enumerate() {
            index.entry(key).or_default().push(row);
        }
        let mut out = Vec::with_capacity(keys.len());
        for (left, key) in keys.iter().copied().enumerate() {
            if let Some(right_rows) = index.get(&key) {
                for right in right_rows {
                    out.push(vec![
                        Value::I64(key),
                        Value::I64(payloads[left]),
                        Value::I64(key),
                        Value::I64(payloads[*right]),
                    ]);
                }
            }
        }
        total = total.wrapping_add(black_box(out.len()));
        black_box(out);
    }
    (start.elapsed(), total)
}

fn ns_per_iteration(duration: Duration) -> u128 {
    duration.as_nanos() / u128::from(ITERATIONS)
}

fn median(values: &mut [u128]) -> u128 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn main() {
    let fixture = fixture();
    let mut cfmd = Vec::new();
    let mut baseline = Vec::new();
    let mut cfmd_rows = 0usize;
    let mut baseline_rows = 0usize;
    for round in 0..ROUNDS {
        if round % 2 == 0 {
            let (elapsed, rows) = cfmd_round(&fixture);
            cfmd.push(ns_per_iteration(elapsed));
            cfmd_rows = cfmd_rows.wrapping_add(rows);
        }
        let (elapsed, rows) = baseline_round(&fixture.keys, &fixture.payloads);
        baseline.push(ns_per_iteration(elapsed));
        baseline_rows = baseline_rows.wrapping_add(rows);
        if round % 2 != 0 {
            let (elapsed, rows) = cfmd_round(&fixture);
            cfmd.push(ns_per_iteration(elapsed));
            cfmd_rows = cfmd_rows.wrapping_add(rows);
        }
    }
    assert_eq!(cfmd_rows, baseline_rows);
    let cfmd_min = *cfmd.iter().min().unwrap();
    let cfmd_max = *cfmd.iter().max().unwrap();
    let baseline_min = *baseline.iter().min().unwrap();
    let baseline_max = *baseline.iter().max().unwrap();
    let cfmd_median = median(&mut cfmd);
    let baseline_median = median(&mut baseline);
    println!("rows={ROWS}");
    println!("cfmd_ns_min={cfmd_min}");
    println!("cfmd_ns_median={cfmd_median}");
    println!("cfmd_ns_max={cfmd_max}");
    println!("baseline_ns_min={baseline_min}");
    println!("baseline_ns_median={baseline_median}");
    println!("baseline_ns_max={baseline_max}");
    println!(
        "cfmd_over_baseline_milli={}",
        cfmd_median.saturating_mul(1000) / baseline_median
    );
}
