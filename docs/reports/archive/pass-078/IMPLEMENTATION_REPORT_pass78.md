# IMPLEMENTATION REPORT — Pass78

Pass78 now contains two compatible production foundations: size-aware semantic work accounting and the first staged Γ-SAMF integration. It deliberately does not delete legacy physical families or switch the general multiway executor.

## Production files

- `crates/kernel-semantics/src/lib.rs`
- `crates/kernel-semantics/src/observable.rs`
- `crates/kernel-semantics/src/support_atom.rs` (new)
- `crates/kernel-plan/src/lib.rs`
- `crates/kernel-plan/src/algebraic_native.rs`

## Semantic work accounting

`SemanticWorkEstimate { value_nodes, payload_bytes }` is a deterministic logical-shape metric for `Value` and `CanonicalEqKey`. Native row/value/I64/typed-algebraic layouts compute the same metric directly. Recovery has a separate advisor semantic-work ceiling and ranks optional rebuilds by structural work before key-cell count. This is a planning metric, not a CPU/RSS claim.

## Γ Support-Atom Materialization Fabric

`SupportAtomFabric<RowId>` is a finite revision-local partition under one product observable. Product `EqClassId` is the support-atom identity. The fabric keeps atom row fibers, reverse row membership, exact product signatures and per-coordinate inverse atom projections.

`MaterializedObservableAtomState` binds the generic fabric to a physical relation and real `PhysicalRowId`, creates the required revision-local observables/product through `RevisionObservableCatalog`, and retains a certified product projection morphism. It exposes exact joint probes, projected probes and count/mass views.

The state is reconstructible physical data only. No revision-local observable/class ID is durable semantic authority.

## Maintenance and publication

Relation deltas update the SAMF state atomically with the relation. Γ drift is rejected before mutation. Layout replacement invalidates the derivative. Runtime publication follows the existing immutable/COW physical-root boundary and never changes logical `Revision=(S,Γ,M)`.

## Migration discipline

Legacy semantic index/statistics and related families remain intact as differential oracles. Pass78 does not yet perform advisor unification, durable SAMF recovery, Group/TopK retirement, global Pareto overlay selection or generated differential maintenance.

## R&D interaction

The Γ-SAMF R&D was integrated only through the Stage A/B substrate above. The later Γ-GCC bundle was reviewed but not merged; it is a candidate shared semantic closure substrate for determinant saturation, lifecycle reachability and finite positive recursion.

## Verification

Frozen source at `2026-09-21 15:44:11 UTC` passed the complete Rust 1.98.1 gate. Debug, release and overflow-release each report **483 passed / 0 failed / 8 ignored**. Exact warmed workspace release and overflow commands were run after package-level warmup. Freeze/post-gate source inventories are identical.
