# PASS27 REPORT — storage-certified maintained-plan leaf boundary

VERIFIED PASS. Wall-clock: **2026-09-19 18:40:34 → 19:00:46 UTC = 20m12s**. Source was frozen before the boundary; post-boundary work was verification, reporting and packaging only.

## 1. Problem

Pass26 closed recursive maintained-plan ownership but exposed a new boundary defect: `Scan` retained a logical base snapshot and re-resolved every removal by semantic row search. A validated physical mutation therefore paid the same membership work twice: once in authoritative storage and again in derived query state. The 1k/10k/50k removal diagnostic was ~47 µs / 0.42 ms / 2.32 ms.

## 2. Hypothesis

Storage already owns the exact stable identity needed by the plan. If the physical transition returns the `PhysicalRowId` values that it actually removed/allocated, a maintained `Scan` can update its reconstructible snapshot by handle rather than re-running semantic membership search. The semantic payload remains in `RelationDelta` for downstream operators.

## 3. Implementation

- Moved the generational handle shape into `kernel-types` as `StableRowHandle { slot, generation }`; `kernel-plan::PhysicalRowId` is now an alias/re-export of that shared kernel type.
- Added `StorageCertifiedRelationDelta` in `kernel-query`: logical `RelationDelta` plus exact removed/inserted stable handles.
- `PhysicalStore::apply_relation_delta_certified` now performs the existing atomic physical planning/validation/commit and returns the exact handle receipt selected by storage. Existing `apply_relation_delta` delegates to it and discards the receipt, so compatibility is preserved.
- Added `PhysicalStore::logical_row_handles` for one-time attachment of storage identity to an already-built maintained Scan snapshot.
- Recursive `Scan` nodes may now carry `MaintainedLeafHandles` (`handle → current snapshot position`). Removal is `BTreeMap` lookup + `swap_remove`; insertion appends and records the new generation handle.
- Added `MaterializedRelPlanState::apply_storage_certified_deltas`. It validates type/shape/handle membership but deliberately does **not** redo semantic row lookup. The resulting logical delta is propagated through the same recursive Filter/Project/Join/Distinct/Group/TopK machinery.
- The old `apply_relation_deltas` path remains intact as a correctness fallback for callers without a storage certificate.

## 4. Falsification

Two hostile paths are covered:

1. PhysicalStore integration: attach initial physical handles, delete one row, reuse/insert a slot, consume the returned certified receipt, and verify the maintained Scan snapshot. Replaying the same stale receipt is rejected before mutation.
2. The existing non-trivial recursive `Filter → Join(Project) → Project → Group Count → TopKWithTies` falsifier now also runs a certified leaf transition with generation-changing slot reuse and matches the full recompute oracle.

After source freeze, the certified Scan integration and full recursive-tree certified path passed **30,600 additional targeted release replays** without failure.

## 5. Performance falsifier/result

`PASS27_STORAGE_PLAN_BRIDGE_BENCH_REPEATS.txt`, medians of three process runs:

| rows | old semantic Scan | certified Scan only | authoritative storage + certified Scan |
|---:|---:|---:|---:|
| 1,000 | 39.6 µs | 1.65 µs | 2.95 µs |
| 10,000 | 411.9 µs | 4.80 µs | 9.24 µs |
| 50,000 | 2.142 ms | 7.12 µs | 15.35 µs |

The Pass26 linear leaf-membership tax is therefore removed on the storage-backed path. The remaining growth is small container/index/receipt overhead rather than a semantic scan over all rows.

This does **not** mean the cross-layer transaction problem is finished: storage currently publishes its mutation before the maintained plan consumes the receipt. The certified plan path is designed to be total for a correctly attached same-revision state, but there is not yet one joint prepare/commit transaction spanning PhysicalStore + maintained plan.

## 6. Verification gate

Final frozen source:

- **225 declared tests**;
- `cargo fmt --all -- --check` PASS;
- `cargo test --workspace` PASS;
- strict workspace Clippy PASS;
- `cargo test --workspace --release` PASS;
- `cargo build --workspace --release` PASS;
- strict rustdoc PASS;
- overflow-checked release tests for `kernel-query`, `kernel-plan`, and `kernel-integration` PASS;
- 19 local workspace crates;
- **28,024 Rust LOC**;
- zero external Cargo sources;
- zero `unsafe`;
- zero TODO/FIXME;
- zero `panic!`/`todo!`/`unimplemented!` macros.

Combined release/overflow invocations hit infrastructure compilation timeouts; every unfinished component was rerun separately on the frozen source and passed.

## 7. Checklist accounting relative to Pass26 final OPEN list

Pass26 carried **11 inherited OPEN items** plus one newly discovered storage→plan leaf problem.

### Closed in Pass27

- [x] storage→maintained-plan leaf certification / stable-handle path;
- [x] duplicate O(n) semantic membership search in storage-backed recursive `Scan`;
- [x] stable row identity shared across physical/query kernels;
- [x] certified deltas propagate through the general recursive maintained tree, not just bare Scan.

### Closed from the inherited Pass26 OPEN checklist

**0 / 11**. Pass27 intentionally attacked the *new* Pass26 storage-boundary blocker rather than one of the 11 older items.

### Still OPEN from the inherited checklist

**11 / 11**:

- [ ] generic Text/F64 maintained TopK order-statistics;
- [ ] I64 TopK constant-factor gap;
- [ ] Group/TopK as typed-batch producers;
- [ ] maintained I64 Group constant-factor gap;
- [ ] indexed generic/Text Group;
- [ ] persisted Text/F64/Bool/entity indexes + planner;
- [ ] nested/multiway/mixed-key joins;
- [ ] remaining physical layouts + OrderedView/pagination;
- [ ] WAL/recovery/durable materializations/crash tests;
- [ ] transaction repair runtime, distribution, formal mechanization;
- [ ] semantic indexing/canonical-key strategy for generic maintained Join.

### New problems exposed in Pass27

- [ ] **joint storage+plan atomic publication:** receipt production currently commits PhysicalStore before maintained-plan publication; introduce a cross-layer prepared transition/commit protocol so neither side can advance alone.
- [ ] **certificate authority sealing:** `StorageCertifiedRelationDelta` is a typed contract but not a capability-sealed security primitive across Rust crates. If hostile in-process callers matter, certificate construction needs a stricter trust boundary.

## 8. Result

Pass27 closes the concrete Pass26 leaf-scan performance defect. Base-row identity is now resolved once by authoritative physical storage and reused by the recursive maintained query tree. The next storage/state architecture problem is no longer row lookup; it is atomic publication of one prepared transition across physical storage and derived maintained state.
