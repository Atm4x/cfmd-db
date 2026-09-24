# PASS33 REPORT — hostile leaf-contract repair + post-COMMIT fail-stop

Status: **VERIFIED** on Rust **1.98.1**.

Pass33 was originally expected to continue directly into durable checkpoint/segment lifecycle after Pass32. That priority changed when the delayed independent hostile review of Pass31 arrived. The review found three correctness defects in the storage→maintained Scan binding that were still present under Pass32 durability, plus one post-COMMIT integration requirement that Pass32 had not fully enforced. Pass33 therefore repairs those blockers before adding any new checkpoint authority.

## 1. Problem

The hostile review falsified the claim that every `RuntimeRevisionBundle` candidate was cross-layer coherent even though the Pass31/32 whole-root publication mechanism itself remained sound.

The four actionable issues were:

1. **H31-01 — bootstrap row↔handle binding.** `RuntimeRevisionBundle::build` accepted semantically equal Bag contents in a different physical order, then attached only a positional handle vector to a maintained Scan built from logical row order.
2. **H31-02 — maintained Scan exact order.** `MaintainedLeafHandles::remove` used `swap_remove` on both payload rows and handles, leaking dense-compaction order into logical Scan output.
3. **H31-03 — resolved evidence row↔handle binding.** removal validation proved only that a handle existed, not that the handle denoted the row payload carried in the delta.
4. **Post-COMMIT uncertainty.** Pass32 correctly surfaced `CommitDurabilityUncertain`, but it still allowed subsequent readers to observe the old in-memory root. If the COMMIT had in fact reached stable storage before the I/O error, serving the old root would violate the durable linearization point.

The delayed review artifact is preserved under `evidence/pass33/hostile_pass31/`.

## 2. Hypothesis

Repair the leaf contract without reintroducing the Pass26 O(n) semantic relookup tax:

```text
PhysicalStore
  -> exact logical-order [(StableRowHandle,row)] bindings
  -> maintained Scan bootstrap verifies exact payload/order
  -> leaf keeps dense payload storage for O(1)-style compaction
  -> separate stable-handle linked logical order
  -> resolved removal validates handle -> exact current row
  -> output traverses logical order, not dense physical order
```

For durability uncertainty:

```text
seal
  -> COMMIT attempt
     -> success: infallible publish
     -> error: RuntimeRevisionCell = RecoveryRequired
               no reads / prepares / derived updates until recovery/restart
```

This keeps physical dense compaction independent from semantic output order and makes an uncertain durable COMMIT a fail-stop condition instead of a normal abort.

## 3. Implementation

Production source changed in:

```text
crates/kernel-query/src/lib.rs
crates/kernel-plan/src/lib.rs
```

No new crate or external dependency was added.

### 3.1 Exact constructive bootstrap binding

`PhysicalStore` now exposes:

```text
logical_rows_with_handles(relation, layout)
    -> Vec<(StableRowHandle, Row)>
```

in authoritative logical scan order.

`MaterializedRelPlanState` no longer accepts a naked handle vector for runtime bootstrap. It accepts exact row bindings through:

```text
attach_storage_rows(relation, &[(handle,row)])
```

and rejects the attachment unless each physical row payload exactly equals the corresponding maintained Scan row in the same logical order.

`RuntimeRevisionBundle::validate_physical_snapshot` was strengthened from semantic Bag/Set equivalence to exact ordered logical payload equality for the selected authoritative physical representation. A semantically equal but reordered selected layout is therefore rejected/rebuilt rather than trusted positionally.

This is deliberate: semantic equality is sufficient for query-set/bag equivalence, but it is insufficient for establishing row identity and deterministic insertion/output order.

### 3.2 Order-preserving maintained leaf representation

The old maintained leaf structure was:

```text
ids: Vec<StableRowHandle>
positions: handle -> dense position
```

with `swap_remove` directly defining both dense and logical order.

Pass33 replaces it with:

```text
dense_ids: Vec<StableRowHandle>
positions: handle -> dense payload position
links: handle -> { previous, next }
logical_head
logical_tail
```

Dense payload rows can still use `swap_remove` without O(n) positional repair. Logical order is updated independently by unlinking one stable-handle node and is preserved across first/middle deletions and subsequent insertions.

A Scan output with attached storage identities now materializes rows by traversing the logical linked sequence and resolving each handle to its current dense payload position. Dense physical movement therefore cannot leak into semantic output order.

### 3.3 Exact row↔handle removal validation

`validate_resolved_leaf_deltas` now checks, for every removal:

```text
handle exists
AND handle is unique in the receipt
AND current row(handle) == resolved removed row payload
```

A forged receipt that says “remove Beta” while carrying Alpha's handle is rejected with `InconsistentIncrementalDelta` before any leaf or parent state mutation.

Inserted handles must still be fresh with respect to the current maintained leaf. Runtime publication remains owned by `kernel-plan`; public construction of detached evidence does not grant publication authority.

### 3.4 Storage emits actual resolved row representatives

`PhysicalStore::apply_relation_delta_resolved_in_place` no longer copies the caller's requested semantic delta into `StorageResolvedRelationDelta` unchanged.

The resolved delta is rebuilt from the rows actually selected/inserted by the physical transition:

```text
removed payloads <- physical_delta.removed
inserted payloads <- physical_delta.inserted
```

This matters when a semantic equality relation can match different concrete representatives. Downstream maintained operators must propagate the actual removed representative from the current source state, not an arbitrary semantically equivalent request representative.

### 3.5 Uncertain COMMIT now fail-stops the runtime

`RuntimeRevisionCell` now owns:

```text
RuntimeRevisionCellState::Serving(Arc<RuntimeRevisionBundle>)
RuntimeRevisionCellState::RecoveryRequired
```

A successful durable COMMIT still immediately publishes the sealed candidate.

If `RevisionDurability::durably_commit` returns an error after the final runtime seal, the sealed writer capability atomically changes the cell to `RecoveryRequired` before releasing the writer lock.

After that:

- `snapshot()` returns `RuntimeRecoveryRequired`;
- `prepare_revision()` cannot proceed because it requires a snapshot;
- reconstructible index publication cannot proceed;
- the old runtime root is not served as though abort were known.

Only reopen/scan/recovery or process restart may decide which revision is durable.

This closes the important distinction:

```text
COMMIT I/O error != proven abort
```

## 4. Hostile falsification

Pass33 ports the independent hostile counterexamples into the production regression suite.

### H31-01 regression

Logical Bag `[1,2]`, selected physical Bag `[2,1]`.

Expected and observed Pass33 result:

```text
RuntimeRevisionBundle::build
-> LogicalPhysicalStateMismatch
```

The runtime can no longer bootstrap a positional row/handle alias.

### H31-02 regression

Start `[1,2,3]`, remove `1` through the real runtime revision path.

Expected and observed:

```text
authoritative Revision: [2,3]
maintained Scan:        [2,3]
```

Dense payload storage may move the last row physically, but logical Scan order does not.

### H31-03 regression

Maintained state contains Alpha at handle 0 and Beta at handle 1. Construct detached evidence:

```text
payload: remove Beta
handle:  handle 0
```

Expected and observed: `InconsistentIncrementalDelta`.

### Post-COMMIT uncertain regression

A durability backend succeeds at PREPARE, then reports an error from COMMIT.

Expected and observed:

```text
commit_revision_durable -> CommitDurabilityUncertain(...)
snapshot                -> RuntimeRecoveryRequired
```

The old root is no longer served after an uncertain durable outcome.

## 5. Verification

Final successful commands after the last production source change:

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
263 declared tests
67 kernel-plan tests
65 kernel-query tests
13 kernel-durability tests
20 workspace crates
32,273 Rust LOC
0 external Cargo sources
0 unsafe occurrences under crates/
```

Evidence: `evidence/pass33/`.

## 6. Checklist Pass33

### Closed exactly in this cycle

1. ✅ H31-01 bootstrap positional row↔handle alias.
2. ✅ H31-02 maintained Scan logical-order corruption after dense `swap_remove`.
3. ✅ H31-03 removed handle↔payload validation gap.
4. ✅ Storage-resolved deltas now propagate actual selected row representatives.
5. ✅ Exact authoritative selected-layout bootstrap order/payload contract.
6. ✅ Independent hostile reproductions promoted into production regression tests.
7. ✅ Uncertain durable COMMIT no longer leaves the old runtime root readable.
8. ✅ `RuntimeRecoveryRequired` fail-stop state blocks further runtime work until recovery/restart.

### Reopened by hostile review and now closed

1. ✅ Initial storage→Scan stable-handle binding correctness.
2. ✅ Order-preserving storage-resolved Scan maintenance.
3. ✅ Row-payload association of resolved removals.

### Previous OPEN narrowed by Pass33

1. 🟨 `CommitDurabilityUncertain` restart/recovery supervision: the unsafe “keep serving old root” behavior is closed; an automated in-process recovery owner is still OPEN.
2. 🟨 Bootstrap coherence: exact correctness is now stronger; a certified digest/root fast path remains OPEN as performance work.

## 7. Historical Pass26 OPEN backlog still active

These items remain project OPEN until explicitly production-closed; later transaction/durability passes do not erase them:

1. ⬜ Generic Text/F64 maintained TopK order-statistics.
2. ⬜ I64 TopK constant-factor gap.
3. ⬜ Group/TopK as typed-batch producers.
4. ⬜ Maintained I64 Group constant-factor gap.
5. ⬜ Indexed generic/Text Group.
6. ⬜ Persisted Text/F64/Bool/entity indexes + planner.
7. ⬜ Nested/multiway/mixed-key joins.
8. ⬜ Remaining physical layouts + OrderedView/pagination.
9. ⬜ WAL/recovery/durable materializations/crash tests — production WAL tail/replay are integrated; durable checkpoint/segments/real crash closure remains OPEN.
10. ⬜ Transaction repair runtime, distribution, formal mechanization.
11. ⬜ Semantic indexing/canonical-key strategy for generic maintained Join — Agent-2 R&D VERIFIED; production integration OPEN.

## 8. New / remaining OPEN after Pass33

1. ⬜ Exact durable base checkpoint encoding and installation.
2. ⬜ Parent-directory fsync/rename + checkpoint/segment manifest.
3. ⬜ WAL segment rotation, truncation and compaction.
4. ⬜ One restart/recovery owner that can replace a `RecoveryRequired` runtime only after validated WAL resolution.
5. ⬜ Real process/filesystem/power-loss crash tests on target filesystems.
6. ⬜ Durable revision-DAG/branch+merge parent representation.
7. ⬜ Client transaction/idempotency identity for crash-after-COMMIT-before-ACK ambiguity.
8. ⬜ Durable schema/Γ/lifecycle/field mutation format.
9. ⬜ Durable materialization configuration authority.
10. ⬜ Candidate runtime construction remains clone-heavy; COW/persistent roots remain OPEN.
11. ⬜ `RwLock` poisoning policy distinct from durability recovery-required state.
12. ⬜ Current maintained-leaf logical traversal is correctness-first (`BTreeMap` lookup per node); benchmark/compact node-layout optimization is OPEN, but delete no longer needs O(n) positional repair.
13. ⬜ Exact selected-layout bootstrap rejects semantically equivalent reordered/representative-different layouts; automated rebuild/canonical selected-layout construction should make this inexpensive operationally.
14. ⬜ Group commit, async durability, replication/consensus.
15. ⬜ CRC-32C remains accidental-corruption detection, not adversarial authentication.

## 9. Recommended next pass

The hostile blocker is now repaired, so the durability sequence can resume:

```text
Pass34
  exact durable base checkpoint
  + atomic checkpoint install
  + manifest/segment lifecycle
  + restart/recovery owner

then
  real crash/fault falsification

then
  durable typed schema/Γ migration or Agent-2 production semantic indexes
```

Do not weaken the exact row/handle/order bootstrap law for performance. A later fast path should certify the same invariant, not replace it with semantic Bag equality.
