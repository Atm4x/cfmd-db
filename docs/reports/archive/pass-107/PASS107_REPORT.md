# PASS107 — ExecGraph V4 production runtime cutover

## Status

- Versioned State V3: **COMPLETE** (carried from Pass105/106).
- ExecGraph V4 unified scheduler: **production planner/patch cutover COMPLETE**.
- ExecGraph V4 flat authoritative NodeId state arena: **OPEN**.
- Stage 6: **OPEN**; Group constant-factor gap remains the main measured residual.
- Historical #21/#22: **OPEN** pending final Stage-6 matrix/closure criteria.

## Problem → hypothesis → implementation → falsification → result

### 1. Revision-candidate path still bypassed V4

**Problem.** Pass106 moved ordinary semantic and storage-resolved public transitions to V4, but `candidate_from_storage_resolved_deltas_for_revision` still reached the old mutable resolved-recursive stack through `apply_storage_resolved_deltas_in_place`.

**Hypothesis.** The revision candidate can use the same validated leaf frames, V4 NodeId scheduler, `GraphPatchSet`, root materialization-before-commit, and transition epoch protocol as the public path.

**Implementation.** Replaced the old in-place resolved recursion with:

`source preflight → validate_resolved_leaf_frames → plan_relation_deltas_execgraph → root materialize → commit_graph_patch_set → epoch publish`.

Source membership now uses the compiled V4 source index rather than recursive `contains_scan_relation`.

**Falsification.** Kernel-query tests and the full workspace gate run with debug shadow parity enabled. Any V4/recursive root-delta or final-state mismatch returns `InconsistentIncrementalDelta`.

**Result.** Green. Revision publication candidate now shares the same V4 planner/patch contract.

### 2. Old resolved recursive executor remained in production source

**Problem.** After the revision-candidate cutover, the previous resolved mutation/propagation functions became unreachable.

**Implementation.** Deleted the dead stack rather than suppressing warnings:

- `validate_resolved_leaf_deltas`
- `apply_resolved_leaf_deltas`
- `propagate_resolved_deltas_inner`
- `contains_scan_relation`
- old mutating Join/Blocker/Group/TopK/SetSupport delta helpers
- `BlockerUpdate`
- obsolete `maintained_delta_from_view`

**Result.** Strict Clippy is clean without dead-code allowances.

### 3. Pass106 Clippy blockers

**Problem.** Two nested delivery error paths and an oversized `plan_execgraph_node` prevented strict Clippy closure.

**Implementation.** Collapsed delivery conditions and split stateful barriers into `plan_execgraph_stateful_node`.

**Result.** Dev and release strict Clippy pass.

### 4. Recursive planner still compiled into release

**Problem.** The recursive planner is valuable as a debug differential oracle, but after V4 cutover it was dead in release builds.

**Implementation.** Gated the recursive patch-tree planner, its plan context, and commit machinery with `cfg(debug_assertions)`.

**Result.** Debug/test keeps V4-vs-recursive parity checking. Release physically excludes the old planner and compiles cleanly with `-D warnings`.

## Verification

- `cargo fmt --all -- --check`: PASS
- `cargo check --workspace --all-targets`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo clippy -p kernel-query --release --all-targets -- -D warnings`: PASS
- `cargo test -p kernel-query --all-targets`: 103 passed / 0 failed / 1 ignored
- `cargo test --workspace --all-targets`: **698 passed / 0 failed / 8 ignored (706 declared)**

Release Stage-6 smoke after cutover:

- `linear_join_group_topk`: median **13.020 µs**, p90 **13.480 µs**
- `blocker_group_topk`: median **10.746 µs**, p90 **11.187 µs**
- allocation probe before the final cfg-only cleanup: median **142 allocator calls/update**, p90 142, max 144

No material performance regression from the V4 runtime cutover is visible against Pass104/105 measurements.

## Production diff

Relative to frozen Pass106, production source changes are confined to:

- `crates/kernel-query/src/lib.rs`

`execgraph.rs` is unchanged from the canonical V4 implementation integrated in Pass105/106.

## Remaining frontier

1. Move mutable operator state from the recursive ownership tree to a flat NodeId arena (or an equivalent stable flat runtime container). This removes per-transition topology discovery and makes NodeId the physical state coordinate, not only the scheduler coordinate.
2. Make V4 scheduling scratch reusable in that runtime container; do not place ephemeral scratch into authoritative revision semantics.
3. Once arena parity is established, retire the recursive tree as a production state layout; retain a separate debug/reference oracle only if useful.
4. Finish Stage 6: attack the remaining Group constant-factor gap, then run the corrected multi-process workload matrix and allocation distributions before deciding #21/#22 closure.
