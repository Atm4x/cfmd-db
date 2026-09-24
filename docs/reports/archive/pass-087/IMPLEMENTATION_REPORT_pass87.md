# IMPLEMENTATION REPORT — Pass87

## Scope

Production integration of historical #14 (authority-uncertainty restart classification) and #15 (barrier-safe group commit / non-authoritative async batching) on top of Pass86. Portable reference code was not transplanted wholesale; the changes were rebased onto the current epoch-qualified retry and Γ-REIC causal model.

## Source delta

Exactly three Rust files differ from Pass86:

- `crates/kernel-durability/src/lib.rs`
- `crates/kernel-durability/src/store.rs`
- `crates/kernel-plan/src/lib.rs`

No Cargo dependency or workspace topology changes were required.

## Historical #14

`StoreFaultHook` now returns a durability result, enabling deterministic fault injection through checkpoint publication. Checkpoint rotation tracks an `authority_uncertain` boundary: failures through pending-manifest sync are known-unpublished and preserve the serving root; manifest rename attempt and later failures poison the store and require reopen. `DurableRevisionStore::requires_recovery()` exposes that classification to the runtime layer.

`RuntimeRevisionCell` and materialization reconfiguration now distinguish ordinary unpublished checkpoint failure from `DurabilityUncertain`. `DurableRuntimeSupervisor::lock_runtime_recovering()` reconstructs a poisoned process-local runtime from durable authority and clears the mutex poison.

Hostile coverage includes all five pre-publication fault points, both post-publication uncertainty points, subsequent checkpoint after safe failure, and deliberate supervisor mutex poisoning.

## Historical #15

`FileRevisionWal` exposes internal unflushed PREPARE/COMMIT append primitives plus an explicit durability barrier; ordinary single-commit methods delegate to those primitives and preserve existing semantics.

`DurableRevisionStore::durably_commit_group` validates an ordered contiguous group, binds epoch-qualified retry identities and independent causal event ids, appends all PREPARE records, performs one prepare barrier, appends all COMMIT records, performs one final commit barrier, and only then mutates in-memory committed state / returns receipts. Invalid chain shape or duplicate transaction ids fail before publication.

`DurableCommitBatcher` is explicitly non-authoritative. Enqueue returns only `Queued`/`FlushRequired`; `flush` is the sole receipt-producing path and pending descriptors remain intact if flushing fails.

Hostile coverage verifies contiguous reopen parity, rejection of noncontiguous/duplicate groups, no acknowledgement before flush, and pending retention after failed flush.

## Verification

Final combined frozen tree:

- `cargo fmt --all -- --check` — PASS
- `cargo check --workspace --all-targets` — PASS
- `cargo clippy --workspace --all-targets -- -D warnings` — PASS
- `cargo test --workspace --all-targets` — **642 passed / 0 failed / 8 ignored**

The last successful combined full gate completed before the 20-minute source cutoff. No production source was modified after the cutoff.
