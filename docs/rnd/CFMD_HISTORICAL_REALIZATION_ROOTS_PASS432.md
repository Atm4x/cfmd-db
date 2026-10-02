# PASS432 R&D — Historical Realization Roots over Shared PhysicalAtoms

## Selected law

A retained historical world is not another database and not another atom store. It is a semantic revision/context plus a realization root over the same immutable physical atom identity space used by the current world.

```text
P = immutable PhysicalAtomStore
rho_current : P -> B
rho_history_i : P -> A_i

live_atoms = deps(rho_current) U union_i deps(rho_history_i)
```

Physical compaction may delete exactly `P - live_atoms`. Releasing one historical authority removes only its root; atoms shared by another current/historical root remain live automatically.

## Durable representation

CFPR v2 carries one atom table and then root topologies. Historical root entries are keyed by causal `RevisionEffectId` and bind their historical `RevisionId`. No atom bytes are repeated per root.

The merger law is intentionally strict: `PhysicalAtomId` is immutable physical lineage identity. Two authorities that assign different payloads to the same ID cannot be merged by renumbering; they are different atom lineages and the operation fails closed.

## Current conservative boundary

P432 does not yet delete the older generation archive for a migration anchor. The existing `HistoricalEpochMaterial` API also needs the old semantic registry/checkpoint/WAL boundary, while the retained realization root currently supplies physical materialization authority only. Until PASS433 binds those semantic/causal pieces to the root, generation pinning remains conservative.

This is not a second target architecture. It is a temporary compatibility payer while physical retention has already moved to the selected shared-root law.
