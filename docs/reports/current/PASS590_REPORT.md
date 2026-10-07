# PASS590 REPORT — PERSISTENCE-LINEAGE AUTHORITY / VOLATILE FENCE EXECUTION

## Status

PASS590 COMPLETE.

Official continuation UTC start: 13:37:24
Functional freeze: 13:50:16
Useful boundary: 13:57:24
Hard boundary: 14:01:24

## Goal

Finish the PASS589 persistence-lineage theorem as one executable no-fallback law:

```text
DurableCut -> VolatileFence -> DurableCut
```

Complete exact fenced repersistence, remove duplicate runtime persistence-state routing, project the transition into the Rust product surface under `PersistenceTransition`, and close crash/CAS/provider regressions before public exposure.

## Selected architecture

There is one live `DurableRevisionStore` authority. Volatile vs durable is a property of its physical backend, not a second runtime enum state. External freshness has one signed sum grammar `FreshnessAuthorityState::{DurableCut, VolatileFence}` and one exact CAS protocol.

Durable -> Volatile linearizes at the fence CAS before physical demotion. Volatile -> Durable stages and verifies recoverable bytes first, consumes the exact fence second, and swaps the live persistence owner last.

No fallback evaluator, fabricated durable cut, dual persistence owner, per-volatile-commit freshness I/O, logical export/import, or compatibility wire route exists.

## Implemented

### PASS590-A — authority grammar + demotion

- Replaced cut-only signed freshness authority with `FreshnessAuthorityState::{DurableCut, VolatileFence}`.
- One signature/digest domain covers both lineage states.
- Built-in TCP authority moved cleanly to protocol v2 with exact state transitions.
- `DurableRevisionStore::demote_to_volatile()` publishes an external fence first when anchored, preserves protection floor and operational authority, then replaces only backend/WAL ownership.
- Old externally anchored durable bytes fail closed after the fence.
- Volatile commit hot path performs no external freshness I/O.

### PASS590-B — exact fenced repersistence

- Added `repersist_volatile_to_single_file` as the canonical volatile -> durable realization.
- Target bytes are fully staged, reopened and verified before any external authority mutation.
- The existing live `ExternalFreshnessState` owner is moved, not cloned or rebound to a second lineage.
- `VolatileFence -> DurableCut` consumes the exact fence record by CAS only after staging verification.
- CAS response loss is reconciled by exact signed-state reread. Applied response-loss converges to success; proven pre-apply failure leaves the volatile source authoritative and retryable.
- Ambiguous/stale/concurrent successor state poisons the source fail-closed rather than continuing volatile writes after a possibly published durable successor.
- Safe pre-CAS failure removes the staged non-authoritative target.
- Successful repersistence poisons the superseded source owner and returns the unique durable owner.

### PASS590-C — one runtime owner + product projection

- Removed `RuntimePersistenceAuthority::{Volatile, Durable}`. It duplicated backend state and could diverge after in-place demotion.
- `RuntimePersistenceAuthority` is now one wrapper around one `DurableRevisionStore`; `is_volatile()` delegates to the sole backend authority.
- Runtime promotion consumes `repersist_volatile_to_single_file`; runtime demotion calls the in-place kernel transition.
- Added public Rust `Database::make_volatile()` and restricted `AdminDatabase::make_volatile()` under the existing `DatabaseControlPermission::PersistenceTransition` gate.
- Existing `Database::persist()` now naturally repersists a fenced lineage without another route.

## Hostile findings closed

- Duplicate runtime enum state was a real post-demotion split-brain risk and was removed immediately rather than deferred.
- Generic freshness CAS response loss previously forced recovery even when the exact next signed state was already published. Exact reread now makes applied response loss idempotently successful.
- Multiprocess TCP regressions still used retired cut-only API vocabulary; they now exercise the v2 sum-state protocol directly.

## Verification

- Focused volatile-fence repersistence regressions: **3 passed / 0 failed**.
- Runtime Durable -> Volatile -> Durable identity regression: **1 passed / 0 failed**.
- TCP external-freshness multiprocess suite: **3 passed / 0 failed**.
- `cargo test -p kernel-durability --lib --offline`: **278 passed / 0 failed / 2 ignored**.
- `cargo test -p cfmd-runtime --lib --offline`: **30 passed / 0 failed**.
- Strict Clippy over `kernel-durability`, `kernel-plan`, `cfmd-runtime` all targets/no deps: **PASS**.
- `cargo fmt --all -- --check`: PASS.
- `cargo check --workspace --offline`: **PASS**.
- Repository manifest resealed over **5195** tracked paths; repository verifier: **PASS**.
- Final repository ZIP excludes `target/` and `.git`; ZIP integrity verified after packaging.

## CHECKPOINT ROADMAP

### CLOSED

1. **PASS590-A — authority grammar + Durable -> Volatile kernel law.** CLOSED.
2. **PASS590-B — exact `VolatileFence -> DurableCut` repersistence.** CLOSED.
3. **PASS590-C — runtime/product projection + CAS/crash-boundary regressions.** CLOSED.

### NEXT — mandatory cleanup checkpoint

4. **PASS591 — hostile cleanup / consolidation.** STOP FEATURE EXPANSION here. Audit the complete PASS590 line for stale cut-only names, duplicate persistence abstractions, generic/SQL-shaped fallbacks, unnecessary image copying, stale protocol helpers, dead compatibility vocabulary, and performance/resource regressions. Run full workspace gates and reconcile every active ledger before selecting another major feature line.

### AFTER PASS591 ONLY

5. Select the next productization frontier from the reconciled ledgers. Do not open another architectural line before PASS591 is clean.

## LEDGER — CLOSED THIS PASS

- Total externally anchored Durable -> Volatile -> Durable persistence transition.
- Missing exact same-lineage fenced repersistence.
- Applied CAS response-loss ambiguity for exact next state.
- Runtime dual-state persistence routing.
- Public Rust Durable -> Volatile projection under `PersistenceTransition` control.
- TCP multiprocess tests still coupled to retired cut-only API.

## LEDGER — OPEN FOR PASS591

- Hostile search for remaining `SignedFreshnessCut`/cut-only naming or wrappers in active code/docs.
- Measure whether canonical persistence staging copies any authority structures unnecessarily for volatile repersistence; remove avoidable O(history)/O(rows) work if found.
- Audit directory-backend demotion semantics and stale physical bytes as a product/storage-policy question; do not confuse absence of external freshness with rollback certification.
- Full repository/workspace verification and current-frontier ledger reconciliation.
- Re-evaluate whether `RuntimePersistenceAuthority` wrapper itself still adds value now that the backend is the sole persistence-state authority.

## SUPERSEDED / DO NOT REINTRODUCE

- Separate runtime `Volatile`/`Durable` enum as a second persistence-state authority.
- Cut-only external freshness grammar or compatibility decoder.
- Fake durable cuts for volatile state.
- External freshness I/O on every volatile commit.
- Duplicate authority ownership during repersistence.
- Fallback to old durable bytes after a published volatile fence.
