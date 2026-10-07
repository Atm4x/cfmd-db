# CFMD R&D — Durable -> Volatile authority theorem (PASS589)

## Status

HOSTILE R&D COMPLETE FOR THE CURRENT AUTHORITY MODEL.

A protection-preserving Durable -> Volatile transition is structurally possible for an unanchored store, but a **total** transition for externally freshness-anchored stores is impossible under the current `SignedFreshnessCut` authority grammar. The blocker is not data serialization and not the runtime persistence enum. It is the absence of a signed non-durable lineage state that can fence the old durable cut while retaining one-shot authority to publish a future durable successor.

The selected next architecture is a single external-freshness lineage state machine with an authenticated `VolatileFence` state. This is not a fallback route and does not require per-commit external freshness I/O while the database is volatile.

## Existing structure already sufficient

The current production architecture is closer to the desired transition than the old ledgers implied:

- `RuntimePersistenceAuthority::{Volatile, Durable}` already owns the same `DurableRevisionStore` semantic/durability-domain state;
- `DurabilityBackend::Volatile` already carries the source `StorageProtectionProfile` as a protection floor;
- `RuntimeRevisionWal::Volatile` uses the same framed prepare/commit/replication protocol as the file WAL;
- `CanonicalPersistenceImage` already carries retry, prepared, replication, history, migration and protection authority;
- therefore no second in-memory database engine, logical export/import, SQL-shaped fallback or semantic copy is required.

The remaining mismatch is external freshness. `DurabilityBackend::Volatile::capabilities()` intentionally reports `external_freshness = false`, and `ExternalFreshnessState` is defined only around a verified concrete durable `FreshnessCut`.

## Formal model

Let a live database authority state be

```text
A = (W, O, P, F, R)
```

where:

- `W` is the current semantic world / Revision;
- `O` is operational authority: retry/idempotency, unresolved prepare, replication, causal/history and migration authority;
- `P` is the persistence protection floor;
- `F` is optional external anti-rollback/freshness authority;
- `R` is the current physical persistence realization.

For a durable externally anchored source:

```text
R = Durable(bytes B)
F = DurableCut(C_B)
```

A correct Durable -> Volatile transition must satisfy:

1. **world preservation**: `W' = W` at the transition linearization point;
2. **operational preservation**: `O' = O`;
3. **protection monotonicity**: `P' = P`, so later persistence cannot weaken source protection;
4. **single authority**: after linearization there is exactly one live persistence realization owner;
5. **rollback exclusion**: once any post-transition volatile commit exists, reopening old durable bytes `B` cannot produce an authoritative live database;
6. **volatile crash semantics**: process loss may destroy post-transition state, but it must not silently resurrect `B` as current authority;
7. **repersistence closure**: while the volatile process survives, it must be possible to publish one future durable successor without inventing another semantic database identity.

## Impossibility lemma for the current freshness grammar

Current external freshness authority can publish only concrete `FreshnessCut` records:

```text
FreshnessCut {
    store_id,
    generation,
    previous_generation,
    generation_digest,
    wal_lsn,
    wal_digest,
    trust_root_epoch,
    deployment_policy_epoch,
}
```

and the provider offers only:

```text
read_signed(store_id)
compare_and_advance_signed(expected_record, next_cut)
compare_and_rebind_signed(source_store_id, expected_source_record, next_cut)
```

Both mutations end in another concrete durable cut.

### Lemma

For an externally anchored source, no total Durable -> Volatile transition can satisfy laws 1–7 using only the current `SignedFreshnessCut` grammar.

### Proof by cases

Assume source durable cut `C0` authenticates bytes `B0`, then transition to volatile state `V` and commit at least one new volatile revision `W1`.

**Case A — leave external freshness at `C0`.**

`B0` remains exactly authenticated by the authority service. After process loss an ordinary freshness-aware open of `B0` succeeds and returns pre-volatile world `W0`. That is a rollback from `W1`, violating law 5/6.

**Case B — advance/rebind freshness to another `FreshnessCut`.**

Every valid next record names a generation digest and WAL prefix digest that recovery expects to correspond to a concrete durable realization. A pure volatile target has no such durable realization. Publishing a fabricated cut violates the existing freshness/recovery invariant; publishing a real cut means a durable successor exists and the operation was not Durable -> Volatile.

These exhaust the current mutation grammar. Therefore the total transition is impossible without extending the external authority state space. QED.

## Important corollary: unanchored demotion is not the hard problem

If `F = None`, the remaining transition is exact with existing primitives:

```text
DurableRevisionStore {
    backend: SingleFile(...),
    wal: File(...),
    authority fields O,
}
    ->
DurableRevisionStore {
    backend: Volatile(P),
    wal: Volatile(next_lsn),
    same authority fields O,
}
```

The protection floor `P` is already representable by `VolatileDurabilityBackend`. The runtime semantic world and operational ledgers need not be rebuilt or routed through a second engine.

This partial operation is mathematically clean, but exposing only the easy unanchored case would leave the public persistence transition law capability-dependent. PASS589 therefore does **not** add a public Durable -> Volatile surface yet.

## Selected universal extension: authenticated `VolatileFence`

The no-fallback solution is to generalize external freshness from “latest durable cut” to “latest persistence-lineage authority state”:

```text
FreshnessAuthorityState =
    DurableCut(FreshnessCut)
    | VolatileFence {
          lineage_id,
          predecessor_record_digest,
          source_generation_digest,
          trust_root_epoch,
          deployment_policy_epoch,
          transition_nonce,
      }
```

The authority provider signs both states under one trust policy and updates them by exact CAS.

### Durable -> Volatile

Under the normal publication/persistence barrier:

1. verify runtime head equals durable store head;
2. capture the source protection floor `P` and exact external source record;
3. CAS `DurableCut(C0) -> VolatileFence(V0)`;
4. only after the signed fence is returned, replace file backend/WAL with volatile backend/WAL while retaining the same in-memory authority fields `O` and runtime identity;
5. drop the durable physical owner.

The fence makes old bytes permanently non-current. A crash after step 3 may lose the database, which is valid volatile semantics, but cannot roll back to the old durable source.

### Volatile commits

No external call is needed per volatile commit. The external service does not certify volatile bytes; it certifies only that **no durable predecessor is currently admissible**.

Thus the hot path remains ordinary in-memory publication.

### Volatile -> Durable

1. stage the new durable target under protection floor `P`;
2. embed the target freshness binding in the first recoverable generation;
3. CAS `VolatileFence(V0) -> DurableCut(C1)` using the exact fence record digest as predecessor authority;
4. only after successful signed publication swap the live owner to the durable store.

Only one target can consume `V0`; concurrent/stale repersistence fails CAS. No old durable source can reappear because its record was replaced by the fence.

## Why this is not fallback/routing

There is one semantic persistence-lineage law:

```text
DurableCut -> VolatileFence -> DurableCut -> ...
```

The two states are authority states, not alternative execution engines. Reads, writes, transactions, Γ semantics, retry, replication and history continue through the same runtime/store machinery. Physical implementations differ only in whether recoverable bytes exist at that instant.

No “if freshness fails, ignore it and use memory” route is permitted. An anchored transition either publishes the signed fence or does not linearize.

## Complexity / performance law

For the selected design:

- anchored Durable -> Volatile: O(1) external CAS + O(1) backend-owner swap, excluding bounded handle teardown;
- unanchored Durable -> Volatile: O(1) backend-owner swap;
- volatile commit hot path: unchanged, no external freshness traffic;
- Volatile -> Durable: ordinary canonical durable staging cost + one O(1) external CAS;
- no O(history), O(rows), archive rewrite or logical re-import is required merely to demote.

Implementation must avoid rebuilding `CanonicalPersistenceImage` solely to switch a live durable owner to volatile. The existing `DurableRevisionStore` should be transformed in place so its in-memory retry/prepared/replication/history authority stays owned rather than cloned.

## Crash linearization

The critical ordering is asymmetric and exact:

```text
Durable -> Volatile:
    external fence first
    local owner swap second

Volatile -> Durable:
    staged durable bytes first
    external durable cut second
    local owner swap third
```

If Durable -> Volatile fails before the fence CAS, source durability remains authoritative. If the process dies after the fence CAS but before/after local swap, the old durable source is fenced and the volatile database may be lost; it must never be resurrected by fallback.

If Volatile -> Durable fails before the durable-cut CAS, the volatile owner remains authoritative and staged bytes are non-authoritative garbage. After the CAS succeeds, the staged durable target is the unique recoverable authority.

## Security consequences

- The protection floor survives in volatile state and constrains every later durable target.
- Durable -> Volatile is a persistence-control operation, not ordinary schema/data permission.
- A volatile fence intentionally trades crash recoverability for rollback safety. Product diagnostics must state that once the fence is published, the old durable file is retired even if the process dies immediately afterward.
- No source encryption key weakening is implied. Memory protection remains governed by the secure-memory threat model; future durable protection remains bounded by `P`.

## Rejected alternatives

- keep the old durable file externally current while writes continue only in memory;
- fabricate a `FreshnessCut` with no corresponding durable generation;
- maintain both a durable owner and volatile owner as simultaneously authoritative;
- external freshness update on every volatile commit;
- logical export/import into a second memory engine;
- silently disable external freshness during demotion;
- copy the full authority/history graph merely to switch the physical owner.

## PASS590 implementation target

Implement the authority-state extension before exposing public Durable -> Volatile DX:

1. replace cut-only external freshness record vocabulary with a signed persistence-lineage authority record that includes `VolatileFence`;
2. extend in-memory/test and TCP authority providers with exact CAS transitions `DurableCut -> VolatileFence` and `VolatileFence -> DurableCut`;
3. add a volatile-fenced state to `ExternalFreshnessState` / persistence owner without adding a second evaluator;
4. add in-place `DurableRevisionStore` physical demotion preserving protection floor and all in-memory operational authority;
5. prove old source reopen fails after fence, post-fence crash never resurrects old bytes, stale/concurrent resume loses CAS, and repersisted target preserves exact semantic/operational authority;
6. only then project the operation into `cfmd-runtime` / `cfmd` under `PersistenceTransition` control.

No FORMAT_VERSION bump is required merely for this R&D conclusion. Any provider/wire grammar change is pre-release authority-protocol work and should be changed cleanly rather than carrying a compatibility branch.


## PASS591 correction — retained-history materialization lower bound

PASS591 hostile execution found one assumption in the original complexity paragraph that was too strong. A directory/single-file durable owner may retain historical epoch authority physically while keeping only anchors in the live store. If that physical owner is retired and the resulting database is required to be genuinely volatile, those retained bytes cannot remain the sole authority on the retired backend. They must first become source-independent volatile authority.

Therefore the exact demotion cost law is:

```text
O(1)                         when retained historical authority is already RAM/self-contained
O(retained historical bytes) when retained authority is disk-only
```

The second term is information-theoretically required by the no-fallback volatile-owner law. It is not O(current rows), not an export/import rebuild, and not O(unretained database history). PASS591 implements this pre-fence materialization and proves directory Durable -> Volatile -> single-file Durable retains the exact historical epoch.
