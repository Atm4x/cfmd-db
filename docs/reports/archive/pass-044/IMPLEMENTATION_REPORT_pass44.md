# IMPLEMENTATION REPORT — Pass44

## problem

Pass43 left multiway Join planning and cross-family access costing open. Hostile source audit additionally found that several legacy I64 Join fast paths bypassed the Pass42 scan-vs-index decision, and non-I64 primitive joins had no profitable transient canonical-index option.

## hypotheses

1. Access costing must apply to every current physical Join family, including fused/batch specializations.
2. A transient primitive semantic index is valid derivative execution state if and only if its key is certified by pinned Γ and candidates are revalidated.
3. Reassociation should first target a narrow provably correct fragment: connected contiguous equality-Join trees, preserving leaf order and exact Bag/output semantics.
4. An indexed right base relation must remain probeable when the left input is already an intermediate result; otherwise reassociation cannot exploit later indexes.

## implementation

- added costed ephemeral-join decision support to `SemanticAccessCostModel`;
- gated ordinary, persisted-I64, ephemeral-I64, `Join -> Project` and typed-batch I64 fast paths through explicit work comparison;
- prevented a rejected persisted I64 path from immediately falling through to an uncosted ephemeral rebuild;
- added execution-local primitive semantic Join indexes for profitable non-I64 joins;
- added multiway equality-tree extraction, interval dynamic programming and physical reassociation for contiguous/in-order leaves;
- included `FilterEqColumns` as equality edges with exact column remapping;
- added indexed direct-right probing from intermediate left rows;
- reused persisted generic semantic and specialized I64 indexes where current statistics make them profitable;
- refactored planner/probe helpers to satisfy strict Clippy without new lint suppressions.

## hostile falsification

- tiny persisted I64 Join is rejected and remains scan;
- fused/batch I64 specializations cannot bypass cost gating;
- large Text Join builds a profitable transient Γ-canonical index and matches logical reference;
- hostile `A ⋈ (B ⋈ C)` reassociates and consumes indexes after intermediate execution;
- `FilterEqColumns`, Bag duplicates and output order survive reassociation;
- release diagnostic demonstrates the intended avoidance of a large hostile intermediate without being treated as a universal speed claim.

## verification/result

Final Rust 1.98.1 fmt/check/debug/release/strict-Clippy/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 332 declared tests, 104 `kernel-plan` tests (103 normal + 1 ignored diagnostic benchmark), 21 crates, 45,361 Rust LOC, 0 external Cargo sources, 0 `unsafe`.

Ignored hostile release benchmark: 5,025,458 ns original physical order vs 32,148 ns reassociated median (~156.322x) on the constructed fixture.

## rejected routes

- no arbitrary leaf permutation/general bushy optimizer claim;
- no structural/custom fake canonical keys;
- no index-always-wins threshold;
- no installation/durability of transient execution indexes;
- no new Clippy suppressions to hide planner shape.

## ledger transition

- concrete problems closed in Pass44: **3**;
- historical Pass43 OPEN fully closed: **0 / 22**;
- historical active OPEN after Pass44: **22**;
- new architectural OPEN created: **0**.

## recommended next step

Unify current Join access families behind one first-class physical candidate/decision interface, then reuse it from direct and multiway planning. That is the cleanest prerequisite for arbitrary leaf permutation, broader layouts and richer selectivity statistics.
