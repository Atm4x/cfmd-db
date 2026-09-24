# Pass78 Γ-SAMF R&D review

Input: `CFMD_RND_GAMMA_SUPPORT_ATOM_MATERIALIZATION_FABRIC_2026-09-21 (1).zip`.

## Decision

**Accepted in staged form.** Pass78 integrates the common support-atom substrate and one production materialized state, but does not perform destructive migration of legacy artifact families.

## Claims accepted

1. For a finite relation support and active Γ-observables `Q`, the product observable induces an exact partition into support atoms.
2. Every single-observable fiber is the disjoint union of atom fibers whose product class projects to the requested observable class.
3. Counts/statistics are exact sums of atom masses; factorized inverse projection need only store atom/class references rather than duplicate row identities.
4. A determinant generator contained in `Q` and closing `Q` may encode the same partition as the full demanded product, but generator choice is physical, not semantic authority.
5. Removing observables may safely retain an over-refined partition; insertion may require refinement, with the full demanded product as a terminating fallback.

## Production corrections relative to the prototype idea

The first candidate implementation introduced a new local support-atom integer namespace. Hostile review rejected that as redundant and alias-prone. Production atom identity is instead the product observable's catalog-local `EqClassId`, reusing the Pass77 nominal boundary.

## What was integrated

- `SupportAtomFabric<RowId>`;
- product-class atom identity;
- exact joint/projected fibers;
- production `MaterializedObservableAtomState<PhysicalRowId>`;
- certified product projection;
- count/mass adapter behavior;
- relation-delta maintenance;
- physical COW publication and retained-size accounting;
- parity tests against legacy semantic index/statistics behavior.

## What remains deliberately separate

- global `ObservableDemand` advisor;
- exact/pruned Pareto overlay selector;
- durable SAMF recipe/recovery;
- ordered/annotation/I64 overlays as the unique representation;
- deletion of legacy semantic index/statistics/quotient/Group/TopK states;
- generated generic differential compiler.

Legacy states remain necessary as falsification oracles until crossover/randomized equivalence is strong enough to retire them.
