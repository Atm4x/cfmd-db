# Pass118 — Historical #17 external freshness / anti-rollback closure

## Status

- Historical #17: **PROD CLOSED Pass118**.
- Historical ledger: **20 / 22 PROD CLOSED**.
- Source freeze: 2026-09-23T18:13:31Z (21:13:31 MSK), before the cycle hard wall.
- Production source fingerprint: `12e09dcd9d9f1ea36a4d7f28b1079969e9e29e86b9f78d7b5575e2a450ea4f80`.

## Problem → hypothesis → implementation → falsification → result

### Problem
Pass117 had authenticated freshness cuts and store integration, but the authority was still only a trait/test authority. #17 remained open because there was no genuinely separate rollback domain, no independent-process restart/fault evidence, and no complete reopen hostile matrix.

### Hypothesis
Keep the store protocol unchanged and supply a real external authority implementation over a canonical bounded TCP protocol. The authority process owns the signing key and a separate atomic CAS state directory; the database process owns only the TCP client and trust roots. If all recovery checks remain before ordinary WAL recovery and all ambiguity poisons/fails closed, this closes the anti-rollback boundary without moving private-key authority into the database process.

### Implementation
1. Added `TcpExternalFreshnessAuthority` production client.
2. Added `TcpExternalFreshnessAuthorityServer` production provider:
   - separate TCP process boundary;
   - Ed25519 signing key remains server-side;
   - versioned bounded CFFA wire protocol;
   - exact-record compare-and-advance CAS;
   - monotone generation/WAL fencing;
   - single-writer state-directory lock;
   - versioned signed-record persistence;
   - atomic `write -> fsync -> rename -> directory fsync` state publication.
3. `ExternalFreshnessAuthority` now requires `Send`, preserving existing `DurableRuntimeSupervisor` thread-transfer invariants.
4. Added independent-process restart/unavailable test using a real TCP authority process and persistent authority directory.
5. Added store hostile matrix for:
   - CAS failure before apply;
   - response loss after apply;
   - unavailable authority;
   - stale deployment-policy identity;
   - local generation rollback;
   - same-generation fork;
   - WAL truncation;
   - authenticated WAL-prefix fork.
6. Existing Pass117 invariants remain in force:
   - plain `open()` cannot bypass externally anchored stores;
   - external verification runs before ordinary store recovery/WAL replay;
   - WAL/checkpoint publication advances the external cut;
   - ambiguous external advancement poisons the live store and requires reopen.

### Falsification
- Full workspace initially exposed a real integration defect: the new authority trait was not `Send`, which broke `DurableRuntimeSupervisor` thread transfer. The trait boundary was corrected to `Debug + Send` and all full gates were rerun.
- Independent authority process is stopped between requests: the client fails instead of falling back locally.
- Authority process restart recovers the exact signed CAS state from its separate directory.
- Authority server rejects generation regression even from a client holding the current record digest.
- Store hostile tests prove response-loss convergence and reject rollback/fork/truncation/stale/unavailable states before normal recovery.

### Result
#17 is closed at the production architecture boundary: a local durable store can no longer establish freshness authority by itself once external freshness is enabled. Authority is an authenticated, independently persisted CAS cut in a separate process/domain, and local rollback/fork/WAL replay is rejected against that cut before recovery publishes a usable store.

## Gates

- `cargo fmt --all -- --check`: PASS.
- `cargo check --workspace --all-targets`: PASS.
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS.
- `kernel-auth`: 12/12 PASS.
- `kernel-durability`: 97/97 unit PASS.
- external freshness multi-process tests: 2/2 PASS.
- replication multi-process tests: 2/2 PASS.
- full workspace: **758 passed / 0 failed / 8 ignored = 766 declared**.

## Changed production/test surface vs Pass117

- `crates/kernel-durability/Cargo.toml`
- `crates/kernel-durability/src/lib.rs`
- `crates/kernel-durability/src/store.rs`
- `crates/kernel-durability/src/freshness_tcp.rs` (new)
- `crates/kernel-durability/tests/external_freshness_multiprocess.rs` (new)
- `Cargo.lock`

## Remaining historical problems

Only **#13** and **#20** remain open.

- #13: supported-platform real durability assurance for the filesystem axioms used by #18 and external authority persistence.
- #20: formal surface-to-kernel mechanization.

Recommended next frontier: #13 first, because it discharges the real-platform assumptions underneath the now-closed #18/#17 durability stack.
