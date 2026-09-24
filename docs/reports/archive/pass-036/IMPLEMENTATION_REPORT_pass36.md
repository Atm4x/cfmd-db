# IMPLEMENTATION REPORT — Pass36

## problem

Pass35 proved that the server can recover the committed target after `COMMIT durable -> process death before runtime publish/ACK`. Three linked operational gaps remained:

1. the retrying client had no durable transaction identity with which to ask whether *its* request committed;
2. reopen still trusted caller-provided materialization specs;
3. `RecoveryRequired` / uncertain-COMMIT recovery was a manual application responsibility rather than a normal runtime owner workflow.

## hypotheses

1. A client transaction ID must be part of the checksummed logical PREPARE identity, not an in-memory request tag.
2. Committed transaction outcomes must survive checkpoint rotation/compaction, otherwise idempotency disappears exactly when the WAL tail is compacted.
3. Maintained materialization specs are durable operational configuration and must be selected by the same published generation as checkpoint + WAL.
4. Reopen/retry should be centralized in one supervisor; lower layers should remain fail-stop rather than guessing an uncertain COMMIT outcome.
5. Physical handles/layout/index state remain reconstructible and must not be added to durable authority.

## implementation

### kernel-types

Added nominal `ClientTransactionId(u128)`.

### kernel-durability WAL

`DurableRevisionDescriptor` mutation codec v2 includes `ClientTransactionId`.

`RecoveryScan` records exact committed transaction outcomes. Conflicting reuse of one transaction ID for a different PREPARE/target is a protocol violation; exact duplicates retain existing WAL idempotence.

### durable generation metadata

Added `metadata-N.cfdm`, a versioned deterministic control-plane metadata file containing:

- maintained `MaterializationId + RelExpr` specs;
- committed `ClientTransactionId -> RevisionId` outcomes.

All current `RelExpr` forms are explicitly encoded; Rust memory representation is not persisted.

Manifest v2 binds the metadata checksum. Publication order now includes metadata fsync before prerequisite directory fsync and final manifest publication. Compaction includes metadata generation files.

### DurableRevisionStore

The store now owns materialization specs and the transaction outcome ledger.

Checkpoint rotation writes both into the next generation. Reopen merges checkpoint-carried transaction outcomes with the committed WAL tail and rejects conflicts.

### kernel-plan durable runtime

`RuntimeRevisionBundle` retains the exact materialization specification registry used to build maintained states.

`DurableRuntime::create` persists those specs. `DurableRuntime::open` no longer takes materialization specs from the caller; it loads them from durable generation authority.

`DurableRuntime::commit_revision(tx_id, request)` returns either:

```text
Committed(receipt)
AlreadyCommitted { target_revision }
```

and rejects same-ID/different-target reuse.

### recovery supervisor

Added `DurableRuntimeSupervisor`, which owns the durable directory, semantic registry and current runtime instance.

On an uncertain COMMIT / `RecoveryRequired` path it:

```text
drops fail-stopped runtime
-> reopens durable authority
-> queries the same ClientTransactionId
-> returns AlreadyCommitted / conflict / safely retries same ID
```

`snapshot` similarly reopens a fail-stopped runtime before serving. Checkpoint reopen failures preserve the concrete `RuntimeRecoveryError` instead of being collapsed into a poison placeholder.

## hostile falsification

Verified cases include:

- actual Pass35 subprocess death after durable COMMIT but before publish/ACK, followed by supervisor retry of the same transaction ID;
- transaction ID surviving checkpoint + obsolete-generation compaction;
- same ID/same target does not double-apply;
- same ID/different target is rejected;
- reopen rebuilds a maintained materialization without caller-supplied specs;
- missing/corrupt published metadata sidecar is corruption;
- forced `RecoveryRequired` is reopened and retried through the supervisor.

## rejected routes

### Keep transaction IDs only in WAL tail

Rejected. Checkpoint rotation would erase retry identity.

### Treat revision ID as client transaction identity

Rejected. Revision identity names semantic state/version, not a retryable external request. Conflating them breaks concurrent/client idempotency semantics.

### Caller re-supplies materialization specs on reopen

Rejected as durable authority leakage. A restart could otherwise rebuild a different runtime configuration from the same published generation.

### Persist row handles or physical layouts to solve reopen configuration

Rejected. These remain reconstructible physical evidence/state and are not semantic/control-plane authority.

### Automatically GC old transaction IDs without an external retention contract

Rejected. Deleting an ID can turn a late retry into a duplicate semantic mutation. Outcome retention/expiry needs an explicit protocol.

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
287 declared tests
74 kernel-plan tests
65 kernel-query tests
30 kernel-durability tests
20 crates
35,711 Rust LOC
0 external Cargo sources
0 unsafe
```

Additional repeated evidence: the client-id/checkpoint retry case, supervisor recovery case and real subprocess COMMIT-before-publish retry were each repeated **10x**; all runs passed. Raw output: `evidence/pass36/DURABLE_CONTROL_STRESS.log`.

## result

Pass36 closes three durable-control-plane problems rather than counting individual tests as solved problems:

1. durable client transaction outcome/idempotent ACK-loss retry;
2. durable materialization specification authority on restart;
3. executable supervisor ownership of fail-stop reopen/outcome-resolution/retry.

The remaining durability frontier is no longer ordinary relation-data commit/restart. It is wider revision semantics (schema/Γ/lifecycle), version migration, configuration evolution, long-term transaction-ledger retention, production-filesystem power-loss validation, and distributed/async durability.
