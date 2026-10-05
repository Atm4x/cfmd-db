# CFMD PASS492 — Bottom-Up Canonical Relational Observation Forest Construction

## Problem

PASS490/PASS491 proved exact canonical state-cell sharing during maintained transitions and reopen, but initial forest construction still called the full single-root `MaterializedRelPlanState::build` for every newly discovered parent subtree and retained only that subtree root state. Shared descendants were therefore canonical in the final forest while their source/data initialization could be repeated transiently during construction.

## Selected law

A forest is constructed in post-order over canonical `RelExpr` identity. Each child semantic cell is interned and initialized first. The parent local maintained cell is then initialized directly from the already-built child output/type using the same Γ-DTC node-local primitives used by the single-root implementation.

The construction authority is therefore:

```text
canonical RelExpr DAG
    -> one source initialization per unique Scan cell
    -> one local initialization per unique semantic cell
    -> separate occurrence/fanout topology
```

There is no temporary independent subtree plan, recovery cache, serialized maintained state or durable node identity.

## Executable hostile scaling

For 64 distinct `FilterEqConst(Scan(R))` roots:

```text
root occurrences          = 64
unique maintained cells   = 65
local cell initializations= 65
source materializations   = 1
source rows materialized  = |R| = 3 in regression
reused subtree references = 63
```

A self-join over `Scan(R)` produces two canonical cells (`Scan`, `Join`) and two input occurrences, while `Scan(R)` is source-materialized once.

All root outputs remain semantically equivalent to independent `MaterializedRelPlanState` construction, and PASS490 transition equivalence / PASS491 shared-root braid tests remain green.

## Complexity boundary

PASS492 eliminates root-expanded descendant/source replay at forest boundary construction. Remaining work is local semantic initialization per unique cell over its exact child outputs. Distinct parent semantics can still each inspect/process the same child output; that is real per-cell semantics, not duplicated descendant construction.

A reduction below this remaining work requires a stronger exact family theorem, not a generic cache. The next R&D target is parameter-family fusion: equality-value dispatch and ordered-cut families that share one maintained child and admit exact batched parent maintenance.
