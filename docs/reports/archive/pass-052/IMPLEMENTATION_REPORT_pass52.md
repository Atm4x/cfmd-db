# IMPLEMENTATION REPORT — Pass52

## problem

Pass51 established an exact maintained Γ-QCN support-state boundary, but every affected fine relation delta still replaced the complete support fixed point. Even a pure row deletion therefore rebuilt canonical support state globally despite the derivative being monotone: support can only disappear.

## hypotheses

1. Pure deletions admit a strictly stronger derivative than universal `Replace`: no removed row can create new support.
2. Stable `PhysicalRowId` is the correct coordinate transport across dense-position changes; local maintenance must not depend on a deleted row being a suffix or on ordinal stability.
3. After exact old→new handle remapping, support loss can propagate through the Γ-QCN dependency graph with a monotone work queue and remain an exact refinement of Pass51 `Replace`.
4. Insertions/resurrection are qualitatively different: mutually supporting rows may activate as a fixed-point SCC, so they must retain the exact Replace fallback until a separately proved activation derivative exists.

## implementation

- added a deletion-local derivative for materialized Γ-QCN support state;
- arbitrary pure deletions are recognized by exact stable-handle subsequence transport rather than physical position special cases;
- base masks and quotient leaf canonical-key arrays are remapped through `PhysicalRowId` without payload recanonicalization;
- affected quotient buckets are rebuilt only for changed leaves;
- a quotient-constraint dependency queue propagates monotone support loss across coordinates until convergence;
- relation delta maintenance tries the local derivative after canonical quotient-factor maintenance and falls back to Pass51 exact Replace if the local preconditions do not hold;
- insertions and mixed deltas deliberately remain on the exact Replace path;
- added `semantic_quotient_support_local_delta_updates` as reconstructible physical observability of successful local derivatives.

## hostile falsification

- an initial suffix-only implementation was rejected as an architecture smell because local maintainability would have depended on physical row position;
- stable-handle remapping handles both suffix and interior deletion;
- deletion from a 100-row duplicate canonical-key bucket exercises masks beyond one machine word and remains exact;
- support loss cascades across multiple quotient coordinates through the dependency queue and matches the fresh Γ-QCN fixed-point oracle;
- insertion after the deletion sequence does not take the local path and correctly falls back to full Replace;
- all post-transition prepared results equal fresh logical evaluation;
- no payload recanonicalization or complete support fixed-point rebuild is needed on the successful deletion-local path;
- strict Clippy passes with no new suppression.

## result

Pass52 refines the Pass51 correctness derivative for arbitrary pure deletions:

```text
Fine(delete-only RelationDelta)
        -> stable-handle coordinate transport
        -> local quotient bucket/mask removal
        -> monotone dependency-queue propagation
        -> exact maintained Γ-QCN support state
```

This is a real fine `Dq` fragment rather than a cache invalidation shortcut. It is still intentionally partial. Insertions/resurrection and mixed deltas use the universal exact `Replace` derivative because support can grow through mutually supporting fixed-point components; no unsound symmetric "turn bits back on" rule was introduced.

## external R&D

The independent structural canonical-key / fixpoint-adjacency investigation was interrupted before delivering its final bundle. None of that parallel code is integrated in Pass52. The reported observations remain R&D evidence only until an actual patch/ZIP/raw results can be hostile-reviewed against the authoritative branch.

## verification

Rust 1.98.1 full fmt/check/debug-test/strict-Clippy/release-test/release-build/strict-rustdoc/overflow-release gate: PASS. Release and overflow invocations that hit external compilation timeouts were treated as neither PASS nor FAIL and rerun separately to actual completion.

Metrics: 358 declared tests; 130 `kernel-plan` (129 normal + 1 ignored diagnostic benchmark); 21 crates; 49,389 Rust LOC; 19 pre-existing `#[allow]` sites and no new suppression; 0 external Cargo sources; 0 `unsafe`.
