# IMPLEMENTATION REPORT — Pass40

## problem

Pass39 integrated exact canonical semantic indexes into maintained Join/Group/TopK, but the physical `PhysicalStore` layer still had only a specialized persisted I64 equality index. Text/F64/Bool/entity primitive equality could not be represented as the same long-lived stable-handle physical index, and direct physical Filter/Join execution therefore could not reuse one.

Separately, the maintained I64 Group hot path still paid a large constant-factor tax for repeated semantic admission and generic change planning.

The user also proposed a first-class `Chain<T>` / `Lineage<T>` primitive. That proposal was treated as a hostile architecture question, not an implementation requirement.

## hypotheses

1. Pass39 canonical equality keys are sufficient to build an exact primitive physical index without changing semantic authority.
2. Physical index payload must remain stable physical row identity, never copied logical rows or durable semantic authority.
3. Γ compatibility must be checked from the pinned semantic contract before maintenance or fast-path use; same SemanticId is not sufficient.
4. The specialized I64 physical path should coexist with, not be replaced by, the generic semantic index.
5. I64 Group can remove repeated administrative work from the common tiny-delta path without weakening validation.
6. `Lineage` adds no current logical expressive power beyond `PartialMap + constraints + Seq-valued derived queries`; any useful performance behavior belongs in physical lowering.

## implementation

### `kernel-plan`: persisted semantic physical indexes

Added `SemanticIndexBinding` and `MaterializedSemanticIndexState` inside `PhysicalStore`.

State contains the exact resolved primitive equivalence and reuses the shared generic index engine:

```text
SemanticBucketIndex<CanonicalEqKey, PhysicalRowId>
```

The final architecture cleanup changed the shared bucket representation from `BTreeSet<I>` to insertion-ordered `Vec<I>` plus the existing exact reverse map. This preserves deterministic logical/tie order for reused physical handles while retaining one live key per identity. The same engine is now used by maintained Join/TopK and persisted physical semantic indexes.

The current builtin primitive equality families are supported through the already-verified Pass39 canonical encoder. Relation reinstall invalidates bound semantic indexes. Relation delta planning validates and applies all bound semantic indexes atomically with the physical relation and existing I64 indexes.

A bound index whose resolved Γ contract no longer matches the supplied context requires rebuild rather than being silently maintained under a new law.

### `kernel-plan`: physical planner/executor consumption

The equality join algorithm label is generalized from `IndexedI64IfAvailable` to `IndexedPrimitiveIfAvailable`.

Direct Scan Filter and equality Join execution first look for an installed compatible primitive semantic index. The candidate bucket is still verified through the resolved semantic equivalence before a row reaches output. If the generic semantic index does not apply, the existing I64 specialization / exact fallback remains.

`RuntimeRevisionCell` and `DurableRuntime` expose whole-root reconstructible semantic-index installation so adding physical derivative state still invalidates stale prepared root versions correctly.

### `kernel-query`: I64 Group hot path

The exact I64 count state no longer re-runs full semantic-module admission for every tiny delta after the state has already pinned the exact context. It validates row shape and uses the admitted I64 path.

The common one-remove/one-insert replacement bypasses generic changes-plan allocation. Singleton-group replacement may reuse the existing group slot rather than tearing down and allocating a new one.

This is a performance optimization only; exact output deltas and atomic error behavior remain unchanged.

## hostile falsification

Verified:

- semantic physical index construction over mixed primitive carriers;
- Text ASCII-CI persisted Filter/Join consumption;
- duplicate semantic representatives and Bag multiplicity;
- atomic semantic-index maintenance with relation delta;
- Γ transition requires index rebuild for maintenance;
- stale Γ index is skipped by query execution rather than used as an oracle;
- no new mutating side effects inside `debug_assert!`;
- debug/release/overflow tests all agree.

The proposed `Chain/Lineage` primitive was separately attacked. No operation in the proposal requires a new fundamental logical type: parenthood is a `PartialMap`, rooted/acyclic/reachability are constraints, path is `Seq`, and ancestor/LCA are recursive derived queries. The design is therefore rejected at the kernel level and retained only as a possible surface refinement + physical specialization.

## benchmark/result

Maintained I64 Group, 50k groups, seven post-unification release process runs:

```text
365/155 = 2.354x
365/152 = 2.401x
379/162 = 2.339x
381/161 = 2.366x
361/150 = 2.406x
370/156 = 2.371x
365/159 = 2.295x
```

Historical benchmark state was around 4.8x. The current gap is a stable roughly 2.3–2.4x and remains OPEN.

Existing persisted-I64 reference path remains healthy:

```text
20k rows
persisted ~3.10 ms
prebuilt hand baseline ~2.69 ms
persisted / hand ~1.154x
ephemeral rebuild ~5.81 ms
```

## rejected routes

1. Replacing specialized I64 physical indexes with the generic semantic index: rejected; specialization still has value.
2. Using host Rust equality/hash/order directly: rejected; Γ remains authoritative.
3. Keeping stale physical semantic indexes usable across Γ changes by SemanticId alone: rejected as unsound.
4. Marking the I64 Group problem closed after a ~2.5x result: rejected; a real constant-factor gap remains.
5. Adding first-class `Chain<T>` / `Lineage<T>` to the universal type/kernel layer: rejected as abstraction tax without new expressive power.
6. Authoritative path compression for lineage parent edges: rejected; shortcuts may only be reconstructible physical state.

## verification

Rust 1.98.1 full gate:

```text
fmt                         PASS
check --workspace           PASS
debug all-target tests      PASS
strict Clippy               PASS
release all-target tests    PASS
release build               PASS
strict rustdoc              PASS
overflow-checked release    PASS
```

311 declared tests; 83 kernel-plan; 69 kernel-query; 30 kernel-semantics; 4 kernel-semantic-index; 21 crates; 40,331 Rust LOC; zero external Cargo sources; zero `unsafe`.

## result

Two architecture problems are closed in Pass40: persisted primitive semantic equality indexes beyond I64 now exist in `PhysicalStore` and direct physical Filter/Join execution consumes them exactly under pinned Γ; and the physical/maintained semantic bucket implementations are unified behind the identity-generic `kernel-semantic-index` engine instead of drifting separately.

I64 Group performance is substantially improved but remains OPEN. Automatic/cost-aware physical index selection, structural/custom canonicalization, multi-column/mixed-key indexes, typed-batch Group/TopK and broader join planning remain future work.
