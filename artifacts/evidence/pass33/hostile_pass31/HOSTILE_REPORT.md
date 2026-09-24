# CFMD Pass31 Hostile Review

Status: **FAIL / BLOCK WAL INTEGRATION UNTIL LEAF CONTRACT IS REPAIRED**.

Reviewed input: `cfmd_workspace_pass31_verified(1).zip`, treating Pass31 as the superset of the originally requested Pass30 hostile target. `AGENT1_WAL_RESULT.zip` was used only as a durability-boundary reference. Production source was not modified.

Toolchain used for independent reruns: Rust 1.98.1 (`rustc 1.98.1 (48a229cea 2026-09-01)`). The pristine Pass31 workspace still passes its advertised debug workspace tests and `clippy -D warnings`; the failures below are therefore missing hostile coverage, not baseline build failures.

## Executive result

The new Pass31 root publication mechanism itself is structurally sound under the reviewed safe-Rust API: one `RwLock<Arc<RuntimeRevisionBundle>>` root, nominal lineage/version freshness, and the writer guard held across `seal -> publish` prevent mixed old/new field publication and stale competing prepares.

However, the root can contain an already-invalid maintained-plan candidate. Three falsifiers reproduce this:

1. **H31-01 / BLOCKER — bootstrap accepts semantically equal Bag rows in a different physical order, then binds storage handles positionally to the logical maintained Scan.** A later valid runtime revision removes the wrong row from maintained state. One atomic Pass31 root then contains logical/physical state for one row and maintained state for another row.
2. **H31-02 / HIGH — resolved Scan removal uses `swap_remove`, violating the project-wide logical insertion/output-order contract.** Even with perfectly aligned bootstrap, removing the first row from `[1,2,3]` produces maintained Scan `[3,2]` while authoritative revision order is `[2,3]`.
3. **H31-03 / HIGH boundary defect — `StorageResolvedRelationDelta` validation checks handle existence/uniqueness but not handle↔row-payload association.** A constructible forged receipt saying “remove Beta” while pointing at Alpha's handle is accepted. This can construct internally inconsistent maintained state. H31-01 turns this nominally detached-validation weakness into a real mainline runtime correctness failure without any hostile caller.

Because Agent-1 WAL would make committed revisions recoverable/durable, integrating it before H31-01/H31-02 are fixed risks making a currently in-memory maintained-state divergence part of the committed runtime path. WAL mechanics are not the correct next production step yet.

## H31-01 — bootstrap row/handle association is not proved

### Problem

`RuntimeRevisionBundle::build` first validates logical and physical relations by semantic `RelationDelta::between_values`, which is order-insensitive for Bag equality, then independently builds maintained state from `revision.state().model`, gets physical handles in physical logical scan order, and calls `attach_storage_handles`.

Relevant pristine Pass31 source:

- `crates/kernel-plan/src/lib.rs:606` — semantic physical snapshot validation.
- `crates/kernel-plan/src/lib.rs:614-625` — maintained state is built from logical model rows, then physical handles are attached.
- `crates/kernel-plan/src/lib.rs:675-700` — physical/logical snapshot comparison uses semantic relation difference rather than proving positional row identity/order.
- `crates/kernel-query/src/lib.rs:1922-1925` explicitly documents the precondition: handle order **must match** the Scan snapshot's current row order.
- `crates/kernel-query/src/lib.rs:1956-1959` checks only length and installs the handle vector; it does not verify row association.

So `build` consumes a precondition it never establishes.

### Falsifier

Logical revision Bag: `[1,2]`.

Physical selected layout: `[2,1]`.

The two bags are semantically equal, so bootstrap succeeds. Maintained Scan is built as `[1,2]`, but receives physical handles `[handle(row=2), handle(row=1)]` positionally.

A valid revision delta removes row `2`. Physical storage correctly returns the handle of physical row `2`; maintained Scan interprets that same handle as its logical row `1` and removes `1`.

Observed after Pass31 `seal().publish()`:

```text
authoritative revision: [1]
maintained Scan:        [2]
```

The failing hostile test is `hostile_bootstrap_semantic_bag_reordering_breaks_storage_handle_alignment`; raw evidence is in `evidence/hostile_bootstrap_reorder_evidence.log` and `evidence/recheck_bootstrap.log`.

### Impact

This invalidates the strongest Pass30/31 claim that `RuntimeRevisionBundle` is a coherent authoritative `Revision + PhysicalStore + materialization registry` root. Publication is atomic, but the candidate may already violate cross-layer coherence.

It also reopens part of the Pass26/27 `storage -> maintained-plan leaf contract`. Pass27 successfully removed the O(n) duplicate semantic lookup, but the stable-handle attachment correctness contract was not fully closed.

## H31-02 — maintained Scan leaks `swap_remove` order

### Problem

`MaintainedLeafHandles::remove` updates both row payload and handle vector with `swap_remove`:

- `crates/kernel-query/src/lib.rs:1542` — `rows.swap_remove(position)`.
- `crates/kernel-query/src/lib.rs:1543` — `self.ids.swap_remove(position)`.

That is O(1), but it is incompatible with the already-established logical insertion/output-order contract from Pass18/19. Authoritative logical `RelationDelta::apply_to_value` removes while preserving sequence order, and physical storage separately preserves logical scan order despite dense compaction.

### Falsifier

Bootstrap is perfectly aligned:

```text
revision: [1,2,3]
physical logical scan: [1,2,3]
maintained Scan: [1,2,3]
```

Remove row `1` through the normal runtime revision transaction.

Observed after publication:

```text
authoritative revision: [2,3]
maintained Scan:        [3,2]
```

The failing hostile test is `hostile_resolved_leaf_delete_preserves_authoritative_scan_order`; raw evidence is in `evidence/hostile_scan_order_evidence.log`.

### Why existing tests missed it

Pass31 helper `runtime_maintained_i64_values` sorts the materialized values before comparing them. This intentionally/accidentally erases the exact logical-order property and lets `[3,2]` compare equal to `[2,3]`.

This is inconsistent with the normative project text that physical compaction must not leak into Bag/Set query output order and with the Pass18/19 order-preservation work.

## H31-03 — resolved evidence validates handle membership, not row binding

### Problem

`validate_resolved_leaf_deltas` checks:

- delta result type;
- number of handles vs rows;
- row type/semantic validity;
- removed handles currently exist;
- inserted handles currently do not exist;
- no duplicate handle in the receipt.

It does **not** verify that each removed handle identifies the row payload paired with it.

Pristine source: `crates/kernel-query/src/lib.rs:2120-2151`.

Then `apply_resolved_leaf_deltas` removes rows only by handle (`2186-2187`) while downstream operator propagation uses the caller-supplied `RelationDelta` row payload. A mismatched receipt can therefore make the Scan child and its parent operator states describe different base changes.

### Falsifier

State contains `Alpha` at handle 0 and `Beta` at handle 1. Construct:

```text
payload: remove Beta
handle:  handle 0 (Alpha)
```

The current API accepts it instead of returning `InconsistentIncrementalDelta`.

Failing test: `hostile_forged_resolved_handle_payload_mismatch_must_be_rejected`; evidence in `evidence/hostile_forged_evidence.log` and `evidence/recheck_forged.log`.

### Authority assessment

Pass31 is correct that possession of a `StorageResolvedRelationDelta` alone cannot publish a `RuntimeRevisionCell`: the runtime root fields and publication capability remain sealed in `kernel-plan`. Therefore this is not a direct arbitrary-runtime-write capability escape.

But the public evidence-validation contract is still incomplete, and H31-01 demonstrates that the same missing binding check can arise through trusted mainline construction rather than malicious receipt minting.

## Atomic root / freshness review

No counterexample was found against these Pass31 mechanisms themselves:

- process-local root lineage uses an atomic unique `root_id`;
- `version` advances on both semantic publication and reconstructible index publication;
- `prepare_revision` operates on an immutable `Arc` snapshot;
- `seal` acquires the sole write guard and compares `(root_id,version)` plus source revision;
- competing prepares from one source cannot both publish;
- readers holding an old `Arc` retain a complete old bundle;
- new readers after publication receive the complete new bundle;
- runtime-bound source objects expose no public mutable field path that bypasses `RuntimeRevisionCell` under ordinary safe Rust.

So the hostile result is not “RwLock publication is broken”; it is “the published candidate invariant is weaker than the report claims”.

## Required architectural correction before WAL

Do **not** fix H31-02 with `Vec::remove` plus O(n) position repair; that would reintroduce the Pass26 leaf update tax the stable-handle path was created to eliminate.

Recommended clean direction:

1. Make the bootstrap contract explicit and constructive. The selected physical relation must expose ordered `(StableRowHandle, row)` pairs (or an equivalent checked binding), not a naked handle vector.
2. Either require exact logical-row/order equality at runtime bootstrap and reject/rebuild a semantically equal-but-reordered physical layout, or perform an explicit semantic bijection from logical rows to physical `(row,handle)` pairs. For CFMD's deterministic output contract, exact selected-layout logical order is the simpler and stronger invariant.
3. Replace `MaintainedLeafHandles { ids: Vec, positions } + RelationValue Vec` as the order-maintenance mechanism. Use an order-preserving stable-node representation: handle->row/slot plus explicit prev/next (or another persistent ordered sequence). Remove remains O(1)/O(log n); output materialization traverses logical order. This mirrors the already-proven Pass19 physical design instead of using `swap_remove` at the derived leaf.
4. Validate each resolved removal as `(handle,row)` against the maintained leaf binding before any candidate propagation. An inserted handle must be fresh and paired with exactly the inserted row. Do this once before applying any child/parent mutation.
5. Add unsorted exact-order tests. Do not sort Scan/materialization outputs in the runtime transaction hostile tests.

A minimal local patch could reject bootstrap reorder and use order-preserving `Vec::remove`, but that is not recommended as the final architecture because it trades correctness for a known O(n) regression.

## WAL / Agent-1 boundary review

Pass31 provides the right **pre-durable freshness shape**, but two integration conditions remain:

1. Agent-1's durable descriptor requirements are still explicitly OPEN: stable encoding/version/checksum, full dependency/DAG/schema/Γ representation, checkpoint/segment recovery. Current `RevisionCommitDescriptor` is sufficient only for the current same-Γ relation-delta class.
2. The existing `SealedRuntimeRevisionTransition` intentionally permits `Drop` as abort-before-publication (`sealed_transition_drop_aborts_without_publication`). That is safe before durable COMMIT. Pass32 must not expose the same abort semantics **after** COMMIT fsync. Once durable COMMIT succeeds, code must immediately perform the infallible root swap or fail-stop into recovery; a normal post-COMMIT return/drop path that leaves the old live root would violate the WAL linearization point.

A clean Pass32 API should make the WAL COMMIT operation and root publication one orchestrated method under the sealed guard, with no user callback/fallible work between successful COMMIT fsync and root replacement.

Availability note: keeping `std::sync::RwLock` write-held across COMMIT fsync blocks new readers for the durability latency. This is correct but may be a production latency issue; treat it as a measured optimization problem after correctness.

## Final verdict

- Pass31 root swap / freshness mechanism: **hostile review passed so far**.
- Pass31 claim of a coherent authoritative runtime root: **falsified** by H31-01 and exact-order H31-02.
- Pass27 leaf contract status: **reopen as correctness-partial**; the performance lookup defect is closed, but row↔handle/order invariants are not.
- `StorageResolvedRelationDelta` public evidence contract: **incomplete validation**.
- Agent-1 WAL integration: **blocked until H31-01/H31-02 are corrected and regression-tested**.
