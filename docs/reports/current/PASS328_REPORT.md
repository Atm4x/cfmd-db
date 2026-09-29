# PASS328 REPORT — Root-bound authenticated authority segments

## Wall clock
- Start: 2026-09-29 07:28:23 UTC
- Functional freeze: 07:48:23 UTC
- Hard boundary: 07:52:23 UTC

## Goal
Reactivate the physically successful P326 linked-segment architecture on top of P327 authenticated `CFAO` objects, without allowing encrypted authority payloads to escape AE v1 and without deleting the P323 generation-contained authority path before compaction can relocate the new external closure safely.

## Result
P328 is an **accepted safety-stage activation**, not the final removal of the P323 O(history) payer. Ordinary and streaming checkpoints now publish a root-bound external authority delta as:

```text
frozen live replication prefix
        -> CFAO(CFAS)
        -> fixed CFLN linked locator
        -> root {SegmentId, locator offset, locator digest}
```

Opening a bound single-file store reconstructs the locator chain, rebuilds the relocatable segment index, authenticates every P327 object, and feeds the existing P325 two-pass segment replay before the file is accepted. The P323 archive remains the semantic recovery source for this pass, so the new chain is shadow-authoritative physical state whose validity is mandatory but whose removal of historical archive copying is intentionally deferred.

## Delivered

### 1. O(1)-per-delta `CFLN` locator
Added a fixed 160-byte locator node binding segment identity/parent, object offset/length, previous locator offset/digest, and its own domain-separated digest. Recovery walks the linked locator chain, rejects physical cycles and parent/link inconsistency, and constructs the existing relocatable `ReplicationAuthoritySegmentIndex` without rewriting a monolithic index on every checkpoint.

### 2. Root binding without pre-release format churn
`RootRecord` now optionally carries `{segment id, locator offset, locator digest}` in a trailing authenticated-by-digest extension. Existing P327 roots with a zero trailing extension remain readable. The existing root core digest/layout is preserved; no `FORMAT_VERSION` bump was introduced.

### 3. Ordinary and streaming physical activation
After the current WAL is sealed, checkpoint publication appends exactly the frozen live authority prefix as one P327 `CFAO` object and one `CFLN` node. The object/locator bytes are synced before the new generation root is published. Streaming checkpoint uses exactly `replication_cut_frames`, so frames after the frozen cut remain in the carried/live suffix rather than leaking into the segment.

Crash law:

```text
before root switch: old root remains authority; appended bytes are orphan/unreachable
at/after root switch: generation + authority locator root become visible together
```

### 4. Recovery validates the external closure
`SingleFileContainer::open_with_encryption` now validates any bound authority root by recovering the locator chain and replaying the complete CFAO chain through P327 authentication and P325 verify-before-mutate semantics. A file cannot silently ignore a corrupted/tampered external authority closure merely because the P323 archive is still present.

### 5. Encryption/relocation law remains unified
`CFLN` stores physical positions, but P327 object AAD does not bind generation or physical offset. `ReplicationAuthoritySegmentId` remains canonical-plaintext identity. Therefore the next compactor can use one algorithm for plaintext and encrypted stores: copy exact reachable authenticated object bytes, rebuild only physical locator nodes, and atomically publish the relocated root. No decrypt/re-encrypt relocation route is required.

### 6. Compaction is fail-closed for the activated format
The existing compactor knows only “active generation + WAL”. Once a root-bound authority closure exists, reusing it would orphan live authority objects. P328 therefore rejects such compaction explicitly instead of silently producing a corrupt file. This is a temporary hostile safety boundary, not a fallback architecture; P329 must replace it with exact-byte closure relocation before the P323 archive is removed.

## Hostile review / deliberate non-cutover
The tempting final step—delete `ReplicationArchiveSectionSource` now—was rejected. Doing so before segment-aware compaction is implemented would trade P326's plaintext regression for a relocation/liveness regression. P328 therefore keeps duplicate persistence for one pass:

- external `CFAO + CFLN + root binding`: physically active and mandatory-valid;
- P323 generation archive: still semantic recovery authority and still O(history) checkpoint cost.

This means P328 proves the security/atomicity integration but does **not** claim the checkpoint complexity win yet.

No generic fallback or SQL-style alternate evaluator was added. Segment verification/replay remains the single P325 authority calculus, and future compaction is designed as opaque authenticated-object relocation independent of encryption mode.

## Verification
- Rust 1.98.1 minimal toolchain restored from the supplied conversation attachment.
- `kernel-durability --lib`: **201/201 PASS**, including multiprocess/subprocess regressions.
- `kernel-durability` strict Clippy `--all-targets -- -D warnings`: **PASS**.
- workspace `cargo check --workspace --all-targets` with `RUSTFLAGS=-D warnings`: **PASS**.
- workspace strict Clippy `--workspace --all-targets -- -D warnings`: **PASS**.
- `kernel-durability` rustdoc with `RUSTDOCFLAGS=-D warnings`: **PASS**.
- `cargo fmt --all -- --check`: **PASS**.
- repository manifest/layout verifier: run after final manifest regeneration during packaging.

## Next target — P329
Finish the cutover rather than adding another route:

1. traverse the reachable `CFLN -> CFAO` closure from the authoritative root during compaction;
2. copy each CFAO as exact authenticated bytes—never decrypt/re-encrypt;
3. rebuild only locator nodes with relocated object offsets/previous links while preserving every `SegmentId`;
4. sync compacted generation/WAL/object closure, then atomically publish the relocated root;
5. add plaintext + AES-GCM-SIV crash/torn/tamper/relocation regressions around every publication boundary;
6. switch recovery's semantic source to segment chain + live WAL;
7. delete P323 historical archive copying and `ReplicationArchiveSectionSource` only after that matrix is green, making ordinary/streaming checkpoint authority cost O(new delta) rather than O(history).
