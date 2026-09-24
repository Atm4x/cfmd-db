# CFMD Pass 19 — bounded generational row slots + first fused Join→Project pipeline

Date: 2026-09-19
Baseline: Pass18 verified (196 declared tests)
Final: Pass19 verified (200 declared tests)
Wall-clock target: 20 minutes
Start: 14:24:58 UTC
End: 14:45:09 UTC
Measured cycle: 20m11s
Implementation frozen before wall-clock boundary; remaining cycle used for verification, benchmarks, spec/report audit and stress replay.

## 1. Problem: stable handles still leaked metadata with lifetime mutation count

Problem -> Pass18 removed O(n) dense-position repair and full-state clone tax, but `PhysicalRowId` was a monotone integer into `positions: Vec<Option<usize>>`. Every delete left a tombstone forever. High-churn relations could therefore accumulate handle metadata proportional to lifetime inserts rather than peak live state.

Hypothesis -> physical row identity should be a reusable generational slot, not an eternal positional/tombstone number. A slot may be reused only after deletion increments its generation; every reference carries `(slot,generation)`, so stale references cannot alias the replacement row.

Implementation -> `PhysicalRowId` is now an opaque generational handle. `InstalledRelation` maintains:

- dense physical row positions for typed storage;
- a slot table containing generation + current physical position;
- a free-list for reusable slots;
- O(1) previous/next links for logical insertion order;
- logical head/tail independent from `swap_remove` physical order.

Delete unlinks one live handle, performs physical `swap_remove`, updates at most one moved-row position, increments the deleted slot generation and places the slot on the free-list. Insert consumes a free slot before extending the slot table and appends the new generation to logical order.

Falsification -> a one-row relation is delete/insert churned 10,000 times. `slots.len()` remains exactly `1`; the original generation-0 handle never resolves again; the current handle reaches generation 10,000. A separate test deletes the first row from `[10,20,30]`, reuses its slot for `40`, and proves logical scan remains `[20,30,40]` despite dense `swap_remove` order.

Result -> lifetime tombstone growth is closed. Slot metadata scales with historical peak simultaneous allocation, not total lifetime mutation count. Generation exhaustion is also prevalidated before commit; a hostile `u64::MAX` test proves the entire `PhysicalStore` remains unchanged on exhaustion.

## 2. Logical order and rebuilt persisted indexes remain deterministic

Problem -> free-slot reuse and `swap_remove` can make physical position order diverge from logical insertion order. Rebuilding a persisted index by enumerating raw physical key columns would therefore leak compaction order into Join output ordering.

Implementation -> fresh `MaterializedI64IndexState::build` now iterates `InstalledRelation::scan_positions()`, i.e. logical order, then records the stable handle at each physical position.

Result -> a rebuilt index and a maintained index share the same logical ordering contract even after churn/compaction.

## 3. Insert planning no longer clones the complete free-list

Problem -> the first generational-slot implementation correctly bounded metadata but `planned_insert_ids()` cloned the entire `free_slots` vector before a small insert. That reintroduced O(peak-slot-count) update planning cost.

Implementation -> planned LIFO allocation now reads directly from:

1. slots removed by the same transition, in reverse removal order;
2. the existing free-list, in reverse order;
3. fresh slots only when both are exhausted.

No free-list clone is performed.

Result -> previewing a small insertion scales with delta size rather than total free-slot count.

## 4. First Join→Project no-intermediate-row fusion

Problem -> Pass18 still materialized the complete joined `Vec<Row>` before a parent `Project` discarded unused columns. This is exactly the logical-`Value` intermediate boundary the ideal spec says should disappear from hot typed pipelines.

Hypothesis -> for `Project(IndexedI64Join(Scan,Scan))`, all required row locations and typed I64 columns are already available at probe time. The executor can emit only requested columns while preserving exact logical Join semantics and output order.

Implementation -> `execute_project_plan` recognizes an indexed I64 Join child and invokes `try_execute_indexed_i64_join_project`. The specialized path:

- validates direct scans and I64 exact equality;
- consumes typed I64 columns directly;
- reuses persisted right-side index when installed, otherwise builds the same ephemeral index fallback;
- probes in logical left/right order;
- materializes only final projected `Value::I64` columns;
- records `fused_join_project_hits` in physical execution stats.

Falsification -> `Project(JoinEq(Scan,Scan), [0])` over `[1,1,2]` with a persisted I64 index exactly matches the logical evaluator, preserves five Bag rows, reports one persisted-index hit, zero ephemeral rebuilds and one fused Join→Project hit.

Result -> this is the first downstream Join composition that crosses the operator boundary without a full intermediate logical row relation. It is a verified slice, not yet a general vectorized/batch DAG.

## 5. Performance diagnostics

Pass18 update behavior remains intact after generational slots:

- 300k-row last-row delete without index: ~15.79–16.55 ms across three final repeats;
- same delete with persisted I64 index: ~0.0106–0.0109 ms;
- first-row indexed deletes remain in the low-microsecond range.

Persisted Join benchmark on 20k unique keys remains noisy and is **not** declared closed:

- run 1 persisted/prebuilt: 1.155x;
- run 2: 1.217x;
- run 3: 1.153x;
- persisted/ephemeral: roughly 0.53x.

The correct current statement is that rebuild cost is eliminated and the remaining implementation gap is roughly 15–20% in these runs, not that indexed Join has reached zero overhead universally.

Evidence:

- `PASS19_PERSISTED_INDEX_BENCH.txt`
- `PASS19_DELTA_UPDATE_BENCH.txt`
- `PASS19_BENCH_REPEATS.txt`
- `PASS19_DELTA_UPDATE_REPEATS.txt`

## 6. Final verification gate

Rust 1.98.1 standalone toolchain.

- `cargo fmt --all -- --check` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace` — PASS
- `cargo test --workspace --release` — PASS
- `cargo build --workspace --release` — PASS
- strict rustdoc (`RUSTDOCFLAGS=-D warnings`) — PASS
- release overflow-check tests for `kernel-plan` + `kernel-integration` — PASS
- fresh post-clean `kernel-plan` release + strict Clippy — PASS
- repeated frozen `kernel-plan` stress replay (>1,000 full lib-suite runs) — PASS
- declared tests: **200/200**
- workspace crates: 19
- Rust LOC: 20,901
- external Cargo sources: 0
- `unsafe`: 0
- TODO/FIXME: 0
- `panic!` / `todo!` / `unimplemented!`: 0

## 7. Status checklist

### Recently closed before Pass19

- [x] O(n) dense-position repair on delete removed by `swap_remove` + stable row handles.
- [x] Full relation/index clone before validated delta removed by plan/validate → in-place commit.
- [x] Persisted I64 index participates in semantic removal candidate lookup.
- [x] Relation + indexes are one atomic physical transition.

### Closed in Pass19

- [x] Lifetime tombstone growth from monotone `PhysicalRowId`.
- [x] Stale-handle aliasing under slot reuse.
- [x] Generation-overflow atomicity hole; exhaustion is rejected during prevalidation before any mutation.
- [x] O(1) logical insertion-order maintenance independent from dense physical order.
- [x] Fresh persisted-index rebuild leaking physical compaction order.
- [x] O(n) free-list clone during small insertion planning.
- [x] First `Join → Project` typed/indexed pipeline without full intermediate joined rows.

### Remaining / newly exposed

- [ ] **General typed batch DAG.** Current fusion is still pattern-specific (`Scan→Filter→Project` and now `Join→Project`). Group/TopK/general operator trees still cross `Vec<Value>` boundaries.
- [ ] **Peak-capacity reclamation policy.** Slot metadata is bounded by historical peak allocated rows, but a relation that permanently shrinks far below its peak retains reusable slot capacity. Need chunk/epoch shrink only if memory benchmarks justify it.
- [ ] **General update-location/index planning.** Unsupported/unindexed key classes still require O(n) semantic row search; multi-index selectivity/cost choice is not implemented.
- [ ] **Other persisted key types.** Text/collation, F64-bitwise/total, Bool and nominal entity IDs need certified native indexes.
- [ ] **Indexed Join constant-factor gap.** Current persisted I64 path is ~15–20% above hand-written prebuilt baseline in the final repeated runs.
- [ ] **Stateful TopK.** Maintained order-statistics with ties and explicit order semantics.
- [ ] **Stateful Group.** Maintained Count and deletion-capable exact/reproducible F64 aggregation.
- [ ] **General long-lived Join/TopK/Group derivative state** beyond the current persisted I64 right-side index fragment.
- [ ] KeyValue / CSR / DenseArray / Inverted / Custom physical runtimes.
- [ ] OrderedView / pagination.
- [ ] WAL/recovery, durable materializations, compaction and crash tests.
- [ ] Transaction-repair scheduler, distribution and formal mechanization after the engine layer is credible.

## Bottom line

Pass19 closes the high-churn handle-metadata leak without sacrificing stable references, exact Join/index behavior, or logical scan order. The physical row layer is now a bounded reusable generational slot system rather than a monotone tombstone table. It also establishes the first indexed Join→Project operator fusion, shifting the immediate execution frontier from row identity/storage bookkeeping toward a genuinely general typed batch DAG and broader persisted index families.
