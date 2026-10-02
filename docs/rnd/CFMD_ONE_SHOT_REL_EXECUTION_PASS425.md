# CFMD PASS425 — one-shot relational execution law

## Problem

One-shot migration preparation must evaluate exact relational semantics over factorized physical authority without constructing a complete source `FiniteModel`, without staging a complete target `Vec<Row>`, and without paying for long-lived maintained Query/Watch indexes.

## Law

```text
RelExecutionSource(P, rho)
    -> exact one-shot operator lowering
    -> RelExecutionSink
    -> physical columns + Γ witness
```

Memory is bounded by output authority plus mathematically inherent operator state. For Bag Union this state is O(1) beyond output. For Bag Difference it is the right-side canonical multiplicity map. For AntiJoin it is the right-side canonical blocker-key support.

Operator-specific lowerings are implementations of this law. They are not semantic routing to independent engines. If an exact lowering is missing, preparation fails closed.

## P425 implementations

- Scan: factorized row visitor.
- Bag Union: direct factorized column-range transfer for direct Scan children; recursive exact row lowering otherwise.
- Set Union: exact canonical support suppression in the recursive lowering.
- Difference: exact Set support subtraction / Bag natural monus via canonical blocker state.
- AntiJoin: exact blocker-key support; right multiplicity is intentionally irrelevant after support becomes nonzero.
- Sink: columnar physical authority; `RelationBaseWitness::build_columnar` creates the final occurrence root without rebuilding target rows.

## Rejected paths

- `MaterializedRelPlanState` bootstrap for one-shot work: exact but ~10.3x slower on the P424 100k Bag Union hostile because it constructs maintained Scan/position authority.
- fallback to the old full-row evaluator for missing one-shot operators.
- fixed batch-size tuning as a substitute for the ownership law.

## Next proof/implementation work

Transport exact occurrence evidence through Set/Join-capable lowerings where derivable; add Filter/Project/Distinct/Join; then Group/TopK with only inherent annotation/order state. Only after this execution/evidence surface is stable should durable PhysicalAtoms/RealizationRoot become the next architecture line.
