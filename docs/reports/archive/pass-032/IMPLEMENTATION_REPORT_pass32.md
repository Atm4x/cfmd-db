# IMPLEMENTATION REPORT — Pass32

Status: **VERIFIED** on Rust **1.98.1**.

## Problem

Pass31 provided a coherent one-root in-memory authority with `prepare -> seal -> infallible publish`, but a committed revision still had no production durable representation or recovery path. Agent-1 had independently verified a logical-WAL protocol, yet its prototype was not connected to the actual runtime transaction capability.

The main implementation requirement was to attach durability without allowing physical storage artifacts to become semantic authority and without reintroducing a fallible step after durable COMMIT.

## Hypotheses

1. Persist logical relation mutations plus pinned semantic revision, never `StableRowHandle`, physical layout or materialized state.
2. Durable PREPARE may precede the final runtime seal because it is non-authoritative and safely orphanable.
3. Durable COMMIT must happen only after the final exact freshness seal and before the infallible whole-root publish.
4. Recovery should replay from one exact trusted base `Revision` and rebuild physical/materialized state from semantic authority.
5. The production WAL needs a deterministic codec, framing/checksums, file locking and strict duplicate/tail protocol rather than directly copying the isolated R&D prototype.

## Implementation

### `kernel-durability`

Added a new std-only workspace crate with:

```text
DurableRelationMutation
DurableRevisionDescriptor
DurablePrepareToken
DurableCommitReceipt
CommittedRevision
RecoveryScan
RevisionDurability
FileRevisionWal
SimulatedRevisionWal
```

The stable relation-data codec covers every current `kernel_model::Value` variant and is independently versioned from the frame format.

The WAL frame is a fixed 36-byte header plus payload and carries:

```text
magic
format version
record kind
flags
payload length
LSN
revision id
payload CRC-32C
header CRC-32C
```

COMMIT binds the exact PREPARE LSN and PREPARE payload CRC.

`FileRevisionWal`:

- appends PREPARE and `sync_data`;
- appends COMMIT and `sync_data`;
- acquires an exclusive file lock;
- uses `create_new` to prevent accidental existing-log truncation;
- poisons itself after write/barrier failure;
- reopens only after scanner validation;
- truncates only scanner-certified safe final tails;
- resumes the scanner-derived next LSN.

### Scanner/recovery protocol

The scanner verifies:

- format/header CRC;
- payload CRC;
- monotone contiguous LSN;
- exact PREPARE payload decoding;
- duplicate identity rules;
- COMMIT→PREPARE binding;
- source revision equals the current durable head.

Incomplete final frame or short trailing garbage can be safely discarded after the last complete validated frame. Enough non-frame bytes to conceal a complete frame are rejected as corruption.

### Mainline transaction integration

Added conversion:

```text
RevisionCommitDescriptor::durable_descriptor()
```

and mainline operation:

```text
RuntimeRevisionCell::commit_revision_durable(...)
```

Exact order:

```text
prepare runtime candidate
-> WAL PREPARE + barrier
-> seal RuntimeRevisionCell
-> WAL COMMIT + barrier
-> infallible root publish
```

A stale seal after PREPARE leaves only an uncommitted log record. A COMMIT durability error does not publish runtime state and is surfaced as `CommitDurabilityUncertain`, because the durable result cannot be inferred from the returned I/O error alone.

### Replay and rebuild

Added:

```text
replay_durable_revisions
recover_runtime_bundle
```

Replay accepts one exact trusted base `Revision`, validates the pinned semantic revision, re-derives relation types from the exact semantic context, applies committed logical mutations and constructs each target through `Revision::build`.

Runtime recovery reconstructs a fresh `PhysicalStore` and maintained registry. Stable handles, indexes and prior process-local root identity are not replayed.

A hostile integration test exposed a physical family bug in the first recovery implementation: logical model rows were incorrectly used as a native RowStore layout binding. Added `LayoutBinding::RECOVERY_ROW_STORE` as an explicit reconstructible recovery representation.

## Hostile falsification

New durability tests cover:

1. CRC-32C known vector;
2. codec round-trip for every current `Value` shape;
3. every byte-prefix cut of multi-revision WAL;
4. PREPARE-without-COMMIT invisibility;
5. every single-bit corruption of a committed stream;
6. wrong source-head COMMIT;
7. exact duplicate prepare/commit idempotence;
8. conflicting duplicate prepare;
9. conflicting duplicate commit;
10. short garbage versus full-frame-sized garbage;
11. non-monotone valid-checksum LSN;
12. accidental create-over-existing-log prevention;
13. safe torn-tail file recovery and LSN continuation.

Integrated `kernel-plan` hostile cases cover durable-before-publish ordering, multiple nonconsecutive revisions, uncertain COMMIT failure and stale-after-durable-PREPARE.

## Verification

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
260 declared tests
65 kernel-plan tests
13 kernel-durability tests
20 workspace crates
31,959 Rust LOC
0 external Cargo sources
0 unsafe occurrences under crates/
```

Evidence: `evidence/pass32/`.

## Rejected routes

1. **Physical WAL of slots/handles/index pages.** Rejected because it couples semantic recovery to reconstructible layout details.
2. **Serialize process-local root lineage/version.** Rejected because those coordinates exist only for live freshness and must be regenerated after restart.
3. **Perform seal/freshness after durable COMMIT.** Rejected because runtime could then refuse a revision that disk already declares committed.
4. **Hold the writer seal while writing PREPARE.** Correct but unnecessarily lengthens exclusive runtime lock time; an orphan PREPARE is safe, so PREPARE can precede seal.
5. **Treat COMMIT write error as guaranteed abort.** Rejected; a sync/write failure can leave uncertain durable state.
6. **Recover physical layouts/handles as authority.** Rejected; recovery rebuilds them from the recovered logical revision.
7. **Use `LayoutFamily::LogicalModelRows` as recovery RowStore.** Rejected by actual integration test; logical pseudo-layout is not a native RowStore family.
8. **Open/create WAL with truncation.** Rejected; creation is `create_new` and existing segments reopen through validated recovery.

## Remaining risks / OPEN

1. Exact durable base checkpoint creation/installation is not implemented.
2. Parent-directory fsync/rename/manifest policy for initial segment and checkpoint files remains filesystem-specific work.
3. Segment rotation, truncation and compaction are not implemented.
4. Real process/filesystem/power-loss crash tests remain to be run on target filesystems.
5. Durable branch/merge revision-DAG parent representation is not yet in the linear runtime WAL tail.
6. Schema/Γ/lifecycle/field migration records are outside the current relation-data codec.
7. Client transaction/idempotency identity is needed to resolve ACK ambiguity cleanly.
8. Materialization specs must get a durable configuration authority instead of being injected manually at recovery.
9. Candidate runtime construction remains clone-heavy.
10. RwLock poisoning and `CommitDurabilityUncertain` need an engine-level restart/recovery owner.
11. Group commit/asynchronous durability/replication are unimplemented performance/distribution layers.
12. CRC-32C is accidental-corruption detection, not adversarial authentication.
13. Historical Pass26 performance/features OPEN items remain active; Agent-2 canonical-index work is still not production-integrated.

## Recommended next integration

Keep the mainline on durability for one more pass:

```text
exact durable base checkpoint
+ checkpoint/segment manifest
+ parent-directory durability
+ segment rotation/truncation
+ one recovery/restart owner
```

After that, run real crash/fault falsification and then define the durable typed migration path for schema/Γ changes. Semantic-index production integration should follow the stable durable revision pipeline rather than being merged concurrently with it.
