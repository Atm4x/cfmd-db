# Pass78 Γ-Observable Measure Fabric R&D orientation

Input reviewed: `CFMD_RND_GAMMA_OBSERVABLE_MEASURE_FABRIC_2026-09-21(1).zip`.

## Decision

The R&D is architecturally relevant but was **not integrated in Pass78**.

The proposed `MaterializedObservableNode` / observable-measure DAG deliberately overlaps current production families (`SemanticIndex`, semantic quotient factor/support, semantic statistics, Group/Distinct/TopK state and their advisors). Integrating more family-specific lifecycle structure before deciding the OMF substrate would create migration debt.

## Constraints accepted for future mainline work

1. Treat index/statistics/quotient/operator state as candidate projections/capabilities over one Γ-observable finite-measure substrate rather than assuming they are permanently separate semantic state families.
2. Γ refinement and revision-local APNF determinant morphisms should share one morphism/materialization DAG boundary.
3. A coarse observable that factors through an already materialized finer observable should be derivable by exact pushforward; raw-row recanonicalization is then a physical choice, not a semantic necessity.
4. Ordered views/TopK need an ordered-measure capability and may still require specialized order-statistics physical lowering.
5. Do not merge advisors before the common observable substrate exists.

## Why Pass78 remains compatible

Pass78 adds only allocator-independent semantic input-work accounting (`SemanticWorkEstimate`) and recovery admission/scheduling based on that metric. It does not add a new artifact family, advisor ontology or query executor. The metric is defined below the current artifact families and can be reused by a future OMF node, fiber, annotation cache, factorized materialization or APNF-derived observable.

## Explicit non-integration

No R&D prototype source was merged. No current semantic index, quotient factor, statistics, Group or TopK state was rewritten as OMF. No executor path was switched.


## Pass78 final disposition

The later Γ-SAMF closeout refined this direction enough for staged production integration. Pass78 therefore *did* add the common support-atom substrate and `MaterializedObservableAtomState`, while retaining all legacy artifact families as oracles. The earlier non-integration decision remains accurate only for the broad OMF prototype and destructive family/advisor migration. See `PASS78_GAMMA_SAMF_RND_REVIEW.md`.
