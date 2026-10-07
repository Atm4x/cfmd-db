# PASS595 REPORT — MINIMUM-TRAVERSAL AUTHENTICATED REPLICATION-AUTHORITY PUBLICATION

## Status
PASS595 COMPLETE.

Official UTC start: 16:34:42
Useful boundary: 16:54:42
Hard boundary: 16:58:42

## Goal
Hostile-measure the CPU payer left by PASS594 replayable replication-authority staging. Reduce source regeneration only if the result remains bounded-memory, content-addressed, fail-closed, and uses the same replication evaluator / CFAS-CFAO publication path.

## Hostile finding
PASS594 removed O(total semantic carrier) staging memory, but the source was replayed five times per immutable-object publication:

1. plan length/count validation;
2. plan content hash;
3. pre-write plan recomputation length/count;
4. pre-write plan recomputation content hash;
5. physical write.

For a `Vec<Vec<u8>>` source this is cheap iteration. For `ReplicationAuthoritySemanticSnapshot`, every traversal deterministically re-encodes all retained semantic records. The payer is therefore real and proportional to retained semantic authority.

## Selected architecture
CFAS content identity is now stream-composable:

`frame_digest = SHA256(frame_stream)`

`segment_id = SHA256(domain_v2 || parent || delta_len || frame_count || frame_digest)`

Planning needs one source traversal. The physical writer performs the second traversal, simultaneously emits bytes and recomputes exact frame digest/length/count. Locator/root publication is possible only after the second traversal matches the frozen plan.

CFAS uses segment version/domain v2. No v1 compatibility route is retained: CFMD remains pre-release and current-only format law is authoritative.

## Implemented
- Replaced the two-pass `ReplicationAuthoritySegmentPlan::from_source` with one measurement/hash traversal.
- Removed separate pre-write `validate_source` replay.
- Write path now performs one emission + proof traversal and rejects any digest/count/length drift.
- The writer stops a drifting source before it can emit beyond the frozen plan bounds.
- Single-file authority publication rolls an unpublished object/locator tail back to its original EOF on failure.
- Segment verification/recovery uses the same v2 two-level digest law.
- Added a counted-source regression proving exactly two source traversals for plan + object publication.
- Added a changing-source regression proving the second traversal fails closed.
- Updated the old pre-emission mismatch regression to the stronger current law: a mismatch may write only bytes within the frozen unreachable object plan and cannot progress to locator/root publication.

## Complexity law
Before PASS595:

`5 * O(replayable source generation) + O(physical write)`

After PASS595:

`2 * O(replayable source generation) + O(physical write)`

This removes three complete semantic-record regeneration traversals, a 60% reduction in source traversals.

Two traversals are the selected lower bound for the current physical law: content id and plaintext length must exist before the authenticated object header/AAD is emitted, while source-drift safety requires proving the bytes actually written. A one-traversal design would require buffering/backpatching or changing the publication primitive and is rejected at this branch boundary.

## Verification
- counted frame-source traversal regression: exactly 2 traversals — PASS;
- changed-source post-write proof regression — PASS;
- segment-chain replay / parent-bound identity — PASS;
- `kernel-durability --lib`: 282 passed / 0 failed / 2 ignored;
- `kernel-query --lib`: 166 passed / 0 failed / 10 ignored;
- `kernel-plan --lib`: 316 passed / 0 failed / 11 ignored;
- `cfmd-runtime --lib`: 30 passed / 0 failed;
- strict all-target no-deps Clippy for `kernel-durability`: PASS;
- workspace check: PASS;
- fmt: PASS.

## Branch decision / next checkpoint
PASS592–PASS595 now form a coherent replication semantic-carrier branch:

- PASS592: semantic authority instead of physical append history;
- PASS593: bounded replay memory;
- PASS594: bounded target staging;
- PASS595: minimum-traversal authenticated publication.

This is now a justified consolidation boundary. **PASS596 is selected as a branch-level hostile cleanup / candidate Git-ready checkpoint.** It will not open a new feature line. It will check CFAS-v1 remnants, dead frame-source/staging routes, stale assumptions, performance regressions, and current branch ledgers. If clean, PASS596 becomes the next declared Git-ready checkpoint after PASS591.

Γ-factorized aggregate productionization remains a separate R&D branch and is not activated by PASS595.

## Ledger
Only the active productization frontier/status/changelog were advanced. No global historical-MD sweep was performed.

## Packaging / integrity
Functional source freeze precedes final manifest/repository/package work. Final UTC end and artifact hashes are recorded after package integrity completes.
