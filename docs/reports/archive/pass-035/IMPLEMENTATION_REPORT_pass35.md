# IMPLEMENTATION REPORT — Pass35

## problem

Pass34 supplied the durable checkpoint/WAL/manifest architecture and restart owner, but its crash claims were still based on synthetic corruption/torn-tail/orphan-state tests. The implementation had not been killed as a real subprocess at PREPARE, COMMIT, checkpoint publication, manifest publication, or compaction boundaries.

## hypotheses

1. A real child process killed after durable PREPARE must reopen at the prior committed revision.
2. A child killed after durable COMMIT must reopen at the target revision even if ACK/runtime publication never occurred.
3. Checkpoint and new-WAL files remain non-authoritative until final manifest publication.
4. Killing obsolete-generation compaction may leave garbage, but must not damage the active published generation.
5. Crash injection should be a private implementation seam; no test hook should become a production authority API.

## implementation

### kernel-durability/store.rs

Factored checkpoint/manifest/compaction internals through a private `StoreFaultHook`.

Production entry points always use `NoStoreFault`. Test-only workers use a blocking hook at named stages:

```text
AfterCheckpointSync
AfterWalSync
AfterPrerequisiteDirectorySync
AfterPendingManifestSync
AfterManifestRename
AfterManifestDirectorySync
BeforeCompactionRemove
AfterCompactionRemove
AfterCompactionDirectorySync
```

Workers signal a test-only marker when the selected point has been reached and then block. The parent calls `Child::kill()`, waits for process termination, and opens the durable store again.

Added subprocess tests for:

- kill after PREPARE;
- kill after COMMIT before ACK;
- six checkpoint/manifest boundaries;
- three compaction boundaries.

### kernel-plan

Added an end-to-end subprocess falsifier for the critical runtime gap:

```text
prepare candidate
-> durable PREPARE
-> seal
-> durable COMMIT
-> child signals killpoint
-> parent kills child before sealed.publish()
```

After restart, `DurableRuntime::open` must recover the target revision from durable authority and rebuild physical + maintained state.

### evidence

Added `PASS35_CRASH_MATRIX.md` and repeated stress evidence:

```text
20x kernel-durability subprocess crash matrix  PASS
10x runtime COMMIT-before-publish crash         PASS
```

## hostile falsification result

No Pass34 ordering defect was found under actual subprocess kill on the current filesystem.

Observed authority boundaries:

```text
PREPARE only                    -> old revision
COMMIT complete                 -> target revision
pre-manifest-rename checkpoint  -> old generation + valid WAL tail
post-manifest-rename            -> new generation visible after process kill
partial obsolete GC             -> active generation remains reopenable
COMMIT before runtime publish   -> target reconstructed on restart
```

The remaining COMMIT-before-ACK problem is **client retry ambiguity**, not server recovery ambiguity. Recovery knows which revision committed; the external client has no durable transaction id yet.

## rejected routes

### `process::exit()` as crash proof

Rejected as the final harness. The worker is now killed externally by the parent using `Child::kill()` after a coordination marker.

### Public crash-injection API

Rejected. Fault hooks are private implementation details and do not widen the production authority surface.

### Claim power-loss safety from process-kill tests

Rejected. A killed process is useful evidence for process crash/restart, but not for storage-controller cache loss or rename persistence without directory fsync after sudden machine power loss.

### Treat COMMIT-before-ACK as automatically idempotent

Rejected. Without a durable client transaction key, a retry cannot yet be identified as the same semantic transaction.

## verification

Rust 1.98.1 final gate:

```text
cargo fmt --all -- --check                                  PASS
cargo check --workspace --all-targets                       PASS
cargo test --workspace --all-targets                        PASS
cargo clippy --workspace --all-targets -- -D warnings       PASS
cargo test --workspace --all-targets --release              PASS
cargo build --workspace --release                           PASS
RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps  PASS
RUSTFLAGS='-C overflow-checks=yes' cargo test ... --release PASS
```

Metrics:

```text
282 declared tests
72 kernel-plan tests
65 kernel-query tests
27 kernel-durability tests
20 crates
34,625 Rust LOC
0 external Cargo sources
0 unsafe
```

## result

Pass35 closes the missing **actual process-kill** evidence for the current durability design. The next core durability problem is no longer “does restart recover the right revision if the writer dies?” for the tested process/filesystem model. It is now the external transaction-outcome protocol: durable idempotency identity, ACK-loss retry semantics, and supervisor/recovery orchestration.
