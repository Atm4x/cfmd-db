use std::hint::black_box;
use std::time::Instant;

use kernel_model::Value;
use kernel_plan::{
    LayoutBinding, LayoutFamily, LayoutId, PhysicalCatalog, Plan, prepare_with_catalog,
};
use kernel_query::{AggregateSpec, OrderDirection, RelExpr};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

fn sid(value: u128) -> SemanticId {
    SemanticId::new(value)
}

fn structural_workload() -> RelExpr {
    RelExpr::PromoteToBag(Box::new(RelExpr::TopKWithTies {
        input: Box::new(RelExpr::Group {
            input: Box::new(RelExpr::Distinct {
                input: Box::new(RelExpr::Project {
                    input: Box::new(RelExpr::FilterEqConst {
                        input: Box::new(RelExpr::JoinEq {
                            left: Box::new(RelExpr::Scan(sid(1))),
                            right: Box::new(RelExpr::Scan(sid(2))),
                            left_column: 0,
                            right_column: 0,
                            equivalence: sid(10),
                        }),
                        column: 0,
                        value: Value::I64(7),
                        equivalence: sid(10),
                    }),
                    columns: vec![0, 1],
                }),
                column_equivalences: vec![sid(10), sid(10)],
            }),
            group_columns: vec![0],
            group_equivalences: vec![sid(10)],
            aggregate: AggregateSpec::Count {
                result_equivalence: sid(10),
            },
        }),
        column: 1,
        ordering: sid(20),
        direction: OrderDirection::Descending,
        k: 10,
    }))
}

fn typed_workload() -> (RelExpr, SemanticContext, SemanticRegistry, PhysicalCatalog) {
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
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(
        relation,
        LayoutBinding {
            id: LayoutId(3),
            family: LayoutFamily::Columnar,
        },
    );
    (logical, context, registry, catalog)
}

fn main() {
    const STRUCTURAL_ITERATIONS: u32 = 200_000;
    const PREPARE_ITERATIONS: u32 = 100_000;

    let logical = structural_workload();
    let mut catalog = PhysicalCatalog::default();
    catalog.bind_relation(
        sid(1),
        LayoutBinding {
            id: LayoutId(1),
            family: LayoutFamily::Columnar,
        },
    );
    catalog.bind_relation(
        sid(2),
        LayoutBinding {
            id: LayoutId(2),
            family: LayoutFamily::KeyValue,
        },
    );

    let start = Instant::now();
    let mut nodes = 0_usize;
    for _ in 0..STRUCTURAL_ITERATIONS {
        let plan = black_box(Plan::lower_with_catalog(
            black_box(&logical),
            black_box(&catalog),
        ));
        nodes = nodes.wrapping_add(black_box(plan.shape().nodes));
        black_box(plan.to_logical_expr());
    }
    let elapsed = start.elapsed();
    println!("structural_iterations={STRUCTURAL_ITERATIONS}");
    println!(
        "ns_per_lower_plus_roundtrip={}",
        elapsed.as_nanos() / u128::from(STRUCTURAL_ITERATIONS)
    );
    println!("observed_plan_nodes={nodes}");

    let (typed, context, registry, typed_catalog) = typed_workload();
    let start = Instant::now();
    for _ in 0..PREPARE_ITERATIONS {
        black_box(
            prepare_with_catalog(
                black_box(typed.clone()),
                black_box(&context),
                black_box(&registry),
                black_box(&typed_catalog),
            )
            .expect("typed benchmark query stays valid"),
        );
    }
    let elapsed = start.elapsed();
    println!("prepare_iterations={PREPARE_ITERATIONS}");
    println!(
        "ns_per_typecheck_checked_lowering={}",
        elapsed.as_nanos() / u128::from(PREPARE_ITERATIONS)
    );
}
