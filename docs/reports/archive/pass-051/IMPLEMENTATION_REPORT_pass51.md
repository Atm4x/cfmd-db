# IMPLEMENTATION REPORT — Pass51

## problem

Pass50 prepared Γ-QCN reused its semantic basis and delta-maintained canonical endpoint factors, but N-way common domains, support masks and the cross-coordinate support fixed point were still rebuilt on every execution. No exact atomic maintenance boundary existed for that dependent state.

## hypotheses

1. The support fixed point is reconstructible physical state, not semantic authority.
2. Existing CFMD change semantics already supplies a correct universal derivative: for a fine relation delta, the dependent support artifact may use an exact `Replace(fresh_support_state)` fallback.
3. Rebuilding that Replace state inside the existing unpublished physical candidate transition preserves atomicity without inventing a second invalidation graph.
4. Fine local propagation can remain a later optimization if the Replace boundary is exact and observable.

## implementation

- added dedicated `semantic_quotient_supports` state to `PhysicalStore`;
- added exact support binding over prepared quotient specs and participating relation/layout leaves;
- support state stores exact pinned semantic context, current stable-handle vectors, quotient constraints and converged base support masks;
- added `PreparedPlan::materialize_semantic_quotient_support`;
- support materialization first ensures Pass50 canonical quotient factors exist;
- prepared execution consumes compatible maintained support state before rebuilding Γ-QCN constraints/fixed point;
- added `ExecutionStats.multiway_join_maintained_quotient_support_hits`;
- relation reinstall invalidates dependent support state;
- exact relation delta updates factors first, then rebuilds each affected support program from the updated factors before one transition epoch is published;
- invalid transitions remain atomic because mutation occurs only on the unpublished candidate store.

## hostile falsification

- fresh factor-backed execution still matches logical reference;
- support materialization is idempotent;
- support-backed execution matches the same reference and records a support hit with zero endpoint-key lookups on the read path;
- missing-row removal is rejected with the entire `PhysicalStore` exactly unchanged;
- valid delete+insert maintains factors and replaces dependent support state atomically;
- post-delta support-backed execution again matches fresh logical reference;
- targeted test passes in debug and release;
- full strict Clippy passes with no new suppression;
- exact pinned `SemanticContext` + stable-handle coherence are required before a support state is consumed.

## result

The correctness layer of incremental Γ-QCN support maintenance is now production-integrated through the universal exact Change/Replace derivative. Repeated prepared reads no longer rebuild the support fixed point once explicitly materialized.

This does **not** close fine-grained incremental efficiency. An affected support program is currently recomputed from maintained factors on each input delta; multi-relation candidate construction may therefore rebuild it more than once. Queue/support-count propagation and revision-level coalescing remain OPEN.

## external R&D

The parallel structural canonical-key / fixpoint adjacency work reported by the independent R&D agent was not merged. It remains R&D evidence pending receipt of the promised ZIP/patch/raw results and hostile review against the authoritative Pass51 workspace.

## verification

Rust 1.98.1 full fmt/check/debug-test/strict-Clippy/release-test/release-build/strict-rustdoc/overflow-release gate: PASS.

Metrics: 357 declared tests; 129 `kernel-plan`; 21 crates; 49,003 Rust LOC; 19 pre-existing `#[allow]` sites and no new suppression; 0 external Cargo sources; 0 `unsafe`.
