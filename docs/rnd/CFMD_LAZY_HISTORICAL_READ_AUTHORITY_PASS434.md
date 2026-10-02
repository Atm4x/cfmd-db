# CFMD PASS434 R&D — lazy historical read authority

## Problem

PASS433 made a retained historical realization root complete durable authority, but `revision_at()` still materializes its entire logical `DatabaseState` before ordinary reads can use it. That is correct, but it is not the desired large-history read cost.

A tempting implementation is to put a cloned `PhysicalAtomStore + HistoricalRevisionRoot` inside `ReadContext`. Hostile review rejects this: `PhysicalAtomStore` is currently a `BTreeMap<PhysicalAtomId, PhysicalAtom>`, so cloning it clones the complete atom map/payload graph. A lock-free historical read handle implemented that way would reintroduce O(all reachable atoms) memory per handle.

## Closed substrate in P434

`kernel-realization::evaluate_relation_expr_factorized` now executes the complete current `RelExpr` IR directly over:

```text
FactorizedRealizationRoot
+ PhysicalAtomStore
+ SemanticContext
+ SemanticRegistry
```

without first materializing a `DatabaseState`.

The function reuses the exhaustive P425–P428 one-shot execution law. Query output is caller-owned; intermediate memory is only operator-inherent state plus result rows.

This proves the query side of lazy historical reads does not require a second query engine.

## Selected next architecture

Do not add per-operation APIs such as `historical_read_field`, `historical_query`, etc. The product/runtime boundary should converge to one read authority concept:

```text
ReadAuthority
    LiveSnapshot(...)
    MaterializedRevision(...)
    RealizationSnapshot {
        revision_id,
        semantic_context,
        shared immutable atom authority,
        realization_root,
    }
```

`ReadContext` remains the single application read surface. Query preparation and authorization continue to use the same semantic context/registry. Execution selects the representation beneath that semantic boundary.

For realization-backed history:

```text
RelExpr
    -> kernel-realization one-shot executor
    -> only referenced physical relations/columns
    -> query result
```

Point field/entity reads should similarly use the existing direct factorized root operations rather than materializing the world.

## Blocking ownership seam

Before a `RealizationSnapshot` can be placed in a long-lived `ReadContext`, the immutable atom graph must be cheap to share. Current `PhysicalAtomStore::clone()` is a deep `BTreeMap` clone.

Acceptable directions:

1. immutable `Arc`-owned physical atom payload/table authority with copy-on-write only when constructing a successor root; or
2. persistent ordered-map ownership from `kernel-persistent` if hostile measurements show path-copy is preferable.

The selected solution must preserve:

```text
one atom payload allocation per live PhysicalAtomId lineage
+ cheap read-handle/root clones
+ exact current + historical reachability GC
```

It must not create one atom store per historical read context.

## P434 hostile proofs

- interrupted streaming publication does not prematurely promote a complete historical root;
- after reopen, legacy generation authority remains pinned until the containing CFPR is successfully published;
- after successful retry/finalize, root-backed authority takes over, the old generation becomes reclaimable, and reopen still reconstructs the historical source;
- 65 roots (current + 64 retained historical) use 67 unique atoms versus 195 naive per-root atom slots in the fixture;
- direct factorized `RelExpr::Difference` execution produces the exact result without a `DatabaseState` materialization.

## Next gate

PASS435 should first make physical atom authority cheaply immutable/shareable and measure clone/root-retention behavior. Only then should `cfmd-runtime::ReadContext` be refactored around a unified read-authority enum and historical query execution be switched from full `Revision` reconstruction to factorized execution.
