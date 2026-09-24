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
const ROUNDS: usize = 7;
const ITERATIONS: u32 = 20;

struct Fixture {
    typed_prepared: PreparedPlan,
    row_prepared: PreparedPlan,
    typed_store: PhysicalStore,
    row_store: PhysicalStore,
    registry: SemanticRegistry,
    key: Vec<i64>,
    secondary: Vec<i64>,
    payload: Vec<i64>,
}

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn materialized_row_relation(key: &[i64], secondary: &[i64], payload: &[i64]) -> NativeRelation {
    NativeRelation::row_store(
        key.iter()
            .zip(secondary)
            .zip(payload)
            .map(|((&a, &b), &c)| vec![Value::I64(a), Value::I64(b), Value::I64(c)])
            .collect(),
    )
}

fn logical_query(relation: SemanticId, equivalence: SemanticId) -> RelExpr {
    RelExpr::Project {
        input: Box::new(RelExpr::FilterEqConst {
            input: Box::new(RelExpr::Project {
                input: Box::new(RelExpr::FilterEqConst {
                    input: Box::new(RelExpr::Scan(relation)),
                    column: 0,
                    value: Value::I64(7),
                    equivalence,
                }),
                columns: vec![1, 2, 0],
            }),
            column: 0,
            value: Value::I64(2),
            equivalence,
        }),
        columns: vec![1],
    }
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
                TypeExpr::Scalar(ScalarType::I64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence; 3],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let logical = logical_query(relation, equivalence);
    let layout = LayoutBinding {
        id: LayoutId(500),
        family: LayoutFamily::Columnar,
    };
    let row_layout = LayoutBinding {
        id: LayoutId(501),
        family: LayoutFamily::RowStore,
    };
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(relation, layout);
    let typed_prepared =
        prepare_with_catalog(logical.clone(), &context, &registry, &catalog).unwrap();
    let mut row_catalog = PhysicalCatalog::default();
    row_catalog.bind_relation(relation, row_layout);
    let row_prepared = prepare_with_catalog(logical, &context, &registry, &row_catalog).unwrap();

    let key = (0..ROWS)
        .map(|i| i64::try_from(i % 16).unwrap())
        .collect::<Vec<_>>();
    let secondary = (0..ROWS)
        .map(|i| i64::try_from((i / 16) % 4).unwrap())
        .collect::<Vec<_>>();
    let payload = (0..ROWS)
        .map(|i| i64::try_from(i).unwrap())
        .collect::<Vec<_>>();

    let mut typed_store = PhysicalStore::default();
    typed_store
        .install(
            relation,
            layout,
            NativeRelation::typed_columnar(vec![
                NativeColumn::I64(key.clone()),
                NativeColumn::I64(secondary.clone()),
                NativeColumn::I64(payload.clone()),
            ])
            .unwrap(),
        )
        .unwrap();
    let mut row_store = PhysicalStore::default();
    row_store
        .install(
            relation,
            row_layout,
            materialized_row_relation(&key, &secondary, &payload),
        )
        .unwrap();

    Fixture {
        typed_prepared,
        row_prepared,
        typed_store,
        row_store,
        registry,
        key,
        secondary,
        payload,
    }
}

fn run_native(f: &Fixture, prepared: &PreparedPlan, store: &PhysicalStore) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0usize;
    for _ in 0..ITERATIONS {
        let (value, _) = prepared
            .execute_native_pinned(black_box(store), &f.registry)
            .unwrap();
        rows = rows.wrapping_add(black_box(value.rows().len()));
    }
    (start.elapsed(), rows)
}

fn run_baseline(f: &Fixture) -> (Duration, usize) {
    let start = Instant::now();
    let mut rows = 0usize;
    for _ in 0..ITERATIONS {
        let mut out = Vec::new();
        for i in 0..ROWS {
            if black_box(f.key[i]) == 7 && black_box(f.secondary[i]) == 2 {
                out.push(vec![Value::I64(f.payload[i])]);
            }
        }
        rows = rows.wrapping_add(black_box(out.len()));
        black_box(out);
    }
    (start.elapsed(), rows)
}

fn per_iter(d: Duration) -> u128 {
    d.as_nanos() / u128::from(ITERATIONS)
}

fn median(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

fn main() {
    let f = fixture();
    let (_, stats) = f
        .typed_prepared
        .execute_native_pinned(&f.typed_store, &f.registry)
        .unwrap();
    println!("rows={ROWS}");
    println!("matches={}", stats.output_rows);
    println!("typed_batch_chain_hits={}", stats.typed_batch_chain_hits);
    println!("typed_values_read={}", stats.values_read);

    let mut typed = Vec::new();
    let mut row = Vec::new();
    let mut base = Vec::new();
    for round in 0..ROUNDS {
        let (d1, n1) = if round % 2 == 0 {
            run_native(&f, &f.typed_prepared, &f.typed_store)
        } else {
            run_native(&f, &f.row_prepared, &f.row_store)
        };
        let (d2, n2) = run_baseline(&f);
        let (d3, n3) = if round % 2 == 0 {
            run_native(&f, &f.row_prepared, &f.row_store)
        } else {
            run_native(&f, &f.typed_prepared, &f.typed_store)
        };
        assert_eq!(n1, n2);
        assert_eq!(n2, n3);
        if round % 2 == 0 {
            typed.push(per_iter(d1));
            row.push(per_iter(d3));
        } else {
            row.push(per_iter(d1));
            typed.push(per_iter(d3));
        }
        base.push(per_iter(d2));
    }
    let typed = median(typed);
    let row = median(row);
    let base = median(base);
    println!("typed_batch_ns_median={typed}");
    println!("row_materialized_ns_median={row}");
    println!("handwritten_ns_median={base}");
    println!("typed_over_row_milli={}", typed.saturating_mul(1000) / row);
    println!(
        "typed_over_handwritten_milli={}",
        typed.saturating_mul(1000) / base
    );
}
