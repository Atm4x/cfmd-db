# CFMD Pass 24 — maintained Join + owned Join→Group→TopK state tree

## 1. Wall clock

- Start: 2026-09-19 17:19:24 UTC
- Frozen-source boundary: 2026-09-19 17:39:31 UTC
- Wall clock: **20m07s**
- No implementation source changed after the boundary.

## 2. Problem

Pass23 left Join, Group and TopK as independently maintained states. The open item was an owned compositional `Join → Group → TopK` state tree. Join itself also lacked a first-class maintained state accepting direct left/right `RelationDelta` inputs.

## 3. Hypothesis

A maintained Join can own both input relations and consume left/right deltas directly. For exact-I64 equality it can keep per-key `BTreeMap<i64, Vec<Row>>` buckets and touch only affected key buckets. A parent Group can consume the Join delta, and TopK can consume the Group delta, forming one owned long-lived pipeline.

For correctness outside the exact-I64 specialization, Join can retain a semantic generic fallback even if that fallback is not performance-grade yet.

## 4. Implementation

### 4.1 `MaterializedJoinDeltaState`

Added long-lived Join state with:

- pinned semantic context and input/result relation types;
- direct `apply_input_deltas(left_delta, right_delta, ...)` boundary;
- exact-I64 maintained storage using left/right `BTreeMap<i64, Vec<Row>>` buckets;
- two-sided delta planning that handles simultaneous left/right inserts/removals exactly once;
- bag multiplicity preservation;
- Set duplicate checks under the declared relation equality;
- generic semantic fallback backed by current `RelationValue` state and exact Γ equality.

The exact-I64 path plans only affected key buckets and commits them after validation. No full relation clone is required.

### 4.2 `MaterializedJoinGroupTopKState`

Added an owned pipeline for the verified fast fragment:

```text
exact-I64 Join
    ↓ RelationDelta
exact-I64 Group Count
    ↓ RelationDelta
I64 TopKWithTies
```

The tree is admitted only when all three child states are on their total exact-I64 fast paths. Leaf deltas are validated by Join; downstream deltas are generated internally and match the already pinned Group/TopK input contracts by construction.

This removes the earlier full-tree transactional staging clone from the hot update path.

### 4.3 Quality cleanup

The first Join helper exceeded Clippy argument/line complexity limits. It was refactored into:

- `JoinI64MaintenanceSpec`;
- `I64JoinChanges`;
- `PlannedI64JoinSide`;
- separate collect/plan/commit helpers.

No new **production** Clippy suppression was added. Two long adversarial test fixtures and the standalone benchmark harness use test/example-only `too_many_lines` suppressions.

## 5. Falsification / hostile tests

Added three principal tests:

1. exact-I64 two-sided Join delta with bag multiplicity, compared against full recompute;
2. generic Text ASCII-CI maintained Join, proving the fallback uses Γ equality rather than Rust equality;
3. sequential leaf deltas through `Join → Group Count → TopK`, compared against full-query recompute after every transition.

Malformed/missing Join removal is rejected atomically and leaves state unchanged.

After source freeze:

- 900 extra exact-I64 Join release replays PASS;
- 1,100 extra owned-tree release replays PASS;
- query release suite passes with `RUST_TEST_THREADS=1` and `16`.

## 6. Performance evidence

`PASS24_STATE_TREE_BENCH.txt` and `PASS24_STATE_TREE_BENCH_REPEATS.txt` record the diagnostic.

For a bounded-output tree where TopK orders by group key (`k=10`):

- 1k state: maintained transition about **4.8–5.8 µs** in clean runs;
- 10k state: about **6.5–11.1 µs** in clean runs;
- process-level scheduler/cache noise is visible, so these are diagnostic ranges, not a stable universal ratio.

The important result is structural: the update path does not show the prior full-state clone scaling.

A hostile benchmark variant ordering TopK by Group Count initially appeared O(n), but diagnosis showed this was exact `WITH TIES` semantics: all groups had count=1, so TopK was required to emit all N tied rows. The cost was proportional to output size, not hidden state scanning.

### Newly exposed build-path problem

Initial tree construction is **not yet compositional**. `Group::build` / `TopK::build` re-evaluate nested child expressions instead of consuming the materialized child snapshot. On large equijoin inputs this reaches the generic logical nested-loop Join and can become O(n²); 50k-state construction does not complete in the diagnostic window.

Thus:

- maintained transition path: compositional for the exact-I64 tree;
- initial state construction: still needs child-snapshot build propagation.

## 7. Verification gate

Frozen source passes:

- **222 declared tests**;
- `cargo test --workspace` PASS;
- `cargo test --workspace --release` PASS;
- strict `cargo clippy --workspace --all-targets -- -D warnings` PASS;
- `cargo fmt --all -- --check` PASS;
- `cargo build --workspace --release` PASS;
- strict rustdoc PASS;
- overflow-checked release tests for `kernel-query` and `kernel-integration` PASS;
- 19 workspace packages, 0 external Cargo sources;
- 0 `unsafe`;
- 0 TODO/FIXME;
- 0 `panic!` / `todo!` / `unimplemented!`;
- **26,147 Rust LOC** in `crates`.

## 8. Checklist delta

### Closed in Pass24

- [x] first-class maintained Join state accepting direct two-sided child deltas;
- [x] exact-I64 indexed Join maintenance without full relation replay;
- [x] generic semantic Join fallback for correctness;
- [x] owned exact-I64 `Join → Group Count → TopK` state tree;
- [x] hot tree update no longer clones all child states;
- [x] simultaneous two-sided Join multiplicity / ASCII-CI fallback / sequential tree transitions falsified against recompute.

### Closed from Pass23's previous OPEN checklist

**1 / 11** previous OPEN items are closed in this cycle:

- [x] `one owned compositional Join/Group/TopK state tree` — closed for the exact-I64 maintained fast fragment.

### Previous OPEN items still open: 10 / 11

- [ ] generic Text/F64 maintained TopK needs indexed/order-statistics state;
- [ ] reduce exact-I64 maintained TopK tiny-microbaseline constant-factor gap;
- [ ] Group/TopK as typed-batch producers for downstream operators;
- [ ] reduce maintained I64 Group fair-microbaseline gap;
- [ ] generic/Text Group indexed semantic lookup at high cardinality;
- [ ] persisted Text/F64/Bool/entity indexes + cost/selectivity planner;
- [ ] nested/multiway/mixed-key joins;
- [ ] remaining physical layouts + OrderedView/pagination;
- [ ] WAL/recovery/durable materializations/crash tests;
- [ ] transaction repair runtime, distribution and formal mechanization.

### New problems exposed in Pass24

- [ ] **compositional initial state build:** parent maintained states must build from child snapshots instead of re-evaluating nested queries; current large Join→Group→TopK construction can fall back to O(n²) logical Join evaluation;
- [ ] generic maintained Join is correctness-first and can still clone/replay whole generic relation values;
- [ ] owned state tree is currently a fixed exact-I64 `Join→Group Count→TopK` fragment, not a general recursive maintained-plan ownership framework;
- [ ] exact `WITH TIES` can legitimately produce O(n) output when the threshold equivalence class contains O(n) rows; this is semantic output cost, not removable internal overhead.

## 9. Result

The missing ownership link between Join, Group and TopK is now executable for the exact-I64 maintained fragment. Deltas flow leaf→Join→Group→TopK without model replay and without full-tree staging clones. The next architectural target is no longer "connect the states"; it is **make state construction itself compositional and generalize ownership beyond one fixed fragment**.
