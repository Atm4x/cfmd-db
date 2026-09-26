use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::{Duration, Instant};

use kernel_model::Value;
use kernel_plan::{
    I64IndexBinding, LayoutBinding, LayoutFamily, LayoutId, NativeColumn, NativeRelation,
    PhysicalCatalog, PhysicalStore, PreparedPlan, prepare_with_catalog,
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
const ITERATIONS: u32 = 12;

struct Fixture {
    prepared: PreparedPlan,
    ephemeral: PhysicalStore,
    persisted: PhysicalStore,
    registry: SemanticRegistry,
    keys: Vec<i64>,
    payloads: Vec<i64>,
    baseline_index: BTreeMap<i64, Vec<usize>>,
}

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn fixture() -> Fixture {
    let relation = sid(600);
    let equivalence = sid(601);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(60));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(60));
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
        id: LayoutId(60),
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
    let native = NativeRelation::typed_columnar(vec![
        NativeColumn::I64(keys.clone().into()),
        NativeColumn::I64(payloads.clone().into()),
    ])
    .unwrap();
    let mut ephemeral = PhysicalStore::default();
    ephemeral
        .install(relation, binding, native.clone())
        .unwrap();
    let mut persisted = PhysicalStore::default();
    persisted.install(relation, binding, native).unwrap();
    persisted
        .install_i64_index(
            I64IndexBinding {
                relation,
                layout: binding,
                key_column: 0,
                equivalence,
            },
            &context,
            &registry,
        )
        .unwrap();
    let mut baseline_index = BTreeMap::<i64, Vec<usize>>::new();
    for (row, key) in keys.iter().copied().enumerate() {
        baseline_index.entry(key).or_default().push(row);
    }
    Fixture {
        prepared,
        ephemeral,
        persisted,
        registry,
        keys,
        payloads,
        baseline_index,
    }
}

fn cfmd_round(
    prepared: &PreparedPlan,
    store: &PhysicalStore,
    registry: &SemanticRegistry,
) -> (Duration, usize) {
    let start = Instant::now();
    let mut total = 0usize;
    for _ in 0..ITERATIONS {
        let (value, stats) = prepared
            .execute_native_pinned(black_box(store), black_box(registry))
            .unwrap();
        total = total.wrapping_add(black_box(value.rows().len()));
        black_box(stats);
    }
    (start.elapsed(), total)
}

fn baseline_round(
    keys: &[i64],
    payloads: &[i64],
    index: &BTreeMap<i64, Vec<usize>>,
) -> (Duration, usize) {
    let start = Instant::now();
    let mut total = 0usize;
    for _ in 0..ITERATIONS {
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

fn ns(d: Duration) -> u128 {
    d.as_nanos() / u128::from(ITERATIONS)
}
fn median(v: &mut [u128]) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn main() {
    let f = fixture();
    let mut ephemeral = Vec::new();
    let mut persisted = Vec::new();
    let mut baseline = Vec::new();
    for _ in 0..ROUNDS {
        ephemeral.push(ns(cfmd_round(&f.prepared, &f.ephemeral, &f.registry).0));
        persisted.push(ns(cfmd_round(&f.prepared, &f.persisted, &f.registry).0));
        baseline.push(ns(baseline_round(&f.keys, &f.payloads, &f.baseline_index).0));
    }
    let e = median(&mut ephemeral);
    let p = median(&mut persisted);
    let b = median(&mut baseline);
    println!("rows={ROWS}");
    println!("ephemeral_ns_median={e}");
    println!("persisted_ns_median={p}");
    println!("baseline_prebuilt_ns_median={b}");
    println!(
        "persisted_over_baseline_milli={}",
        p.saturating_mul(1000) / b
    );
    println!(
        "persisted_over_ephemeral_milli={}",
        p.saturating_mul(1000) / e
    );
}
