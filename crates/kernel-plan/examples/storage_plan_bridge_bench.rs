use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

use kernel_model::{FiniteModel, Value};
use kernel_plan::{
    I64IndexBinding, LayoutBinding, LayoutFamily, LayoutId, NativeColumn, NativeRelation,
    PhysicalStore,
};
use kernel_query::{MaterializedRelPlanState, RelExpr, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn setup(
    rows: usize,
) -> (
    SemanticContext,
    SemanticRegistry,
    SemanticId,
    LayoutBinding,
    FiniteModel,
    NativeRelation,
    RelationDelta,
) {
    let relation = sid(27_000);
    let equivalence = sid(27_001);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(27));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(27));
    schema
        .define_relation(RelationDef {
            id: relation,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![equivalence],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let rows_data = (0..rows)
        .map(|index| vec![Value::I64(i64::try_from(index).unwrap())])
        .collect::<Vec<_>>();
    let mut model = FiniteModel::default();
    model.relations.insert(relation, rows_data);
    let native = NativeRelation::typed_columnar(vec![NativeColumn::I64(
        (0..rows)
            .map(|index| i64::try_from(index).unwrap())
            .collect(),
    )])
    .unwrap();
    let query = RelExpr::Scan(relation);
    let result_type = query.typecheck(&context, &registry).unwrap();
    let delta = RelationDelta {
        inserted: Vec::new(),
        removed: vec![vec![Value::I64(i64::try_from(rows - 1).unwrap())]],
        result_type,
    };
    (
        context,
        registry,
        relation,
        LayoutBinding {
            id: LayoutId(27),
            family: LayoutFamily::Columnar,
        },
        model,
        native,
        delta,
    )
}

fn median(mut samples: Vec<u128>) -> u128 {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn run(rows: usize) -> (u128, u128, u128) {
    let (context, registry, relation, binding, model, native, delta) = setup(rows);
    let query = RelExpr::Scan(relation);
    let mut semantic_samples = Vec::new();
    let mut certified_samples = Vec::new();
    let mut end_to_end_samples = Vec::new();

    for _ in 0..7 {
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let mut deltas = BTreeMap::new();
        deltas.insert(relation, delta.clone());
        let start = Instant::now();
        state
            .apply_relation_deltas(black_box(&deltas), &context, &registry)
            .unwrap();
        semantic_samples.push(start.elapsed().as_nanos());
        black_box(state);
    }

    for _ in 0..7 {
        let mut store = PhysicalStore::default();
        store.install(relation, binding, native.clone()).unwrap();
        store
            .install_i64_index(
                I64IndexBinding {
                    relation,
                    layout: binding,
                    key_column: 0,
                    equivalence: sid(27_001),
                },
                &context,
                &registry,
            )
            .unwrap();
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let rows = store.logical_rows_with_handles(relation, binding).unwrap();
        state.attach_storage_rows(relation, &rows).unwrap();
        let certified = store
            .apply_relation_delta_resolved(relation, binding, &delta, &context, &registry)
            .unwrap();
        let mut certified_map = BTreeMap::new();
        certified_map.insert(relation, certified);
        let start = Instant::now();
        state
            .apply_storage_resolved_deltas(black_box(&certified_map), &context, &registry)
            .unwrap();
        certified_samples.push(start.elapsed().as_nanos());
        black_box(state);
    }

    for _ in 0..7 {
        let mut store = PhysicalStore::default();
        store.install(relation, binding, native.clone()).unwrap();
        store
            .install_i64_index(
                I64IndexBinding {
                    relation,
                    layout: binding,
                    key_column: 0,
                    equivalence: sid(27_001),
                },
                &context,
                &registry,
            )
            .unwrap();
        let mut state =
            MaterializedRelPlanState::build(&query, &model, &context, &registry).unwrap();
        let rows = store.logical_rows_with_handles(relation, binding).unwrap();
        state.attach_storage_rows(relation, &rows).unwrap();
        let start = Instant::now();
        let certified = store
            .apply_relation_delta_resolved(
                relation,
                binding,
                black_box(&delta),
                &context,
                &registry,
            )
            .unwrap();
        let mut certified_map = BTreeMap::new();
        certified_map.insert(relation, certified);
        state
            .apply_storage_resolved_deltas(&certified_map, &context, &registry)
            .unwrap();
        end_to_end_samples.push(start.elapsed().as_nanos());
        black_box((state, store));
    }

    (
        median(semantic_samples),
        median(certified_samples),
        median(end_to_end_samples),
    )
}

fn main() {
    for rows in [1_000_usize, 10_000, 50_000] {
        let (semantic, certified, end_to_end) = run(rows);
        println!(
            "rows={rows} semantic_scan_ns={semantic} certified_scan_ns={certified} storage_plus_certified_ns={end_to_end}"
        );
    }
}
