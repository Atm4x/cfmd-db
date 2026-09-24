# IMPLEMENTATION REPORT — Pass 26

General recursive maintained-plan ownership and leaf-boundary falsification.

## Status

VERIFIED PASS after final wall-clock boundary update. Wall-clock: **2026-09-19 18:12:40 → 18:32:52 UTC = 20m12s**. Source was frozen before the boundary; the remaining time is frozen-state audit/reporting/packaging only.

## 1. Problem

Pass25 had compositional construction and maintenance only for a fixed `Join → Group Count → TopKWithTies` owner. The previous OPEN checklist still contained **general recursive maintained-plan ownership**: arbitrary current `RelExpr` trees had no single owner that recursively constructed child state and propagated exact child deltas.

## 2. Hypothesis

Every current relational operator can participate in one recursive owner if:

1. stateful nodes (`Join`, `Group`, `TopK`) build from child snapshots and consume exact child `RelationDelta`;
2. stateless unary nodes (`Filter`, Bag `Project`, `PromoteToBag`) transform child deltas directly;
3. Set projection and Distinct keep support-count state;
4. all base leaf deltas are prevalidated before any mutation.

## 3. Implementation

Added `MaterializedRelPlanState` with recursive node variants for all current `RelExpr` forms:

- `Scan`;
- `FilterEqConst`;
- `Project` for Bag and Set;
- `JoinEq`;
- `Distinct`;
- `Group`;
- `TopKWithTies`;
- `PromoteToBag`.

`MaterializedJoinDeltaState` gained `build_from_input_values`, so Join can be constructed from already materialized recursive child snapshots rather than re-evaluating child queries.

The new public `apply_relation_deltas` accepts exact base-relation deltas keyed by `SemanticId`. It:

1. rejects deltas for relations outside the tree;
2. prevalidates every referenced Scan leaf, including type/semantic shape and removal consistency;
3. only then recursively applies the leaf deltas;
4. transforms/propagates exact operator deltas upward without model replay.

Set `Project` and `Distinct` use `MaterializedSetSupportState`; Join/Group/TopK reuse their maintained states. Leaf commit was also changed from whole-`RelationValue` clone to mutation planning plus in-place commit.

## 4. Falsification

Two new hostile recursive-tree tests cover all current operator families across non-fixed shapes:

1. `Filter → Join(Project) → Project → Group Count → TopKWithTies`, with sequential left/right changes, exact delta comparison against full recompute, output snapshot comparison, and invalid-removal atomicity;
2. `Filter → Project → Distinct → PromoteToBag`, including duplicate-support insert/remove transitions where Distinct output must remain unchanged until the last support disappears.

Both match the full recompute oracle. Invalid base removal leaves the entire recursive state unchanged.

## 5. Performance falsifier

`PASS26_RECURSIVE_TREE_BENCH.txt` compares the older fixed exact-I64 tree against the new general recursive owner on the same unique-key workload:

| rows | recursive build | fixed update | recursive update |
|---:|---:|---:|---:|
| 1,000 | 1.37 ms | 4.98 µs | 46.8 µs |
| 10,000 | 14.16 ms | 8.05 µs | 0.421 ms |
| 50,000 | 75.87 ms | 9.47 µs | 2.323 ms |

Construction remains comparable to the fixed compositional tree. Update scaling exposed a new leaf-specific tax: `Scan` still owns a logical base snapshot and semantically searches it on removals. This is **not** a replay tax in Join/Group/TopK; it is duplicate authoritative-table work at the plan leaf.

The result is therefore retained as an architectural success plus an explicit new storage-boundary performance defect. It is not hidden by weakening leaf consistency checks.

## 6. Verification gate

Final frozen source:

- **224 declared tests**;
- `cargo fmt --all -- --check` PASS;
- `cargo test --workspace` PASS;
- strict workspace Clippy PASS;
- `cargo test --workspace --release` PASS;
- `cargo build --workspace --release` PASS;
- strict rustdoc PASS;
- overflow-checked release tests for `kernel-query` + `kernel-integration` PASS;
- 19 local workspace crates;
- **27,278 Rust LOC**;
- zero external Cargo sources;
- zero `unsafe`;
- zero TODO/FIXME;
- zero `panic!`/`todo!`/`unimplemented!` macros.

The first combined release chain hit an infrastructure timeout during compilation; the release workspace test and remaining build/doc/overflow gates were rerun separately on the frozen source and passed.

Frozen audit: a fresh copy without the original `target/` rebuilt and passed the two recursive-plan release falsifiers plus strict `kernel-query` Clippy. The frozen release test binary then passed **7,200** additional targeted recursive-plan replays. Full release `kernel-query` tests also passed with `RUST_TEST_THREADS=1` and `16`. Production `#[allow(clippy::...)]` count did not increase; one new allow is test-only on the long adversarial fixture.

## 7. Checklist accounting relative to Pass25 final OPEN list

Pass25 final had **12 OPEN items**.

### Closed in Pass26

- [x] general recursive maintained-plan ownership beyond the fixed exact-I64 `Join→Group Count→TopK` fragment.

### Closed from previous OPEN checklist

**1 / 12**.

### Still OPEN from previous checklist

**11 / 12**:

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
- [ ] semantic indexing/canonical-key strategy for generic maintained Join so `O(|Δ|·n)` becomes indexed rather than linear opposite-side scan.

### New problems discovered in Pass26

- [ ] **storage→maintained-plan leaf contract:** recursive `Scan` currently re-checks/removes rows from its own logical snapshot, causing O(n) semantic lookup on validated removals. The derived plan should receive a certified delta/stable row handle from authoritative physical storage instead of acting as a second table store.

## 8. Result

Pass26 closes the recursive ownership architecture for the current relational language: the runtime is no longer hard-wired to one Join→Group→TopK shape. The principal new defect is now sharply separated from query-state composition: base-table mutation authority is duplicated at Scan leaves. The next correct step is to connect the already-existing physical stable-row-handle machinery to the recursive plan leaf boundary rather than adding another ad-hoc logical index inside `kernel-query`.
