# PASS403 — bounded current-B relation delta overlay

PASS403 replaces PASS402's O(|R|) reference detachment as the intended hot-write lowering for prepared general-relation migration outputs.

The semantic authority remains `kernel-query::PreparedRelationRewrite`. A prepared general target relation may additionally prepare a physical write index before cutover. That preparation builds the exact Γ support witness and persistent physical routing metadata. It is optional physical preparation, not a fallback or second semantic engine.

After cutover, one current-B write follows:

```text
PreparedRelationRewrite_B
    -> certify exact RelationBaseWitness in O(1) authority identity
    -> update only touched Γ support paths
    -> immutable RelationRows delta atom for inserts
    -> persistent occurrence/order overlay
    -> current RealizationRoot'
```

Set removals resolve by pinned Γ canonical class, not Rust equality. Bag writes preserve exact multiplicity. No source-schema provenance, inverse migration, SQL writable-view routing, unstable semantic row ordinal, or full endpoint materialization participates.

The overlay owns persistent copy-on-write routing structures (`PersistentOrdMap` / `PersistentVec`), so realization-root cloning remains bounded. Base relation columns remain immutable. A relation scan lowers contiguous untouched base runs directly to the underlying factorized column expression and resolves only overlay occurrences separately. Compaction materializes the current endpoint back to native factorized columns and drops obsolete delta atoms/base target atoms from current reachability.

General migration preparation does not eagerly pay the write-index cost. `PreparedFactorizedRelation::prepare_write_overlay` is a pre-cutover physical specialization for relations that require bounded current-B mutation. If it is absent, bounded overlay installation fails closed; there is no O(N) fallback.

100k-row hostile measurements, release, warm runs:

- ordinary general-relation preparation retained ~31.7–33.5 ms in repeated runs; cutover ~44.6–46.9 us (same qualitative class as P401);
- write-index preparation: ~294–335 ms, one-time pre-cutover physical work;
- prepared one-row semantic rewrite after warm witness path: ~5.6–6.5 us;
- realization-root clone: ~3.5–3.8 us;
- one-row overlay install: ~22.8–26.5 us;
- full 100k-column scan with one appended overlay occurrence: stable repeated runs around native cost class (~0.92–1.01x); one 1.69x scheduler/cache outlier was observed and not treated as the conclusion.

The remaining R&D issue is reducing the one-time write-index preparation cost and eliminating duplicated Γ indexing between semantic support and physical routing. A promising next direction is one shared occurrence-support structure in `kernel-query` that carries canonical class multiplicity plus stable occurrence handles, so physical realization can reuse the exact witness instead of constructing a second canonical index.
