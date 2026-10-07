use std::{hint::black_box, time::Instant};

use kernel_model::Value;
use kernel_persistent::PersistentOrdMap;
use kernel_schema::{Schema, SemanticContext, SemanticEnvironment};
use kernel_semantics::{
    CanonicalEqKey, EquivalenceModule, SemanticRegistry,
    fiber_retention::SharedRowKeyMassRetention,
    observable::{ObservableClassSignature, RevisionObservableCatalog},
    support_atom::SupportAtomFabric,
};
use kernel_types::{SchemaRevisionId, SemanticEnvId, SemanticId};

const ROWS: usize = 4096;
const DISTINCT: usize = 257;
const READS: usize = 200_000;
const REPETITIONS: usize = 7;

struct Fixture {
    owned: PersistentOrdMap<u64, Vec<CanonicalEqKey>>,
    shared: SharedRowKeyMassRetention<u64, CanonicalEqKey>,
    catalog: RevisionObservableCatalog,
    fabric: SupportAtomFabric<u64>,
}

fn context_for(module: EquivalenceModule) -> (SemanticContext, SemanticRegistry, SemanticId) {
    let equivalence = SemanticId::new(10);
    let mut registry = SemanticRegistry::default();
    let digest = registry.install_equivalence(module);
    let mut environment = SemanticEnvironment::new(SemanticEnvId::new(9));
    environment.pin_module(equivalence, digest);
    (
        SemanticContext {
            schema: Schema::new(SchemaRevisionId::new(7)),
            environment,
        },
        registry,
        equivalence,
    )
}

fn build_fixture(module: EquivalenceModule, value: impl Fn(usize) -> Value) -> Fixture {
    let (context, registry, equivalence) = context_for(module);
    let mut catalog = RevisionObservableCatalog::new(&context).unwrap();
    let observable = catalog
        .register_equivalence(&registry, &context, equivalence)
        .unwrap();
    let product = catalog.register_product(vec![observable]).unwrap();
    let mut fabric = SupportAtomFabric::new(&catalog, product, vec![observable]).unwrap();

    let mut classes = Vec::with_capacity(DISTINCT);
    let mut product_classes = Vec::with_capacity(DISTINCT);
    let mut keys = Vec::with_capacity(DISTINCT);
    for distinct in 0..DISTINCT {
        let class = catalog
            .observe_value(&registry, &context, observable, &value(distinct))
            .unwrap();
        let key = match &catalog.class_record(class).unwrap().signature {
            ObservableClassSignature::Canonical(key) => key.clone(),
            ObservableClassSignature::Product(_) => unreachable!(),
        };
        classes.push(class);
        product_classes.push(catalog.intern_product_class(product, vec![class]).unwrap());
        keys.push(key);
    }

    let mut owned = PersistentOrdMap::default();
    let mut shared = SharedRowKeyMassRetention::default();
    for row in 0..ROWS {
        let distinct = row % DISTINCT;
        let row = row as u64;
        owned.insert(row, vec![keys[distinct].clone()]);
        shared.insert(row, vec![keys[distinct].clone()]).unwrap();
        fabric
            .insert(
                &catalog,
                row,
                product_classes[distinct],
                &[classes[distinct]],
            )
            .unwrap();
    }

    Fixture {
        owned,
        shared,
        catalog,
        fabric,
    }
}

fn sampled_row(iteration: usize) -> u64 {
    ((iteration.wrapping_mul(2_654_435_761) ^ (iteration >> 7)) % ROWS) as u64
}

fn measure(mut probe: impl FnMut(u64) -> CanonicalEqKey) -> f64 {
    let started = Instant::now();
    for iteration in 0..READS {
        black_box(probe(sampled_row(iteration)));
    }
    started.elapsed().as_secs_f64() * 1_000_000_000.0 / f64::from(u32::try_from(READS).unwrap())
}

fn run_case(name: &str, fixture: &Fixture) {
    let mut owned = Vec::with_capacity(REPETITIONS);
    let mut shared = Vec::with_capacity(REPETITIONS);
    let mut samf = Vec::with_capacity(REPETITIONS);

    for _ in 0..REPETITIONS {
        owned.push(measure(|row| {
            fixture.owned.get(&row).unwrap().first().unwrap().clone()
        }));
        shared.push(measure(|row| {
            fixture
                .shared
                .row_key(&row)
                .unwrap()
                .first()
                .unwrap()
                .clone()
        }));
        samf.push(measure(|row| {
            let signature = fixture.fabric.row_signature(&row).unwrap();
            let record = fixture.catalog.class_record(signature[0]).unwrap();
            match &record.signature {
                ObservableClassSignature::Canonical(key) => key.clone(),
                ObservableClassSignature::Product(_) => unreachable!(),
            }
        }));
    }

    let range = |values: &[f64]| {
        let low = values.iter().copied().fold(f64::INFINITY, f64::min);
        let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (low, high)
    };
    let (owned_low, owned_high) = range(&owned);
    let (shared_low, shared_high) = range(&shared);
    let (samf_low, samf_high) = range(&samf);
    println!(
        "{name}: owned={owned_low:.2}..{owned_high:.2} ns/op shared={shared_low:.2}..{shared_high:.2} ns/op samf={samf_low:.2}..{samf_high:.2} ns/op"
    );
}

fn text256(distinct: usize) -> Value {
    let prefix = format!("{distinct:08x}:");
    let mut text = String::with_capacity(256);
    text.push_str(&prefix);
    while text.len() < 256 {
        text.push(char::from(b'a' + u8::try_from(distinct % 26).unwrap()));
    }
    Value::Text(text)
}

fn main() {
    println!(
        "semantic-fiber row-key calibration: N={ROWS} D={DISTINCT} reads={READS} reps={REPETITIONS}"
    );
    let i64_fixture = build_fixture(EquivalenceModule::I64Exact, |distinct| {
        Value::I64(i64::try_from(distinct).unwrap())
    });
    run_case("i64", &i64_fixture);

    let text_fixture = build_fixture(EquivalenceModule::TextExact, text256);
    run_case("text256", &text_fixture);
}
