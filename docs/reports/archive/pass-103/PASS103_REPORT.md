# CFMD Pass103 — Stage 6 performance checkpoint

Status: **CHECKPOINT / STAGE 6 OPEN**. Parent: Pass102 Stage 5 COMPLETE.

## Result

Stage 6 was not closed. Correctness and architectural gates are green, but hostile performance/allocation evidence still falsifies production closure of historical #21/#22.

### Corrected TopK comparator

The old `maintained_top_k_bench` hand comparator was still invalid: for a `0 -> 50000` replacement it removed `0` but failed to add the displaced threshold row `10`. The comparator now independently recomputes the selected-with-ties hand state before/after and diffs those states.

This exposed an O(n) scalar TopK plan path. `I64TopKState::plan_signed` cloned the full `PagedRadix` state and predecessor/successor materialized `ordered_entries()`. Pass103 replaces the common admitted unit replacement with a compact checked `I64TopKPatch`; direct predecessor/successor queries no longer build a full ordered vector. Promotion/fallback remains read-only `Replace(next)` planning.

Post-fix release result (50k rows, k=10): maintained median **563 ns**, corrected hand baseline **670 ns**, maintained/full replay ~= **0.002x**. The 2000-step bidirectional oracle test remains green.

### Whole-tree transition planning

Stage 5 removed internal `RelationDelta` materialization but public transition still cloned the entire `MaterializedRelPlanState` before mutation. `Arc::make_mut` therefore COW-cloned heavy Join/Group/TopK state along the path.

Pass103 replaces the ordinary recursive path with immutable **whole-tree plan -> root materialize -> commit**:

- all leaf frames are validated first;
- recursive nodes produce a `MaintainedRelPlanPatch` tree plus `AdaptiveDelta` effect without mutating authoritative state;
- root `RelationDelta` is materialized before commit;
- only then is the patch tree committed and epoch advanced;
- invalid planning/materialization remains fail-atomic;
- old ordinary mutating recursive helpers were removed rather than retained as a legacy parallel path.

This reduced the observed whole-chain cost from about 30 ms/update to:

- Filter→Project→Join→Group→TopK: median **6.55 ms**, p90 **7.24 ms**;
- AntiJoin→Project→Group→TopK: median **5.58 ms**, p90 **6.19 ms**.

This is a material improvement, but not a closure-quality result.

## Remaining falsifiers

1. Corrected standalone Group benchmark remains poor: **694 ns maintained vs 137 ns hand baseline (~5.07x)**.
2. Independent allocation probe on the whole maintained chain reports **108,466 allocator calls/update** (median and p90 in the current fixture).
3. Required multi-process release distributions and full workload matrix were not promoted to closure evidence because the above two falsifiers already block Stage 6.
4. Therefore historical #21/#22 remain OPEN; no PROD CLOSED claim is made.

## Verification

- `cargo fmt --all -- --check`: PASS
- `cargo check --workspace --all-targets`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo test --workspace --all-targets`: **700 declared / 692 passed / 0 failed / 8 ignored**
- recursive targeted tests: 8/8 PASS
- atomic targeted tests: 8/8 PASS

## Next action

Attack allocator and Group constant-factor residuals before any further closure benchmarking. The first target should be the Group I64 count planner/commit path and whole-tree patch allocation topology. Only after those are paid should Stage 6 rerun independent multi-process distributions, dense/sparse/outlier/tie workloads, allocation gates, endpoint equivalence, and stale/failure boundaries.
