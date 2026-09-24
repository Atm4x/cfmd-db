# CFMD Pass43 Report — Semantic Index Advisor / Lifecycle / Shared Reuse

**Status:** VERIFIED on Rust 1.98.1.

**Source window:** 2026-09-20 11:01:44–11:21:54 UTC. Production source was frozen after the window; subsequent work was verification, benchmarks, documentation, evidence and packaging only.

**Scope:** reconstructible physical semantic-index lifecycle for the current builtin primitive `MaterializedSemanticIndexState` family. Pass43 does not change semantic authority, transaction authority, durable Revision semantics, or Γ laws.

## 1. Problem

Pass42 could decide whether to use an **already installed** compatible persisted semantic index, but the physical layer still had no production policy boundary for deciding whether repeated workload justified creating, retaining, sharing, rebuilding or evicting such an index.

A naïve `create_index_if_missing` would also have introduced two architectural defects:

1. no amortization of index-build work against expected future execution work;
2. no ownership distinction between an index created by an advisor and one explicitly installed by another physical-policy layer, making policy eviction unsafe.

## 2. Workload-driven semantic-index advisor

Pass43 adds:

```text
SemanticIndexWorkloadSample {
    plan,
    expected_executions,
}

SemanticIndexAdvisorPolicy {
    max_managed_key_cells,
}
```

and `PhysicalStore::advise_semantic_indexes(...)`.

For the current primitive semantic-index family the advisor discovers eligible direct Filter/composite-Filter and direct two-way equality Join/composite-Join access opportunities, including those below ordinary unary physical nodes. It derives only Γ-certified primitive `SemanticIndexBinding`s; unsupported structural/custom equivalence still produces no invented canonical key and continues through exact fallback.

For each binding it aggregates demand across workload samples before making a decision. The correctness-first work model is:

### Filter opportunity

```text
scan_work        = row_count
index_probe_work = 1 + matching_rows
per_exec_saving  = max(scan_work - index_probe_work, 0)
```

### Join opportunity

```text
nested_work      = left_rows * right_rows
average_bucket   = ceil(right_rows / distinct_index_keys)
indexed_work     = left_rows * (1 + average_bucket)
per_exec_saving  = max(nested_work - indexed_work, 0)
```

### Prospective build

```text
gross_saving = Σ(per_exec_saving * expected_executions)
build_work    = row_count * composite_key_parts    // only if no compatible index exists
net_saving    = gross_saving - build_work
```

A missing/stale index is selected only when `gross_saving > build_work`. Compatible already-installed indexes pay no build work. Competing advisor-managed choices are selected deterministically by benefit density under the configured managed key-cell budget.

`max_managed_key_cells` is deliberately a deterministic **proxy unit**, not a claim about exact allocated bytes. Exact byte-level memory accounting remains OPEN.

## 3. Shared reuse and ownership boundary

Demand for the same `SemanticIndexBinding` from multiple samples/operators is aggregated. Build cost is charged once and one physical index is shared.

`PhysicalStore` now records which semantic indexes are advisor-managed. This state is reconstructible physical policy state only.

Rules:

- a compatible manually/externally installed semantic index may be reused by the advisor;
- an external index does not become advisor-owned merely because it was reused;
- the advisor may evict only indexes it owns;
- explicit `install_semantic_index(...)` relinquishes advisor ownership for that binding;
- relation reinstall invalidates both semantic index state and its advisor ownership metadata.

This prevents a local lifecycle policy from silently deleting physical state owned by another policy layer.

## 4. Atomic reconcile and runtime publication

Advice is evaluated before physical mutation. Every selected missing/stale index is completely built and validated first. Only after all preparations succeed does the store reconcile advisor-owned evictions/insertions and advance the physical transition epoch once.

`RuntimeRevisionCell::advise_semantic_indexes(...)` applies the result through the existing immutable-root publication boundary:

- clone current reconstructible physical root;
- run advisor reconciliation against the pinned semantic Revision;
- if physical state did not change, return the report without publishing a new runtime root;
- if it changed, publish one new `Arc<RuntimeRevisionBundle>` root version;
- semantic `Revision=(S,Γ,M)` and maintained materialization semantics are unchanged.

`DurableRuntime` exposes the same operation using its already-bound semantic registry. Advisor state/indexes are not serialized as semantic durable authority; restart may reconstruct or re-advise them.

## 5. Hostile / regression evidence

Pass43 verifies:

- two individually unprofitable one-shot samples for the same binding can jointly amortize one shared build;
- one isolated selective execution can still be rejected when build work exceeds expected saving;
- advisor budget eviction removes an advisor-owned lower-value index but preserves/reuses an externally installed compatible index;
- changing the module contract behind the same `SemanticId` makes a Γ-bound index stale and forces rebuild before reuse;
- a mixed `(TextAsciiCaseInsensitive, I64Exact)` composite Join candidate is created and produces exactly the logical reference result;
- access opportunities are discovered below a `Project` boundary instead of requiring the root plan itself to be Filter/Join;
- physical advisor changes publish exactly one new runtime root while an identical no-op follow-up does not republish;
- all seven advisor hostile tests pass in debug and release;
- no advisor/reconcile mutation is hidden inside `debug_assert!`/`assert!`.

The advisor reads compatible existing index statistics through borrowed state; it does not clone the complete existing index merely to estimate access work.

## 6. Performance diagnostics

Pass43 makes no claim that the current cost units are calibrated wall-clock latency or that one index family is globally optimal. Existing release diagnostics were rerun to keep old performance debt visible.

### Maintained I64 Group

```text
groups=50000
maintained_median_ns=275
baseline_median_ns=137
ratio=2.007x
```

The residual constant-factor gap remains OPEN.

### Maintained I64 TopK

```text
rows=50000, k=10
maintained_median_ns=559
baseline_median_ns=108
ratio=5.175x
full_replay_median_like_ns=268933
```

The hand-written constant-factor gap remains OPEN even though maintained execution is far below full replay cost.

### Existing semantic Join crossover diagnostic

```text
right rows    indexed ns    scan ns    scan/indexed
1,000         1,256         437        0.347x
10,000        1,236         4,618      3.736x
50,000        1,326         24,596     18.549x
```

This again falsifies any rule that an index is always faster: at 1k rows the deliberately minimal scan wins, while larger relations show the indexed scaling advantage.

Evidence: `evidence/pass43/09_maintained_group_bench.log`, `10_maintained_top_k_bench.log`, `11_semantic_index_join_bench.log`.

## 7. Verification

Final Rust 1.98.1 gate after production source freeze:

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

Workspace metrics at freeze:

- 326 declared Rust tests;
- 98 `kernel-plan` tests;
- 21 crates;
- 43,838 Rust LOC under `crates/`;
- 0 external Cargo registry/git sources;
- 0 `unsafe` in `crates/`.

## 8. Problem ledger

### CLOSED exactly in Pass43

1. ✅ **Current primitive semantic-index family had no production lifecycle/advisor boundary beyond “use an already installed index”.** Explicit workload samples now drive deterministic create/rebuild/retain/reuse/evict decisions with build amortization and a managed-budget policy. Multiple demands for one binding share one build/index.
2. ✅ **An automatic index policy had no ownership-safe atomic publication model.** Advisor-managed ownership is separate from external/manual index ownership; only owned indexes are evictable, all selected rebuilds are prepared before reconciliation, and runtime changes publish through one immutable physical-root swap without changing semantic Revision authority.

### Advanced but still OPEN

1. 🟨 **Physical memory budgeting.** Pass43 supplies a deterministic managed `key_cells` budget, not allocator/byte/resident-memory accounting, pressure feedback or background rebuild scheduling.
2. 🟨 **Automatic lifecycle.** The lifecycle engine is production-integrated for explicit workload horizons, but workload telemetry/decay/online scheduling is not autonomous yet.
3. 🟨 **Shared reuse.** Shared demand aggregation/reuse is implemented for the current generic primitive `MaterializedSemanticIndexState` family. Cross-family/layout reuse is still open.
4. 🟨 **Cross-family access-path choice.** The specialized persisted I64 index remains a separate hot-path family. Pass43 does not claim to choose optimally between specialized I64, generic semantic index and future layouts.
5. 🟨 **I64 Group/TopK constant factors.** Current diagnostic ratios are ~2.007x and ~5.175x respectively; neither is CLOSED.

### Historical / active OPEN after Pass43

1. ⬜ Structural/custom-equivalence canonical indexing and typed production.
2. ⬜ Nested/multiway/bushy Join planning and general join-order search.
3. ⬜ Multi-family physical access advisor: specialized I64 vs generic semantic index vs future physical layouts.
4. ⬜ Autonomous workload statistics/telemetry, retention decay and lifecycle scheduling.
5. ⬜ Exact physical-index byte/resident-memory budgeting and rebuild scheduling.
6. ⬜ Canonical-key encoding/version migration for long-lived physical caches.
7. ⬜ Remaining physical layouts and explicit `OrderedView`/pagination.
8. ⬜ Secondary-index / alternate-layout rebuild performance after recovery.
9. ⬜ Clone-heavy runtime transaction candidates → COW/persistent roots.
10. ⬜ Durable revision DAG / branch+merge ancestry.
11. ⬜ General historical durable-format migration framework.
12. ⬜ Arbitrary/plugin semantic executable artifact packaging/signing/deployment.
13. ⬜ Transaction intent/outcome retention + GC.
14. ⬜ Streaming/chunked checkpoints/metadata.
15. ⬜ Real machine power-loss assurance and Windows/network-FS/FUSE durability semantics.
16. ⬜ General lock-poison/restart policy.
17. ⬜ Group commit / async durability.
18. ⬜ Replication / consensus and broader distribution architecture.
19. ⬜ Durable-store authentication/MAC; current CRC32C is corruption detection only.
20. ⬜ Formal power-loss proof for rename/fsync/GC protocol.
21. ⬜ Transaction repair runtime.
22. ⬜ Formal mechanization.

## 9. Rejected / deliberately deferred routes

- no semantic/durable “index ownership” concept; advisor ownership is reconstructible physical policy metadata;
- no unconditional auto-build threshold copied from one benchmark;
- no claim that `key_cells` equals bytes;
- no eviction of externally installed indexes;
- no invented canonical keys for structural/custom equivalence;
- no hurried rule that generic semantic I64 indexes dominate the specialized I64 family;
- no claim that explicit workload advice is already an autonomous adaptive optimizer.

## 10. Result / next direction

Pass43 closes the first real lifecycle loop for the current primitive semantic-index family without turning physical policy into semantic authority.

The next highest-value connected cluster is now **multiway join planning + multi-family access costing**: enumerate nested/bushy join orders, consume actual relation/index statistics, and compare scan vs existing generic semantic index vs specialized I64 vs prospective build under one physical policy. Exact byte memory budgeting/telemetry can then replace the current key-cell proxy without changing the authority boundary.
