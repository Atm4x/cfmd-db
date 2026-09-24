# IMPLEMENTATION REPORT — Pass33

Status: **VERIFIED** on Rust **1.98.1**.

## Problem

A delayed hostile review of Pass31 found three leaf-contract defects that remained underneath Pass32 WAL integration: positional bootstrap could bind handles to the wrong logical rows, maintained Scan deletion leaked `swap_remove` order, and resolved evidence did not prove removed handle↔row association. The same review also required Pass32 to fail-stop after an uncertain durable COMMIT rather than continue serving the old root.

Because those are correctness blockers below the durability layer, Pass33 repairs them before implementing durable checkpoint/segment authority.

## Hypotheses

1. Bootstrap must be constructive: attach exact ordered `(StableRowHandle, Row)` bindings, never a naked positional handle vector after only semantic Bag equality.
2. Dense payload compaction and semantic logical order must be separate data structures.
3. Removed resolved evidence must validate `handle -> exact current row` before any mutation/propagation.
4. Storage should propagate the actual concrete row representative it removed.
5. A COMMIT I/O error after the final seal is an uncertain durability outcome; runtime must enter a recovery-required fail-stop state under the writer guard.

## Implementation

### `kernel-query`

Replaced `MaintainedLeafHandles { ids, positions }` with a dense/logical split:

```text
dense_ids
positions
links(handle -> previous/next)
logical_head
logical_tail
```

Removal still uses dense `swap_remove` for payload/position maintenance, but logical ordering is updated independently by unlinking the stable handle. Scan output follows the logical links and therefore preserves authoritative insertion order.

Replaced runtime bootstrap attachment with:

```text
attach_storage_rows(relation, &[(StableRowHandle, Row)])
```

The exact row payload and order must match the Scan snapshot.

Resolved-removal validation now requires exact current row equality for every supplied handle.

### `kernel-plan`

Added:

```text
PhysicalStore::logical_rows_with_handles(...)
```

and changed runtime bootstrap to pass exact logical-order physical row bindings into every maintained Scan.

`RuntimeRevisionBundle::validate_physical_snapshot` now requires exact ordered selected-layout payload equality with the authoritative revision, not merely semantic Bag/Set equality.

Storage-resolved deltas are reconstructed from actual `physical_delta.removed/inserted` row payloads rather than blindly cloning the requested delta.

### Durability fail-stop

`RuntimeRevisionCell` now stores an internal state:

```text
Serving(Arc<RuntimeRevisionBundle>)
RecoveryRequired
```

On `durably_commit()` error after seal, the sealed transition sets `RecoveryRequired` before releasing the writer guard. `snapshot`, prepare, and derived root publication then reject work until recovery/restart.

A successful COMMIT still immediately invokes infallible whole-root publication.

## Hostile falsification

Promoted hostile reproductions into production tests:

1. semantically equal but reordered physical Bag bootstrap is rejected;
2. first-row deletion from `[1,2,3]` preserves maintained output `[2,3]`;
3. forged “Beta payload + Alpha handle” removal is rejected;
4. uncertain COMMIT makes the live cell unreadable with `RuntimeRecoveryRequired`.

The original hostile report, ledger and repro patch are preserved in `evidence/pass33/hostile_pass31/`.

## Verification

All of the following pass on Rust 1.98.1:

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

Metrics:

```text
263 declared tests
67 kernel-plan tests
65 kernel-query tests
13 kernel-durability tests
20 crates
32,273 Rust LOC
0 external Cargo sources
0 unsafe
```

## Rejected routes

1. **Use `Vec::remove` at the maintained leaf.** Correct order, but reintroduces known O(n) positional repair on deletion.
2. **Keep positional handle attachment and only strengthen comments.** Does not establish row identity.
3. **Accept semantic Bag equality at bootstrap and compute a semantic bijection.** Possible but ambiguous with duplicates/representatives and unnecessarily weak for the selected authoritative physical copy. Exact selected-layout correspondence is a cleaner invariant.
4. **Continue propagating caller-requested removed representatives.** Wrong when semantic equality can match a different concrete source representative.
5. **Treat `durably_commit` error as normal abort.** Rejected because stable storage may already contain COMMIT.
6. **Publish candidate despite COMMIT error.** Also wrong: durable outcome is unknown, not known committed.
7. **Keep serving old root after uncertain COMMIT.** Rejected; fail-stop until recovery is required.

## Remaining risks / OPEN

1. Durable exact base checkpoint/manifest/segment lifecycle remains the next mainline durability block.
2. No automatic in-process recovery owner yet replaces `RecoveryRequired`; restart/reopen+scan is still required operationally.
3. Real filesystem/power-loss crash testing remains OPEN.
4. Candidate runtime construction remains clone-heavy.
5. Maintained leaf ordered output traverses `BTreeMap` handle maps; benchmark and compact-node optimization remain future performance work.
6. Strict selected-layout exact bootstrap may force rebuild when an otherwise semantically equivalent physical copy has different row order/representatives; this is intentional for correctness but needs an efficient reconstruction path.
7. Schema/Γ/lifecycle/field durable migrations and revision-DAG durability are not covered by the current relation-data WAL codec.
8. Historical Pass26 performance/features OPEN backlog remains active, including Agent-2 semantic-index production integration.

## Recommended next implementation

Resume the durability plan as Pass34:

```text
exact base checkpoint codec/authority
-> temp write + fsync
-> atomic rename/install + parent-dir durability
-> manifest selecting checkpoint + active WAL segment
-> segment rotation/truncation
-> restart/recovery owner
```

Then run actual crash/fault injection before widening the durable mutation class.
