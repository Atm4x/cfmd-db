# CFMD Pass 25 — compositional state construction + local generic Join maintenance

## Status

VERIFIED PASS. Wall-clock: **2026-09-19 17:49:38 → 18:09:54 UTC = 20m16s**. Source was frozen before the boundary; the final minutes were frozen-state benchmark repeats, release falsifiers, thread-scheduling checks, and integrity audit.

## 1. Problem

Pass24 closed hot-path ownership for the exact-I64 `Join → Group Count → TopKWithTies` state tree, but left two concrete gaps:

1. initial tree construction re-evaluated nested child queries, so `Group::build` and `TopK::build` could fall back into repeated generic logical Join evaluation and become effectively O(n²);
2. generic maintained Join still cloned both input relations and recomputed complete before/after joins on every tiny input delta.

## 2. Hypotheses

1. Parent maintained states can be constructed from already materialized child `RelationValue` snapshots without weakening semantic validation.
2. Generic Join deltas can be derived exactly as `ΔL × oldR + nextL × ΔR`; full before/after replay is not required.

## 3. Implementation

### 3.1 Compositional child-snapshot construction

Added internal snapshot construction paths for `MaterializedGroupDeltaState` and `MaterializedTopKDeltaState`.

`MaterializedJoinGroupTopKState::build` now:

1. builds Join once;
2. materializes Join's maintained snapshot;
3. constructs Group from that snapshot;
4. materializes Group's maintained snapshot;
5. constructs TopK from that snapshot.

The old nested `evaluate()` path is no longer used by parent state construction in this fragment.

### 3.2 Generic maintained Join without full replay

Generic Join update now:

1. validates and plans both input mutations before publication;
2. derives removed/inserted Join rows only from changed rows against the opposite state;
3. uses the prospective next-left state when applying right-side changes, so simultaneous two-sided changes are counted exactly once;
4. commits both input relation mutations only after every semantic comparison succeeds.

This removes the previous full input clones + full before/after Join replay. Without a semantic index it is still O(|Δ|·n), which is retained as an explicit performance gap rather than hidden.

## 4. Falsification

- Existing owned-tree sequential delta oracle remains green.
- The owned-tree test now also compares snapshot-built Join/Group/TopK states to ordinary standalone state construction.
- Generic ASCII-CI Join test now covers both simultaneous inserts and simultaneous semantic removals with different case representatives.
- Strict Clippy remains green with no new production suppression: Pass24 17 → Pass25 17 total `#[allow(clippy::...)]` occurrences.

## 5. Performance evidence

`PASS25_STATE_TREE_BUILD_BENCH.txt`, unique-key exact-I64 tree:

- 1k rows: 1.52 ms build
- 10k rows: 14.01 ms build
- 50k rows: 74.92 ms build

The 50k initial build now completes normally; Pass24's equivalent construction did not complete in the diagnostic window because parents re-evaluated the nested logical Join. Three frozen repeats measured ~69–89 ms at 50k. The generic-Join/state-tree targeted release tests also completed 2,000 additional frozen replays without failure.

Maintained updates remain microsecond-scale in the same benchmark. This is diagnostic evidence, not a universal performance claim.

## 6. Verification gate

Final frozen state:

- 222 declared tests;
- `cargo fmt --all -- --check` PASS;
- `cargo test --workspace` PASS;
- strict workspace Clippy PASS;
- `cargo test --workspace --release` PASS;
- `cargo build --workspace --release` PASS;
- strict rustdoc PASS;
- overflow-checked release tests for `kernel-query` + `kernel-integration` PASS;
- 19 local workspace crates;
- 26,477 Rust LOC;
- zero external Cargo sources;
- zero `unsafe`;
- zero TODO/FIXME;
- zero `panic!`/`todo!`/`unimplemented!` macros.

## 7. Checklist accounting relative to Pass24 final OPEN list

Pass24 final had 14 OPEN items total: 10 inherited OPEN items plus 4 newly discovered problems.

### Closed in Pass25

- [x] compositional initial-state build: parents consume child snapshots instead of re-evaluating nested queries;
- [x] generic maintained Join no longer uses full relation clones + full before/after replay for each tiny delta.

### Closed from previous OPEN checklist

**2 / 14**.

### Still OPEN from previous checklist

**12 / 14**:

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
- [ ] general recursive maintained-plan ownership beyond fixed exact-I64 `Join→Group Count→TopK`;
- [ ] semantic indexing/canonical-key strategy for generic maintained Join so O(|Δ|·n) becomes indexed rather than linear-opposite-side scan.

### New problems discovered in Pass25

No new semantic correctness problem was discovered after freeze. The remaining generic Join O(|Δ|·n) cost is the sharpened form of the already-known generic indexing gap, not a new correctness defect.

## 8. Result

Pass25 closes the construction asymmetry left by Pass24: both initial construction and subsequent delta maintenance are now compositional for the exact-I64 owned tree. It also removes the worst generic-Join maintenance behavior (full clone + full replay), while leaving semantic indexing as the explicit next performance frontier.
