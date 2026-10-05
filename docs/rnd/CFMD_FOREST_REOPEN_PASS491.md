# PASS491 — Forest-backed relational causal reopen

## Selected law

For one semantic epoch, the reconstructible relational causal state is a multi-root `RelObservationForest`. Durable observations keep only their existing `(effect_id, observation_id, observed frontier, RelExpr)` authority. Forest/node identity is derived runtime representation.

Reopen therefore performs:

```text
collect unique query roots
-> build one forest at boundary world
-> one backward exact-history sweep
-> each relevant effect: one forest rewind
-> emit observation root views sharing the reconstructed forest Arc
```

Causal certification groups observation capsules by shared forest snapshot, plans proposed deltas once per forest, and projects per-root impacts. This preserves root-specific causal outcomes without replaying shared Γ-DTC cells.

## Scaling law

For history length H, unique forest cells N, exact fanout F and actual historical forest transitions T, reconstruction target is:

```text
O(H + N + actual unique-cell transitions + exact fanout)
```

not independent per-root maintained-plan work. In the P488 64-filter / 4-hot-effect workload, the reconstructed forest has 65 cells (one shared Scan + 64 filters), 64 fanout edges and performs four forest rewinds. Actual visited cells are 65 * 4 = 260 instead of duplicating the Scan under each root.

## Hostile finding — construction payer remains

`RelObservationForest::intern_subtree` currently initializes each canonical parent cell by transiently building a full `MaterializedRelPlanState` for that subtree and retaining only its root local state. The resulting forest is canonical and transition work is shared, but initial construction may repeatedly scan/materialize source data across many roots.

This must not be hidden behind a cache. The next exact architecture step is bottom-up local-cell initialization: recursively intern children once, obtain their maintained outputs, and construct only the parent-local Γ-DTC state from those child outputs using existing node kernels. That should make forest build work align with canonical unique cells rather than independent root subtrees.
