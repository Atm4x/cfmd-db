# PASS593 REPORT — BOUNDED-MEMORY REPLICATION SEMANTIC CARRIER

## Status

**PASS593 COMPLETE.**

PASS593 continues the replication-authority semantic-carrier branch after PASS592. It removes the replay-side full encoded semantic-carrier buffer while preserving one replication evaluator, verify-before-install semantics, and the existing authenticated immutable `CFAS/CFAO` realization path.

## Timing correction and continuation boundary

The earlier PASS593 checkpoint was incorrectly labelled a hard-stop checkpoint. A reliable UTC start/end pair had not been recorded for that cycle, and therefore a 24-minute hard-stop violation could not be established. This report supersedes that timing claim; the architectural work in that checkpoint remains valid.

Continuation start: **2026-10-07 15:46:58 UTC**.
Useful boundary: **2026-10-07 16:06:58 UTC**.
Hard boundary: **2026-10-07 16:10:58 UTC**.
Final completion time is recorded after package sealing.

## LEDGER — GOAL

Eliminate the PASS592 replay-side `O(total encoded semantic carrier)` transient-memory payer without introducing:

- a second replication evaluator;
- a history fallback;
- partial installation of unauthenticated semantic authority;
- an incremental positional parser spanning arbitrary byte-chunk boundaries.

Target law:

`semantic records -> incremental digest/decode -> temporary exact snapshot -> verified END -> atomic semantic install`

## LEDGER — SELECTED IMPLEMENTATION

The arbitrary-byte incremental parser was rejected. The selected carrier grammar is record-oriented:

`BEGIN(version, record_count) -> RECORD* -> END(canonical_len, digest)`

Each RECORD is one deterministic semantic-authority item. Replay incrementally hashes canonical `(record_len, record_bytes)` input and decodes directly into a temporary `ReplicationAuthoritySemanticSnapshot`. The live `ReplicationAuthorityJournal` is unchanged until END verifies the exact record count, canonical length, SHA-256 digest, required singleton authority, and duplicate-key invariants.

The existing replication frame `MAX_PAYLOAD_LEN` remains the hard upper bound for one encoded semantic record. No artificial smaller bound is claimed.

## LEDGER — IMPLEMENTED

- Replaced PASS592 arbitrary semantic-base CHUNK accumulation with deterministic semantic RECORD framing.
- Removed `SemanticAuthorityBaseReplay.bytes: Vec<u8>` and the replay-side full-carrier ownership it represented.
- Replay state now retains only expected/observed counts, canonical byte count, incremental SHA-256 state, and the temporary decoded semantic snapshot.
- Added exact duplicate-key and required-singleton validation during record decoding.
- Semantic-base state is installed only after valid END; malformed/truncated/incomplete bases have no semantic effect.
- Record encoder/decoder are structurally decomposed by authority family: causal, membership, election, decision, security, recovery, and publication.
- Preserved the same replication evaluator and authenticated `CFAS/CFAO` physical authority path; PASS593 does not introduce a second authority representation.
- Preserved the PASS592 append-history compression regression and extended observability to record count, canonical length, and maximum record length.
- Added/retained incomplete-before-END atomicity regression coverage.

## Correctness and complexity result

Previous replay transient memory:

`O(decoded semantic authority + total encoded semantic carrier)`

PASS593 replay transient memory:

`O(decoded semantic authority + max semantic record + hash/parser state)`

The semantic snapshot itself remains necessarily `O(retained semantic authority)`. PASS593 removes only the redundant second full encoded-carrier copy.

The implementation does not install partial authority before authentication and does not recover through portable physical journal history.

## Hostile result / next measured payer

PASS593 exposes the next distinct physical payer rather than hiding it behind the replay improvement.

Target staging still retains the complete semantic-base frame sequence in `pending_single_file_frames` / `live_single_file_frames` before ordinary authenticated immutable-object publication. That can still cost `O(total encoded semantic authority)` transient physical staging memory even though replay itself is now bounded per record.

This is the candidate for PASS594 R&D:

`semantic records -> direct/streamed authenticated immutable CFAS/CFAO realization`

The admissible solution must preserve the one-evaluator law and verify/authenticate the same semantic authority. A second evaluator, journal-history fallback, or dual semantic representation is rejected.

## Verification

Final source gates:

- `cargo test -p kernel-durability --lib --offline`: **280 passed / 0 failed / 2 ignored**.
- `cargo test -p kernel-plan --lib --offline`: **316 passed / 0 failed / 11 ignored**.
- `cargo test -p cfmd-runtime --lib --offline`: **30 passed / 0 failed**.
- `cargo check --workspace --offline`: **PASS**.
- `cargo clippy -p kernel-durability -p kernel-plan -p cfmd-runtime --all-targets --no-deps --offline -- -D warnings`: **PASS**.
- `cargo fmt --all -- --check`: **PASS**.
- no Clippy suppression was added for the new carrier code.

Repository sealing:

- archive manifest resealed over **5198 existing tracked paths**: PASS.
- `sha256sum -c --quiet REPOSITORY_MANIFEST.sha256`: PASS.
- `bash scripts/verify-repository.sh`: PASS.
- final repository state has `target/ = 0` and `.git = 0`; ZIP integrity and the exact request-end UTC timestamp are reported in the final handoff after archive hashing, avoiding self-referential timestamp churn inside the hashed archive.

## CLOSED THIS PASS

1. Replay-side `O(total semantic-carrier bytes)` buffering.
2. Partial semantic installation before complete semantic-base authentication.
3. Arbitrary byte-chunk parser pressure for semantic-base restore.
4. Record encoder/decoder monolith debt under strict Clippy.
5. PASS593 final cross-layer acceptance debt from the intermediate checkpoint.

## OPEN — IMMEDIATE

1. **PASS594 — physical semantic-record staging payer.** Determine whether semantic records can lower directly/streamingly into authenticated immutable replication-authority objects with bounded transient staging storage.
2. Preserve one replication evaluator and exact authority semantics while changing only the physical ownership/publication pipeline.
3. Measure peak staging memory and authenticated-object construction cost before selecting the implementation.

## OPEN — DEFERRED / RETURN AFTER CURRENT LINE

- Protocol/Python/.NET projection until the Rust persistence/authority contract settles.
- Native Windows secure-memory hardening and public binary/performance budgets.
- Automatic semantic-fiber retention synthesis until concrete evidence reopens it.

## SUPERSEDED / DO NOT EXTEND

- PASS592 BEGIN/CHUNK/END full-carrier buffering on replay.
- A second incremental positional parser over arbitrary byte chunks.
- Installing semantic-base state before final carrier authentication.
- Treating every PASS as an automatic cleanup/global Markdown audit boundary.

## PERFORMANCE BASELINES TO PRESERVE

- Canonical authority transfer remains `O(retained semantic authority)`, not `O(physical append history)`.
- Replay encoded transient memory remains `O(max semantic record + hash/parser state)`, not `O(total carrier)`.
- Existing replication-frame `MAX_PAYLOAD_LEN` remains the hard per-record bound.
- No external freshness traffic is added to volatile commits.

## NEXT RECOMMENDED PASS

**PASS594 — direct/streamed semantic-record physical lowering.** Attack `pending_single_file_frames` / `live_single_file_frames` accumulation and target bounded authenticated immutable-object staging without a second evaluator or history fallback.

## CHECKPOINT ROADMAP

- Architectural branch: replication-authority semantic carrier / physical realization.
- PASS591: last declared CLEAN / Git-ready checkpoint for the preceding persistence-lineage branch.
- PASS592: semantic authority carrier — CLOSED.
- PASS593: bounded-memory replay — CLOSED.
- PASS594: physical staging accumulation — NEXT.
- Cleanup/Git checkpoint: **not scheduled yet**. Select one only when this branch reaches a coherent consolidation boundary.
