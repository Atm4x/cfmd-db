# IMPLEMENTATION REPORT — Pass77

Status: **VERIFIED**.

Pass77 integrates the semantic foundation from the Γ-observable/measure and Γ-APNF R&D while deliberately leaving the general multiway executor unchanged.

## Production changes

### Exact structural finite-measure keys

- unordered `Set` / `Bag` / `Map` canonical equality keys now use one finite counting-measure normal form;
- coarse Γ collisions are aggregated explicitly rather than producing duplicate child keys that the durable decoder rejects;
- canonical-key durable encoding moved to revision **v2** and semantic-index key binding revision moved with it, so old derived cache/index state rebuilds instead of being silently reinterpreted.

### Revision-local semantic observable foundation

- `RevisionObservableId` / `EqClassId` are nominal revision-local realization identities;
- separate observable catalogs cannot alias IDs even for the same `SemanticRevision`;
- pinned Γ equivalences and product observables are exact observable definitions;
- `CertifiedSemanticMorphism` is n-ary -> m-ary and remains catalog/revision-bound;
- product projection and materialized/composed morphisms are reconstructible derivatives.

### Γ Anchor-Pullback substrate

New `kernel-semantics::anchor_pullback` implements:

- `RevisionFiniteMeasure` with exact weighted support;
- safe finite-factor derivation of hyper-determinant morphisms;
- `DeterminantTheory` as the least closure induced by certified morphisms;
- incidence/worklist closure and value propagation;
- deterministic inclusion-minimal generator selection without minimum-key claims;
- `AnchorMeasureState` with lossless weighted reconstruction;
- `AnchorPullbackNormalForm` query-local core;
- determinant-backed branch-free certificates.

Direct/transitively redundant morphisms are retained: logical redundancy never implies physical eviction.

## Non-claims

Pass77 does not replace the current JOIN executor and does not close the historical general multiway item. APNF lowering from arbitrary query IR, incremental/durable anchor maintenance, the residual pullback engine and crossover benchmarking remain OPEN.

The determinant closure is the semantic object; a minimized FD list is not durable authority.
