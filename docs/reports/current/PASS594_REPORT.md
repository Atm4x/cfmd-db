# PASS594 REPORT — CQ INTEGRATION + STREAMED REPLICATION-AUTHORITY STAGING

## Status
PASS594 COMPLETE.

Official UTC start: 15:57:21
Useful boundary: 16:17:21
Hard boundary: 16:21:21

## Goal
Hostile-check and, only if clean, integrate the supplied PASS591-based CQ semantic-identity R&D into the current PASS593 tree; then continue the active replication-authority branch by eliminating target-side accumulation of the complete semantic-base physical frame sequence before authenticated immutable-object publication.

## CQ integration verdict
The supplied delta is orthogonal to PASS592/PASS593: every production-touched pre-existing file in `kernel-query` plus `Cargo.lock` was unchanged between PASS591 and PASS593. The merge therefore required no semantic conflict resolution.

Integrated production scope is deliberately narrow:
- proof-checked semantic interning for admitted Set CQ operators (`Scan`, equality filters, `Project`, `JoinEq`);
- structural/durable `RelExpr` identity remains authoritative;
- unsupported, Bag, order-dependent, negation/difference/union, distinct/group/top-k shapes fail closed to the structural path;
- Γ-factorized aggregate code remains R&D/test/example-only.

## Replication staging finding
PASS593 removed replay-side full-carrier buffering, but canonical persistence staging still encoded every semantic-base frame into `pending_single_file_frames` / `live_single_file_frames` before first checkpoint publication. Peak transient encoded memory therefore remained O(total semantic carrier).

## Selected architecture
One replayable physical frame-source contract now feeds the existing authenticated segment/object lowering:

`semantic authority -> ReplicationAuthorityFrameSource -> CFAS segment plan -> CFAO object -> locator root`

Both ordinary captured frame slices and `ReplicationAuthoritySemanticSnapshot` implement the same source law. This is not a second evaluator or format route.

The semantic snapshot source deterministically regenerates the exact former BEGIN/RECORD*/END replication frames one record at a time. Canonical persistence staging installs the semantic state without capturing those physical frames, then supplies the snapshot itself as the first-checkpoint authority source.

## Implemented
- Added `ReplicationAuthorityFrameSource` and ordinary frame-slice adapter.
- Generalized segment identity planning and immutable object writing over replayable sources.
- Added deterministic semantic-base frame streaming from `ReplicationAuthoritySemanticSnapshot`.
- Added in-memory semantic installation without physical-carrier capture.
- Canonical single-file persistence staging, volatile repersistence and external-freshness rebind now publish semantic base directly from the snapshot source.
- Existing live/streaming checkpoint frame batches continue through the same source abstraction.
- Existing semantic-carrier roundtrip regression now proves direct installation retains zero live captured frames and streamed bytes equal the legacy buffered carrier byte-for-byte.

## Complexity law
Before PASS594 staging:
`O(decoded semantic authority + total encoded semantic-base frames)` transient memory.

After PASS594 staging:
`O(decoded semantic authority + max semantic record/frame + CFAO encryption chunk)` transient encoded memory.

The semantic authority itself remains O(retained semantic authority), which is information-theoretically required.

## Verification
- `kernel-query --lib`: 166 passed / 0 failed / 10 ignored.
- `kernel-plan --lib`: 316 passed / 0 failed / 11 ignored.
- `kernel-durability --lib`: 280 passed / 0 failed / 2 ignored.
- `cfmd-runtime --lib`: 30 passed / 0 failed.
- strict all-target no-deps Clippy for kernel-durability/kernel-query/kernel-plan/cfmd-runtime: PASS.
- workspace check: PASS.
- fmt check: PASS.

## Hostile remainder / next target
The memory payer is closed. The remaining measurable cost is repeated deterministic replay of the semantic frame source while computing segment identity, pre-validating immutable-object publication, and writing the object. This preserves fail-before-publication semantics but can encode the semantic record stream multiple times.

Next R&D should determine whether a clean authenticated writer can reduce replay count without buffering the carrier or weakening source-drift detection/publication atomicity. Γ-factorized aggregate productionization remains a separate open branch and must first prove JointMass capability bridging plus hostile skew/self-join/multi-key frontiers.

## Ledger
Current branch goals, selected implementation, completed work, open payers and checkpoint roadmap are appended to `docs/status/PRODUCTIZATION_LEDGER.md`. No global ledger sweep was performed.

## Packaging / integrity
Functional source freeze occurred before final ledger/manifest/package work. Final request end is recorded in the chat after ZIP integrity and hashes are complete.
