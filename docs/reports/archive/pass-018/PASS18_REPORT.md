# CFMD Pass 18 — O(1) dense-slot repair, clone-free atomic delta commit, index-assisted removal planning

Date: 2026-09-19
Baseline: Pass17 verified (194 declared tests)
Final: Pass18 verified (196 declared tests)
Wall-clock target: 20 minutes
Start: 14:00:29 UTC
Code/audit wall-clock boundary: 14:20:42 UTC (20m13s). Code was frozen before the boundary; the final interval contained verification/benchmark replay only.

## 1. Problem chain

Pass17 gave persisted Join indexes stable `PhysicalRowId`s, but two update-path costs remained hidden underneath them:

1. deleting a dense physical row still used `Vec::remove`, shifting all following positions and repairing the full suffix of the row-id map;
2. `apply_relation_delta` cloned the complete installed relation and every bound materialized index before mutation to obtain atomic publication.

Thus index identity was stable, but an ordinary delete was still O(n) in physical maintenance even when the semantic row was easy to locate.

## 2. Stable dense slots without physical-order leakage

Physical row deletion now uses `swap_remove` for row-store and all column representations. `InstalledRelation` invalidates the removed `PhysicalRowId` and updates the position of only the one row moved into the vacated dense slot.

This immediately produced a hostile counterexample: raw physical iteration changed Bag/Join result ordering after deletion. The fix is architectural rather than benchmark-specific: logical scan order is now derived from monotonically allocated stable row handles, while dense physical order is free to change. Non-identity layouts therefore materialize scans in handle order; fused filter/project safely falls back to that order-preserving path after compaction; indexed and ephemeral I64 joins iterate the same logical handle order.

A structural regression test verifies that deleting physical row 2 of 8 invalidates only handle 2, moves handle 7 into physical slot 2, and leaves all unrelated handle positions untouched.

## 3. Clone-free two-phase atomic update

The first benchmark after swap-remove falsified the expected end-to-end improvement. Because `apply_relation_delta` still cloned complete relations/indexes, first-row delete latency scaled almost linearly:

- 10k rows: ~0.124 ms without index / ~0.390 ms with persisted index;
- 100k: ~1.54 ms / ~8.21 ms;
- 300k: ~4.41 ms / ~37.5 ms.

The update path was therefore rewritten into two phases:

1. **plan/validate:** check result type, resolve all removed semantic rows to stable handles, validate inserted physical values, and validate every affected persisted index delta without mutating state;
2. **commit:** apply already validated handle removals/inserts in place and update all bound index states.

No full relation or index clone is required for ordinary validation failures. A hostile test submits both a nonexistent removal and a wrong-typed insert and asserts the entire `PhysicalStore` is exactly unchanged after each rejection.

After this change, first-row removal is only a few microseconds even at 300k rows; the previous full-state clone tax disappears.

## 4. Persisted indexes now accelerate update planning too

Removing a semantic row still required locating it. Without an index, deleting the last row remains a linear semantic scan:

- 10k: ~0.50 ms;
- 100k: ~7.24 ms;
- 300k: ~17.0 ms.

Pass18 therefore allows the prevalidation planner to use a compatible persisted I64 index as a **candidate locator**. The index supplies stable row handles for the key bucket, but every candidate is still checked using complete semantic row equality before it may be removed. Thus duplicate keys remain correct and the physical index does not become semantic authority.

With the persisted index, last-row delete becomes approximately:

- 10k: ~0.003 ms;
- 100k: ~0.006 ms;
- 300k: ~0.012 ms.

Raw evidence is in `PASS18_DELTA_UPDATE_BENCH_INDEXED.txt`.

## 5. New problem exposed by the fix

`PhysicalRowId` allocation is monotone and the dense handle->position vector retains tombstones for deleted handles. Dense physical data no longer shifts linearly, but a high-churn long-lived relation can grow handle metadata with historical insert count rather than live cardinality.

The next storage problem is therefore **bounded stable-handle lifecycle**: chunk/epoch compaction or another indirection mechanism that permits reclaiming handle metadata without invalidating live index handles. This is narrower and more honest than the previous generic “stable slot storage” item.

Unindexed/unsupported-key semantic row resolution also remains O(n); general index selection and non-I64 maintained indexes are now more valuable because they improve both queries and updates.

## 6. Verification gate

Rust 1.98.1 standalone toolchain.

- 196 declared tests;
- `cargo fmt --all -- --check` PASS;
- `cargo test --workspace` PASS;
- `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- `cargo test --workspace --release` PASS;
- `cargo build --workspace --release` PASS;
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps` PASS;
- release tests for `kernel-plan` + `kernel-integration` with forced overflow checks PASS;
- 19 crates;
- 20,428 Rust LOC;
- zero external Cargo registry/git sources;
- zero `unsafe`;
- zero TODO/FIXME/panic-shaped macros.

## 7. Remaining problems, ordered

1. General typed batch-to-batch pipeline; Join still materializes `Vec<Value>` output rows.
2. Bounded stable-handle/chunk lifecycle under high churn; current handle metadata retains tombstones.
3. General update-location/index planning: unsupported/unindexed keys still require O(n) semantic row search; choose among multiple indexes by cost/selectivity.
4. Persisted indexes for Bool, F64Bitwise, Text/collation and nominal entity IDs.
5. Close the remaining roughly 10% persisted-vs-prebuilt I64 Join execution gap without benchmark-specific shortcuts.
6. Maintained TopK order-statistics.
7. Maintained Group state, especially deletion-capable exact/reproducible F64 state/provenance.
8. Long-lived derivative ownership for general Join/TopK/Group integrated with physical structures.
9. Runtime for KeyValue/Adjacency/CSR/DenseArray/Inverted/Custom layouts.
10. OrderedView/pagination.
11. WAL/recovery, durable materializations, compaction/crash tests.
12. Observation-repair concurrency scheduler, distribution, and formal mechanization.

## 8. Status checklist

### Recently closed / carried from Pass17
- [x] Persisted I64 Join index stores stable physical handles rather than copied logical rows.
- [x] Relation + dependent index maintenance is one physical transition; standalone index-delta path is retired.
- [x] Stable handles survive dense physical position changes without payload aliasing.

### Closed in Pass18
- [x] O(n) dense-position suffix repair on delete removed via `swap_remove` + one moved-position repair.
- [x] Physical compaction order no longer leaks into logical scan/Join output order.
- [x] Full relation/index clone on every validated delta removed by two-phase plan/validate -> in-place commit.
- [x] Invalid remove/type delta rejected before mutation; store remains unchanged.
- [x] Persisted I64 index reused for update candidate location with full semantic equality verification.
- [x] Linear last-row removal on indexed I64 relation reduced to microsecond-scale in the 300k benchmark.

### Still open / newly exposed
- [ ] General typed batch-to-batch execution without `Vec<Value>` boundaries.
- [ ] Bounded handle metadata / chunk or epoch compaction under long-lived high churn.
- [ ] General index-assisted update location for Text/F64/Bool/entity keys and planner cost selection.
- [ ] Remaining ~10% persisted I64 Join execution gap on the current microbenchmark.
- [ ] Maintained TopK and Group physical state.
- [ ] General long-lived Join/TopK/Group derivative ownership.
- [ ] Other physical layouts and explicit OrderedView/pagination.
- [ ] WAL/recovery/durable materializations/crash testing.
- [ ] Observation-repair concurrency, distribution, and formal mechanization.
