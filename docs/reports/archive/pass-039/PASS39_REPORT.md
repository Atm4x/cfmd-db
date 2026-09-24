# PASS39 REPORT — production semantic canonical keys + maintained Join/Group/TopK indexes

Status: **VERIFIED** on Rust **1.98.1**.

Pass39 integrates the previously verified Agent-2 canonical-key/index research into the production maintained-query path. CLOSED items below are actual eliminated architecture/performance problems; individual tests are evidence and are not counted as separate problems.

## 1. Problem

After Pass38 the durable control plane was sufficiently complete to return to the historical data-plane backlog. Three closely related maintained-query gaps shared one root cause:

1. primitive non-I64 Join still needed a semantic full scan on the opposite side for each changed row;
2. non-I64/composite primitive Group lookup could fall back to a vector scan over existing groups;
3. generic Text/F64 TopK maintained state lacked a semantic order-key index and relied on a generic ordered-row path with linear update costs.

The correctness constraint is stronger than ordinary Rust `Eq`/`Hash`/`Ord`: every physical/derived lookup must implement the exact equality/ordering pinned by Γ.

## 2. Design

### 2.1 Exact canonical keys are resolved from pinned Γ

`kernel-semantics` now exposes exact production representations for every currently supported builtin primitive equivalence:

- UnitExact;
- BoolExact;
- I64Exact;
- F64Bitwise;
- TextExact;
- TextAsciiCaseInsensitive;
- LiveEntityIdExact(entity_type);
- HistoricalEntityIdExact(entity_type).

`ResolvedPrimitiveEquivalence::canonical_key` satisfies the required law:

```text
canonical_eq_key(a) == canonical_eq_key(b)
IFF
Γ.equivalent(a,b)
```

Ordering keys are provided for:

- I64Ascending;
- F64Total;
- TextBinary;
- TextAsciiCaseInsensitive;
- TextAsciiCaseInsensitiveThenBinary.

`ResolvedPrimitiveOrdering::canonical_key` preserves the exact pinned comparator. The F64Total encoding is checked against Rust `total_cmp` over hostile values and 100,000 deterministic bit-pattern pairs.

### 2.2 Reusable `kernel-semantic-index`

A new zero-external-dependency crate, `kernel-semantic-index`, contains:

```text
SemanticIndexBinding
  semantic_revision
  exact SemanticId -> ModuleDigest dependencies
  key_encoding_revision

SemanticBucketIndex<Key, Identity>
  ordered key -> identity bucket
  identity -> key reverse map
```

Identity is deliberately generic. Internal derived query rows use local `IndexedRowId`; the abstraction is not coupled to `StableRowHandle` and therefore does not turn storage identity into semantic authority.

The binding explicitly invalidates an index when Γ revision, module digest dependencies, or canonical-key encoding revision changes. Maintained query states additionally pin the complete `SemanticContext`, so an operation under a different Γ is rejected before index use.

### 2.3 Maintained Join

Join selection is now:

```text
exact I64 specialized fast path
    else current builtin primitive equivalence -> SemanticIndexed
    else structural/custom equivalence -> exact GenericScan fallback
```

For `SemanticIndexed`, both sides maintain canonical-key buckets and monotone local derivative row IDs. Removal planning probes only the corresponding semantic bucket and then performs exact full-row semantic verification. Changed-row join propagation probes only the opposite canonical bucket.

No hash-only equality shortcut is used.

### 2.4 Maintained Group

For group keys whose equivalence modules are all currently canonicalizable primitive modules, Group maintains:

```text
Vec<CanonicalEqKey> -> group slot
```

This supports composite primitive keys as well as TextAsciiCaseInsensitive. The existing single-I64 specialization remains intact. Structural/custom equivalence groups preserve the old exact semantic-scan fallback rather than pretending to have a canonical representation that has not been proved.

### 2.5 Maintained TopK

The I64 specialized path remains unchanged. Other current builtin primitive ordering modules use `SemanticOrdered`:

```text
CanonicalOrderKey -> ordered tie bucket of IndexedRowId
IndexedRowId -> exact row
```

Updates are planned against the relevant tie bucket and then committed. Output traverses the ordered buckets in the requested direction and retains the complete threshold tie bucket, preserving `TopKWithTies` semantics.

## 3. Hostile falsification

The pass includes/retains hostile coverage for:

- ASCII-CI case variants including ASCII changes inside Unicode strings;
- TextExact vs TextAsciiCaseInsensitive distinction;
- F64 `+0.0/-0.0`, infinities, subnormals and distinct positive/negative NaN payloads;
- entity type/liveness namespaces;
- duplicate semantic-key buckets;
- remove+insert in one Join delta;
- composite primitive Group keys;
- hostile F64 Total ordering through maintained TopK;
- structural Join equivalence remaining on exact semantic-scan fallback;
- semantic binding invalidation on Γ/digest changes.

A release-only falsifier found during Pass39 is worth recording. Initial index build/commit code performed mutating `insert` operations inside `debug_assert!`. Debug tests passed, but release compiled the side effects away and produced empty indexes. All side effects were moved outside assertions, a release regression was added, and a static audit found no remaining mutating expressions hidden inside `debug_assert!` in the new semantic-index paths.

## 4. Benchmark

Integrated release benchmark: `TextAsciiCaseInsensitive`, one changed left row, one-row output bucket, right side of 1k/10k/50k rows. The comparison baseline is intentionally conservative: a minimal direct `eq_ignore_ascii_case` full scan, which does less work than the old complete generic Join maintenance path.

Median across three benchmark runs:

| right rows | integrated semantic index | minimal full scan | scan / indexed |
|---:|---:|---:|---:|
| 1,000 | 1,631 ns | 374 ns | 0.23x |
| 10,000 | 1,439 ns | 4,620 ns | 3.21x |
| 50,000 | 1,438 ns | 18,800 ns | 13.07x |

Scaling from 1k -> 50k:

```text
semantic-index update: 0.88x (noise-level / effectively flat)
minimal full scan:     50.27x
```

Therefore Pass39 closes the asymptotic full-scan problem, not the small-relation constant factor. Around 1k rows the index is slower than a minimal scan, so planner/index-selection thresholds remain an explicit OPEN optimization problem.

Raw benchmark evidence: `evidence/pass39/SEMANTIC_INDEX_JOIN_BENCH.log`.

## 5. Verification

Final Rust 1.98.1 gate:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Metrics:

```text
308 declared tests
69 kernel-query tests
30 kernel-semantics tests
3 kernel-semantic-index tests
21 crates
39,584 Rust LOC
0 external Cargo sources
0 unsafe
```

## 6. Problem ledger

### Closed exactly in Pass39

1. ✅ **Production exact canonical-key/index foundation for current builtin primitive semantic modules was missing.** Canonical equality/order laws now live in `kernel-semantics`; the reusable identity-generic `kernel-semantic-index` is bound to pinned Γ dependencies and key-encoding revision.
2. ✅ **Generic maintained Join for current builtin primitive equivalences performed `O(|Δ| * n)` opposite-side semantic scans.** It now uses exact semantic buckets; structural/custom equivalence remains an explicit fallback rather than an unproved fast path.
3. ✅ **Maintained Group lookup for canonicalizable non-I64/composite primitive keys used semantic vector fallback.** Composite canonical group keys now maintain direct lookup; structural/custom group equivalence remains fallback.
4. ✅ **Generic Text/F64 maintained TopK lacked a performance-grade semantic ordering structure.** Current non-I64 builtin orderings now use maintained ordered semantic-key/tie buckets with exact WITH-TIES semantics.

### Closed from the historical Pass26 backlog

1. ✅ Generic Text/F64 maintained TopK order-statistics for the currently supported builtin orderings.
2. ✅ Agent-2 canonical-key strategy and production semantic-index integration for generic maintained Join over the currently supported builtin primitive equivalences.

### Partially advanced, still OPEN

1. 🟨 **Indexed generic/Text Group.** Text and every composite key composed only from current builtin primitive equivalences are indexed. Structural/custom equivalence keys still require exact semantic scan until a canonical structural-key law is designed and proved.
2. 🟨 **Planner use of semantic indexes.** Maintained query state selects the index structurally, but there is no cost model/threshold. The benchmark shows that a 1k-row minimal scan can beat the index on constant factor.

### Historical Pass26 OPEN backlog still active

1. ⬜ I64 TopK constant-factor gap.
2. ⬜ Group/TopK as typed-batch producers.
3. ⬜ Maintained I64 Group constant-factor gap.
4. ⬜ Structural/custom-equivalence Group canonical indexing; primitive/Text portion is closed in Pass39.
5. ⬜ Persisted Text/F64/Bool/entity indexes in `PhysicalStore` + physical planner integration. Pass39 indexes are reconstructible maintained-query state, not persisted storage indexes.
6. ⬜ Nested/multiway/mixed-key joins and broader join planning.
7. ⬜ Remaining physical layouts + OrderedView/pagination.
8. ⬜ Remaining durability assurance/history work: durable DAG history, general migration, arbitrary semantic artifacts, power-loss/cross-filesystem validation, scalability/distributed/authenticated durability.
9. ⬜ Transaction repair runtime, distribution and formal mechanization.

### Newly clarified OPEN after Pass39

1. ⬜ Cost-based semantic-index selection / small-relation threshold; current maintained primitive path always indexes even where a tiny scan may be cheaper.
2. ⬜ Canonical structural keys for Product/Option/Sum/Collection/recursive/custom equivalences. Current exact scan fallback is correct but not indexed.
3. ⬜ Persist/rebuild policy for semantic indexes at storage level; current maintained indexes are intentionally reconstructible in-memory derivative state.
4. ⬜ Shared index reuse across multiple materializations/operators. Each maintained operator currently owns its own derivative index.
5. ⬜ Index memory accounting/budgeting and eviction policy.
6. ⬜ Canonical-key encoding migration policy if `KEY_ENCODING_REVISION` changes; current binding correctly invalidates but does not migrate.
7. ⬜ Benchmarks/constant-factor tuning for primitive Group and Text/F64 TopK beyond the functional/asymptotic structure established here.

Practical next direction: Pass39 removes the main Agent-2 research-to-production gap. The next highest-value mainline is either (a) extend semantic indexing down into persisted `PhysicalStore` plus planner/cost model, closing historical Pass26 item 5, or (b) attack typed-batch Group/TopK and I64 constant-factor gaps before broader multiway joins. The architecture now supports either without changing semantic authority.
