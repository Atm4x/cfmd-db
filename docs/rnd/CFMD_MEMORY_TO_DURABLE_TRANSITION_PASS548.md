# CFMD R&D — Memory -> Durable transition law (PASS548)

## Status
Selected pre-format architecture. This note defines the transition law; it does not yet add the public `Database::memory()` / `persist()` frontend.

## Hostile finding
The current runtime couples publication and persistence inside `DurableRuntime { cell, durability: Mutex<DurableRevisionStore>, ... }`. Merely adding `Storage::Memory` to the product enum would therefore create a fake backend unless the persistence authority is separated from semantic publication authority.

Memory -> durable is **not** schema migration, import/export, or a new logical revision. It is a same-world durability realization transition:

```text
same Database identity
same current Revision R
same SemanticContext / Schema.Access
same causal/history/retry authority
volatile persistence authority
        ->
released durable persistence authority
```

No Watch event or semantic history event is generated merely because bytes became durable.

## Selected internal shape
Refactor the runtime persistence owner to one authority boundary, conceptually:

```text
RuntimePublicationAuthority
    RuntimeRevisionCell
    SemanticRegistry
    notifier/watch authority

RuntimePersistenceAuthority
    Volatile(VolatilePersistenceState)
    Durable(DurableRevisionStore)
```

Both variants must consume/produce the same canonical durable-domain records. `Volatile` is not a simplified database engine: it retains exact causal records, retry/idempotency state, migration programs, retained historical authority and physical/root metadata in memory.

## `persist(path)` linearization law
Promotion occurs under the existing publication barrier.

1. Freeze only publication ordering, not the semantic world.
2. Capture a canonical `PersistenceImage` from volatile authority:
   - current revision/root;
   - complete retained causal transition closure;
   - retry/idempotency horizon and outcomes;
   - retained historical roots/closures and pins;
   - schema-migration programs / Access approvals;
   - realization/materialization/physical-artifact authority required for exact reopen.
3. Build the target `.cfmd` in staging using the **same released FORMAT_VERSION** as an ordinary durable DB.
4. Verify staged reopen/certificate against the captured authority (head, schema/env identity, causal/retry roots, retained revision set).
5. Publish the final durable root/file atomically. This durable publication is the transition linearization point.
6. After that point, swapping the live runtime persistence owner from `Volatile` to the already-open `DurableRevisionStore` must be infallible/no-I/O.

Failure before step 5 leaves the live memory DB authoritative and unchanged. A crash after step 5 must reopen the durable DB exactly; no second semantic commit is required.

Transactions formed before the barrier but not yet committed may publish after the swap through the same semantic commit path. Existing Context/read/watch objects keep the same runtime/database identity.

## `save_as` is a different operation
`persist(path)` changes the persistence authority of the live DB while preserving its runtime identity.

`save_as(path)` should instead create an independently reopenable durable copy/snapshot and leave the source memory DB volatile. Copy identity/fork semantics should be specified separately; do not implement `persist` by calling logical export/import.

## Encryption
Encrypted promotion must create encrypted durable bytes from the first staged durable write. No plaintext `.cfmd` or temporary plaintext checkpoint is permitted. The volatile source remains governed by the separate memory-protection threat model.

## FORMAT_VERSION consequence
Promotion does **not** need a memory-specific on-disk format. The released format must be capable of bootstrapping the complete authoritative persistence image, including causal/retry/history roots—not only the current logical rows.

Therefore the runtime persistence seam and canonical `PersistenceImage` contract should be closed before the first released FORMAT_VERSION, while public `Database::memory()/persist()` syntax may follow once that kernel/runtime theorem is proven.

## Recommended implementation order
1. PASS549: canonical enum/sum physical representation.
2. PASS550: separate persistence authority + volatile canonical state + atomic Memory -> Durable transition regression (internal/runtime surface first).
3. Then freeze first released FORMAT_VERSION against the now-complete bootstrap image law.
4. Public embedded frontend (`Database::memory`, `persist`, `save_as`) may be added immediately with PASS550 or deferred to product-surface work if it adds no new semantics.

## Rejected designs
- serialize current rows and import them into a new DB;
- create a new semantic revision on save;
- omit history/retry authority during promotion;
- memory-only alternative query/change semantics;
- copy to a temporary plaintext file before encryption;
- dual live volatile+durable authorities after successful linearization.

## PASS550 implementation update
PASS550 introduced the first source-independent durability-domain bootstrap object, `CanonicalPersistenceImage`.

The image is deliberately not a logical export. It binds an exact current `Revision` to the durability-owned authority needed to reopen that same world: semantic registry, materialization/physical-artifact descriptors, exact current factorized realization when it is already bound to the current cut, migration complements, retry/idempotency epochs and committed outcomes, and the causal revision-effect/frontier ledger.

`DurableRevisionStore::stage_single_file_from_persistence_image(...)` writes an ordinary encrypted-or-plain single-file store from that image, rotates it to a self-contained checkpoint, reopens it, and compares the authoritative cut before returning. The source image is immutable, so a staging failure cannot mutate source authority. No new semantic revision or history event is created.

Hostile review also identified authority classes that are not yet honest to flatten into this image:

- retained historical epoch archives/closures;
- unresolved prepared transactions;
- replication authority;
- external freshness ownership;
- an active streaming checkpoint publication.

PASS550 therefore fails closed when any of these are present. It is forbidden to silently drop them merely to make `persist()` succeed. PASS551 must define their portable persistence-image representation (or exact quiescence/transfer law) before the first released `FORMAT_VERSION` is frozen.

This changes the pre-format conclusion slightly: the bootstrap-image law is now concrete, but released format freeze remains blocked until every durability-owned authority that can exist in a live volatile runtime has an exact image representation.

## PASS553 implementation update — provider-independent promotion prerequisite
PASS553 closes the production TCP external-freshness rebind prerequisite discovered by PASS552. `TcpExternalFreshnessAuthority` now has one server-side atomic `CompareAndRebind` wire operation. The authority service performs source-record CAS, target trust-chain validation, target signing, crash-recoverable target publication and source invalidation as one serialized rebind transaction. A fsynced rebind journal is completed on authority-server startup before requests are accepted.

This ordering matters for the runtime owner theorem: introducing `RuntimePersistenceAuthority { Volatile, Durable }` while only the test/in-memory freshness authority could rebind would create provider-dependent promotion routing. PASS554 may now treat external freshness as one capability law across the built-in production provider as well. The released FORMAT_VERSION remains deliberately unfrozen until that owner seam is complete.

## PASS554 implementation update — persistence protection is authority, not backend configuration
Hostile review of the final runtime-owner transition found a security hole in the PASS550–PASS553 bootstrap theorem: `CanonicalPersistenceImage` preserved semantic/causal/history/retry/replication/freshness authority but did not preserve the source store's at-rest protection floor. A caller holding the kernel staging capability could therefore capture an encrypted source and ask the staging primitive to create `StorageEncryption::None`.

That is forbidden before any `Durable -> Volatile -> Durable` or public `persist/save_as` surface exists. PASS554 makes the protection floor part of the canonical persistence authority:

- the floor is derived from the *actually opened source backend*, never caller-declared metadata;
- unencrypted source may promote to any target protection;
- direct AEAD source may promote only to the same AEAD class or an externally wrapped target using that AEAD;
- externally wrapped source may promote only to the same external provider identity with non-regressing provider epoch and the same AEAD class;
- wrapped -> direct, encrypted -> plaintext, provider substitution and provider-epoch rollback fail before the target file is created.

The future `VolatilePersistenceState` must carry this floor unchanged. Memory residency itself is governed by the separate volatile-memory threat model, but entering memory is not authority to later persist a weaker durable representation.

Schema/Access authority remains orthogonal and is already carried by the exact current `Revision`. A future public `persist`, `save_as`, backend-switch or encryption-reconfiguration surface must additionally require an explicit DB-owned persistence/export capability; ordinary read/write permission must never imply persistence reconfiguration or whole-database export.

PASS554 intentionally does not fake the final `RuntimePersistenceAuthority { Volatile, Durable }` owner after discovering this missing authority dimension. The owner moves to PASS555 and must consume the now security-bound canonical image.

## PASS578 productization update
The selected theorem is now exposed at the Rust product boundary. `Database::memory::<S>()` / `memory_from_schema(...)` instantiate the existing volatile persistence authority; `Database::persist(...)` and `persist_with_encryption(...)` invoke the canonical same-revision stage/reopen/verify/owner-swap path.

Hostile product review explicitly rejected an intermediate proposal to require exclusive `Database` ownership. That rule is correct for external-freshness authority transfer, where a source trust cut must be retired, but it contradicts this document's persistence-transition law: Context/read/watch and other runtime handles must survive a representation-only promotion. Product persistence location is therefore derived from the runtime persistence owner rather than cached per facade clone.

Restricted persistence transition is guarded by Schema-owned `DatabaseAdministration::PersistenceTransition` before provider resolution or target I/O. Durable -> Volatile remains absent, and `save_as` remains a separate copy/fork problem rather than an alias for `persist`.
