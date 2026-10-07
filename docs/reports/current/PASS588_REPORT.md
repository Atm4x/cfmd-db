# PASS588 REPORT — IMMUTABLE REPLICATION-AUTHORITY CUTOVER SEAL

## Status

PASS588 COMPLETE.

Official UTC request/start: 12:22:10
Authoritative clean-tree implementation start after mandatory reads: 12:27:57
Functional freeze: 12:31:02
Useful boundary: 12:42:10
Hard boundary: 12:46:10

## Goal

Hostile-audit the PASS587 recommendation to activate P328 replication authority physically. Determine the actual current production state before adding another route, fallback, archive representation, or format migration. If the authenticated immutable `CFAS/CFAO + CFLN` design is already the real checkpoint/recovery/compaction authority, seal that cutover and remove any remaining legacy grammar surface that could resurrect P323 monolithic archive copying.

## Hostile finding

PASS587's continuation ledger was stale relative to the current repository.

The production SingleFile path had already progressed beyond the historical PASS328 safety-stage report:

- checkpoint rotation appends only the frozen live replication prefix as a content/parent-bound `CFAS` segment inside authenticated immutable `CFAO` storage;
- one fixed `CFLN` node links the new segment to the already-durable authority root;
- an empty replication delta returns the existing root and writes no new authority object/locator;
- recovery reconstructs the locator chain and replays authenticated segments through the existing `ReplicationAuthorityJournal` evaluator;
- physical compaction traverses the reachable authority closure, copies each `CFAO` as exact stored bytes, rebuilds only physical locators, validates the relocated closure, and publishes the relocated root before reclamation;
- production code no longer constructs a generation `ReplicationAuthority` section.

The remaining defect was therefore not missing physical activation. It was a dead pre-cutover discriminator still present in `SingleFileSectionKind`/decode grammar. That dead kind was a latent second representation: future code could accidentally start emitting the old monolithic section again even though the current writer/recovery path had already converged on immutable authority objects.

## Exact architecture / no-fallback law

PASS588 makes the existing immutable authority design explicit as the sole active checkpoint representation:

```text
active WAL replication suffix
        |
checkpoint cut
        v
new delta frames
        -> CFAS canonical segment
        -> CFAO authenticated immutable object
        -> CFLN physical locator
        -> root {segment id, locator offset, locator digest}
```

If the checkpoint cut contains no new authority frames:

```text
new authority root = previous authority root
```

No archive section, snapshot route, error-driven fallback, SQL-shaped recovery path, or second evaluator exists.

The cost law is:

```text
ordinary checkpoint authority work = O(new authority delta)
explicit physical compaction       = O(reachable retained authority)
recovery                            = O(reachable retained authority + live WAL suffix)
```

P324's negative theorem still matters: exact authority can contain retained causal history, so an exact monolithic snapshot has an Omega(retained history) lower bound in the worst case. Immutable parent-bound segments avoid paying that lower bound on every checkpoint without pretending retained history is discardable.

## Implemented

### 1. Removed dead P323 generation-section grammar

Removed `SingleFileSectionKind::ReplicationAuthority = 4` and its decoder arm.

Raw section kind `4` now fails closed as unsupported instead of remaining an unused compatibility hook. `FORMAT_VERSION` was not changed.

This is pre-release architecture cleanup, not a data migration and not a fallback route.

### 2. Strengthened authority-root regression

Added a test-only observation of the current replication authority binding:

```text
(segment id, locator offset, locator digest)
```

The ordinary SingleFile replication round-trip now performs a second checkpoint rotation with no new replication frames and proves that the exact authority binding is unchanged. This mechanically guards the zero-delta law: checkpoint publication cannot manufacture a fresh authority object/locator merely because another database generation is published.

The streaming checkpoint regression preserves its stricter cut law: authority written after the pinned streaming cut remains in the live WAL suffix, so the newly published checkpoint root has no segment binding yet. The following ordinary checkpoint then segments that suffix exactly once.

### 3. Corrected current architecture/spec authority

Updated:

- `SPEC.md`;
- `README.md`;
- `docs/spec/CFMD_CORE_SPEC.md`;
- `docs/architecture/REPLICATION_AUTHORITY_COMPACTION.md`;
- `docs/status/KERNEL_VERSION_COMPAT_INVENTORY.md`;
- `docs/status/PRODUCTIZATION_LEDGER.md`;
- `CHANGELOG.md`.

Historical PASS309/P323/P328 reports remain historical evidence and were not rewritten.

## Verification

Rust environment:

- Rust 1.98.1 `rustc/cargo/rustfmt/clippy`: PASS.
- Independent random-file smoke compile: PASS.
- Uploaded Rust tar removed only after successful smoke compile; installed toolchain retained.

Frozen gates:

- `cargo test -p kernel-durability --lib --offline`: **273 passed / 0 failed / 2 ignored**.
- strict `cargo clippy -p kernel-durability --all-targets --no-deps --offline -- -D warnings`: **PASS**.
- `cargo check --workspace --offline`: **PASS**.
- `cargo fmt --all -- --check`: **PASS**.
- `bash scripts/verify-repository.sh`: **PASS**.

No new Clippy suppression was added.

## LEDGER — GOAL

Close the stale P328/P323 replication-authority continuation by validating the actual current physical authority path, then remove any residual mechanism that could reintroduce monolithic checkpoint authority as a second representation.

## LEDGER — SELECTED IMPLEMENTATION

Use exactly one representation law:

- live authority enters the shared SingleFile WAL;
- checkpoint cuts append only new authority delta as immutable `CFAS/CFAO`;
- root reachability through `CFLN` is the only checkpoint authority locator;
- recovery always uses the maintained replication-journal evaluator;
- compaction relocates exact authenticated object bytes and rebuilds only locators;
- no new delta means no new authority object;
- legacy generation section kind `4` is unsupported rather than retained as compatibility.

## LEDGER — IMPLEMENTED

- Production immutable-object checkpoint/recovery/compaction path hostile-verified against current source.
- Dead `ReplicationAuthority` generation section variant and decoder arm removed.
- Zero-delta root-reuse regression added.
- Streaming pinned-cut behavior revalidated.
- Current spec/README/architecture/version inventory corrected.
- Productization ledger corrected so successor passes do not attempt to reimplement P328 again.

## CLOSED THIS PASS

- PASS587 stale P328 physical-activation continuation.
- P323 monolithic replication-authority generation archive as an active or fallback representation.
- Dead SingleFile section discriminator capable of reopening that representation.
- Ambiguity over checkpoint complexity: ordinary replication-authority publication is `O(new authority delta)`; reachable-history copying is confined to explicit physical compaction.
- Empty-delta checkpoint ambiguity: exact root binding is retained unchanged.

## OPEN — IMMEDIATE

1. Keep `save_as` absent; `fork_to` names independent-live-database semantics precisely.
2. Durable -> Volatile remains fail-closed until a protection-floor + external-freshness import/declassification theorem exists.
3. Hostile-audit the next independent Rust durability/productization payer rather than reopening P323/P328.

## OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- Protocol/Python/.NET projections after the Rust public contract is deliberately stabilized.
- Semantic-fiber advisor/profile synthesis remains paused.
- Public performance/binary-size budgets.
- Native Windows secure-memory hardening/expansion.
- Remaining final Context/zero-downtime client DX and migration frontend work from the master ledger.

## SUPERSEDED / DO NOT EXTEND

- Generation-contained replication-authority archive as SingleFile checkpoint authority.
- `SingleFileSectionKind::ReplicationAuthority` / raw section kind `4` as an active or compatibility grammar branch.
- Shadow-authoritative dual persistence of immutable segment closure plus monolithic archive.
- Snapshot-at-every-checkpoint replacement: exact retained causal authority has a worst-case history lower bound and must not be rewritten merely to rotate a checkpoint.
- Generic recovery fallback when immutable object authentication/replay fails.

## PERFORMANCE BASELINES TO PRESERVE

- Ordinary checkpoint replication-authority work remains `O(new authority delta)`.
- Empty authority delta writes no `CFAS`, `CFAO`, or `CFLN`; the prior root binding is reused exactly.
- Physical compaction alone may pay `O(reachable retained authority)` and must exact-copy authenticated `CFAO` bytes rather than decrypt/re-encrypt for relocation.
- Recovery uses one replication semantic evaluator and bounded authenticated object/frame replay.
- No whole-archive materialization, no full-history checkpoint rewrite, no second recovery engine.
- PASS581 shared semantic-lane/QCN physical-owner independence from PASS587 remains unchanged.

## NEXT RECOMMENDED PASS

**PASS589 — Durable -> Volatile protection/freshness authority theorem (hostile R&D first).**

Do not expose conversion merely because volatile storage is easy to instantiate. First classify which durable authority is semantic state versus a non-discardable protection/freshness obligation. A valid theorem must explain how persistence protection floor and external anti-rollback freshness are imported, discharged, or explicitly declassified under Schema-owned administration authority without silently weakening guarantees. If no exact law exists, keep Durable -> Volatile absent and move to the next Rust productization payer.

## PROJECT RULES / CONTINUATION CONTRACT

`PROJECT_RULES.md`, the preceding PASS587 report, the current `PRODUCTIZATION_LEDGER` tail, and `POST_PASS400_MASTER_LEDGER` tail were read for the authoritative PASS588 tree before implementation. An initial scratch audit tree was discarded and PASS587 was re-extracted cleanly before the changes recorded by this report.

PASS589 and every successor PASS MUST read `PROJECT_RULES.md` before implementation, read the immediately preceding PASS report and active ledger tails, and carry this same instruction forward again. Every new PASS report MUST retain explicit `LEDGER — GOAL`, `LEDGER — SELECTED IMPLEMENTATION`, `LEDGER — IMPLEMENTED`, and the full continuation buckets required by the project rules.

## Packaging / integrity

Functional source was frozen before final report/manifest/package work. Repository manifest sealed over **5193** tracked entries and `bash scripts/verify-repository.sh` passed. Final repository ZIP excludes `target/` and `.git`. ZIP integrity, package timestamp and SHA-256 are reported with the external PASS588 report artifact after packaging.
