# IMPLEMENTATION REPORT — Pass65

Pass65 is a hostile-reviewed integration of R&D Program5 on top of frozen Pass64. No unrelated planner/lifecycle feature work was added.

## Production changes

### kernel-model

`DatabaseState::normalize_certified()` now returns the normalized state together with the final shared `DenseEntityIds` and `LiveRefSensitivityIndex`, so `Revision::build` no longer compiles those derivatives twice when normalization has already established them.

`LiveRefSensitivityIndex` now separates immutable field sensitivity roots from `Arc`-shared per-relation partitions. `with_relations_recompiled` refreshes only touched relation partitions while sharing untouched payloads.

### kernel-validation

`DenseTypeExtents` can be supplied to `validate_state_with_extents`; `validate_relations_with_extents` validates only a certified touched relation set against already established revision-local type extents.

### kernel-revision

`Revision` retains `DenseTypeExtents`. `RelationUpdateCandidate<'a>` is source-bound to the exact Revision that created it; callers can replace relation rows and then consume the candidate with `build()` but cannot substitute another source Revision.

The candidate build revalidates pinned Γ, locally performs the relation-row LiveRef normalization that full `Revision::build` would perform, validates only touched relations, recompiles only touched LiveRef sensitivity partitions, and reuses source dense identity/lifecycle/type extents.

The defensive `Revision::build_relation_update` entry point remains for arbitrary externally supplied states and checks static state plus all untouched relations before entering the certified path.

### kernel-plan

Relation-data WAL recovery now constructs a source-bound candidate from the current authoritative Revision, applies durable relation deltas to that candidate, and uses the certified incremental compiler instead of generic full `Revision::build` for every committed relation-data record.

## Hostile corrections to R&D

The R&D patch was not accepted verbatim. Pass65 fixed three correctness holes before production integration:

1. cross-source candidate rebinding;
2. omitted pinned-Γ registry validation;
3. semantic mismatch with full normalization for dangling nested LiveRef rows.

See `PASS65_RND_PROGRAM5_REVIEW.md`.

## Performance evidence

40 × 5,000-row release diagnostic, seven runs, clone/drop outside compiler timing:

- full compiler median: 3.407 ms;
- hardened certified compiler median: 0.294 ms;
- compiler-only ratio: 11.604×;
- candidate full-state clone median: 4.331 ms.

This closes redundant full revision compilation for the certified relation-only boundary, not persistent logical snapshot construction.

## Verification

Rust 1.98.1 full workspace gate PASS on frozen source: fmt, check, debug tests, strict Clippy, release tests, release build, strict rustdoc and overflow-check release tests. Cold long-running release/overflow invocations that hit the external timeout were not counted; warmed retries completed.
