# PASS591 REPORT — PERSISTENCE-TRANSITION HOSTILE CLEANUP / CONSOLIDATION

## Status

PASS591 COMPLETE.

Official UTC start: 14:21:26
Functional freeze: 14:34:04
Useful boundary: 14:41:26
Hard boundary: 14:45:26

## PROJECT RULES / CONTINUATION CONTRACT

`PROJECT_RULES.md` was read before implementation. PASS592 MUST read `PROJECT_RULES.md`, this PASS591 report, and the active ledger tails before implementation, and MUST carry this same requirement forward.

## LEDGER — GOAL

Stop feature expansion after PASS590 and hostile-audit the complete persistence-transition line: stale cut-only/API vocabulary, duplicate persistence ownership, fallback/generic seams, directory-backend demotion, persistence-image complexity, provider/wire helpers, tests, ledgers and repository gates.

## LEDGER — SELECTED IMPLEMENTATION

Keep one runtime/store authority. Remove the zero-semantics runtime wrapper. Before retiring any durable backend, convert only retained historical authority that still physically depends on that backend into the existing source-independent portable historical closure. This is the minimum information-preserving transition and introduces no alternate evaluator or fallback.

## LEDGER — IMPLEMENTED

- Removed `RuntimePersistenceAuthority`; `DurableRuntime` directly owns `Mutex<DurableRevisionStore>` and the store backend remains the sole persistence-state authority.
- Added `materialize_retained_history_for_volatile()` and invoke it before external fence publication/backend retirement.
- Added regression proving a directory-backed migration epoch remains exactly available after Durable -> Volatile and after repersisting into a single-file durable target.
- Corrected SPEC/R&D/active ledgers: demotion is O(1) only when retained history is already RAM/self-contained; otherwise O(bytes of retained historical authority) is required by the no-fallback volatile-owner law.
- Classified `compare_and_rebind_signed` as live cross-lineage/store authority-transfer semantics, not stale cut-only compatibility routing.
- Added mandatory `CHECKPOINT ROADMAP` law to `PROJECT_RULES.md`.

## Hostile findings

### 1. Real retained-history authority loss

PASS590 transformed backend/WAL ownership in place, but directory/single-file retained historical epochs may still have only anchors in RAM while their materialization authority lives on the physical backend. Retiring that backend without capture left the volatile owner unable to answer `historical_epoch_material()` or construct an exact later persistence image.

The fix linearizes before fencing: first make every missing retained historical closure source-independent in RAM; only then may freshness fence publication and backend retirement occur. If capture fails, demotion has not linearized and the durable source remains authoritative.

### 2. PASS589 O(1) demotion bound was too strong

A genuine volatile owner cannot depend on bytes whose only authority remains on a retired durable backend. Moving those retained bytes is information-theoretically necessary. The exact law is:

```text
O(1)                         if retained historical authority is already self-contained
O(retained historical bytes) if retained authority is disk-only
```

This is not O(current rows), not logical export/import, and not O(unretained history).

### 3. Duplicate runtime persistence wrapper had no remaining law

After PASS590 removed the `Volatile/Durable` enum, `RuntimePersistenceAuthority` was only a tuple wrapper plus deref/trait forwarding around a type that already implements `RevisionDurability`. It is deleted rather than retained as decorative routing.

### 4. Next measured payer: replication transfer history

`CanonicalPersistenceImage` still obtains replication authority as portable physical journal frames and target staging replays those frames. That pays for the path by which authority was reached, not only the current authority itself. `ReplicationAuthoritySemanticSnapshot` is already exhaustive over the semantic state and explicitly excludes physical frame ownership.

PASS591 does not open that new feature line. It selects PASS592 to replace transfer-time physical-history payment with one canonical exact semantic carrier. The target bound is O(retained semantic replication authority), with semantically required retained effects/votes/locks/membership/authentication state preserved exactly.

## Compatibility / fallback classification

- No cut-only decoder or mutation route exists.
- `FreshnessCut` remains a legitimate payload of `FreshnessAuthorityState::DurableCut`.
- `compare_and_rebind_signed` remains current because authority transfer changes lineage/store identity; it is not a fallback for persistence transitions.
- No SQL-shaped persistence fallback, logical export/import path, second in-memory engine, or compatibility format route was found in the PASS590 line.

## Verification

Completed before functional freeze:

- focused directory retained-history demotion/repersistence regression: PASS.
- `cargo check -p kernel-plan --offline`: PASS after wrapper removal.
- `cargo check -p cfmd-runtime --offline`: PASS after wrapper removal.
- `cargo test -p kernel-durability --lib --offline`: **279 passed / 0 failed / 2 ignored**.
- `cargo test -p kernel-plan --lib --offline`: **316 passed / 0 failed / 11 ignored**.

- `cargo test -p cfmd-runtime --lib --offline`: **30 passed / 0 failed**.
- strict Clippy over `kernel-durability`, `kernel-plan`, `cfmd-runtime` all targets/no deps with `-D warnings`: PASS.
- `cargo check --workspace --offline`: PASS.
- `cargo fmt --all -- --check`: PASS.
- repository manifest resealed over **5196** tracked paths; repository verifier: PASS.
- final package excludes `target/` and `.git`; ZIP integrity is verified after packaging.

## CLOSED THIS PASS

- Mandatory PASS591 persistence-transition cleanup checkpoint.
- Duplicate runtime persistence wrapper.
- Backend-retirement loss of retained historical epoch authority.
- Stale SPEC statement that Durable -> Volatile was not public.
- Incorrect unconditional O(1) durable -> volatile complexity claim.

## OPEN — IMMEDIATE

1. **PASS592 — canonical replication-authority semantic carrier.** Replace canonical-image/fork transfer of physical replication journal history with an exact current semantic carrier derived from the exhaustive semantic snapshot.
2. Prove carrier restore equals journal replay for every authority field and crash/reopen boundary.
3. Benchmark transfer bytes/CPU/peak memory versus frame-history depth and retained semantic-state size.

## OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- Protocol/Python/.NET projections until the Rust authority contract is deliberately stabilized.
- Native Windows secure-memory hardening and public performance/binary-size budgets.
- Semantic-fiber automatic retention synthesis until concrete evidence reopens it.

## SUPERSEDED / DO NOT EXTEND

- `RuntimePersistenceAuthority` as wrapper or enum over `DurableRevisionStore`.
- Unconditional O(1) demotion claim in presence of disk-only retained historical authority.
- Treating portable replication frames as semantic authority merely because replay is exact.
- Any persistence transition that keeps the retired durable backend as hidden historical fallback.

## PERFORMANCE BASELINES TO PRESERVE

- Volatile commit hot path performs no external freshness I/O.
- Persistence transition performs no logical current-row rebuild/import.
- Durable -> Volatile copies only authority that must survive retirement of its physical owner.
- PASS592 must improve transfer dependence from physical journal history toward retained semantic authority without weakening exact replication state.

## CHECKPOINT ROADMAP

1. **PASS591 hostile cleanup/consolidation:** CLOSED; feature freeze may end after final gates/package.
2. **PASS592 canonical replication-authority semantic carrier:** next architecture/R&D checkpoint.
3. **PASS593 hostile + performance consolidation:** mandatory stop after PASS592. No third major feature line before PASS593 is clean.

## NEXT RECOMMENDED PASS

**PASS592 — canonical replication-authority semantic carrier.** Build one exact portable semantic authority representation for persistence/fork transfer, remove full historical frame replay from that path, and prove/measure the new bound. Do not alter incremental `CFAS/CFAO` checkpoint realization unless the same semantic carrier yields a strictly cleaner single-law integration.
