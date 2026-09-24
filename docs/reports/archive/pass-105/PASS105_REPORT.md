# CFMD Pass105 — Versioned State V3 + ExecGraph V4 Unified Stage A

Status: **CHECKPOINT / STATE V3 COMPLETE / EXECGRAPH V4 STAGE A COMPLETE / STAGE 6 OPEN**.
Parent: Pass104 Stage6 kernel/allocation checkpoint.

## Problem → hypothesis → implementation → falsification → result

### 1. Versioned State V3 production rebase

Problem: Pass104 still needed the final V3 runtime convergence contract: Group publication could retain recoverable decisions until commit, and the unrevisioned storage-resolved hot path needed to share the ordinary immutable maintained planner.

Hypothesis: rebase the V3 invariants, not its older Pass103 source snapshot, preserving the newer Pass104 canonical Scan lookup and TopK changed-bucket overlay.

Implementation:
- added `sealed_group_v3.rs`;
- Group now plans `plan_delta_view -> seal_group_patch` and commits only a total `SealedGroupPatch`;
- semantic and storage-resolved Scan frames converge on `MaintainedScanCommitPatch` and the same recursive maintained patch tree;
- the obsolete unrevisioned `PreparedMaterializedRelPlanTransition` detached candidate path is absent;
- revision-bound detached candidate publication remains intentionally preserved;
- Pass104 canonical Scan lookup and TopK allocation fixes were retained;
- recursive build re-entry was removed: one root differential/physical program is compiled and shared through child state construction.

Falsification:
- V3 integration-guide symbol audit;
- storage-resolved mismatch/atomicity tests;
- sealed Group tests;
- full workspace fmt/check/clippy/test gates.

Result: **Versioned State V3 COMPLETE in production**.

### 2. ExecGraph naming correction and V4 replacement

Problem: an initial Pass105 scaffold followed the older ExecGraph V3 two-lowering design (`StraightLineRoute` / `SparseMaskRoute`). The later `CFMD_RND_EXECGRAPH_V4_UNIFIED_2026-09-23.zip` explicitly supersedes that physical split.

Hypothesis: State V3 is orthogonal to graph scheduling, so replace only the preliminary scheduler layer while preserving the V3 state/patch contract.

Implementation:
- deleted the V3 `SourceLowering`, `StraightLineRoute`, `SparseMaskRoute`, `RouteCostModel`, `ExecutionRouteKind`, and `ExecutionLoweringStats` design;
- compile one immutable postorder `PreparedRelGraph` for every current `RelExpr` shape;
- compile source relation -> consumer edge targets and node -> consumer edge targets;
- introduced one `UnifiedTransitionProgram`, with no compile-time/runtime route choice;
- introduced the V4 reusable scheduling machinery: one continuation register, hierarchical 64-way activation queue, and lazily paged inbox arena;
- source ingress validation now uses the precompiled graph source table rather than recursive source discovery;
- added all-shape, repeated-source, queue ordering/dedup, continuation/spill, and failed-reset tests.

Falsification:
- repository-wide grep confirms no retired V3 route symbols remain;
- kernel-query: 103 passed / 0 failed / 1 ignored;
- full workspace: 698 passed / 0 failed / 8 ignored.

Result: **ExecGraph V4 Stage A COMPLETE**: the production compile artifact and unified scheduling machine are integrated for all relational node shapes.

### 3. What Stage A deliberately does not claim

Mutable operator state is still owned by the recursive `MaintainedRelPlanNode` tree. The ordinary transition still uses the proven whole-tree immutable recursive planner for semantic execution. V4 has not yet become the kernel-dispatch executor and there is not yet one flat NodeId-addressed `GraphPatchSet` commit arena.

That is the next integration stage, not a hidden completion claim. The V4 R&D map itself recommends migrating barriers incrementally and differential-testing against the recursive oracle before deleting it.

## Gates

- `cargo fmt --all -- --check`: PASS
- `cargo check --workspace --all-targets`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo test --workspace --all-targets`: PASS
- declared: 706
- passed: 698
- failed: 0
- ignored: 8

## Historical / Stage6 status

Historical closure remains **14 / 22 PROD CLOSED**. #21/#22 remain OPEN. Stage6 remains OPEN because the standalone Group constant-factor residual measured in Pass104/early Pass105 remains materially above the hand baseline and the V4 scheduler has not yet taken over runtime kernel dispatch.

## Next frontier

1. Split mutable maintained state into a NodeId-addressed arena without changing kernel semantics.
2. Make `UnifiedTransitionProgram` drive real `AdaptiveDelta` mailboxes.
3. Migrate Difference/AntiJoin first, then Join, Group, TopK, set-support and linear islands, with shadow parity against the recursive planner at each step.
4. Accumulate one guarded `GraphPatchSet`; keep publication/commit fail-atomic.
5. Delete the recursive executor only after full differential parity.
6. Re-run Stage6 multi-process workload/allocation matrix and address the remaining Group constant-factor gap before considering #21/#22 closure.
