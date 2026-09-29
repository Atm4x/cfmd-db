# PASS326 REPORT — Physical segment integration hostile rejection

## Wall clock
- Start: 2026-09-29 06:33:18 UTC
- Functional freeze: 2026-09-29 06:53:18 UTC
- Hard boundary: 06:57:18 UTC

## Goal
Move P325 replication-authority segments into the real SingleFile authority path and remove P323 historical archive copying without weakening AE v1, crash recovery, or compaction.

## Result
The candidate physical integration was **rejected and rolled back before packaging**. It successfully exercised root-bound immutable `CFAS` segments, O(1) linked locator metadata, ordinary/streaming checkpoint segment publication, and segment-chain recovery; the complete `kernel-durability` suite passed 193/193 plus multiprocess tests, and strict Clippy/check passed on the candidate.

Hostile post-implementation review then found a release-blocking architectural violation: the candidate appended `CFAS` segment bytes outside encrypted generation sections. For an encrypted database that would place replication-authority payloads outside the existing AE v1 section/WAL encryption boundary. Passing durability tests does not make that acceptable.

Because the useful window had already frozen, the unsafe candidate was not patched opportunistically. All production source changes from the candidate were rolled back to the verified P325 baseline; this repository therefore preserves the last accepted security semantics rather than shipping a partially encrypted authority format.

## What the rejected prototype proved
- O(new delta) linked locator metadata is sufficient for ordinary checkpoint publication; a full `CFAI` rewrite is unnecessary.
- Root-bound locator `{segment id, locator offset, locator digest}` can reconstruct a relocatable in-memory index and feed the existing P325 two-pass verified replay.
- Streaming checkpoint cuts can publish exactly the frozen live-frame prefix as one child segment instead of copying ancestors.
- P323's monolithic archive source can be deleted once the external segment closure is protected by the same encryption/key hierarchy and compaction can relocate it safely.
- Segment-aware compaction cannot reuse the current "copy active generation + WAL only" implementation; until relocation is implemented it must fail/skip rather than orphan authority.

## Newly discovered required law
External replication-authority objects are storage payloads and MUST remain under AE v1 when encryption is enabled. The next implementation therefore needs a dedicated domain-separated authority-segment codec (or a generalized authenticated object codec) whose AAD binds at least segment identity/parent and object kind while preserving the plaintext `ReplicationAuthoritySegmentId` identity law.

## Next target — P327
Implement encrypted/authenticated immutable authority objects first, then activate the linked-locator root and segment-chain recovery together with segment-aware compaction. Only after encrypted and plaintext stores both pass crash/torn/relocation regressions should P323 archive copying be removed from production.
