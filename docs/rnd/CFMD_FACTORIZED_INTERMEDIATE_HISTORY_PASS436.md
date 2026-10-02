# CFMD R&D — FACTORIZED INTERMEDIATE HISTORY (PASS436)

## Result
A retained complete historical realization root is no longer useful only at its exact migration-source revision. Exact reversible relation-only history can be replayed forward in representation space.

```text
R0 retained root
  + exact relation delta d01
      -> R1 factorized snapshot
  + exact relation delta d12
      -> R2 factorized snapshot
```

Each step materializes only the relation named by the durable effect, delegates row semantics and Γ equality to `kernel-query::RelationDelta`, writes a fresh direct columnar endpoint, and preserves persistent sharing for every untouched PhysicalAtom.

## Why this is not a fallback
The semantic law remains the durable exact effect. The new code only changes its representation-space realization. Effects containing model/schema semantics for which no factorized lowering is yet proved are not interpreted approximately; the existing exact logical history path remains authoritative.

## Next theorem/implementation target
Define a representation-space action of `DurableModelDelta` on a factorized root covering carrier presence/membership, field values, lifecycle entities/roots and keeps-alive edges. The goal is one compositional model-delta law, not per-feature routing.
