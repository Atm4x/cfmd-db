use kernel_exact::ExactNatural;
use kernel_model::{FiniteModel, Value};
use kernel_query::{AggregateSpec, RelExpr, RelObservationForest, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{CanonicalEqKey, EquivalenceModule, SemanticRegistry};
use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

struct Fixture {
    context: SemanticContext,
    registry: SemanticRegistry,
    equality: SemanticId,
    relations: Vec<SemanticId>,
    model: FiniteModel,
    raw: Vec<Vec<Vec<Value>>>,
}

fn med(mut v: Vec<Duration>) -> Duration {
    v.sort_unstable();
    v[v.len() / 2]
}

fn fixture(keys: i64, fanout: i64) -> Fixture {
    let eq = SemanticId::new(1_090_000);
    let mut reg = SemanticRegistry::default();
    let digest = reg.install_equivalence(EquivalenceModule::I64Exact);
    let mut env = SemanticEnvironment::new(SemanticEnvId::new(1_090_000));
    env.pin_module(eq, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(1_090_000));
    let rels = (0..3)
        .map(|i| SemanticId::new(1_091_000 + i))
        .collect::<Vec<_>>();
    for &r in &rels {
        schema
            .define_relation(RelationDef {
                id: r,
                columns: vec![
                    TypeExpr::Scalar(ScalarType::I64),
                    TypeExpr::Scalar(ScalarType::I64),
                ],
                semantics: RelationSemantics::Bag {
                    column_equivalences: vec![eq, eq],
                },
            })
            .unwrap();
    }
    let ctx = SemanticContext {
        schema,
        environment: env,
    };
    let mut model = FiniteModel::default();
    let mut raw = Vec::new();
    for (ri, &r) in rels.iter().enumerate() {
        let mut rows = Vec::new();
        for k in 0..keys {
            for p in 0..fanout {
                rows.push(vec![
                    Value::I64(k),
                    Value::I64(i64::try_from(ri).unwrap() * 1_000_000 + k * fanout + p),
                ]);
            }
        }
        model.relations.insert(r, rows.clone());
        raw.push(rows);
    }
    Fixture {
        context: ctx,
        registry: reg,
        equality: eq,
        relations: rels,
        model,
        raw,
    }
}

fn query(rels: &[SemanticId], eq: SemanticId) -> RelExpr {
    let rs = RelExpr::JoinEq {
        left: Box::new(RelExpr::Scan(rels[0])),
        right: Box::new(RelExpr::Scan(rels[1])),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    let rst = RelExpr::JoinEq {
        left: Box::new(rs),
        right: Box::new(RelExpr::Scan(rels[2])),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    RelExpr::Group {
        input: Box::new(rst),
        group_columns: vec![],
        group_equivalences: vec![],
        aggregate: AggregateSpec::Count {
            result_equivalence: eq,
        },
    }
}

#[derive(Clone)]
struct FactorizedCount {
    equality: SemanticId,
    r: BTreeMap<CanonicalEqKey, ExactNatural>,
    s: BTreeMap<CanonicalEqKey, ExactNatural>,
    t: BTreeMap<CanonicalEqKey, ExactNatural>,
    total: ExactNatural,
}

impl FactorizedCount {
    fn counts(
        rows: &[Vec<Value>],
        equality: SemanticId,
        context: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> BTreeMap<CanonicalEqKey, ExactNatural> {
        let mut counts = BTreeMap::<CanonicalEqKey, ExactNatural>::new();
        for row in rows {
            let key = registry
                .canonical_equivalence_key(context, equality, &row[0])
                .unwrap();
            counts.entry(key).or_default().add_u128(1);
        }
        counts
    }

    fn build(
        raw: &[Vec<Vec<Value>>],
        equality: SemanticId,
        context: &SemanticContext,
        registry: &SemanticRegistry,
    ) -> Self {
        let r = Self::counts(&raw[0], equality, context, registry);
        let s = Self::counts(&raw[1], equality, context, registry);
        let t = Self::counts(&raw[2], equality, context, registry);
        let mut total = ExactNatural::zero();
        for (key, rv) in &r {
            let Some(sv) = s.get(key) else { continue };
            let Some(tv) = t.get(key) else { continue };
            let mut contribution = rv.clone();
            contribution.multiply_assign(sv);
            contribution.multiply_assign(tv);
            total.add_assign(&contribution);
        }
        Self {
            equality,
            r,
            s,
            t,
            total,
        }
    }

    fn insert_r(&mut self, key: &Value, context: &SemanticContext, registry: &SemanticRegistry) {
        let key = registry
            .canonical_equivalence_key(context, self.equality, key)
            .unwrap();
        let Some(sv) = self.s.get(&key) else {
            self.r.entry(key).or_default().add_u128(1);
            return;
        };
        let Some(tv) = self.t.get(&key) else {
            self.r.entry(key).or_default().add_u128(1);
            return;
        };
        let mut contribution = sv.clone();
        contribution.multiply_assign(tv);
        self.total.add_assign(&contribution);
        self.r.entry(key).or_default().add_u128(1);
    }
}

fn memory_probe(mode: &str, fanout: i64) {
    let fixture = fixture(32, fanout);
    if mode == "forest" {
        let q = query(&fixture.relations, fixture.equality);
        let (forest, stats) = RelObservationForest::build_with_stats(
            std::slice::from_ref(&q),
            &fixture.model,
            &fixture.context,
            &fixture.registry,
        )
        .unwrap();
        println!(
            "memory_mode=forest fanout={fanout} cells={}",
            stats.unique_cells
        );
        std::hint::black_box(forest);
    } else {
        let factor = FactorizedCount::build(
            &fixture.raw,
            fixture.equality,
            &fixture.context,
            &fixture.registry,
        );
        println!(
            "memory_mode=factor fanout={fanout} total={:?}",
            factor.total
        );
        std::hint::black_box(factor);
    }
    std::hint::black_box(fixture);
}

fn run_frontier(fanout: i64) {
    let fixture = fixture(32, fanout);
    let q = query(&fixture.relations, fixture.equality);
    let mut forest_build = Vec::new();
    let mut forest_delta = Vec::new();
    let mut factor_build = Vec::new();
    let mut factor_delta = Vec::new();
    let mut cells = 0usize;
    let mut visited = 0usize;
    for _ in 0..7 {
        let t = Instant::now();
        let (mut forest, stats) = RelObservationForest::build_with_stats(
            std::slice::from_ref(&q),
            &fixture.model,
            &fixture.context,
            &fixture.registry,
        )
        .unwrap();
        forest_build.push(t.elapsed());
        cells = stats.unique_cells;
        forest.bind_revision(RevisionId::new(0)).unwrap();
        let t = Instant::now();
        let mut f = FactorizedCount::build(
            &fixture.raw,
            fixture.equality,
            &fixture.context,
            &fixture.registry,
        );
        factor_build.push(t.elapsed());
        let insert = vec![Value::I64(0), Value::I64(9_999_999)];
        let delta = RelationDelta {
            inserted: vec![insert],
            removed: vec![],
            result_type: RelExpr::Scan(fixture.relations[0])
                .typecheck(&fixture.context, &fixture.registry)
                .unwrap(),
        };
        let ds = BTreeMap::from([(fixture.relations[0], delta)]);
        let t = Instant::now();
        let (_next, effects, v) = forest
            .candidate_from_relation_deltas_for_revision_with_stats(
                RevisionId::new(1),
                &ds,
                &fixture.context,
                &fixture.registry,
            )
            .unwrap();
        forest_delta.push(t.elapsed());
        visited = v;
        assert_eq!(effects.len(), 1);
        let before = f.total.clone();
        let t = Instant::now();
        f.insert_r(&Value::I64(0), &fixture.context, &fixture.registry);
        factor_delta.push(t.elapsed());
        let mut expected_delta = ExactNatural::zero();
        expected_delta.add_u128(u128::try_from(fanout * fanout).unwrap());
        let mut observed_delta = f.total.clone();
        assert!(observed_delta.checked_sub_assign(&before));
        assert_eq!(observed_delta, expected_delta);
    }
    println!(
        "fanout={fanout} rows_each={} logical_join_rows={} cells={cells} visited={visited} forest_build_ns={} factor_build_ns={} build_ratio={:.3} forest_delta_ns={} factor_delta_ns={} delta_ratio={:.3}",
        32 * fanout,
        32 * fanout * fanout * fanout,
        med(forest_build.clone()).as_nanos(),
        med(factor_build.clone()).as_nanos(),
        med(forest_build).as_secs_f64() / med(factor_build).as_secs_f64(),
        med(forest_delta.clone()).as_nanos(),
        med(factor_delta.clone()).as_nanos(),
        med(forest_delta).as_secs_f64() / med(factor_delta).as_secs_f64()
    );

    if fanout == 16 {
        run_batch_frontier(&fixture, &q, fanout);
    }
}

fn run_batch_frontier(fixture: &Fixture, q: &RelExpr, fanout: i64) {
    const BATCH: u64 = 256;
    let (mut forest, _) = RelObservationForest::build_with_stats(
        std::slice::from_ref(q),
        &fixture.model,
        &fixture.context,
        &fixture.registry,
    )
    .unwrap();
    forest.bind_revision(RevisionId::new(0)).unwrap();
    let mut factor = FactorizedCount::build(
        &fixture.raw,
        fixture.equality,
        &fixture.context,
        &fixture.registry,
    );
    let result_type = RelExpr::Scan(fixture.relations[0])
        .typecheck(&fixture.context, &fixture.registry)
        .unwrap();

    let t = Instant::now();
    for step in 1..=BATCH {
        let delta = RelationDelta {
            inserted: vec![vec![
                Value::I64(0),
                Value::I64(20_000_000 + i64::try_from(step).unwrap()),
            ]],
            removed: vec![],
            result_type: result_type.clone(),
        };
        let ds = BTreeMap::from([(fixture.relations[0], delta)]);
        let (next, effects, _) = forest
            .candidate_from_relation_deltas_for_revision_with_stats(
                RevisionId::new(step),
                &ds,
                &fixture.context,
                &fixture.registry,
            )
            .unwrap();
        assert_eq!(effects.len(), 1);
        forest = next;
    }
    let forest_batch = t.elapsed();

    let t = Instant::now();
    for _ in 0..BATCH {
        factor.insert_r(&Value::I64(0), &fixture.context, &fixture.registry);
    }
    let factor_batch = t.elapsed();
    let mut expected = FactorizedCount::build(
        &fixture.raw,
        fixture.equality,
        &fixture.context,
        &fixture.registry,
    )
    .total;
    let mut increment = ExactNatural::zero();
    increment.add_u128(u128::from(BATCH) * u128::try_from(fanout * fanout).unwrap());
    expected.add_assign(&increment);
    assert_eq!(factor.total, expected);
    println!(
        "batch_updates={BATCH} fanout={fanout} forest_total_ns={} factor_total_ns={} batch_delta_ratio={:.3}",
        forest_batch.as_nanos(),
        factor_batch.as_nanos(),
        forest_batch.as_secs_f64() / factor_batch.as_secs_f64()
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    if let Some(mode) = args.next()
        && matches!(mode.as_str(), "forest" | "factor")
    {
        let fanout = args
            .next()
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(16);
        memory_probe(&mode, fanout);
        return;
    }
    for &fanout in &[2_i64, 4, 8, 16, 24, 32] {
        run_frontier(fanout);
    }
}
