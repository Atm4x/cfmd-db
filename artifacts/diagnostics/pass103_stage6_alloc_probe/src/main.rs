use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};

use kernel_model::{FiniteModel, Value};
use kernel_query::{
    AggregateSpec, MaterializedRelPlanState, OrderDirection, RelExpr, RelationDelta,
};
use kernel_schema::{
    RelationDef, RelationSemantics, ScalarType, Schema, SemanticContext, SemanticEnvironment,
    TypeExpr,
};
use kernel_semantics::{EquivalenceModule, OrderingModule, SemanticRegistry};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

struct CountingAllocator;
static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct Fixture {
    context: SemanticContext,
    registry: SemanticRegistry,
    query: RelExpr,
    model: FiniteModel,
    forward: BTreeMap<SemanticId, RelationDelta>,
    backward: BTreeMap<SemanticId, RelationDelta>,
}

fn setup(rows: i64) -> Fixture {
    let left = SemanticId::new(61_000);
    let right = SemanticId::new(61_001);
    let eq = SemanticId::new(61_002);
    let order = SemanticId::new(61_003);
    let mut registry = SemanticRegistry::default();
    let eq_digest = registry.install_equivalence(EquivalenceModule::I64Exact);
    let order_digest = registry.install_ordering(OrderingModule::I64Ascending);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(61_000));
    environment.pin_module(eq, eq_digest);
    environment.pin_module(order, order_digest);
    let mut schema = Schema::new(SchemaRevisionId::new(61_000));
    schema
        .define_relation(RelationDef {
            id: left,
            columns: vec![
                TypeExpr::Scalar(ScalarType::I64),
                TypeExpr::Scalar(ScalarType::I64),
            ],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![eq, eq],
            },
        })
        .unwrap();
    schema
        .define_relation(RelationDef {
            id: right,
            columns: vec![TypeExpr::Scalar(ScalarType::I64)],
            semantics: RelationSemantics::Bag {
                column_equivalences: vec![eq],
            },
        })
        .unwrap();
    let context = SemanticContext {
        schema,
        environment,
    };
    let filtered = RelExpr::FilterEqConst {
        input: Box::new(RelExpr::Scan(left)),
        column: 1,
        value: Value::I64(1),
        equivalence: eq,
    };
    let projected = RelExpr::Project {
        input: Box::new(filtered),
        columns: vec![0],
    };
    let joined = RelExpr::JoinEq {
        left: Box::new(projected),
        right: Box::new(RelExpr::Scan(right)),
        left_column: 0,
        right_column: 0,
        equivalence: eq,
    };
    let grouped = RelExpr::Group {
        input: Box::new(joined),
        group_columns: vec![0],
        group_equivalences: vec![eq],
        aggregate: AggregateSpec::Count {
            result_equivalence: eq,
        },
    };
    let query = RelExpr::TopKWithTies {
        input: Box::new(grouped),
        column: 0,
        ordering: order,
        direction: OrderDirection::Ascending,
        k: 10,
    };
    let mut model = FiniteModel::default();
    model.relations.insert(
        left,
        (0..rows)
            .map(|i| vec![Value::I64(i), Value::I64(1)])
            .collect(),
    );
    model
        .relations
        .insert(right, (0..rows).map(|i| vec![Value::I64(i)]).collect());
    let left_type = RelExpr::Scan(left).typecheck(&context, &registry).unwrap();
    let forward = RelationDelta {
        removed: vec![vec![Value::I64(0), Value::I64(1)]],
        inserted: vec![vec![Value::I64(rows + 1), Value::I64(1)]],
        result_type: left_type.clone(),
    };
    let backward = RelationDelta {
        inserted: forward.removed.clone(),
        removed: forward.inserted.clone(),
        result_type: left_type,
    };
    Fixture {
        context,
        registry,
        query,
        model,
        forward: BTreeMap::from([(left, forward)]),
        backward: BTreeMap::from([(left, backward)]),
    }
}

fn main() {
    let fixture = setup(50_000);
    let mut state = MaterializedRelPlanState::build(
        &fixture.query,
        &fixture.model,
        &fixture.context,
        &fixture.registry,
    )
    .unwrap();
    let mut samples = Vec::new();
    for _ in 0..200 {
        for delta in [&fixture.forward, &fixture.backward] {
            ALLOCS.store(0, Ordering::SeqCst);
            let out = state
                .apply_relation_deltas(delta, &fixture.context, &fixture.registry)
                .unwrap();
            samples.push(ALLOCS.load(Ordering::SeqCst));
            black_box(out);
        }
    }
    samples.sort_unstable();
    println!(
        "stage6_alloc samples={} median_alloc_calls={} p90_alloc_calls={} max_alloc_calls={}",
        samples.len(),
        samples[samples.len() / 2],
        samples[samples.len() * 9 / 10],
        samples[samples.len() - 1]
    );
}
