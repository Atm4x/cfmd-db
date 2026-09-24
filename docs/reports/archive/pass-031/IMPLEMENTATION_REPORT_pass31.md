# IMPLEMENTATION REPORT — Pass31

Status: **VERIFIED** on Rust **1.98.1**.

## Problem

Pass30 had the correct semantic owner (`Revision + PhysicalStore + materialization registry`) and correct logical `prepare -> seal -> publish` ordering, but freshness still retained a whole source bundle and reader publication remained abstract. Cross-crate query preparation and a type named `StorageCertifiedRelationDelta` also left misleading capability/authority surfaces immediately before WAL integration.

## Hypotheses

1. A runtime transition needs nominal source identity, not structural equality against a cloned source bundle.
2. One immutable `Arc<RuntimeRevisionBundle>` is the natural reader-visible unit of publication.
3. The last freshness check and writer exclusivity belong in `seal()` so a durability layer can safely fsync COMMIT while the seal is held.
4. Storage handles passed to maintained Scan are evidence, not semantic authority; naming/API should encode that fact.
5. Query-layer candidate construction may remain accessible as detached computation, but its prepared publication capability must not escape `kernel-query`.

## Implementation

### Runtime root identity

Added process-local lineage/version identity:

```text
RuntimeRootIdentity { root_id, RuntimeRootVersion }
```

The root lineage is unique per `RuntimeRevisionBundle::build`; root version increments for both semantic root publication candidates and reconstructible physical-index root publication.

`PreparedRuntimeRevisionTransition` retains:

```text
RevisionCommitDescriptor
source RuntimeRootIdentity
candidate RuntimeRevisionBundle
output deltas
```

It no longer retains a full source `RuntimeRevisionBundle` clone.

### RuntimeRevisionCell

Added:

```text
RuntimeRevisionCell = RwLock<Arc<RuntimeRevisionBundle>>
RuntimeRevisionSnapshot = Arc<RuntimeRevisionBundle>
```

`RuntimeRevisionCell` is now the authoritative public transaction/publication boundary:

```text
snapshot
prepare_revision
install_i64_index
```

`RuntimeRevisionBundle::prepare_revision` is private.

### Seal and publish

`PreparedRuntimeRevisionTransition::seal` acquires the writer guard and validates exact nominal source root identity plus source revision.

`SealedRuntimeRevisionTransition` holds the writer guard. No other cell publication can pass until it publishes or is dropped.

`publish` is infallible and swaps one `Arc` root.

### Query capability cleanup

`PreparedMaterializedRelPlanTransition` became private.

The runtime uses:

```text
candidate_from_storage_resolved_deltas_for_revision
```

which returns a detached maintained candidate and exact output delta, not a live-publication capability.

### Resolved storage evidence

Renamed:

```text
StorageCertifiedRelationDelta
-> StorageResolvedRelationDelta
```

and all relevant `certified` API terminology to `resolved`.

`StorageResolvedRelationDelta::from_parts` remains public intentionally. It is not trusted authority: maintained leaves validate its handles, direct revision-bound legacy mutation is rejected, and the object cannot publish into `RuntimeRevisionCell`.

### Physical derived-state publication

`RuntimeRevisionCell::install_i64_index` clones and updates a candidate physical substate, increments root version under the same semantic revision, and swaps the complete root. Existing reader snapshots stay coherent and prepared semantic transitions become stale.

## Hostile falsification

Added:

1. old reader retains old physical and maintained state after semantic root publication;
2. a transition prepared on one root lineage cannot seal against an independently built identical root;
3. reconstructible index publication advances root version while preserving semantic revision and invalidates old prepare;
4. all prior transaction hostile cases remain green.

Static audits confirm:

- no `StorageCertifiedRelationDelta` remains under production `crates/`;
- no public `PreparedMaterializedRelPlanTransition` remains;
- runtime prepared transition no longer stores a full source bundle;
- public resolved evidence is documented as non-authoritative.

## Verification

Final successful commands:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets --release
cargo build --workspace --release
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps
RUSTFLAGS='-C overflow-checks=yes' cargo test --workspace --all-targets --release
```

All **PASS**.

Workspace metrics:

```text
243 declared tests
61 kernel-plan tests
19 workspace crates
30,053 Rust LOC
0 external Cargo sources
0 unsafe occurrences under crates/
```

Evidence: `evidence/pass31/`.

## Rejected routes

1. **Keep full source bundle and compare structural equality at seal.** Correct but expensive and does not provide nominal lineage protection.
2. **Expose separate mutable fields and rely on caller locking.** Would reintroduce a mixed-revision reader-visible state possibility.
3. **Hide `StorageCertifiedRelationDelta::new` and keep certificate semantics.** Cosmetic sealing; the real contract is evidence validation, not token possession.
4. **Give query crate a public prepared publication capability.** Creates a second authority boundary and complicates future WAL ordering.
5. **Treat process-local root id/version as durable revision identity.** Rejected explicitly; durable authority remains logical revision/WAL data.

## Remaining risks / OPEN

1. Candidate construction still deep-clones substantial runtime state; migrate to persistent/COW subroots after durability semantics are fixed.
2. Schema/Γ-changing revisions still require a typed rebuild/migration transaction.
3. WAL/recovery/checkpoint/segment integration remains OPEN despite Agent-1 verified R&D.
4. `RevisionCommitDescriptor` still needs stable binary encoding/versioning/checksum.
5. RwLock poisoning/recovery policy must be defined alongside durability.
6. Alternate physical layouts/replicas need a reconstructible derived registry.
7. Bootstrap exact coherence needs a cheaper certified root/digest fast path with exact fallback.
8. Historical Pass26 performance/features OPEN items remain active; Agent-2 only R&D-closes canonical-key feasibility, not production index integration.

## Recommended integration

Pass32 should integrate Agent-1 logical WAL directly at the now-concrete boundary:

```text
cell.prepare_revision(...)
    -> prepared.seal(&cell)
    -> durable PREPARE/COMMIT/fsync using sealed.descriptor()
    -> sealed.publish()
```

Recovery must reconstruct a validated `Revision`, physical store, materialization registry and then create a fresh process-local runtime root lineage. Runtime root identity itself must never enter the durable log.
