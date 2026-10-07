use kernel_model::{FiniteModel, Value};
use kernel_query::{RelExpr, RelObservationForest, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{RevisionId, SchemaRevisionId, SemanticEnvId, SemanticId};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

fn permutations<T: Copy>(values: &[T]) -> Vec<Vec<T>> {
    fn rec<T: Copy>(i: usize, a: &mut [T], out: &mut Vec<Vec<T>>) {
        if i == a.len() {
            out.push(a.to_vec());
            return;
        }
        for j in i..a.len() {
            a.swap(i, j);
            rec(i + 1, a, out);
            a.swap(i, j);
        }
    }
    let mut a = values.to_vec();
    let mut out = Vec::new();
    rec(0, &mut a, &mut out);
    out
}

fn query(order: &[SemanticId], eq: SemanticId) -> RelExpr {
    let mut q = RelExpr::Scan(order[0]);
    for &r in &order[1..] {
        q = RelExpr::JoinEq {
            left: Box::new(q),
            right: Box::new(RelExpr::Scan(r)),
            left_column: 0,
            right_column: 0,
            equivalence: eq,
        };
    }
    RelExpr::Project {
        input: Box::new(q),
        columns: vec![0],
    }
}

fn fixture(
    rows: i64,
) -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    Vec<SemanticId>,
    FiniteModel,
) {
    let eq = SemanticId::new(990_000);
    let mut reg = SemanticRegistry::default();
    let digest = reg.install_equivalence(EquivalenceModule::I64Exact);
    let mut env = SemanticEnvironment::new(SemanticEnvId::new(990_000));
    env.pin_module(eq, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(990_000));
    let rels = (0..5)
        .map(|i| SemanticId::new(991_000 + i))
        .collect::<Vec<_>>();
    for &r in &rels {
        schema
            .define_relation(RelationDef {
                id: r,
                columns: vec![TypeExpr::Scalar(ScalarType::I64)],
                semantics: RelationSemantics::Set {
                    column_equivalences: vec![eq],
                },
            })
            .unwrap();
    }
    let ctx = SemanticContext {
        schema,
        environment: env,
    };
    let mut model = FiniteModel::default();
    for (i, &r) in rels.iter().enumerate() {
        let n = rows + i64::from(i > 0);
        model
            .relations
            .insert(r, (0..n).map(|x| vec![Value::I64(x)]).collect());
    }
    (ctx, reg, eq, rels, model)
}

fn med(mut v: Vec<Duration>) -> Duration {
    v.sort_unstable();
    v[v.len() / 2]
}

fn main() {
    let (ctx, reg, eq, rels, model) = fixture(512);
    let all = permutations(&rels)
        .into_iter()
        .map(|o| query(&o, eq))
        .collect::<Vec<_>>();
    for &n in &[1usize, 2, 6, 24, 120] {
        let roots = &all[..n];
        let mut builds = Vec::new();
        let mut deltas = Vec::new();
        let mut cells = 0usize;
        for _ in 0..9 {
            let t = Instant::now();
            let (mut forest, stats) =
                RelObservationForest::build_with_stats(roots, &model, &ctx, &reg).unwrap();
            builds.push(t.elapsed());
            cells = stats.unique_cells;
            forest.bind_revision(RevisionId::new(0)).unwrap();
            let rel = rels[0];
            let delta = RelationDelta {
                inserted: vec![vec![Value::I64(10_000)]],
                removed: Vec::new(),
                result_type: RelExpr::Scan(rel).typecheck(&ctx, &reg).unwrap(),
            };
            let ds = BTreeMap::from([(rel, delta)]);
            let t = Instant::now();
            let (_, effects, visited) = forest
                .candidate_from_relation_deltas_for_revision_with_stats(
                    RevisionId::new(1),
                    &ds,
                    &ctx,
                    &reg,
                )
                .unwrap();
            deltas.push(t.elapsed());
            assert_eq!(effects.len(), n);
            if n == 120 {
                eprintln!("work roots={n} cells={cells} visited={visited}");
            }
        }
        println!(
            "roots={n} cells={cells} build_ns={} delta_ns={}",
            med(builds).as_nanos(),
            med(deltas).as_nanos()
        );
    }
}
