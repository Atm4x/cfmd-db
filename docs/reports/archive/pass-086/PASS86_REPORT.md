# PASS86 REPORT — historical #11 durable idempotency epoch / retry-GC closure

Status: **PROD CLOSED** for historical problem #11.
Baseline: frozen Pass85 / PWRC-verified workspace.
Source freeze: **2026-09-22 20:55:09 UTC**.

## Problem

Pass85 rejected the portable #11 patch because raw `ClientTransactionId` was still overloaded as both retry identity and Γ-REIC causal-effect identity. Epoch reuse could therefore alias causal effects; a crash after reuse but before checkpoint also had no replay-stable independent event identity.

## Production resolution

- Added `IdempotencyEpoch` and `DurableTransactionKey { epoch, transaction_id }`.
- Retry ledger and recovery maps are keyed by the composite identity.
- Added durable `current_idempotency_epoch` and `minimum_retry_epoch`.
- Added `RetryHistoryExpired`; expired retries cannot fall through to fresh execution.
- Added monotone epoch advance and retry-history expiry/GC APIs.
- Decoupled `RevisionEffectId` from raw transaction ids.
- Causal effect identity is allocated before prepare and persisted in the WAL descriptor, so recovery after a pre-checkpoint crash is replay-stable.
- `DurableRevisionEffectRecord` now contains its canonical durable intent; Γ-REIC no longer depends on retry-ledger payload retention.
- Metadata codec advanced to v12; mutation/WAL codec advanced to v9 with backward decoding and legacy zero-epoch compatibility.
- Runtime exposes epoch-qualified outcome queries and retry-horizon operations.

## Hostile closure gates

PASS:
- raw transaction-id reuse in a later epoch produces distinct exact retry entries and distinct causal effects;
- crash after new-epoch reuse before checkpoint reconstructs the current epoch from WAL and preserves both outcomes;
- retry GC removes old exact retry intent, persists the watermark through checkpoint/reopen, and returns `RetryHistoryExpired` afterward;
- old Γ-REIC causal ideal remains self-contained after retry payload GC;
- same composite retry key still rejects conflicting intent;
- existing causal-frontier/multi-parent tests were rebased to independent event identity rather than tx-id numerology.

## Verification

- `cargo clippy --workspace --all-targets -- -D warnings`: PASS.
- Full workspace tests: **636 passed, 0 failed, 8 ignored**.
- `cargo fmt --all -- --check`: PASS.
- `cargo check --workspace --all-targets`: PASS.
- Source delta versus Pass85: exactly 4 Rust files.

Changed source files:
- `crates/kernel-durability/src/lib.rs`
- `crates/kernel-durability/src/metadata.rs`
- `crates/kernel-durability/src/store.rs`
- `crates/kernel-plan/src/lib.rs`

## Boundary / non-claims

#11 closes bounded **retry-ledger** retention. Γ-REIC causal history intentionally remains a separate durable concern; its future compaction/causal-DAG policy belongs to historical #8 and is not silently counted as retry history.

## Next checkpoint

**Pass87: historical #14 — authority-uncertainty restart classification / restart-poison policy.** After #14, proceed to #15 barrier-safe group commit, then #19 bounded repair unless hostile evidence changes dependency order.
