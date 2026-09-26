use std::hint::black_box;
use std::time::Instant;

use kernel_model::Value;
use kernel_plan::{
    I64IndexBinding, LayoutBinding, LayoutFamily, LayoutId, NativeColumn, NativeRelation,
    PhysicalStore,
};
use kernel_query::{RelExpr, RelationDelta};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn run(rows: usize, persisted_index: bool, remove_last: bool) -> u128 {
    let relation = sid(700);
    let equivalence = sid(701);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(70));
    environment.pin_module(equivalence, digest);
    let mut schema = Schema::new(SchemaRevisionId::new(70));
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
    let binding = LayoutBinding {
        id: LayoutId(70),
        family: LayoutFamily::Columnar,
    };
    let keys = (0..rows)
        .map(|i| i64::try_from(i).unwrap())
        .collect::<Vec<_>>();
    let payloads = (0..rows)
        .map(|i| i64::try_from(i * 3).unwrap())
        .collect::<Vec<_>>();
    let native = NativeRelation::typed_columnar(vec![
        NativeColumn::I64(keys.into()),
        NativeColumn::I64(payloads.into()),
    ])
    .unwrap();
    let result_type = RelExpr::Scan(relation)
        .typecheck(&context, &registry)
        .unwrap();
    let target = if remove_last {
        i64::try_from(rows - 1).unwrap()
    } else {
        0
    };
    let delta = RelationDelta {
        inserted: Vec::new(),
        removed: vec![vec![Value::I64(target), Value::I64(target * 3)]],
        result_type,
    };
    let mut samples = Vec::new();
    for _ in 0..7 {
        let mut store = PhysicalStore::default();
        store.install(relation, binding, native.clone()).unwrap();
        if persisted_index {
            store
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
        }
        let start = Instant::now();
        store
            .apply_relation_delta(relation, binding, black_box(&delta), &context, &registry)
            .unwrap();
        samples.push(start.elapsed().as_nanos());
        black_box(store);
    }
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn main() {
    for rows in [10_000_usize, 100_000, 300_000] {
        let first_no_index = run(rows, false, false);
        let first_with_index = run(rows, true, false);
        let last_no_index = run(rows, false, true);
        let last_with_index = run(rows, true, true);
        println!(
            "rows={rows} first_no_index_ns={first_no_index} first_persisted_index_ns={first_with_index} last_no_index_ns={last_no_index} last_persisted_index_ns={last_with_index}"
        );
    }
}
