# CFMD Implementation Report — Pass56

**Status:** VERIFIED, Rust 1.98.1.

Pass56 removes maintained Join `GenericScan` for the currently admitted primitive+structural Γ equality universe.

## Production change: `kernel-query`

Added `StructuralIndexedJoinSide` and `StructuralIndexedJoinStorage`:

- structural values are keyed through `SemanticRegistry::canonical_equivalence_key` under pinned Γ;
- maintained rows have stable `IndexedRowId` identities;
- exact canonical buckets preserve deterministic insertion order;
- reverse identity→key state makes removal exact;
- structural output probes buckets rather than rescanning the opposite relation;
- structural delta maintenance plans both sides before commit, probes only matching canonical buckets, and keeps semantic row equality for identity/set-validation inside a narrowed class.

Deleted `MaintainedJoinStorage::GenericScan` and its Join-specific scan delta machinery. Generic relation mutation planning remains because maintained Scan leaves use it for exact delta validation/commit.

## Hostile coverage

The former structural-fallback test is converted into a structural-index test. A structural Product using `TextAsciiCaseInsensitive` proves differently represented values join through one canonical class. Independent left/right insertions are then applied through maintained state and compared with `rel_delta_by_recompute` under Γ.

Workspace search reports zero `GenericScan` symbols after freeze.

## Boundaries

No logical Join law, semantic equality law, durable key encoding, persisted structural index format or Γ-QCN structural factor format changed. Future custom/plugin equality without a certified canonical representation remains a broader OPEN rather than a hidden current scan path.

## Verification

All mandatory Rust 1.98.1 gates pass on frozen source: fmt, workspace check, debug tests, strict Clippy, release tests, release build, strict rustdoc and overflow-check release tests.

Metrics: 365 declared tests, 72 `kernel-query`, 130 `kernel-plan`, 21 crates, 50,534 Rust LOC, 19 pre-existing lint suppressions, no new suppressions, 0 unsafe, 0 `GenericScan` symbols.
